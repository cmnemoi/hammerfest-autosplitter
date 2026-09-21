#!/usr/bin/env python3
"""Looks for a pointer chain from the module data to an object.

Why. Everything AVM1 is built again at every game launch -- the GameMode, the
GameManager, the class object, and even the interned strings of the SWF
constant pool (measured by `anchors.py`). So no heap address can serve as an
anchor from one game to the next, and that is what forces a new scan of a
hundred MB every time.

The module, on the other hand, does not move for the life of the process. And
the Flash player must hold a pointer to the context it runs, otherwise it
would not know which movie to play. A chain

    pepflashplayer.dll + offset -> +offset -> ... -> object

would therefore follow every new instance by itself, and make the resolution
as direct as that of an ordinary autosplitter.

Method. A backward search, level by level:

  level 0    the addresses `a` where *(a) == target
  level n    the addresses `b` where *(b) falls in [a-MAXOFF, a], that is,
             which point at the object that contains `a`

We stop as soon as an address falls inside the data sections of the module:
that is a static root, and the path reads backwards.

The candidates are sorted to prefer those *outside* the AVM1 heap. AVM1
objects reference each other heavily -- the score reverse counted 2390
referents for a single object -- while the exit we look for goes through the
C++ structures of the player.

Usage:  ptrscan.py --target 0x... [--depth 4] [--max-off 0x400]
        ptrscan.py --map            (just the memory map, to get your bearings)
"""
import argparse
import sys

import numpy as np

import winmem

PLUGIN = r"pepflashplayer\.dll"


def readable_regions(p):
    """Every committed and readable region, not only the private heap.

    The C++ structures of the player do not live in the AVM1 heap. Excluding
    them would cut the chain in the middle.
    """
    return p.regions(writable_only=False, private_only=False)


def module_data(p, base, end):
    """The *writable* sections of the module: `.data`, `.bss`.

    This is the only memory that is both fixed for the life of the process and
    likely to hold a pointer into the heap.
    """
    return [(a, b) for a, b in p.regions(writable_only=True, private_only=False)
            if base <= a < end]


def chunks(p, regions, chunk=8 << 20):
    """Yields (address, array of 64 bit integers) for all the memory."""
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
    """Addresses whose content falls in [t - max_off, t] for a target t.

    -> [(address, target, gap)]
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
                    help="candidates kept per level")
    ap.add_argument("--limit", type=int, default=200_000,
                    help="maximum hits per level, a safety limit")
    ap.add_argument("--map", action="store_true", help="print the memory map")
    a = ap.parse_args()

    pids = [a.pid] if a.pid else winmem.ppapi_pids()
    if not pids:
        sys.exit("no --type=ppapi process: start a game.")
    p = winmem.Proc(pids[0])
    base, end, _path = p.module(PLUGIN)
    regions = readable_regions(p)
    data = module_data(p, base, end)

    total = sum(b - x for x, b in regions)
    print("pid            %d" % p.pid)
    print("module         0x%x .. 0x%x" % (base, end))
    print("readable memory %d regions, %.0f MiB" % (len(regions), total / (1 << 20)))
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
            print("  level %d: no referent, dead end" % level)
            return
        statics = [h for h in hits if in_module_data(h[0])]
        if statics:
            print("\n  STATIC ROOT at level %d:" % level)
            for addr, tgt, gap in statics[:10]:
                print("    module+0x%-8x -> ... -> 0x%x  (ecart %d)"
                      % (addr - base, tgt, gap))
            return

        # A structure field sits a few tens of bytes from its base. A gap of
        # several hundred is almost always a coincidence inside an array. So
        # we explore the smallest gaps first.
        hits.sort(key=lambda h: (h[2], h[0]))
        kept = hits[: a.width]
        print("  level %d: %d referents, %d kept (gap 0: %d)"
              % (level, len(hits), len(kept), sum(1 for h in hits if h[2] == 0)))
        for addr, tgt, gap in kept[:6]:
            print("      0x%x -> 0x%x  (ecart %d)" % (addr, tgt, gap))

        new_chains = {}
        for addr, tgt, gap in kept:
            new_chains[addr] = [gap] + chains.get(tgt, [])
        chains = new_chains
        targets = set(new_chains)

    print("\n  no static root at depth %d." % a.depth)


if __name__ == "__main__":
    main()
