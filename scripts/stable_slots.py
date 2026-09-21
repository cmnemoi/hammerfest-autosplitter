#!/usr/bin/env python3
"""Finds the memory slots the Flash player reuses from one game to the next.

`anchors.py` showed that everything AVM1 is built again at every game launch,
down to the interned strings of the SWF. But the player itself survives. So
there must be slots at a **constant address** whose *content* changes to point
at the new movie. Those are exactly the pointers we look for, and we can
isolate them by difference rather than by exploration.

    capture   game 1: record every address that points at the root MovieClip
    verify    game 2: read those same addresses again, and keep only the ones
              that now point at the new MovieClip

What passes this filter belongs to the player, not to the game. The climb up
to the module data then becomes a directed search, instead of a blind one.

Usage:
    stable_slots.py --capture       during the first game
    stable_slots.py --verify        after starting another one (same process)
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
    """The objects that point at the current movie, AS side and native side.

    `GameMode.root` is the ActionScript wrapper of the main MovieClip. The
    qword at +0x10 leads to the matching C++ object, in a region far smaller
    than the MMgc heap.
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
            out[hfmap.clear(name) + ".native"] = native
    return out


def module_statics(p, base, end):
    """The qwords of the module data sections that point into a heap.

    These are the only addresses that are both fixed for the life of the
    process and likely to lead to the player structures. The ones whose value
    is *identical* from one game to the next point at what survives; that is
    where to start again.
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
    """Every address whose content is exactly one of the values."""
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
        sys.exit("choose --capture or --verify")

    hf = hf_state.attach(a.pid, verbose=False)
    if hf is None:
        sys.exit("no game resolved: start a Hammerfest game.")
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
        print("\n%d referent(s) recorded in %s" % (total, STORE))
        for name, addr in objects.items():
            n = len(found.get(addr, []))
            print("  %-12s %d referent(s)" % (name, n))
        print("\nStart another game, without closing EternalTwin, then --verify.")
        return

    with open(STORE, encoding="utf-8") as f:
        old = json.load(f)
    if old["pid"] != hf.pid:
        sys.exit("different process (%d then %d): the player was rebuilt, "
                 "so the comparison means nothing." % (old["pid"], hf.pid))

    new_values = set(objects.values())
    print("\nslots whose content followed the new movie:")
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
        print("  none. The slots themselves are rebuilt.")
    else:
        print("\n%d emplacement(s) stable(s)" % len(stable))

    # The module data: the only memory fixed for the life of the process.
    # What changes there from one game to the next follows something that was
    # rebuilt -- so, possibly, the current movie.
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
    print("\nstatic pointers of the module: %d unchanged, %d changed"
          % (len(same), len(changed)))
    print("  unchanged  -> player structures, which survive across games")
    print("  changed    -> they follow something rebuilt at every game")
    for addr, was, now in changed[:20]:
        print("    module+0x%-8x 0x%x -> 0x%x   %s"
              % (addr - hf.base, was, now, region_of(hf, now)))


def region_of(hf, addr):
    for a, b in hf.p.regions(writable_only=False, private_only=False):
        if a <= addr < b:
            size = (b - a) / 1024
            inside = hf.base <= addr < hf.end
            return "region 0x%x (%.0f Kio)%s" % (a, size,
                                                 "  IN THE MODULE" if inside else "")
    return ""


if __name__ == "__main__":
    main()
