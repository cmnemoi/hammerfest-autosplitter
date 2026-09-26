#!/usr/bin/env python3
"""Records the linear memory of Ruffle in a Firefox tab, and trims it.

Two outputs:

    fixtures/<name>/                 the whole linear memory, out of git
        metadata.json, heap.bin.gz
    fixtures/replay/<name>/          the pages that hold the game, in git
        index.txt, heap.bin.gz

The trimming is the one `trim_ruffle_capture.py` does for desktop: replay the
reading on the capture, keep the pages it reads outside the two sweeps, and
check that the search still finds the same game when every other page reads
as zeros.

`index.txt` carries, beside the regions, a `linear` line: the base of the
linear memory in the process, and its size. Every pointer in it is an offset
from that base.

Usage:
    uv run --group analysis python scripts/capture_ruffle_web.py --name ruffle-web-main-world
"""
import argparse
import gzip
import json
import pathlib
import sys

import ruffle_state
import ruffle_web_state as web

ROOT = pathlib.Path(__file__).resolve().parent.parent
PAGE = 4096
KIB = 1 << 10


class Snapshot:
    """A copy of the linear memory, served by offset, and the pages read."""

    def __init__(self, data, kept=None):
        self.data = data
        self.size = len(data)
        self.kept = kept
        self.pages = set()

    def read(self, offset, size):
        if not 0 <= offset <= self.size - size:
            return None
        first, last = offset // PAGE, (offset + size - 1) // PAGE
        if size < web.CHUNK // 2:
            self.pages.update(range(first, last + 1))
        chunk = bytearray(self.data[offset:offset + size])
        if self.kept is not None:
            for page in range(first, last + 1):
                if page not in self.kept:
                    start = max(page * PAGE, offset) - offset
                    end = min((page + 1) * PAGE, offset + size) - offset
                    chunk[start:end] = bytes(end - start)
        return bytes(chunk)

    def u32(self, offset):
        data = self.read(offset, 4)
        return int.from_bytes(data, "little") if data else None


def game_state(linear, build):
    heap = web.WebHeap(linear, build)
    entries = heap.entries_of(web.K.K_WORLD)
    game_mode = heap.game_mode(heap.owners(entries)) if entries else None
    if game_mode is None:
        return None
    world = heap.get(game_mode, web.K.K_WORLD)
    chrono = heap.get(game_mode, web.K.K_CHRONO)
    fields = [(world, web.K.K_CURRENT_ID), (world, web.K.K_PREVIOUS_ID), (world, web.K.K_SET_NAME),
              (game_mode, web.K.K_CURRENT_DIM), (game_mode, web.K.K_DURATION),
              (game_mode, web.K.K_LOCK), (game_mode, web.K.K_GAME_OVER),
              (chrono, web.K.K_FRAME_TIMER), (chrono, web.K.K_GAME_TIMER)]
    return game_mode, [heap.get(obj, key) for obj, key in fields]


def pages_of_the_game(linear, build, game_mode):
    """The pages read from the GameMode down, back to its entry, and the build."""
    heap = web.WebHeap(linear, build)
    web.build_of(linear)
    manager = heap.get(game_mode, web.K.K_MANAGER)
    for obj in (game_mode, heap.get(game_mode, web.K.K_WORLD), heap.get(game_mode, web.K.K_CHRONO), manager):
        heap.is_object(obj.address)
        for entry in heap.properties(obj).values():
            heap.value(entry)
    heap.get(manager, web.K.K_CURRENT)
    return set(linear.pages)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--name", required=True)
    arguments = parser.parse_args()

    for pid in web.processes():
        process = ruffle_state.Process(pid)
        for base, end in web.linear_memories(process):
            live = web.Linear(process, base, end)
            build = web.build_of(live)
            if build is None or game_state(live, build) is None:
                continue
            data = process.read(base, end - base)
            if data is None:
                continue
            capture(arguments.name, base, build, data)
            return
    sys.exit("no game in Ruffle in a Firefox tab")


def capture(name, base, build, data):
    whole = game_state(Snapshot(data), build)
    if whole is None:
        sys.exit("the game moved during the capture: try again")
    print(f"captured {len(data) >> 20} MiB at {base:#x}, build {build}: {whole}")

    directory = ROOT / "fixtures" / name
    directory.mkdir(parents=True, exist_ok=True)
    with gzip.open(directory / "heap.bin.gz", "wb", compresslevel=1) as stream:
        stream.write(data)
    metadata = {"player": "ruffle-web", "build": build, "linear": {"base": hex(base), "size": len(data)},
                "note": "The game runs during a capture. No clock in it is exact."}
    (directory / "metadata.json").write_text(json.dumps(metadata, indent=1), encoding="utf-8")

    reading = Snapshot(data)
    kept = pages_of_the_game(reading, build, whole[0])
    trimmed = game_state(Snapshot(data, kept), build)
    if trimmed != whole:
        sys.exit(f"the trimmed capture reads another state:\n  {whole}\n  {trimmed}")
    print(f"{len(kept)} pages keep the game")

    world = web.K.WORLDS.get(whole[1][2], whole[1][2])
    lines = [f"linear {hex(base)} {len(data)}",
             f"state level={int(whole[1][0])} set={world} dim={int(whole[1][3] or 0)} game_mode={hex(whole[0].address)}"]
    out = ROOT / "fixtures" / "replay" / name
    out.mkdir(parents=True, exist_ok=True)
    stored = 0
    with gzip.open(out / "heap.bin.gz", "wb", compresslevel=9) as fixture:
        offset = 0
        while offset < len(data):
            is_kept = offset // PAGE in kept
            run = offset
            while run < len(data) and (run // PAGE in kept) == is_kept:
                run += PAGE
            run = min(run, len(data))
            if is_kept:
                fixture.write(data[offset:run])
                lines.append(f"region {hex(base + offset)} {run - offset} {stored}")
                stored += run - offset
            else:
                lines.append(f"region {hex(base + offset)} {run - offset} -")
            offset = run
    (out / "index.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"{out}: {stored // KIB} KiB kept, {(out / 'heap.bin.gz').stat().st_size // KIB} KiB gzipped")


if __name__ == "__main__":
    main()
