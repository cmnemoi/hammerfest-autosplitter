#!/usr/bin/env python3
"""Looks for the Hammerfest property names inside a browser running Ruffle.

Windows only. It answers one question, and nothing else: does the anchor
method survive on Ruffle?

The reader anchors on an interned string of the SWF, then on the tables that
cite it. Adobe's AVM1 stores such a string in UTF-16. Ruffle stores it as a
`WString`, one byte per character when every character fits in eight bits
(`wstr/src/ptr.rs`, `WIDE_MASK`), so the six names are plain ASCII there.

Read the counts:

  nothing            Ruffle did not load the game, or the names are built on
                     the fly rather than interned.
  one hit per name   probably the downloaded SWF itself, not a live string.
  several per name   the anchor method survives. Look at the bytes in front:
                     a length of 5 or 6 signs a Ruffle string.

Usage:
    ruffle_probe.py                  every firefox.exe
    ruffle_probe.py --exe chrome.exe
    ruffle_probe.py --pid 1234
"""
import argparse
import subprocess
import sys

import winmem

# The names the reader depends on, from vendor/hf.map.json.
KEYS = {
    "world": "]=[]8",
    "manager": "+(WHk",
    "current": "0]Ss)",
    "fVersion": "5aj8K(",
    "currentId": "-BBEO",
    "gameChrono": "8qkdA",
}


def pids_by_name(exe):
    ps = subprocess.run(
        ["powershell", "-NoProfile", "-Command",
         "Get-CimInstance Win32_Process -Filter \"Name='%s'\" | "
         "ForEach-Object {$_.ProcessId}" % exe],
        capture_output=True, text=True,
    ).stdout
    return [int(x) for x in ps.split() if x.isdigit()]


def probe(pid, show):
    try:
        p = winmem.Proc(pid)
    except OSError as e:
        print("pid %d: %s" % (pid, e))
        return
    # Ruffle's linear memory is private and writable, like the AVM1 heap. We
    # take the mapped ranges too, because that rule was wrong once already.
    regions = p.regions(writable_only=True, private_only=False)
    total = sum(b - a for a, b in regions)
    print("pid %d: %d writable regions, %.0f MiB" % (pid, len(regions), total / (1 << 20)))

    for name, key in KEYS.items():
        for encoding, pattern in (("latin1", key.encode("latin1")),
                                  ("utf16", key.encode("utf-16-le"))):
            hits = p.scan(pattern, regions=regions)
            if not hits:
                continue
            print("   %-10s %-6s %d hits" % (name, encoding, len(hits)))
            for addr in hits[:show]:
                before = p.read(addr - 16, 16)
                print("      %#x  before: %s" % (addr, before.hex(" ") if before else "unreadable"))
    p.close()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--exe", default="firefox.exe", help="process name to search")
    ap.add_argument("--pid", type=int, action="append", help="a process, instead of --exe")
    ap.add_argument("--show", type=int, default=4, help="addresses printed per hit list")
    args = ap.parse_args()

    pids = args.pid or pids_by_name(args.exe)
    if not pids:
        sys.exit("no process found. Is the game open?")
    for pid in pids:
        probe(pid, args.show)


if __name__ == "__main__":
    main()
