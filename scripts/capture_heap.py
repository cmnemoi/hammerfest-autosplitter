#!/usr/bin/env python3
"""Records the memory of the Flash plugin, or of a Flash projector, as a
replayable fixture. On Windows and on Linux.

One capture serves two purposes at once:

* **documentation** -- real addresses, real objects, real bytes, so the
  diagrams of the low level concepts do not have to be invented;
* **tests and benchmarks** -- the resolution can be replayed off line, with no
  game running and no Flash.

What a fixture must preserve, beyond the bytes: the base address of every
region, its size, the flags Windows reports, and the range of the plugin
module. The scanner steers itself on all of those. A flat dump loses them, and
with them the differential scan, the module range test, and every address in
the object graph.

## Layout

    fixtures/<name>/
        metadata.json     every address, every size, every flag
        heap.bin.gz       the regions, concatenated in address order
        module.bin.gz     the plugin image, for Binary::recognize

One stream rather than one file per region: the heap holds hundreds of
regions, and a single gzip stream shares its dictionary across all of them.
`metadata.json` gives the offset of each region inside the stream, so a reader
still addresses them one by one.

## What a capture is not

**It is not atomic.** The game keeps running while we read, so the last region
is younger than the first by the duration of the capture. The object graph
survives that -- the AVM1 objects do not move inside a game -- but the clocks
do not. `state_before` and `state_after` bracket the smear, and a fixture must
never be used to assert an exact clock value.

Usage:
    capture_heap.py                       one capture in fixtures/<timestamp>
    capture_heap.py --name menus          name it
    capture_heap.py --no-module           skip the plugin image
    capture_heap.py --raw                 no compression, to measure the cost
    capture_heap.py --level 6             smaller, and slower to take
    capture_heap.py --pid 1234            this process: a projector, say
    capture_heap.py --pid 1234 --words 4  the Windows projector, a 32-bit
                                          program, under Wine too
"""
import argparse
import datetime
import gzip
import hashlib
import json
import os
import sys
import time

import hf_state
import platform_memory

# Read size. Large enough that the cost of a call disappears, small enough
# that one unreadable page does not lose much.
CHUNK = 8 << 20
# Second try, when a block fails. One page that has gone away must not cost
# the whole block.
RETRY = 64 << 10
# One progress line per this many bytes.
PROGRESS = 32 << 20
# Compression level.
#
# A capture is not atomic, so the time it takes is the error it carries. Level
# 1 costs a quarter of the time of level 6 and 28 % more bytes, measured over
# 7.7 MiB. Fixtures stay out of git, so the bytes are the cheap side of that
# trade.
FAST = 1

MIB = 1 << 20


def read_region(proc, base, size, holes):
    """Yields the bytes of one region, in address order, with no gap.

    An unreadable block becomes zeros of the same length, and its address is
    appended to `holes`. The length is what matters: byte *i* of the stream is
    the byte at `base + i`, whatever happened while we read.
    """
    pos = base
    end = base + size
    while pos < end:
        want = min(CHUNK, end - pos)
        buf = proc.read(pos, want)
        if buf and len(buf) == want:
            yield buf
            pos += want
            continue
        # Cut it up: the failure usually covers one page, not eight MiB.
        for sub in range(pos, pos + want, RETRY):
            n = min(RETRY, pos + want - sub)
            small = proc.read(sub, n)
            if small and len(small) == n:
                yield small
            else:
                holes.append({"base": hex(sub), "size": n})
                yield bytes(n)
        pos += want


def capture_stream(proc, path, items, raw=False, level=FAST):
    """Writes the items into one stream and returns their index.

    `items` is a list of `(base, size, label)`. The index gives, for each one,
    its offset inside the stream, its digest and its unreadable holes.
    """
    def opener(name, mode):
        if raw:
            return open(name, mode)
        return gzip.open(name, mode, compresslevel=level)

    index = []
    offset = 0
    done = 0
    next_print = PROGRESS
    started = time.time()

    with opener(path, "wb") as out:
        for base, size, label in items:
            digest = hashlib.sha256()
            holes = []
            for buf in read_region(proc, base, size, holes):
                out.write(buf)
                digest.update(buf)
                done += len(buf)
                if done >= next_print:
                    next_print += PROGRESS
                    rate = done / MIB / max(time.time() - started, 1e-6)
                    print("  %6.0f MiB read  %5.0f MiB/s"
                          % (done / MIB, rate), file=sys.stderr)
            index.append({
                "label": label,
                "base": hex(base),
                "size": size,
                "offset": offset,
                "sha256": digest.hexdigest(),
                "holes": holes,
            })
            offset += size
    return index, done


