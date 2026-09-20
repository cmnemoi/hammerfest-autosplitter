#!/usr/bin/env python3
"""Quelles adresses survivent a un relancement de partie ?

Le journal de l'autosplitter montre que `GameManager` est recree a chaque
lancement -- six adresses differentes dans un seul process plugin. Tant que
l'ancre est ce GameManager-la, chaque partie coute un balayage complet du tas.

`GameManager.SELF` est un `static var` : il vit sur l'objet de *classe*, cree une
fois avec le SWF. S'il survit aux parties, la chaine

    classe -> SELF -> GameManager -> current -> GameMode

se parcourt en quelques lectures, et le balayage retombe a une fois par process.

Ce script affiche les adresses candidates. Le releve n'a de sens que compare a
lui-meme : lancer une partie, relever, quitter, relancer, relever a nouveau, et
regarder ce qui a bouge.

Usage:  anchors.py [--pid N]
"""
import argparse
import struct
import sys

import avm1
import hf_state
import hfmap
import winmem

K_SELF = hfmap.obf("SELF")
K_CURRENT = hfmap.obf("current")
K_MANAGER = hfmap.obf("manager")


def string_objects(av, text):
    """Tous les objets String valant `text`."""
    out = []
    for buf in av.p.scan(text.encode("utf-16-le"), align=2, regions=av.heaps):
        for ref in av.p.scan(struct.pack("<Q", buf), align=8, regions=av.heaps):
            so = ref - (av.L.str_buf or 8)
            if av.string_at(so) == text:
                out.append(so)
    return out


def tables_owning(av, key):
    """Toutes les tables qui possedent `key`, via l'objet String interne."""
    out = []
    for so in string_objects(av, key):
        for slot in av.p.scan_tagged(so, regions=av.heaps):
            if av.key_at(slot) != key:
                continue
            t = base_from_keyslot(av, slot)
            if t is not None and t not in out:
                out.append(t)
    return out


def base_from_keyslot(av, keyslot):
    """Remonte au debut de la table depuis un slot de clef."""
    first = keyslot
    while av.key_at(first - av.L.tbl_stride) is not None:
        first -= av.L.tbl_stride
    t = first - av.L.tbl_keys
    return t if av.p.u64(t) == av.L.tbl_vt and av.capacity(t) else None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int)
    a = ap.parse_args()

    hf = hf_state.attach(a.pid, verbose=False)
    if hf is None:
        sys.exit("pas de partie resolue : lance une partie Hammerfest.")
    av = hf.av

    print("pid            %d" % hf.pid)
    print("module         0x%x" % hf.base)
    print()

    gm = hf.gm
    manager = av.child(gm, K_MANAGER)
    world = hf.world()
    print("GameMode       0x%x   (recree a chaque partie)" % gm)
    print("world          0x%x" % (world or 0))
    print("GameManager    0x%x   (recree a chaque partie, d'apres les logs)"
          % (manager or 0))

    # L'objet de classe : celui qui possede la statique SELF.
    print("\nrecherche de l'objet de classe (clef SELF = %r)..." % K_SELF)
    classes = tables_owning(av, K_SELF)
    for t in classes:
        self_atom = av.get(t, K_SELF)
        points_to_manager = (
            self_atom is not None and av.table_of(self_atom) == manager
        )
        print("  classe 0x%x   SELF -> 0x%x   %s"
              % (t, (self_atom or 0) & ~7,
                 "== GameManager courant" if points_to_manager else "(autre)"))

    # Les chaines internees du pool de constantes du SWF.
    print("\nchaines internees (survivent tant que le SWF est charge) :")
    for key in (hfmap.obf("world"), hfmap.obf("fVersion"), K_SELF):
        for so in string_objects(av, key):
            print("  String %-8r 0x%x" % (key, so))

    print("\nA comparer avec un second releve apres avoir relance une partie :")
    print("  ce qui ne bouge pas peut servir d'ancre et supprime le balayage.")


if __name__ == "__main__":
    main()
