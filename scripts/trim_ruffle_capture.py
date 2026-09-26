#!/usr/bin/env python3
"""Turns a Ruffle capture into a fixture small enough for git.

A capture of Ruffle is three hundred MiB, and the game needs about a hundred
and fifty pages of it. So this tool keeps pages, not regions:

1. replay the reading of the game on the capture, and note every page it
   reads outside the two sweeps;
2. check that the search still finds the same game, with the same state, when
   every other page reads as zeros;
3. write the pages kept, in the format `src/reader/tests/reader/replay.rs`
   reads. A region is cut into runs: the runs kept carry bytes, the others
   are `-` and read as zeros.

The regions stay in the map, at their address and their size, so the search
sweeps the same ranges it would in the live process.

Usage:
    trim_ruffle_capture.py ruffle-main-world
        reads fixtures/ruffle-main-world, writes fixtures/replay/ruffle-main-world
"""
import argparse
import bisect
import gzip
import json
import pathlib
import sys

import ruffle_state

ROOT = pathlib.Path(__file__).resolve().parent.parent
PAGE = 4096
KIB = 1 << 10


class Capture(ruffle_state.Process):
    """A capture served as a process, which notes the pages it is asked for.

    With `kept`, every page outside it reads as zeros, as it will in the
    fixture.
    """

    def __init__(self, metadata, stream, kept=None):
        self.reads = 0
        self.bytes = 0
        self.regions = metadata["heap"]["regions"]
        self.bases = [int(region["base"], 16) for region in self.regions]
        self.plugin = metadata["plugin"]
        self.stream = stream
        self.kept = kept
        self.pages = set()

    def read(self, address, size):
        if not 0 < address < 1 << 47:
            return None
        index = bisect.bisect_right(self.bases, address) - 1
        if index < 0:
            return None
        region, base = self.regions[index], self.bases[index]
        if address + size > base + region["size"]:
            return None
        self.reads += 1
        self.bytes += size
        first, last = address // PAGE, (address + size - 1) // PAGE
        if size < ruffle_state.CHUNK // 2:
            self.pages.update(range(first, last + 1))
        offset = region["offset"] + address - base
        data = bytearray(self.stream[offset:offset + size])
        if self.kept is not None:
            for page in range(first, last + 1):
                if page not in self.kept:
                    start = max(page * PAGE, address) - address
                    end = min((page + 1) * PAGE, address + size) - address
                    data[start:end] = bytes(end - start)
        return bytes(data)

    def heap(self):
        return [(base, base + region["size"]) for base, region in zip(self.bases, self.regions)
                if not region["module"]]

    def module(self):
        base = int(self.plugin["base"], 16)
        return base, base + self.plugin["size"]


def game_state(capture):
    """Every field the reader reads, from a search that starts from nothing."""
    heap = ruffle_state.RuffleHeap(capture)
    ranges = capture.heap()
    game_mode = heap.game_mode(heap.owners(heap.buckets_of(ruffle_state.K_WORLD, ranges), ranges))
    if game_mode is None:
        return None
    world = heap.get(game_mode, ruffle_state.K_WORLD)
    chrono = heap.get(game_mode, ruffle_state.K_CHRONO)
    fields = [(world, ruffle_state.K_CURRENT_ID), (world, ruffle_state.K_PREVIOUS_ID),
              (world, ruffle_state.K_SET_NAME), (game_mode, ruffle_state.K_CURRENT_DIM),
              (game_mode, ruffle_state.K_DURATION), (game_mode, ruffle_state.K_LOCK),
              (game_mode, ruffle_state.K_GAME_OVER), (game_mode, ruffle_state.K_END_MODE),
              (chrono, ruffle_state.K_FRAME_TIMER), (chrono, ruffle_state.K_GAME_TIMER),
              (chrono, ruffle_state.K_HALTED_TIMER), (chrono, ruffle_state.K_FL_STOP)]
    return game_mode, [heap.get(obj, key) for obj, key in fields]


def pages_of_the_game(capture, game_mode):
    """The pages read from the GameMode down, and back to its bucket."""
    heap = ruffle_state.RuffleHeap(capture)
    bucket = heap.properties(game_mode)[ruffle_state.K_WORLD]
    heap.is_bucket_of(bucket, ruffle_state.K_WORLD)
    heap.first_bucket(bucket)
    manager = heap.get(game_mode, ruffle_state.K_MANAGER)
    for obj in (game_mode, heap.get(game_mode, ruffle_state.K_WORLD),
                heap.get(game_mode, ruffle_state.K_CHRONO), manager):
        heap.is_object(obj.address)
        for bucket_of_property in heap.properties(obj).values():
            heap.value(bucket_of_property)
    heap.get(manager, ruffle_state.K_CURRENT)
    return set(capture.pages)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("name", help="the capture under fixtures/")
    arguments = parser.parse_args()

    source = ROOT / "fixtures" / arguments.name
    metadata = json.loads((source / "metadata.json").read_text(encoding="utf-8"))
    with gzip.open(source / metadata["heap"]["file"], "rb") as packed:
        stream = packed.read()

    whole = game_state(Capture(metadata, stream))
    if whole is None:
        sys.exit("the capture holds no game")
    kept = pages_of_the_game(Capture(metadata, stream), whole[0])
    trimmed = game_state(Capture(metadata, stream, kept))
    if trimmed != whole:
        sys.exit(f"the trimmed capture reads another state:\n  {whole}\n  {trimmed}")
    print(f"{len(kept)} pages keep the game: {whole}")

    out = ROOT / "fixtures" / "replay" / arguments.name
    out.mkdir(parents=True, exist_ok=True)
    plugin_base = int(metadata["plugin"]["base"], 16)
    state = metadata["state_before"]
    lines = [f"plugin {hex(plugin_base)} {hex(plugin_base + metadata['plugin']['size'])}",
             f"state level={state['level']} set={state['set']} dim={state['dim']} game_mode={state['game_mode']}"]
    stored = 0
    with gzip.open(out / "heap.bin.gz", "wb", compresslevel=9) as fixture:
        for region in metadata["heap"]["regions"]:
            base = int(region["base"], 16)
            end = base + region["size"]
            address = base
            while address < end:
                is_kept = address // PAGE in kept
                run = address
                while run < end and (run // PAGE in kept) == is_kept:
                    run += PAGE
                run = min(run, end)
                if is_kept:
                    offset = region["offset"] + address - base
                    fixture.write(stream[offset:offset + run - address])
                    lines.append(f"region {hex(address)} {run - address} {stored}")
                    stored += run - address
                else:
                    lines.append(f"region {hex(address)} {run - address} -")
                address = run
    (out / "index.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
    packed_size = (out / "heap.bin.gz").stat().st_size
    print(f"{out}: {stored // KIB} KiB kept, {packed_size // KIB} KiB gzipped")


if __name__ == "__main__":
    main()
