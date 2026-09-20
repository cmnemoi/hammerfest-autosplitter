#!/usr/bin/env python3
"""Cherche une chaine courte : donnees du module -> film courant.

`stable_slots.py` a montre que les donnees du module contiennent des pointeurs
dont l'*adresse* est fixe mais dont la *valeur* suit ce que le lecteur recree a
chaque partie. C'est la forme meme d'une racine statique.

Ici on fait se rejoindre les deux bouts, au lieu d'explorer a l'aveugle :

    en avant    depuis chaque pointeur statique, les valeurs atteintes en 1, 2
                puis 3 sauts, en lisant une fenetre autour de chaque objet
    en arriere  les adresses dont le contenu vaut exactement le MovieClip
                courant -- elles sont peu nombreuses

Une jonction a lieu quand une valeur atteinte en avant designe un objet dont la
fenetre contient une de ces adresses : le chemin est alors complet, et se lit
`module+X -> +off1 -> ... -> film`.

Usage:  findchain.py [--depth 3] [--window 0x200]
"""
import argparse
import json
import os
import sys
import time

import hf_state
import hfmap
import ptrscan
import stable_slots

RANKING = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                       os.pardir, ".xrefs.json")


def plausible(v, heaps):
    return v is not None and v > 0x10000 and any(a <= v < b for a, b in heaps)


def window_values(p, addr, window):
    """Les qwords contenus dans [addr, addr+window), avec leur offset."""
    buf = p.read(addr, window)
    if not buf:
        return []
    out = []
    for i in range(0, (len(buf) // 8) * 8, 8):
        out.append((i, int.from_bytes(buf[i:i + 8], "little")))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int)
    ap.add_argument("--depth", type=int, default=3)
    ap.add_argument("--window", type=lambda x: int(x, 0), default=0x200)
    ap.add_argument("--min-xrefs", type=int, default=4,
                    help="ne partir que des statiques que le code lit")
    a = ap.parse_args()

    hf = hf_state.attach(a.pid, verbose=False)
    if hf is None:
        sys.exit("pas de partie resolue : lance une partie Hammerfest.")
    p = hf.p
    heaps = ptrscan.readable_regions(p)

    targets = stable_slots.movie_objects(hf)
    targets["GameMode"] = hf.gm
    print("pid %d   module 0x%x" % (hf.pid, hf.base))
    for name, addr in targets.items():
        print("  cible %-12s 0x%x" % (name, addr))

    # En arriere : qui pointe exactement vers ces objets.
    back = stable_slots.referents(p, targets.values())
    slots = sorted({s for lst in back.values() for s in lst})
    print("\n%d adresse(s) pointent vers une cible" % len(slots))
    if not slots:
        sys.exit("aucun referent : rien a raccorder.")

    def junction(value):
        """`value` designe-t-il un objet dont la fenetre contient un referent ?"""
        for s in slots:
            if value <= s < value + a.window:
                return s
        return None

    statics = stable_slots.module_statics(p, hf.base, hf.end)
    print("%d pointeur(s) statique(s) dans les donnees du module" % len(statics))

    # Filtre par nature plutot que par hasard : un vrai global est lu depuis
    # de nombreux endroits du code, un bucket d'allocateur ne l'est pas. Le
    # classement vient de `xrefs.py --save`.
    if a.min_xrefs and os.path.exists(RANKING):
        with open(RANKING, encoding="utf-8") as f:
            ranking = {int(k): v for k, v in json.load(f).items()}
        statics = {addr: v for addr, v in statics.items()
                   if ranking.get(addr - hf.base, 0) >= a.min_xrefs}
        print("  %d retenue(s) avec au moins %d reference(s) de code"
              % (len(statics), a.min_xrefs))

    # En avant, niveau par niveau. Chaque noeud retient son chemin d'offsets.
    frontier = {v: (addr - hf.base, []) for addr, v in statics.items()}
    seen = set(frontier)

    wanted = set(targets.values())

    def resolves(static_off, path):
        """La chaine mene-t-elle *encore* a une cible, deux fois a distance ?

        Beaucoup de pointeurs des donnees du module sont des tetes de liste ou
        des buckets d'allocateur : leur valeur change en permanence, meme au
        sein d'une partie. Une jonction relevee a un instant donne ne vaut rien
        tant qu'elle ne s'est pas reproduite.
        """
        for attempt in range(2):
            if attempt:
                time.sleep(1.5)
            value = p.u64(hf.base + static_off)
            for off in path:
                if not value:
                    return False
                value = p.u64((value & ~7) + off)
            if ((value or 0) & ~7) not in wanted:
                return False
        return True

    for level in range(a.depth):
        found = []
        for value, (static_off, path) in frontier.items():
            s = junction(value)
            if s is not None:
                found.append((static_off, path, value, s))
        if found:
            print("  niveau %d : %d jonction(s) brute(s), verification..."
                  % (level, len(found)))
            solid = [f for f in found if resolves(f[0], f[1])]
            if solid:
                print("\n  CHAINE STABLE :")
                for static_off, path, value, _s in solid[:10]:
                    chain = " -> ".join("+0x%x" % o for o in path)
                    print("    module+0x%-8x %s  -> 0x%x"
                          % (static_off, chain or "(direct)", value))
                return
            print("    aucune ne se reproduit : pointeurs volatils.")

        nxt = {}
        for value, (static_off, path) in frontier.items():
            for off, v in window_values(p, value, a.window):
                if v in seen or not plausible(v, heaps):
                    continue
                seen.add(v)
                nxt[v] = (static_off, path + [off])
        print("  niveau %d : %d noeuds -> %d" % (level, len(frontier), len(nxt)))
        frontier = nxt
        if not frontier:
            break

    print("\n  aucune jonction a profondeur %d." % a.depth)


if __name__ == "__main__":
    main()
