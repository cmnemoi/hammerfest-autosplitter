#!/usr/bin/env python3
"""Reads the current Hammerfest level from Ruffle in Firefox, on Linux.

A second opinion, as `ruffle_state.py` is for Ruffle desktop. It checks the
layout in `docs/concepts/ruffle-web-heap.md` on a live game: every step prints
what it found, so a wrong offset shows where it breaks.

The resolution chain:

    Isolated Web Co / Web Content     the processes of the tabs
    an rw range followed by a         the linear memory of a wasm module,
    reserve of 4 GiB or more          base = start + 0x1000
    the vtables of a known build      which build of Ruffle, and a proof
    scan FNV32("]=[]8")               the hash of the `world` key
    -> entry = hit - 0x24             check: its key reads "]=[]8"
    -> the object that owns it        check: its vtable says 4 / 80
    GameMode, whose manager names it back as `current`

Every pointer in the linear memory is a u32 offset from its base.

Usage (numpy comes with the `analysis` group):
    uv run --group analysis python scripts/ruffle_web_state.py
    ruffle_web_state.py           one reading
    ruffle_web_state.py --dump    GameMode, world and Chrono in full
"""
import argparse
import os
import struct
import sys
import time

import numpy

import hfmap
import ruffle_state

PROCESS_NAMES = ("Isolated Web Co", "Web Content")
GiB = 1 << 30
CHUNK = 1 << 20

# From docs/concepts/ruffle-web-heap.md, read in the released .wasm.
HEADER_PAGE = 0x1000
BUILDS = {
    "extensions": {"object": (0x2C69A0, (4, 80, 4417, 4418)), "string": (0x315F30, (4, 20, 6388, 6389))},
    "mvp": {"object": (0x253890, (4, 80, 2923, 2924)), "string": (0x316310, (4, 20, 6373, 6374))},
}
GC_VTABLE = -4
GC_FLAGS = 0xF
OBJECT_ENTRIES = 0x04          # cap, ptr, len: three u32
ENTRY_SIZE = 40
ENTRY_KEY = 0x20
ENTRY_HASH = 0x24
VALUE_POINTER = 0x04
VALUE_NUMBER = 0x08
STRING_META = 0x04
WIDE = 1 << 31

TAG_UNDEFINED, TAG_NULL, TAG_BOOL, TAG_NUMBER, TAG_STRING, TAG_OBJECT, TAG_CLIP = range(7)

K = ruffle_state  # the obfuscated keys


def key_hash32(key):
    return ruffle_state.key_hash(key) & 0xFFFFFFFF


def processes():
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        try:
            with open(f"/proc/{pid}/comm") as comm:
                if comm.read().strip() in PROCESS_NAMES:
                    yield int(pid)
        except OSError:
            continue


def linear_memories(process):
    """Ranges shaped like a wasm linear memory: rw, then a reserve >= 4 GiB."""
    maps = list(process.maps())
    for (start, end, permissions, path), after in zip(maps, maps[1:]):
        reserve = after[1] - after[0] if after[0] == end and after[2].startswith("---") else 0
        if permissions.startswith("rw") and path == "" and reserve + (end - start) >= 4 * GiB:
            yield start + HEADER_PAGE, end


class Linear:
    """The linear memory of one wasm module, addressed by offset."""

    def __init__(self, process, base, end):
        self.process = process
        self.base = base
        self.size = end - base

    def read(self, offset, size):
        if not 0 <= offset <= self.size - size:
            return None
        return self.process.read(self.base + offset, size)

    def u32(self, offset):
        data = self.read(offset, 4)
        return struct.unpack("<I", data)[0] if data else None


def build_of(linear):
    for name, build in BUILDS.items():
        if all(linear.read(offset, 16) == struct.pack("<4I", *expected)
               for offset, expected in build.values()):
            return name
    return None


