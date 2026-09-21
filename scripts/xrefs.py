#!/usr/bin/env python3
"""Counts the cross references to the global data of the Flash player.

Why. The blind chain search (`findchain.py`) does not converge: the module
data holds mostly list heads and allocator buckets, which happen to hold movie
objects and change all the time. Trying them at random leads nowhere.

But their *nature* is readable in the code. A real global singleton is read
from many places -- that is what makes it a global. An allocator bucket is
touched by a handful of instructions, and most often through a computed index
rather than direct relative addressing. Counting the instructions that
reference each address therefore sorts the candidates by nature.

The disassembly is a linear sweep: it produces wrong instructions where code
is interleaved with data. That does not matter here. We do not try to rebuild
functions, only to count, and the noise does not gather on one address.

Usage:
    xrefs.py                     ranks the statics recorded by stable_slots
    xrefs.py --rva 0x1e58468     details the references to one address
"""
import argparse
import json
import os
import re
import sys

import capstone
import pefile

HERE = os.path.dirname(os.path.abspath(__file__))
STORE = os.path.join(HERE, os.pardir, ".slots.json")
RANKING = os.path.join(HERE, os.pardir, ".xrefs.json")
DLL = (r"C:\Users\Charles-Meldhine\AppData\Local\Programs\Eternaltwin"
       r"\resources\app\plugins\flash\win32-x64\pepflashplayer.dll")

RIP = re.compile(r"\[rip \+ (0x[0-9a-f]+)\]|\[rip - (0x[0-9a-f]+)\]")


def executable_sections(pe):
    for s in pe.sections:
        # IMAGE_SCN_MEM_EXECUTE
        if s.Characteristics & 0x20000000:
            yield s


def count_rip_targets(path, want=None):
    """-> {target rva: number of instructions that reference it}

    `want` limits the count to a set of rvas, which saves us from keeping a
    dictionary of several million entries.
    """
    pe = pefile.PE(path, fast_load=True)
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    # Without this the sweep stops at the first byte it cannot decode -- and
    # there are many, since the code is interleaved with jump tables and
    # data.
    md.skipdata = True
    counts = {}
    detail = {}
    for section in executable_sections(pe):
        base = section.VirtualAddress
        data = section.get_data()
        print("  %s : %.1f Mio a partir de rva 0x%x"
              % (section.Name.rstrip(b"\x00").decode(), len(data) / (1 << 20), base),
              file=sys.stderr)
        for addr, size, mnemonic, op_str in md.disasm_lite(data, base):
            m = RIP.search(op_str)
            if not m:
                continue
            disp = int(m.group(1), 16) if m.group(1) else -int(m.group(2), 16)
            target = addr + size + disp
            if want is not None and target not in want:
                continue
            counts[target] = counts.get(target, 0) + 1
            if len(detail.setdefault(target, [])) < 6:
                detail[target].append((addr, mnemonic, op_str))
    return counts, detail


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dll", default=DLL)
    ap.add_argument("--rva", type=lambda x: int(x, 0),
                    help="details the references to this rva")
    ap.add_argument("--top", type=int, default=25)
    ap.add_argument("--save", action="store_true",
                    help="writes the ranking to .xrefs.json")
    a = ap.parse_args()

    if a.rva is not None:
        want = {a.rva}
    else:
        if not os.path.exists(STORE):
            sys.exit("no capture: run stable_slots.py --capture first.")
        with open(STORE, encoding="utf-8") as f:
            saved = json.load(f)
        module = saved["module"]
        want = {int(k) - module for k in saved["statics"]}
        print("%d statics recorded, module 0x%x" % (len(want), module))

    print("desassemblage de %s" % os.path.basename(a.dll))
    counts, detail = count_rip_targets(a.dll, want)

    if a.rva is not None:
        print("\nrva 0x%x : %d reference(s)" % (a.rva, counts.get(a.rva, 0)))
        for addr, mnemonic, op_str in detail.get(a.rva, []):
            print("  0x%-8x %s %s" % (addr, mnemonic, op_str))
        return

    ranked = sorted(counts.items(), key=lambda kv: -kv[1])
    print("\n%d statics referenced by code, the first %d:"
          % (len(ranked), a.top))
    print("  a global read everywhere is a candidate; a bucket is not.")
    for rva, n in ranked[:a.top]:
        print("    module+0x%-9x %4d reference(s)" % (rva, n))

    if a.save:
        with open(RANKING, "w", encoding="utf-8") as f:
            json.dump({str(rva): n for rva, n in ranked}, f, indent=1)
        print("\n  ranking written to %s" % RANKING)

    orphans = len(want) - len(ranked)
    print("\n  %d statics are referenced by no instruction in "
          "adressage relatif :" % orphans)
    print("  these are data reached by computation -- arrays, buckets.")


if __name__ == "__main__":
    main()
