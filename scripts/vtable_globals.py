#!/usr/bin/env python3
"""Which globals do the methods of AVM1 objects read?

`xrefs.py` ranked the globals of the whole binary by number of references. The
best ranked turned out to be the allocator and the C++ runtime of the player.
That is expected: they are touched everywhere, but they do not lead to the
interpreter.

Here we turn the question round. We know three AVM1 vtables, read from the
memory of a live process:

    String        MODULE+0x1756db8
    ScriptObject  MODULE+0x1749ed8
    table         MODULE+0x174a460
    MovieClip     MODULE+0x1749f48

Their entries are the methods of these objects. The globals that *those
functions* read belong to the AVM1 subsystem. A global shared by many of them
is a good candidate for the interpreter context -- that is, the root we look
for.

Usage:  vtable_globals.py [--entries 48] [--budget 0x400]
"""
import argparse
import os
import re
import sys
from collections import Counter, defaultdict

import capstone
import pefile

DLL = (r"C:\Users\Charles-Meldhine\AppData\Local\Programs\Eternaltwin"
       r"\resources\app\plugins\flash\win32-x64\pepflashplayer.dll")

# Relevees a l'execution, module-relatives (voir reverse-engineering.md).
VTABLES = {
    "String": 0x1756DB8,
    "ScriptObject": 0x1749ED8,
    "table": 0x174A460,
    "MovieClip": 0x1749F48,
}

RIP = re.compile(r"\[rip \+ (0x[0-9a-f]+)\]|\[rip - (0x[0-9a-f]+)\]")


def section_of(pe, rva):
    for s in pe.sections:
        if s.VirtualAddress <= rva < s.VirtualAddress + max(s.Misc_VirtualSize,
                                                            s.SizeOfRawData):
            return s
    return None


def read_rva(pe, rva, size):
    s = section_of(pe, rva)
    if s is None:
        return b""
    off = s.get_PointerToRawData_adj() + (rva - s.VirtualAddress)
    return pe.__data__[off:off + size]


def vtable_functions(pe, vt_rva, count):
    """The rvas of the functions listed in a vtable."""
    base = pe.OPTIONAL_HEADER.ImageBase
    raw = read_rva(pe, vt_rva, count * 8)
    out = []
    for i in range(0, len(raw) - 7, 8):
        va = int.from_bytes(raw[i:i + 8], "little")
        rva = va - base
        if 0 < rva < pe.OPTIONAL_HEADER.SizeOfImage and section_of(pe, rva):
            s = section_of(pe, rva)
            if s.Characteristics & 0x20000000:      # executable
                out.append(rva)
    return out


def globals_used(pe, md, func_rva, budget, data_ranges):
    """The globals a function reads, up to `ret` or until the budget runs out."""
    code = read_rva(pe, func_rva, budget)
    found = set()
    for addr, size, mnemonic, op_str in md.disasm_lite(code, func_rva):
        if mnemonic.startswith("ret"):
            break
        m = RIP.search(op_str)
        if not m:
            continue
        disp = int(m.group(1), 16) if m.group(1) else -int(m.group(2), 16)
        target = addr + size + disp
        if any(a <= target < b for a, b in data_ranges):
            found.add(target)
    return found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dll", default=DLL)
    ap.add_argument("--entries", type=int, default=48,
                    help="vtable entries examined")
    ap.add_argument("--budget", type=lambda x: int(x, 0), default=0x400,
                    help="bytes disassembled per function")
    ap.add_argument("--top", type=int, default=20)
    a = ap.parse_args()

    pe = pefile.PE(a.dll, fast_load=True)
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.skipdata = True

    # The writable data sections: that is where the globals live.
    data_ranges = [(s.VirtualAddress,
                    s.VirtualAddress + max(s.Misc_VirtualSize, s.SizeOfRawData))
                   for s in pe.sections if s.Characteristics & 0x80000000]
    print("sections de donnees : %s"
          % ", ".join("0x%x..0x%x" % r for r in data_ranges))

    counts = Counter()
    by_vtable = defaultdict(set)
    total = 0
    for name, vt in VTABLES.items():
        funcs = vtable_functions(pe, vt, a.entries)
        print("  %-12s vtable module+0x%-9x %d fonction(s)"
              % (name, vt, len(funcs)))
        for rva in funcs:
            total += 1
            for g in globals_used(pe, md, rva, a.budget, data_ranges):
                counts[g] += 1
                by_vtable[g].add(name)

    print("\n%d fonctions examinees, %d globals distincts" % (total, len(counts)))
    print("the most shared -- an interpreter context is seen by "
          "beaucoup de methodes :")
    for g, n in counts.most_common(a.top):
        print("    module+0x%-9x %3d fonction(s)   vtables: %s"
              % (g, n, ",".join(sorted(by_vtable[g]))))


if __name__ == "__main__":
    main()