class WebHeap:
    def __init__(self, linear, build):
        self.linear = linear
        self.object_vtable = BUILDS[build]["object"][0]

    def string(self, offset):
        units, meta = (self.linear.u32(offset), self.linear.u32(offset + STRING_META))
        if units is None or meta is None:
            return None
        length = meta & ~WIDE
        if length > 4096:
            return None
        if meta & WIDE:
            data = self.linear.read(units, length * 2)
            return data.decode("utf-16-le", "replace") if data is not None else None
        data = self.linear.read(units, length)
        return data.decode("latin-1") if data is not None else None

    def value(self, entry):
        data = self.linear.read(entry, 16)
        if data is None:
            return None
        tag = data[0]
        pointer = struct.unpack_from("<I", data, VALUE_POINTER)[0]
        if tag == TAG_UNDEFINED:
            return "undefined"
        if tag == TAG_NULL:
            return None
        if tag == TAG_BOOL:
            return bool(data[1])
        if tag == TAG_NUMBER:
            return struct.unpack_from("<d", data, VALUE_NUMBER)[0]
        if tag == TAG_STRING:
            return self.string(pointer)
        if tag == TAG_OBJECT:
            return ruffle_state.Obj(pointer)
        if tag == TAG_CLIP:
            return ruffle_state.Clip(pointer)
        return f"<tag {tag}>"

    def is_object(self, offset):
        vtable = self.linear.u32(offset + GC_VTABLE)
        return vtable is not None and vtable & ~GC_FLAGS == self.object_vtable

    def properties(self, obj):
        header = self.linear.read(obj.address + OBJECT_ENTRIES, 12)
        if header is None:
            return {}
        capacity, pointer, length = struct.unpack("<3I", header)
        if length > capacity or length > 4096:
            return {}
        found = {}
        for index in range(length):
            entry = pointer + index * ENTRY_SIZE
            key = self.string(self.linear.u32(entry + ENTRY_KEY) or 0)
            if key is not None:
                found[key] = entry
        return found

    def get(self, obj, key):
        entry = self.properties(obj).get(key)
        return self.value(entry) if entry is not None else None

    def entries_of(self, key):
        """Pass A: the entries whose hash is the hash of `key`."""
        pattern = struct.pack("<I", key_hash32(key))
        found = []
        for base in range(0, self.linear.size, CHUNK):
            data = self.linear.read(base, min(CHUNK, self.linear.size - base))
            if data is None:
                continue
            at = data.find(pattern)
            while at != -1:
                entry = base + at - ENTRY_HASH
                if at % 4 == 0 and entry % 8 == 0:
                    key_offset = self.linear.u32(entry + ENTRY_KEY)
                    tag = self.linear.read(entry, 1)
                    if tag and tag[0] <= TAG_CLIP and key_offset and self.string(key_offset) == key:
                        found.append(entry)
                at = data.find(pattern, at + 1)
        return found

    def owners(self, entries):
        """Pass B: the objects whose map holds these entries."""
        lowest, highest = min(entries), max(entries)
        owners = []
        for base in range(0, self.linear.size, CHUNK):
            data = self.linear.read(base, min(CHUNK, self.linear.size - base))
            if data is None:
                continue
            # Only the words that could point near the entries: numpy keeps
            # this pass from walking tens of millions of words in Python.
            words = numpy.frombuffer(data[: len(data) // 4 * 4], dtype="<u4").astype(numpy.int64)
            near = numpy.nonzero((words <= highest) & (words + 4096 * ENTRY_SIZE > lowest))[0]
            for at in (int(index) * 4 for index in near):
                if at + 8 > len(data):
                    continue
                pointer, length = struct.unpack_from("<II", data, at)
                if not (pointer <= highest and pointer + length * ENTRY_SIZE > lowest) or length > 4096:
                    continue
                if any(pointer <= entry < pointer + length * ENTRY_SIZE and (entry - pointer) % ENTRY_SIZE == 0
                       for entry in entries):
                    obj = base + at - (OBJECT_ENTRIES + 4)
                    if self.is_object(obj) and ruffle_state.Obj(obj) not in owners:
                        owners.append(ruffle_state.Obj(obj))
        return owners

    def game_mode(self, candidates):
        for obj in candidates:
            manager = self.get(obj, K.K_MANAGER)
            if isinstance(manager, ruffle_state.Obj) and self.get(manager, K.K_CURRENT) == obj:
                return obj
        return None


def dump(heap, obj, title):
    print(f"\n{title} {obj}")
    for key, entry in heap.properties(obj).items():
        print(f"  {hfmap.clear(key):24} {key!r:12} {heap.value(entry)!r}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dump", action="store_true")
    arguments = parser.parse_args()

    found_any = False
    for pid in processes():
        process = ruffle_state.Process(pid)
        for base, end in linear_memories(process):
            linear = Linear(process, base, end)
            build = build_of(linear)
            print(f"pid {pid}: linear memory at {base:#x}, {linear.size >> 20} MiB, build {build}")
            if build is None:
                continue
            found_any = True
            heap = WebHeap(linear, build)
            started = time.monotonic()
            entries = heap.entries_of(K.K_WORLD)
            print(f"  pass A: {len(entries)} entries of {K.K_WORLD!r} (`world`)")
            if not entries:
                continue
            owners = heap.owners(entries)
            print(f"  pass B: {len(owners)} objects own them")
            game_mode = heap.game_mode(owners)
            print(f"  search: {time.monotonic() - started:.2f} s")
            if game_mode is None:
                print("  no GameMode in this tab")
                continue
            world = heap.get(game_mode, K.K_WORLD)
            chrono = heap.get(game_mode, K.K_CHRONO)
            set_name = heap.get(world, K.K_SET_NAME)
            print(f"\n  GameMode {game_mode} (offset)")
            print(f"    world      {K.WORLDS.get(set_name, set_name)!r}")
            print(f"    level      {heap.get(world, K.K_CURRENT_ID)!r}   previous {heap.get(world, K.K_PREVIOUS_ID)!r}")
            print(f"    dimension  {heap.get(game_mode, K.K_CURRENT_DIM)!r}")
            print(f"    duration   {heap.get(game_mode, K.K_DURATION)!r} cycles")
            print(f"    lock       {heap.get(game_mode, K.K_LOCK)!r}   game over {heap.get(game_mode, K.K_GAME_OVER)!r}")
            print(f"    frame      {heap.get(chrono, K.K_FRAME_TIMER)!r}")
            if arguments.dump:
                dump(heap, game_mode, "GameMode")
                dump(heap, world, "world")
                dump(heap, chrono, "Chrono")
    if not found_any:
        sys.exit("no Ruffle in a Firefox tab: is a game open on eternalfest.net, with the Ruffle extension?")


if __name__ == "__main__":
    main()
