#!/usr/bin/env python3
"""Trouve les emplacements memoire que le lecteur Flash reutilise d'une partie
a l'autre.

`anchors.py` a montre que tout ce qui est AVM1 est recree a chaque lancement de
partie, jusqu'aux chaines internees du SWF. Mais le lecteur, lui, survit : il
doit donc bien exister des emplacements **a adresse constante** dont le
*contenu* change pour designer le nouveau film. Ce sont exactement les
pointeurs qu'on cherche, et on peut les isoler par difference plutot que par
exploration.

    capture   partie 1 : noter toutes les adresses qui pointent vers le
              MovieClip racine
    verify    partie 2 : relire ces memes adresses, et ne garder que celles qui
              pointent maintenant vers le nouveau MovieClip

Ce qui passe ce filtre appartient au lecteur, pas au jeu. La remontee jusqu'aux
donnees du module devient alors une recherche dirigee, au lieu d'une
exploration en aveugle.

Usage:
    stable_slots.py --capture       pendant la premiere partie
    stable_slots.py --verify        apres en avoir relance une (meme process)
"""
import argparse
import json
import os
import sys

import hf_state
import hfmap
import ptrscan
import winmem

STORE = os.path.join(os.path.dirname(os.path.abspath(__file__)), os.pardir,
                     ".slots.json")

K_ROOT = hfmap.obf("root")
K_MC = hfmap.obf("mc")


def movie_objects(hf):
    """Les objets qui designent le film courant, cote AS et cote natif.

    `GameMode.root` est l'enveloppe ActionScript du MovieClip principal ; le
    qword a +0x10 mene a l'objet C++ correspondant, dans une region bien plus
    petite que le tas MMgc.
    """
    out = {}
    for name in (K_ROOT, K_MC):
        atom = hf.av.get(hf.gm, name)
        if atom is None:
            continue
        wrapper = atom & ~7
        out[hfmap.clear(name) + ".as"] = wrapper
        native = hf.p.u64(wrapper + 0x10)
        if native:
            out[hfmap.clear(name) + ".natif"] = native
    return out


def module_statics(p, base, end):
    """Les qwords des sections de donnees du module qui pointent vers un tas.

    Ce sont les seules adresses a la fois fixes pour la duree du process et
    susceptibles de mener aux structures du lecteur. Celles dont la valeur est
    *identique* d'une partie a l'autre designent ce qui survit ; c'est de la
    qu'il faut repartir.
    """
    data = ptrscan.module_data(p, base, end)
    heaps = [(a, b) for a, b in ptrscan.readable_regions(p)
             if not (base <= a < end)]

    def in_heap(v):
        return any(a <= v < b for a, b in heaps)

    out = {}
    for start, stop in data:
        buf = p.read(start, stop - start)
        if not buf:
            continue
        for i in range(0, (len(buf) // 8) * 8, 8):
            v = int.from_bytes(buf[i:i + 8], "little")
            if v > 0x10000 and in_heap(v):
                out[start + i] = v
    return out


def referents(p, values):
    """Toutes les adresses dont le contenu vaut exactement une des valeurs."""
    regions = ptrscan.readable_regions(p)
    hits = ptrscan.find_pointers(p, regions, set(values), 0, 2_000_000)
    found = {}
    for addr, target, _gap in hits:
        found.setdefault(target, []).append(addr)
    return found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--capture", action="store_true")
    ap.add_argument("--verify", action="store_true")
    ap.add_argument("--pid", type=int)
    a = ap.parse_args()
    if a.capture == a.verify:
        sys.exit("choisir --capture ou --verify")

    hf = hf_state.attach(a.pid, verbose=False)
    if hf is None:
        sys.exit("pas de partie resolue : lance une partie Hammerfest.")
    objects = movie_objects(hf)

    print("pid            %d" % hf.pid)
    print("module         0x%x" % hf.base)
    for name, addr in objects.items():
        print("  %-12s 0x%x" % (name, addr))

    if a.capture:
        found = referents(hf.p, objects.values())
        statics = module_statics(hf.p, hf.base, hf.end)
        data = {
            "pid": hf.pid,
            "module": hf.base,
            "objects": objects,
            "referents": {str(k): v for k, v in found.items()},
            "statics": {str(k): v for k, v in statics.items()},
        }
        with open(STORE, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=1)
        total = sum(len(v) for v in found.values())
        print("\n%d referent(s) enregistre(s) dans %s" % (total, STORE))
        for name, addr in objects.items():
            n = len(found.get(addr, []))
            print("  %-12s %d referent(s)" % (name, n))
        print("\nRelance une partie, sans fermer EternalTwin, puis --verify.")
        return

    with open(STORE, encoding="utf-8") as f:
        old = json.load(f)
    if old["pid"] != hf.pid:
        sys.exit("process different (%d puis %d) : le lecteur a ete recree, "
                 "la comparaison n'a pas de sens." % (old["pid"], hf.pid))

    new_values = set(objects.values())
    print("\nemplacements dont le contenu a suivi le nouveau film :")
    stable = []
    for target, addrs in old["referents"].items():
        was = int(target)
        for addr in addrs:
            now = hf.p.u64(addr)
            if now in new_values:
                which = [k for k, v in objects.items() if v == now][0]
                print("  0x%x   0x%x -> 0x%x   (%s)  %s"
                      % (addr, was, now, which, region_of(hf, addr)))
                stable.append(addr)
    if not stable:
        print("  aucun. Les emplacements eux-memes sont recrees.")
    else:
        print("\n%d emplacement(s) stable(s)" % len(stable))

    # Les donnees du module : la seule memoire fixe pour la duree du process.
    # Ce qui y change d'une partie a l'autre suit quelque chose de recree --
    # donc, potentiellement, le film courant.
    old_statics = {int(k): v for k, v in old.get("statics", {}).items()}
    if not old_statics:
        return
    same, changed = [], []
    for addr, was in old_statics.items():
        now = hf.p.u64(addr)
        if now == was:
            same.append((addr, was))
        elif now and now > 0x10000:
            changed.append((addr, was, now))
    print("\npointeurs statiques du module : %d inchanges, %d modifies"
          % (len(same), len(changed)))
    print("  inchanges  -> structures du lecteur, qui survivent aux parties")
    print("  modifies   -> suivent quelque chose de recree a chaque partie")
    for addr, was, now in changed[:20]:
        print("    module+0x%-8x 0x%x -> 0x%x   %s"
              % (addr - hf.base, was, now, region_of(hf, now)))


def region_of(hf, addr):
    for a, b in hf.p.regions(writable_only=False, private_only=False):
        if a <= addr < b:
            size = (b - a) / 1024
            inside = hf.base <= addr < hf.end
            return "region 0x%x (%.0f Kio)%s" % (a, size,
                                                 "  DANS LE MODULE" if inside else "")
    return ""


if __name__ == "__main__":
    main()
