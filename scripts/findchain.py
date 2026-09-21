#!/usr/bin/env python3
"""Looks for a short chain: module data -> current movie.

`stable_slots.py` showed that the module data holds pointers whose *address*
is fixed but whose *value* follows what the player rebuilds at every game.
That is the very shape of a static root.

Here we make both ends meet, instead of exploring blind:

    forward     from each static pointer, the values reached in 1, 2 then 3
                hops, reading a window around each object
    backward    the addresses whose content is exactly the current MovieClip
                -- there are few of them

A junction happens when a value reached forward points at an object whose
window holds one of those addresses. The path is then complete, and it reads
`module+X -> +off1 -> ... -> movie`.

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
    """The qwords held in [addr, addr+window), with their offset."""
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
                    help="start only from the statics the code reads")
    a = ap.parse_args()

    hf = hf_state.attach(a.pid, verbose=False)
    if hf is None:
        sys.exit("no game resolved: start a Hammerfest game.")
    p = hf.p
    heaps = ptrscan.readable_regions(p)

    targets = stable_slots.movie_objects(hf)
    targets["GameMode"] = hf.gm
    print("pid %d   module 0x%x" % (hf.pid, hf.base))
    for name, addr in targets.items():
        print("  target %-12s 0x%x" % (name, addr))

    # Backward: what points exactly at these objects.
    back = stable_slots.referents(p, targets.values())
    slots = sorted({s for lst in back.values() for s in lst})
    print("\n%d address(es) point at a target" % len(slots))
    if not slots:
        sys.exit("no referent: nothing to join.")

    def junction(value):
        """Does `value` point at an object whose window holds a referent?"""
        for s in slots:
            if value <= s < value + a.window:
                return s
        return None

    statics = stable_slots.module_statics(p, hf.base, hf.end)
    print("%d static pointer(s) in the module data" % len(statics))

    # Filter by nature rather than by chance: a real global is read from many
    # places in the code, an allocator bucket is not. The ranking comes from
    # `xrefs.py --save`.
    if a.min_xrefs and os.path.exists(RANKING):
        with open(RANKING, encoding="utf-8") as f:
            ranking = {int(k): v for k, v in json.load(f).items()}
        statics = {addr: v for addr, v in statics.items()
                   if ranking.get(addr - hf.base, 0) >= a.min_xrefs}
        print("  %d kept with at least %d code reference(s)"
              % (len(statics), a.min_xrefs))

    # Forward, level by level. Each node keeps its own path of offsets.
    frontier = {v: (addr - hf.base, []) for addr, v in statics.items()}
    seen = set(frontier)

    wanted = set(targets.values())

    def resolves(static_off, path):
        """Does the chain *still* lead to a target, twice, apart in time?

        Many pointers in the module data are list heads or allocator buckets:
        their value changes all the time, even inside one game. A junction seen
        at one instant is worth nothing until it happens again.
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
            print("  level %d: %d raw junction(s), checking..."
                  % (level, len(found)))
            solid = [f for f in found if resolves(f[0], f[1])]
            if solid:
                print("\n  STABLE CHAIN:")
                for static_off, path, value, _s in solid[:10]:
                    chain = " -> ".join("+0x%x" % o for o in path)
                    print("    module+0x%-8x %s  -> 0x%x"
                          % (static_off, chain or "(direct)", value))
                return
            print("    none happens again: volatile pointers.")

        nxt = {}
        for value, (static_off, path) in frontier.items():
            for off, v in window_values(p, value, a.window):
                if v in seen or not plausible(v, heaps):
                    continue
                seen.add(v)
                nxt[v] = (static_off, path + [off])
        print("  level %d: %d nodes -> %d" % (level, len(frontier), len(nxt)))
        frontier = nxt
        if not frontier:
            break

    print("\n  no junction at depth %d." % a.depth)


if __name__ == "__main__":
    main()
