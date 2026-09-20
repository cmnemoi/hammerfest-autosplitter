#!/usr/bin/env python3
"""Cherche une chaine de pointeurs depuis les donnees du module jusqu'a un objet.

Pourquoi. Tout ce qui est AVM1 est recree a chaque lancement de partie -- le
GameMode, le GameManager, l'objet de classe, et jusqu'aux chaines internees du
pool de constantes du SWF (mesure par `anchors.py`). Aucune adresse du tas ne
peut donc servir d'ancre d'une partie a l'autre, et c'est ce qui force a
rebalayer cent Mo a chaque fois.

Le module, lui, ne bouge pas de tout le process. Et le lecteur Flash garde
forcement un pointeur vers le contexte qu'il execute, sinon il ne saurait pas
quel film faire tourner. Une chaine

    pepflashplayer.dll + offset -> +offset -> ... -> objet

suivrait donc automatiquement chaque nouvelle instanciation, et rendrait la
resolution aussi directe que celle d'un autosplitter ordinaire.

Methode. Recherche a rebours, niveau par niveau :

  niveau 0   les adresses `a` telles que *(a) == cible
  niveau n   les adresses `b` telles que *(b) tombe dans [a-MAXOFF, a],
             c'est-a-dire qui pointent vers l'objet contenant `a`

On s'arrete des qu'une adresse tombe dans les sections de donnees du module :
c'est une racine statique, et le chemin se lit a l'envers.

Le tri des candidats privilegie ceux qui ne sont *pas* dans le tas AVM1 : les
objets AVM1 se referencent massivement entre eux (le reverse du score avait
compte 2390 referents pour un seul objet), alors que la sortie cherchee passe
par les structures C++ du lecteur.

Usage:  ptrscan.py --target 0x... [--depth 4] [--max-off 0x400]
        ptrscan.py --map            (juste la carte memoire, pour se reperer)
"""
import argparse
import sys

import numpy as np

import winmem

PLUGIN = r"pepflashplayer\.dll"


def readable_regions(p):
    """Toutes les regions engagees et lisibles, pas seulement le tas prive.

    Les structures C++ du lecteur ne vivent pas dans le tas AVM1 : les exclure
    reviendrait a couper la chaine en son milieu.
    """
    return p.regions(writable_only=False, private_only=False)


def module_data(p, base, end):
    """Les sections *ecrivables* du module : `.data`, `.bss`.

    C'est la seule memoire a la fois fixe pour la duree du process et
    susceptible de contenir un pointeur vers le tas.
    """
    return [(a, b) for a, b in p.regions(writable_only=True, private_only=False)
            if base <= a < end]


def chunks(p, regions, chunk=8 << 20):
    """Rend (adresse, tableau d'entiers 64 bits) pour toute la memoire."""
    for a, b in regions:
        start = (a + 7) & ~7
        pos = start
        while pos < b:
            n = min(chunk, b - pos) & ~7
            if n == 0:
                break
            buf = p.read(pos, n)
            if buf and len(buf) >= 8:
                usable = len(buf) & ~7
                yield pos, np.frombuffer(buf[:usable], dtype=np.uint64)
            pos += n


def find_pointers(p, regions, targets, max_off, limit):
    """Adresses dont le contenu tombe dans [t - max_off, t] pour un t cible.

    -> [(adresse, cible, ecart)]
    """
    order = np.sort(np.array(sorted(targets), dtype=np.uint64))
    hits = []
    for base, arr in chunks(p, regions):
        idx = np.searchsorted(order, arr, side="left")
        inside = idx < order.size
        if not inside.any():
            continue
        clamped = np.minimum(idx, order.size - 1)
        cand = order[clamped]
        gap = cand - arr
        hit = inside & (gap <= np.uint64(max_off))
        for i in np.nonzero(hit)[0]:
            hits.append((int(base + i * 8), int(cand[i]), int(gap[i])))
            if len(hits) >= limit:
                return hits
    return hits


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int)
    ap.add_argument("--target", type=lambda x: int(x, 0),
                    help="adresse a atteindre")
    ap.add_argument("--depth", type=int, default=4)
    ap.add_argument("--max-off", type=lambda x: int(x, 0), default=0x100)
    ap.add_argument("--width", type=int, default=64,
                    help="candidats retenus par niveau")
    ap.add_argument("--limit", type=int, default=200_000,
                    help="hits max par niveau, garde-fou")
    ap.add_argument("--map", action="store_true", help="afficher la carte")
    a = ap.parse_args()

    pids = [a.pid] if a.pid else winmem.ppapi_pids()
    if not pids:
        sys.exit("pas de process --type=ppapi : lance une partie.")
    p = winmem.Proc(pids[0])
    base, end, _path = p.module(PLUGIN)
    regions = readable_regions(p)
    data = module_data(p, base, end)

    total = sum(b - x for x, b in regions)
    print("pid            %d" % p.pid)
    print("module         0x%x .. 0x%x" % (base, end))
    print("memoire lisible %d regions, %.0f Mio" % (len(regions), total / (1 << 20)))
    print("donnees module  %d regions, %.0f Kio"
          % (len(data), sum(b - x for x, b in data) / 1024))
    for x, b in data:
        print("    0x%x .. 0x%x   module+0x%x" % (x, b, x - base))
    if a.map or a.target is None:
        return

    def in_module_data(addr):
        return any(x <= addr < b for x, b in data)

    print("\ncible 0x%x" % a.target)
    targets = {a.target}
    chains = {a.target: []}

    for level in range(a.depth):
        hits = find_pointers(p, regions, targets, 0 if level == 0 else a.max_off,
                             a.limit)
        if not hits:
            print("  niveau %d : aucun referent, impasse" % level)
            return
        statics = [h for h in hits if in_module_data(h[0])]
        if statics:
            print("\n  RACINE STATIQUE au niveau %d :" % level)
            for addr, tgt, gap in statics[:10]:
                print("    module+0x%-8x -> ... -> 0x%x  (ecart %d)"
                      % (addr - base, tgt, gap))
            return

        # Un champ de structure est a quelques dizaines d'octets de sa base ;
        # un ecart de plusieurs centaines est presque toujours une coincidence
        # dans un tableau. On explore donc les plus petits ecarts d'abord.
        hits.sort(key=lambda h: (h[2], h[0]))
        kept = hits[: a.width]
        print("  niveau %d : %d referents, %d retenus (ecart 0 : %d)"
              % (level, len(hits), len(kept), sum(1 for h in hits if h[2] == 0)))
        for addr, tgt, gap in kept[:6]:
            print("      0x%x -> 0x%x  (ecart %d)" % (addr, tgt, gap))

        new_chains = {}
        for addr, tgt, gap in kept:
            new_chains[addr] = [gap] + chains.get(tgt, [])
        chains = new_chains
        targets = set(new_chains)

    print("\n  aucune racine statique a profondeur %d." % a.depth)


if __name__ == "__main__":
    main()
