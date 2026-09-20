#!/usr/bin/env python3
"""Compte les references croisees vers les donnees globales du lecteur Flash.

Pourquoi. La recherche de chaine a l'aveugle (`findchain.py`) ne converge pas :
les donnees du module contiennent surtout des tetes de liste et des buckets
d'allocateur, qui contiennent fortuitement des objets du film et changent en
permanence. Les essayer au hasard ne mene nulle part.

Mais leur *nature* se lit dans le code. Un vrai singleton global est lu depuis
de nombreux endroits -- c'est ce qui en fait un global. Un bucket d'allocateur
est touche par une poignee d'instructions, et le plus souvent par index calcule
plutot qu'en adressage relatif direct. Compter les instructions qui referencent
chaque adresse trie donc les candidats par nature.

Le desassemblage est un balayage lineaire : il produit des instructions fausses
la ou le code est entrecoupe de donnees. Ca n'a pas d'importance ici, on ne
cherche pas a reconstruire des fonctions mais a compter, et le bruit ne se
concentre pas sur une adresse particuliere.

Usage:
    xrefs.py                     classe les statiques releves par stable_slots
    xrefs.py --rva 0x1e58468     detaille les references a une adresse
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
    """-> {rva cible: nombre d'instructions qui la referencent}

    `want` limite le comptage a un ensemble de rva, ce qui evite de garder un
    dictionnaire de plusieurs millions d'entrees.
    """
    pe = pefile.PE(path, fast_load=True)
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_64)
    # Sans cela le balayage s'arrete au premier octet indecodable -- et il y en
    # a beaucoup, le code etant entrecoupe de tables de sauts et de donnees.
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
                    help="detaille les references a cette rva")
    ap.add_argument("--top", type=int, default=25)
    ap.add_argument("--save", action="store_true",
                    help="ecrit le classement dans .xrefs.json")
    a = ap.parse_args()

    if a.rva is not None:
        want = {a.rva}
    else:
        if not os.path.exists(STORE):
            sys.exit("pas de capture : lancer stable_slots.py --capture d'abord.")
        with open(STORE, encoding="utf-8") as f:
            saved = json.load(f)
        module = saved["module"]
        want = {int(k) - module for k in saved["statics"]}
        print("%d statiques relevees, module 0x%x" % (len(want), module))

    print("desassemblage de %s" % os.path.basename(a.dll))
    counts, detail = count_rip_targets(a.dll, want)

    if a.rva is not None:
        print("\nrva 0x%x : %d reference(s)" % (a.rva, counts.get(a.rva, 0)))
        for addr, mnemonic, op_str in detail.get(a.rva, []):
            print("  0x%-8x %s %s" % (addr, mnemonic, op_str))
        return

    ranked = sorted(counts.items(), key=lambda kv: -kv[1])
    print("\n%d statiques referencees par du code, les %d premieres :"
          % (len(ranked), a.top))
    print("  un global lu partout est un candidat ; un bucket ne l'est pas.")
    for rva, n in ranked[:a.top]:
        print("    module+0x%-9x %4d reference(s)" % (rva, n))

    if a.save:
        with open(RANKING, "w", encoding="utf-8") as f:
            json.dump({str(rva): n for rva, n in ranked}, f, indent=1)
        print("\n  classement ecrit dans %s" % RANKING)

    orphans = len(want) - len(ranked)
    print("\n  %d statiques ne sont referencees par aucune instruction en "
          "adressage relatif :" % orphans)
    print("  ce sont des donnees atteintes par calcul -- tableaux, buckets.")


if __name__ == "__main__":
    main()
