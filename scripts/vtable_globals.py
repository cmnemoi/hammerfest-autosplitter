#!/usr/bin/env python3
"""Quels globals les methodes des objets AVM1 consultent-elles ?

`xrefs.py` classait les globals du binaire entier par nombre de references. Les
mieux classes se sont reveles etre l'allocateur et le runtime C++ du lecteur :
normal, ils sont touches partout, mais ils ne menent pas a l'interpreteur.

Ici on retourne la question. On connait trois vtables AVM1, relevees dans la
memoire d'un process vivant :

    String        MODULE+0x1756db8
    ScriptObject  MODULE+0x1749ed8
    table         MODULE+0x174a460
    MovieClip     MODULE+0x1749f48

Leurs entrees sont les methodes de ces objets. Les globals que *ces
fonctions-la* consultent appartiennent au sous-systeme AVM1. Un global partage
par beaucoup d'entre elles est un bon candidat pour le contexte de
l'interpreteur -- c'est-a-dire la racine qu'on cherche.

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

# Relevees a l'execution, module-relatives (voir hammerfest-level-re.md).
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
    """Les rva des fonctions listees dans une vtable."""
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
    """Les globals lus par une fonction, jusqu'a `ret` ou epuisement du budget."""
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
                    help="entrees de vtable examinees")
    ap.add_argument("--budget", type=lambda x: int(x, 0), default=0x400,
                    help="octets desassembles par fonction")
    ap.add_argument("--top", type=int, default=20)
    a = ap.parse_args()

    pe = pefile.PE(a.dll, fast_load=True)
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    md.skipdata = True

    # Les sections de donnees inscriptibles : c'est la que vivent les globals.
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
    print("les plus partages -- un contexte d'interpreteur est vu par "
          "beaucoup de methodes :")
    for g, n in counts.most_common(a.top):
        print("    module+0x%-9x %3d fonction(s)   vtables: %s"
              % (g, n, ",".join(sorted(by_vtable[g]))))


if __name__ == "__main__":
    main()
