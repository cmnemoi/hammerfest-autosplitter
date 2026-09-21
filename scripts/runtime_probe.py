"""Measures the cache of the LiveSplit DLL, in a timer private to this test.

Usage : mise run probe-runtime [--dll CHEMIN_ASR_CAPI_DLL]
The DLL and LiveSplit are not modified. The allocations belong to this script.
"""
import argparse
import ctypes as C
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


class Runtime:
    """The C interface of the ASR component, with a fake timer and local callbacks."""
    def __init__(self, path):
        self.dll = C.CDLL(str(path))
        self.runtime = None
        self.values = {}
        self.messages = []
        self.timer_state = 0
        ptr = C.c_void_p
        state = C.CFUNCTYPE(C.c_int32)
        segment = C.CFUNCTYPE(C.c_int32, C.c_int32)
        action = C.CFUNCTYPE(None)
        game_time = C.CFUNCTYPE(None, C.c_int64)
        variable = C.CFUNCTYPE(None, ptr, C.c_size_t, ptr, C.c_size_t)
        log = C.CFUNCTYPE(None, ptr, C.c_size_t)
        def set_variable(k, nk, v, nv):
            self.values[C.string_at(k, nk).decode()] = C.string_at(v, nv).decode()
        def log_message(p, n):
            self.messages.append(C.string_at(p, n).decode(errors="replace"))
        def start():
            self.timer_state = 1
        def reset():
            self.timer_state = 0
        self.callbacks = [state(lambda: self.timer_state),
                          state(lambda: 0 if self.timer_state else -1), segment(lambda _: -1),
                          action(start), action(lambda: None), action(lambda: None),
                          action(lambda: None), action(reset),
                          game_time(lambda _: None), action(lambda: None),
                          action(lambda: None), variable(set_variable), log(log_message)]
        self.dll.Runtime_new.argtypes = [C.c_char_p, ptr, state, state, segment,
                                        action, action, action, action, action,
                                        game_time, action, action, variable, log]
        self.dll.Runtime_new.restype = ptr
        self.dll.Runtime_step.argtypes = [ptr]
        self.dll.Runtime_step.restype = C.c_bool
        self.dll.Runtime_tick_rate.argtypes = [ptr]
        self.dll.Runtime_tick_rate.restype = C.c_uint64
        self.dll.Runtime_drop.argtypes = [ptr]
        self.dll.Runtime_drop.restype = None
        self.next_tick = 0

    def load(self, wasm):
        self.runtime = self.dll.Runtime_new(str(wasm).encode(), None, *self.callbacks)
        if not self.runtime:
            raise RuntimeError("\n".join(self.messages))
        self.next_tick = time.perf_counter()

    def step(self):
        time.sleep(max(0, self.next_tick - time.perf_counter()))
        if not self.dll.Runtime_step(self.runtime):
            raise RuntimeError("\n".join(self.messages[-10:]))
        self.next_tick += self.dll.Runtime_tick_rate(self.runtime) / 10_000_000

    def run_for(self, duration):
        deadline = time.perf_counter() + duration
        while time.perf_counter() < deadline:
            self.step()

    def wait(self, key, value, timeout=5):
        deadline = time.perf_counter() + timeout
        while time.perf_counter() < deadline:
            self.step()
            if self.values.get(key) == str(value):
                return self.values.copy()
        raise TimeoutError(f"Attente de {key}={value}; variables={self.values}")

    def close(self):
        if self.runtime:
            self.dll.Runtime_drop(self.runtime)
            self.runtime = None


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--dll", type=Path)
    ap.add_argument("--out", type=Path, default=Path("logs/runtime-probe.json"))
    args = ap.parse_args()
    if args.dll is None:
        command = ("(Get-Process LiveSplit -ErrorAction Stop).Modules | "
                   "Where-Object { $_.ModuleName -eq 'asr_capi.dll' } | "
                   "Select-Object -ExpandProperty FileName -Unique")
        result = subprocess.run(["powershell", "-NoProfile", "-Command", command],
                                capture_output=True, text=True, check=True)
        paths = result.stdout.strip().splitlines()
        if len(paths) != 1:
            ap.error("Open LiveSplit with the ASR component, or pass --dll.")
        args.dll = Path(paths[0])
    dll = args.dll.resolve(strict=True)
    root = Path(__file__).resolve().parents[1]
    out = root / "target" / "runtime-probe"
    out.mkdir(parents=True, exist_ok=True)
    control = (C.c_uint64 * 2)()
    source = (root / "scripts/cache_probe.rs").read_text(encoding="utf-8")
    source = source.replace("__PROBE_PID__", str(os.getpid()))
    source = source.replace("__PROBE_CONTROL__", str(C.addressof(control)))
    rust = out / "cache_probe.rs"
    wasm = out / "cache_probe.wasm"
    rust.write_text(source, encoding="utf-8")
    subprocess.run(["mise", "exec", "--", "rustc", str(rust), "--edition=2021",
                    "--crate-type=cdylib", "--target=wasm32-unknown-unknown",
                    "-C", "opt-level=2", "-C", "panic=abort", "-o", str(wasm)],
                   cwd=root, check=True)

    k = C.WinDLL("kernel32", use_last_error=True)
    k.VirtualAlloc.argtypes = [C.c_void_p, C.c_size_t, C.c_ulong, C.c_ulong]
    k.VirtualAlloc.restype = C.c_void_p
    k.VirtualFree.argtypes = [C.c_void_p, C.c_size_t, C.c_ulong]
    k.VirtualFree.restype = C.c_int
    report = {"dll": str(dll), "sha256": hashlib.sha256(dll.read_bytes()).hexdigest(),
              "trials": []}
    runtime = Runtime(dll)
    try:
        runtime.load(wasm)
        for trial, delay in enumerate((.1, .35, .65, .9), 1):
            page = k.VirtualAlloc(None, 65536, 0x2000, 0x01)  # reserve, no access
            if not page:
                raise C.WinError(C.get_last_error())
            try:
                control[1] = page
                control[0] = trial
                runtime.wait("ready", trial)
                runtime.run_for(delay)
                if k.VirtualAlloc(page, 4096, 0x1000, 0x04) != page:
                    raise C.WinError(C.get_last_error())
                C.c_ubyte.from_address(page).value = 0x5a
                values = runtime.wait("done", trial)
                row = {"trial": trial, "commit_delay_ms": delay * 1000,
                       **{key: int(values[key]) / 1000
                          for key in ("direct_us", "cached_us", "fresh_us")}}
                row = {key.replace("_us", "_ms"): value for key, value in row.items()}
                row["cached_minus_direct_ms"] = row["cached_ms"] - row["direct_ms"]
                row["fresh_minus_direct_ms"] = row["fresh_ms"] - row["direct_ms"]
                report["trials"].append(row)
                print(json.dumps(row), flush=True)
            finally:
                k.VirtualFree(page, 0, 0x8000)
    finally:
        runtime.close()
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"Rapport : {args.out}")


if __name__ == "__main__":
    main()
