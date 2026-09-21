#!/usr/bin/env python3
"""Which addresses survive a game restart?

The autosplitter log shows that `GameManager` is built again at every launch
-- six different addresses inside one single plugin process. While the anchor
is that GameManager, every game costs a full heap scan.

`GameManager.SELF` is a `static var`: it lives on the *class* object, created
once with the SWF. If it survives across games, then the chain

    class -> SELF -> GameManager -> current -> GameMode

is walked in a few reads, and the scan falls back to once per process.

This script prints the candidate addresses. The reading only means something
compared to itself: start a game, read, quit, start again, read again, and
look at what moved.

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
    """Every String object whose value is `text`."""
    out = []
    for buf in av.p.scan(text.encode("utf-16-le"), align=2, regions=av.heaps):
        for ref in av.p.scan(struct.pack("<Q", buf), align=8, regions=av.heaps):
            so = ref - (av.L.str_buf or 8)
            if av.string_at(so) == text:
                out.append(so)
    return out


def tables_owning(av, key):
    """Every table that owns `key`, through the interned String object."""
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
    """Walks back to the start of the table from a key slot."""
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
        sys.exit("no game resolved: start a Hammerfest game.")
    av = hf.av

    print("pid            %d" % hf.pid)
    print("module         0x%x" % hf.base)
    print()

    gm = hf.gm
    manager = av.child(gm, K_MANAGER)
    world = hf.world()
    print("GameMode       0x%x   (rebuilt at every game)" % gm)
    print("world          0x%x" % (world or 0))
    print("GameManager    0x%x   (rebuilt at every game, per the logs)"
          % (manager or 0))

    # The class object: the one that owns the SELF static.
    print("\nlooking for the class object (SELF key = %r)..." % K_SELF)
    classes = tables_owning(av, K_SELF)
    for t in classes:
        self_atom = av.get(t, K_SELF)
        points_to_manager = (
            self_atom is not None and av.table_of(self_atom) == manager
        )
        print("  classe 0x%x   SELF -> 0x%x   %s"
              % (t, (self_atom or 0) & ~7,
                 "== GameManager courant" if points_to_manager else "(autre)"))

    # The interned strings of the SWF constant pool.
    print("\ninterned strings (they survive while the SWF is loaded):")
    for key in (hfmap.obf("world"), hfmap.obf("fVersion"), K_SELF):
        for so in string_objects(av, key):
            print("  String %-8r 0x%x" % (key, so))

    print("\nCompare with a second reading after a game restart:")
    print("  what does not move can serve as an anchor and remove the scan.")


if __name__ == "__main__":
    main()
