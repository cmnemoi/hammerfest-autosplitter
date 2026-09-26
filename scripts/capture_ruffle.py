#!/usr/bin/env python3
"""Records the memory of a live Ruffle as a replayable fixture, on Linux.

The same layout as `capture_heap.py`, so `replay_fixture.py` trims it with no
change:

    fixtures/<name>/
        metadata.json     every address and size, and the state of the game
        heap.bin.gz       the regions, concatenated in address order

One difference with Pepper Flash: the regions include the mappings of the
`ruffle` executable. The reader proves the type of an object by the vtable in
its GC header, and that vtable lives in the module. A capture without the
module could find buckets, and never an object.

Like every capture, it is not atomic: the game runs while we read, so
`state_before` and `state_after` bracket it, and no clock in it is exact.

Usage:
    capture_ruffle.py --name ruffle-main-world
"""
import argparse
import gzip
import json
import os
import pathlib
import sys
import time

import ruffle_state

ROOT = pathlib.Path(__file__).resolve().parent.parent
MIB = 1 << 20


def state_of(heap, ranges):
    """The game as the second opinion reads it, for the fixture's oracle."""
    buckets = heap.buckets_of(ruffle_state.K_WORLD, ranges)
    game_mode = heap.game_mode(heap.owners(buckets, ranges))
    if game_mode is None:
        sys.exit("no GameMode: is a game running, past the black screen?")
    world = heap.get(game_mode, ruffle_state.K_WORLD)
    set_name = heap.get(world, ruffle_state.K_SET_NAME)
    return {
        "level": int(heap.get(world, ruffle_state.K_CURRENT_ID)),
        "set": ruffle_state.WORLDS.get(set_name, set_name),
        "dim": int(heap.get(game_mode, ruffle_state.K_CURRENT_DIM) or 0),
        "game_mode": hex(game_mode.address),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--name", required=True)
    parser.add_argument("--pid", type=int)
    arguments = parser.parse_args()

    process = ruffle_state.Process(arguments.pid or ruffle_state.find_pid())
    heap = ruffle_state.RuffleHeap(process)
    executable = os.readlink(f"/proc/{process.pid}/exe")
    module_ranges = [(start, end) for start, end, permissions, path in process.maps()
                     if path == executable and permissions.startswith("r")]
    heap_ranges = process.heap()

    before = state_of(heap, heap_ranges)
    print(f"before: {before}")

    directory = ROOT / "fixtures" / arguments.name
    directory.mkdir(parents=True, exist_ok=True)
    regions = []
    offset = 0
    started = time.monotonic()
    with gzip.open(directory / "heap.bin.gz", "wb", compresslevel=1) as stream:
        for start, end in sorted(module_ranges + heap_ranges):
            data = process.read(start, end - start)
            if data is None:
                continue
            stream.write(data)
            regions.append({"base": hex(start), "size": end - start, "offset": offset,
                            "module": (start, end) in module_ranges})
            offset += end - start

    after = state_of(heap, heap_ranges)
    print(f"after:  {after}")
    metadata = {
        "player": "ruffle",
        "note": "The game runs during a capture. No clock in it is exact.",
        "plugin": {"base": hex(heap.module[0]), "size": heap.module[1] - heap.module[0]},
        "heap": {"file": "heap.bin.gz", "regions": regions},
        "state_before": before,
        "state_after": after,
    }
    (directory / "metadata.json").write_text(json.dumps(metadata, indent=1), encoding="utf-8")
    print(f"{len(regions)} regions, {offset / MIB:.0f} MiB in {time.monotonic() - started:.1f} s -> {directory}")


if __name__ == "__main__":
    main()
