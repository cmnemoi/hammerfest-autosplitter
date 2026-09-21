"""Resume les departs normaux ou HF_DIAG (delai de detection, pas delai visuel)."""
import argparse
import math
from pathlib import Path
import re
import statistics


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("logs", type=Path, nargs="+", help="journaux exportes")
    args = ap.parse_args()
    groups = {"false": [], "true": [], "standard": []}
    diagnostic_lines = 0
    origins = 0
    contexts = {"Premier depart du module": [], "Premier depart du processus suivant": [], "Relances": []}
    for path in args.logs:
        try:
            data = path.read_bytes()
            text = data.decode("utf-16" if data.startswith((b"\xff\xfe", b"\xfe\xff")) else "utf-8-sig")
        except (OSError, UnicodeError) as error:
            ap.exit(1, f"Lecture impossible : {path} : {error}\n")
        if not text.strip():
            ap.exit(1, f"Journal vide : {path} ({len(data)} octets).\n"
                    "Exporter le panneau Logs avec Save dans asr-debugger, "
                    "puis utiliser le chemin du fichier exporte.\n")
        pending = None
        for line in text.splitlines():
            diagnostic_lines += "HF_DIAG " in line
            if "HF_START " in line:
                fields = dict(re.findall(r"(\w+)=(\S+)", line))
                ms = int(fields["elapsed_ms"])
                if ms >= 0:
                    label = ("Premier depart du module" if fields.get("first_module") == "true"
                             else "Premier depart du processus suivant" if fields.get("first_process") == "true"
                             else "Relances")
                    contexts[label].append(ms)
            normal = re.search(r"Hammerfest: depart date, (-?\d+) ms deja ecoulees", line)
            if normal:
                pending = int(normal[1])
                continue
            if "Hammerfest: partie lancee" in line:
                if pending is not None:
                    groups["standard"].append(pending)
                pending = None
                continue
            if "Hammerfest: autosplitter demarre" in line or "Run reset." in line:
                pending = None
            if "HF_DIAG event=origin " not in line:
                continue
            # Le diagnostic emet aussi la ligne ordinaire juste avant :
            # ne compter ce depart qu'une fois, dans son groupe A ou B.
            pending = None
            origins += 1
            fields = dict(re.findall(r"(\w+)=(\S+)", line))
            if fields.get("start") == "true" and fields.get("fresh") in ("false", "true"):
                groups[fields["fresh"]].append(int(fields["elapsed_ms"]))
    if not any(groups.values()):
        if diagnostic_lines == 0:
            reason = ("Aucun depart mesure. Il faut une ligne 'depart date' suivie "
                      "de 'partie lancee', ou une origine HF_DIAG avec start=true.")
        elif origins == 0:
            reason = "Traces HF_DIAG presentes, mais aucune origine de partie detectee."
        else:
            reason = (f"{origins} origine(s) detectee(s), mais aucun depart automatique "
                      "avec un mode A/B reconnu. Verifier les champs start et fresh.")
        ap.exit(1, reason + "\n")
    print("Delai reconstruit a la premiere lecture (ms). Ce n'est pas le delai visuel.")
    print("Mode        N    zeros    mediane      P95      max")
    for mode, values in groups.items():
        label = {"false": "A normal", "true": "B 100 ms", "standard": "Sans A/B"}[mode]
        if mode == "standard" and not values:
            continue
        if mode != "standard" and not diagnostic_lines and not values:
            continue
        if not values:
            print(f"{label:10}  0")
            continue
        values.sort()
        p95 = values[math.ceil(.95 * len(values)) - 1]
        print(f"{label:10} {len(values):3} {values.count(0):8} "
              f"{statistics.median(values):10.1f} {p95:8} {values[-1]:8}")
    if groups["standard"]:
        print("Sans A/B : departs du journal ordinaire; le mode de cache n'y est pas indique.")
    if any(contexts.values()):
        print("\nContexte des departs mesures (ms) :")
        for label, values in contexts.items():
            if values:
                print(f"{label} : N={len(values)}, max={max(values)}, "
                      f">100ms={sum(v > 100 for v in values)}, >500ms={sum(v > 500 for v in values)}")
    else:
        print("Contexte non enregistre : impossible de classer automatiquement premiers departs et relances.")
    print("Avec moins de 100 departs par contexte, le P99 empirique correspond au maximum observe ; il reste peu documente.")
    if len(args.logs) > 1:
        print("Attention : les fichiers sont additionnes. Ne pas fournir deux exports de la meme session.")


if __name__ == "__main__":
    main()
