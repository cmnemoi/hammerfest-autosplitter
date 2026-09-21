"""Capture plusieurs departs avec un timer prive et sans export manuel."""
import argparse
from pathlib import Path
from datetime import datetime
import json
import re
import time

from runtime_probe import Runtime


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--dll", type=Path, required=True)
    ap.add_argument("--wasm", type=Path, default=Path(
        "target/diagnostics/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm"))
    ap.add_argument("--out", type=Path, help="nouveau fichier, sans ecraser un ancien journal")
    ap.add_argument("--seconds", type=float, default=900)
    ap.add_argument("--runs", type=int, default=25)
    args = ap.parse_args()
    if args.seconds <= 0 or args.runs <= 0:
        ap.error("seconds et runs doivent etre positifs")
    if args.out is None:
        args.out = Path("logs") / datetime.now().strftime("relances-%Y%m%d-%H%M%S-%f.txt")
    args.out.parent.mkdir(parents=True, exist_ok=True)
    status_path = args.out.parent / "capture-status.json"
    status = {"state": "preparing", "log": str(args.out.resolve()), "runs": 0,
              "target_runs": args.runs, "late_runs": 0, "last_delay_ms": None}

    def save_status():
        status["updated_at"] = datetime.now().isoformat(timespec="seconds")
        temporary = status_path.with_suffix(".tmp")
        temporary.write_text(json.dumps(status, indent=2) + "\n", encoding="utf-8")
        temporary.replace(status_path)

    runtime = Runtime(args.dll.resolve(strict=True))
    pending_delay = None
    save_status()
    try:
        runtime.load(args.wasm.resolve(strict=True))
        with args.out.open("x", encoding="utf-8") as stream:
            stream.write(f"HF_CAPTURE runs={args.runs} seconds={args.seconds} fresh=true\n")
            stream.flush()
            status["state"] = "running"
            save_status()
            print(f"CAPTURE PRETE : {args.out} ({args.runs} departs ou {args.seconds:g} secondes)", flush=True)
            deadline = time.perf_counter() + args.seconds
            next_status = time.perf_counter() + 5
            while time.perf_counter() < deadline:
                runtime.step()
                new_start = False
                for message in runtime.messages:
                    stream.write(message + "\n")
                    if "HF_START " in message:
                        match = re.search(r"elapsed_ms=(-?\d+)", message)
                        pending_delay = int(match[1]) if match else None
                    if "event=start_called" in message:
                        status["runs"] += 1
                        status["last_delay_ms"] = pending_delay
                        if pending_delay is not None:
                            status["late_runs"] += pending_delay > 100
                        print(f"Depart {status['runs']}/{args.runs} : {pending_delay} ms", flush=True)
                        pending_delay = None
                        new_start = True
                runtime.messages.clear()
                stream.flush()
                if new_start or time.perf_counter() >= next_status:
                    save_status()
                    next_status = time.perf_counter() + 5
                if status["runs"] >= args.runs:
                    break
            status.update(state="finished", reason="runs" if status["runs"] >= args.runs else "timeout")
    except KeyboardInterrupt:
        status.update(state="finished", reason="interrupted")
    except Exception as error:
        status.update(state="failed", error=str(error))
        raise
    finally:
        runtime.close()
        save_status()
    print(f"CAPTURE TERMINEE : {status['runs']} departs, {status['late_runs']} au-dessus de 100 ms. {args.out}", flush=True)
    if not status["runs"]:
        ap.exit(1, "Aucun depart automatique capture pendant cette periode.\n")


if __name__ == "__main__":
    main()
