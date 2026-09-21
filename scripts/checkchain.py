#!/usr/bin/env python3
"""Checks that a chain `module+X -> +off -> ...` really leads to the current movie.

A chain found once is worth nothing: everything AVM1 is built again at every
game. So the only valid test is to walk it again **after a game restart** and
compare with an independent resolution.

Usage:
    checkchain.py 0x1e58468 0xa8 0x178
"""
import argparse
import sys

import hf_state
import stable_slots


def follow(p, base, static_off, offsets):
    """Walks the chain and returns every step, so it can be read."""
    steps = []
    addr = base + static_off
    value = p.u64(addr)
    steps.append(("module+0x%x" % static_off, addr, value))
    for off in offsets:
        if not value:
            break
        addr = (value & ~7) + off
        nxt = p.u64(addr)
        steps.append(("+0x%x" % off, addr, nxt))
        value = nxt
    return steps


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("static", type=lambda x: int(x, 0),
                    help="offset inside the module, e.g. 0x1e58468")
    ap.add_argument("offsets", nargs="+", type=lambda x: int(x, 0))
    ap.add_argument("--pid", type=int)
    a = ap.parse_args()

    hf = hf_state.attach(a.pid, verbose=False)
    if hf is None:
        sys.exit("no game resolved: start a Hammerfest game.")

    targets = stable_slots.movie_objects(hf)
    targets["GameMode"] = hf.gm

    print("pid %d   module 0x%x" % (hf.pid, hf.base))
    print("\nindependent resolution (by scan):")
    for name, addr in targets.items():
        print("  %-12s 0x%x" % (name, addr))

    steps = follow(hf.p, hf.base, a.static, a.offsets)
    print("\nchaine :")
    for label, addr, value in steps:
        print("  %-14s [0x%x] = 0x%x" % (label, addr, value or 0))

    final = (steps[-1][2] or 0) & ~7
    match = [n for n, v in targets.items() if v == final]
    print()
    if match:
        print("  MATCH: the chain leads to %s" % ", ".join(match))
    else:
        print("  matches no target (got 0x%x)" % final)


if __name__ == "__main__":
    main()