def snapshot(hf):
    """The game state, or None.

    Never fatal: a capture taken in the menus is useful too, and it is the
    only way to record a heap that carries no GameMode at all.
    """
    if hf is None:
        return None
    try:
        state = hf.snapshot()
    except OSError:
        return None
    if state is None:
        return None
    state["game_mode"] = hex(hf.gm)
    return state


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int)
    ap.add_argument("--out", default="fixtures")
    ap.add_argument("--name", help="directory name, default: a timestamp")
    ap.add_argument("--raw", action="store_true",
                    help="no compression, to measure what it saves")
    ap.add_argument("--level", type=int, default=FAST,
                    help="gzip level, 1 to 9. The default keeps the capture "
                         "short, because its duration is its error.")
    ap.add_argument("--no-module", action="store_true",
                    help="skip the plugin image and halve the fixture. Only "
                         "Binary::recognize reads those bytes; the layout "
                         "seed and the module range test need the range "
                         "alone, which metadata.json always carries.")
    ap.add_argument("--words", type=int, choices=(4, 8), default=8,
                    help="the width of a pointer: 4 for the Windows projector")
    args = ap.parse_args()

    pids = [args.pid] if args.pid else platform_memory.flash_pids(hf_state.PLUGIN)
    if not pids:
        sys.exit("no Flash process: is a Flash instance alive?")
    proc = platform_memory.Proc(pids[0])
    module = proc.module(hf_state.PLUGIN)
    if module is None:
        sys.exit("pid %d carries no Flash plugin" % pids[0])
    base, end, plugin_path = module

    name = args.name or datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    directory = os.path.join(args.out, name)
    os.makedirs(directory, exist_ok=True)

    print("pid            %d" % proc.pid, file=sys.stderr)
    print("plugin         %s" % plugin_path, file=sys.stderr)
    print("module         0x%x .. 0x%x  (%.1f MiB)"
          % (base, end, (end - base) / MIB), file=sys.stderr)

    # Resolve before we read, so that the fixture says what it holds. The
    # process is opened read only, so this changes nothing in the game.
    hf = hf_state.attach(proc.pid, verbose=False, word=args.words)
    before = snapshot(hf)

    # A 32-bit program points nowhere past 4 GiB: the rest is Wine's.
    info = [r for r in proc.region_info()
            if args.words == 8 or r["end"] <= hf_state.FOUR_GIB]
    raw_total = sum(r["size"] for r in info)
    print("heap           %d regions, %.1f MiB"
          % (len(info), raw_total / MIB), file=sys.stderr)
    print("", file=sys.stderr)
    print("capturing into %s" % directory, file=sys.stderr)

    suffix = ".bin" if args.raw else ".bin.gz"
    started = time.time()
    heap_index, heap_read = capture_stream(
        proc,
        os.path.join(directory, "heap" + suffix),
        [(r["base"], r["size"], "region-%03d" % i) for i, r in enumerate(info)],
        args.raw,
        args.level,
    )
    module_index = []
    module_read = 0
    if not args.no_module:
        module_index, module_read = capture_stream(
            proc,
            os.path.join(directory, "module" + suffix),
            [(base, end - base, os.path.basename(plugin_path))],
            args.raw,
            args.level,
        )
    elapsed = time.time() - started

    after = snapshot(hf)

    # Put the flags back next to the region they came from: Windows reports
    # its protection, state and type, Linux its permissions. They are
    # recorded, never used to select: the selection already happened, in
    # `region_info`.
    for entry, region in zip(heap_index, info):
        for flag, value in region.items():
            if flag not in ("base", "end", "size"):
                entry[flag] = hex(value) if isinstance(value, int) else value

    stored = sum(
        os.path.getsize(os.path.join(directory, f))
        for f in os.listdir(directory)
        if f.startswith(("heap", "module"))
    )
    unreadable = sum(h["size"] for e in heap_index for h in e["holes"])
    metadata = {
        "tool": "scripts/capture_heap.py",
        "format": 1,
        "captured_at": datetime.datetime.now(datetime.timezone.utc)
                               .replace(microsecond=0).isoformat(),
        "duration_s": round(elapsed, 2),
        "pid": proc.pid,
        "compression": "none" if args.raw else "gzip -%d" % args.level,
        "plugin": {
            "path": plugin_path,
            "base": hex(base),
            "end": hex(end),
            "size": end - base,
            "file": ("module" + suffix) if module_index else None,
        },
        "heap": {"file": "heap" + suffix, "regions": heap_index},
        "totals": {
            "regions": len(heap_index),
            "heap_bytes": heap_read,
            "module_bytes": module_read,
            "stored_bytes": stored,
            "unreadable_bytes": unreadable,
        },
        "state_before": before,
        "state_after": after,
        "note": "Not atomic: the game ran during the capture. The object "
                "graph holds, the clocks do not. Never assert an exact clock "
                "value against this fixture.",
    }
    path = os.path.join(directory, "metadata.json")
    with open(path, "w", encoding="utf-8", newline="\n") as out:
        json.dump(metadata, out, indent=2)
        out.write("\n")

    total_read = heap_read + module_read
    print("")
    print("%d regions, %.1f MiB read in %.1f s"
          % (len(heap_index), total_read / MIB, elapsed))
    print("stored         %.1f MiB  (%.1f %% of the bytes read)"
          % (stored / MIB, 100.0 * stored / max(total_read, 1)))
    # The module is the same binary in every capture and half the storage.
    # Saying so is what makes --no-module an informed choice.
    for part, count in (("heap", heap_read), ("module", module_read)):
        if count:
            name = part + suffix
            print("  %-12s %5.1f MiB of %5.1f MiB read"
                  % (name, os.path.getsize(os.path.join(directory, name)) / MIB,
                     count / MIB))
    if unreadable:
        print("unreadable     %.2f MiB, zero filled" % (unreadable / MIB))
    print("level          %s -> %s"
          % (before["level"] if before else None,
             after["level"] if after else None))
    print("")
    print(path)


if __name__ == "__main__":
    main()
