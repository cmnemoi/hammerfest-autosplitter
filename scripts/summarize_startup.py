"""Summarises normal or HF_DIAG starts (detection delay, not visual delay)."""
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
    contexts = {"First start of the module": [], "First start of the next process": [], "Later starts": []}
    for path in args.logs:
        try:
            data = path.read_bytes()
            text = data.decode("utf-16" if data.startswith((b"\xff\xfe", b"\xfe\xff")) else "utf-8-sig")
        except (OSError, UnicodeError) as error:
            ap.exit(1, f"Lecture impossible : {path} : {error}\n")
        if not text.strip():
            ap.exit(1, f"Empty log: {path} ({len(data)} bytes).\n"
                    "Export the Logs panel with Save in asr-debugger, "
                    "then use the path of the exported file.\n")
        pending = None
        for line in text.splitlines():
            diagnostic_lines += "HF_DIAG " in line
            if "HF_START " in line:
                fields = dict(re.findall(r"(\w+)=(\S+)", line))
                ms = int(fields["elapsed_ms"])
                if ms >= 0:
                    label = ("First start of the module" if fields.get("first_module") == "true"
                             else "First start of the next process" if fields.get("first_process") == "true"
                             else "Later starts")
                    contexts[label].append(ms)
            normal = re.search(r"Hammerfest: start dated, (-?\d+) ms already elapsed", line)
            if normal:
                pending = int(normal[1])
                continue
            if "Hammerfest: game started" in line:
                if pending is not None:
                    groups["standard"].append(pending)
                pending = None
                continue
            if "Hammerfest: autosplitter demarre" in line or "Run reset." in line:
                pending = None
            if "HF_DIAG event=origin " not in line:
                continue
            # The diagnostics build also prints the ordinary line just
            # before, so count this start once only, in its own group.
            pending = None
            origins += 1
            fields = dict(re.findall(r"(\w+)=(\S+)", line))
            if fields.get("start") == "true" and fields.get("fresh") in ("false", "true"):
                groups[fields["fresh"]].append(int(fields["elapsed_ms"]))
    if not any(groups.values()):
        if diagnostic_lines == 0:
            reason = ("No start measured. A 'start dated' line followed by "
                      "'game started' is needed, or an HF_DIAG origin with "
                      "start=true.")
        elif origins == 0:
            reason = "HF_DIAG traces present, but no game origin found."
        else:
            reason = (f"{origins} origin(s) found, but no automatic start "
                      "with a known mode. Check the start and fresh fields.")
        ap.exit(1, reason + "\n")
    print("Delay rebuilt at the first read (ms). This is not the visual delay.")
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
        print("Starts from the ordinary log; the cache mode is not given there.")
    if any(contexts.values()):
        print("\nContext of the measured starts (ms):")
        for label, values in contexts.items():
            if values:
                print(f"{label} : N={len(values)}, max={max(values)}, "
                      f">100ms={sum(v > 100 for v in values)}, >500ms={sum(v > 500 for v in values)}")
    else:
        print("Contexte non enregistre : impossible de classer automatiquement premiers departs et relances.")
    print("With fewer than 100 starts per context, the empirical P99 equals the maximum observed; it stays weakly supported.")
    if len(args.logs) > 1:
        print("Warning: the files are added together. Do not give two exports of the same session.")


if __name__ == "__main__":
    main()
