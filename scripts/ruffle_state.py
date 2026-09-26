#!/usr/bin/env python3
"""Reads the current Hammerfest level and clock from a live Ruffle, on Linux,
and from Ruffle for Windows under Wine: its objects are the same.

A second opinion, as `hf_state.py` is for Pepper Flash. It is also how the
layout in `docs/concepts/ruffle-heap.md` gets checked: every step below prints
what it found, so a wrong offset shows where it breaks.

The resolution chain, with no hard coded address:

    ruffle                        the process, /proc/<pid>/mem and maps
    scan FNV("]=[]8")             the hash of the `world` key, kept in every
                                  property map entry that has that key
    -> bucket = hit - 0x30        check: its key reads "]=[]8", its tag <= 6
    -> walk back to entries.ptr   the first bucket of the map
    -> scan entries.ptr           the object whose map holds that Vec
    -> object = hit - 0x10        check: its GC vtable says 8 / 160
    GameMode, the object whose `manager` names it back as `current`
    GameMode.world                -> setName, currentId (an f64 in Ruffle)
    GameMode.gameChrono           -> fl_stop ? haltedTimer : frameTimer-gameTimer

Usage:
    ruffle_state.py               one reading
    ruffle_state.py --dump        prints GameMode, world and Chrono in full
    ruffle_state.py --pid 1234    a given process, not the first `ruffle`

Under Wine, the kernel names the process `main`, and its executable is Wine's
own: the process is the one that maps `ruffle.exe`, and its module runs as far
as the PE header of `ruffle.exe` says.
"""
import argparse
import os
import struct
import subprocess
import sys
import time

import hfmap

PROCESS_NAME = "ruffle"
WINDOWS_EXECUTABLE = "ruffle.exe"
CHUNK = 1 << 20

# From docs/concepts/ruffle-heap.md, read in the Linux 0.6.0 binary.
BUCKET_STRIDE = 56
BUCKET_KEY = 0x28
BUCKET_HASH = 0x30
OBJECT_ENTRIES_CAPACITY = 0x08
OBJECT_ENTRIES_POINTER = 0x10
OBJECT_ENTRIES_LENGTH = 0x18
OBJECT_SIZE = 160
STRING_META = 0x10
WIDE = 1 << 31
GC_HEADER_VTABLE = -0x08
GC_FLAGS = 0xF

TAG_UNDEFINED, TAG_NULL, TAG_BOOL, TAG_NUMBER, TAG_STRING, TAG_OBJECT, TAG_CLIP = range(7)

K_WORLD = hfmap.obf("world")
K_MANAGER = hfmap.obf("manager")
K_CURRENT = hfmap.obf("current")
K_CURRENT_ID = hfmap.obf("currentId")
K_PREVIOUS_ID = hfmap.obf("_previousId")
K_SET_NAME = hfmap.obf("setName")
K_CHRONO = hfmap.obf("gameChrono")
K_DURATION = hfmap.obf("duration")
K_CURRENT_DIM = hfmap.obf("currentDim")
K_GAME_OVER = hfmap.obf("fl_gameOver")
K_LOCK = hfmap.obf("fl_lock")
K_END_MODE = hfmap.obf("endModeTimer")
K_FRAME_TIMER = hfmap.obf("frameTimer")
K_GAME_TIMER = hfmap.obf("gameTimer")
K_HALTED_TIMER = hfmap.obf("haltedTimer")
K_FL_STOP = hfmap.obf("fl_stop")

WORLDS = {hfmap.obf(name): name for name in
          ("xml_adventure", "xml_deepnight", "xml_hiko", "xml_ayame", "xml_hk")}


def key_hash(key):
    """The hash Ruffle keeps beside a property key.

    FNV-1a 64 over each unit, lowered and written as a u16 LE, then one byte
    0xff. See `property_map.rs:198-201` in Ruffle.
    """
    hashed = 0xcbf29ce484222325
    units = b"".join(ord(character.lower() if "A" <= character <= "Z" else character)
                     .to_bytes(2, "little") for character in key)
    for byte in units + b"\xff":
        hashed ^= byte
        hashed = (hashed * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return hashed


class Process:
    """Read only access to /proc/<pid>/mem, with what it cost."""

    def __init__(self, pid):
        self.pid = pid
        self.mem = open(f"/proc/{pid}/mem", "rb", buffering=0)
        self.reads = 0
        self.bytes = 0

    def read(self, address, size):
        # A user space address fits in 47 bits. Anything else is not a
        # pointer, whatever the bytes said.
        if not 0 < address < 1 << 47:
            return None
        self.reads += 1
        self.bytes += size
        try:
            data = os.pread(self.mem.fileno(), size, address)
        except OSError:
            return None
        return data if len(data) == size else None

    def u64(self, address):
        data = self.read(address, 8)
        return struct.unpack("<Q", data)[0] if data else None

    def u32(self, address):
        data = self.read(address, 4)
        return struct.unpack("<I", data)[0] if data else None

    def maps(self):
        with open(f"/proc/{self.pid}/maps") as maps:
            for line in maps:
                fields = line.split(maxsplit=5)
                start, end = (int(bound, 16) for bound in fields[0].split("-"))
                path = fields[5].strip() if len(fields) > 5 else ""
                yield start, end, fields[1], path

    def module(self):
        """The range of the ruffle executable itself.

        Under Wine, one page of `ruffle.exe` is mapped from its file and its
        sections are copied into anonymous memory: its PE header says how far
        it runs.
        """
        windows = [start for start, _, _, path in self.maps() if path.lower().endswith(WINDOWS_EXECUTABLE)]
        if windows:
            base = min(windows)
            nt_headers = self.u32(base + 0x3C)
            return base, base + self.u32(base + nt_headers + 0x18 + 0x38)
        executable = os.readlink(f"/proc/{self.pid}/exe")
        ranges = [(start, end) for start, end, _, path in self.maps() if path == executable]
        return min(start for start, _ in ranges), max(end for _, end in ranges)

    def heap(self):
        """Readable, writable, and anonymous or the brk heap."""
        return [(start, end) for start, end, permissions, path in self.maps()
                if permissions.startswith("rw") and path in ("", "[heap]")]


class RuffleHeap:
    def __init__(self, process):
        self.process = process
        self.module = process.module()

    # -- strings, values, objects -------------------------------------------

    def string(self, repr_address):
        units = self.process.u64(repr_address)
        meta = self.process.u32(repr_address + STRING_META)
        if units is None or meta is None:
            return None
        length = meta & ~WIDE
        if length > 4096:
            return None
        if meta & WIDE:
            data = self.process.read(units, length * 2)
            return data.decode("utf-16-le", "replace") if data is not None else None
        data = self.process.read(units, length)
        return data.decode("latin-1") if data is not None else None

    def value(self, address):
        data = self.process.read(address, 16)
        if data is None:
            return None
        tag = data[0]
        payload = struct.unpack("<Q", data[8:])[0]
        if tag == TAG_UNDEFINED:
            return "undefined"
        if tag == TAG_NULL:
            return None
        if tag == TAG_BOOL:
            return bool(data[1])
        if tag == TAG_NUMBER:
            return struct.unpack("<d", data[8:])[0]
        if tag == TAG_STRING:
            return self.string(payload)
        if tag == TAG_OBJECT:
            return Obj(payload)
        if tag == TAG_CLIP:
            return Clip(payload)
        return f"<tag {tag}>"

    def gc_type(self, value_address):
        """(align, size) of a GC value, from the vtable in its header."""
        tagged = self.process.u64(value_address + GC_HEADER_VTABLE)
        if tagged is None:
            return None
        vtable = tagged & ~GC_FLAGS
        if not self.module[0] <= vtable < self.module[1]:
            return None
        return self.process.u64(vtable), self.process.u64(vtable + 8)

    def is_object(self, address):
        return self.gc_type(address) == (8, OBJECT_SIZE)

    def properties(self, obj):
        """Every property of an object, by key, as the map holds them."""
        header = self.process.read(obj.address + OBJECT_ENTRIES_CAPACITY, 24)
        if header is None:
            return {}
        capacity, pointer, length = struct.unpack("<QQQ", header)
        if length > capacity or length > 4096:
            return {}
        entries = self.process.read(pointer, length * BUCKET_STRIDE) if length else b""
        if entries is None:
            return {}
        found = {}
        for index in range(length):
            bucket = pointer + index * BUCKET_STRIDE
            key_address = struct.unpack_from("<Q", entries, index * BUCKET_STRIDE + BUCKET_KEY)[0]
            key = self.string(key_address)
            if key is not None:
                found[key] = bucket
        return found

    def get(self, obj, key):
        bucket = self.properties(obj).get(key)
        return self.value(bucket) if bucket is not None else None

    # -- the search ----------------------------------------------------------

    def is_bucket_of(self, bucket, key):
        data = self.process.read(bucket, BUCKET_STRIDE)
        if data is None or data[0] > TAG_CLIP:
            return False
        key_address, hashed = struct.unpack_from("<QQ", data, BUCKET_KEY)
        return hashed == key_hash(key) and self.string(key_address) == key

    def is_a_bucket(self, bucket):
        """Any coherent bucket, whatever its key."""
        data = self.process.read(bucket, BUCKET_STRIDE)
        if data is None or data[0] > TAG_CLIP:
            return False
        key_address, hashed = struct.unpack_from("<QQ", data, BUCKET_KEY)
        key = self.string(key_address)
        return key is not None and hashed == key_hash(key)

    def scan(self, ranges, patterns):
        """Every 8-aligned address where one of `patterns` lies, in one pass.

        One pass for all patterns, as `scan_u64_any` does in the reader: one
        pass per pattern reads the heap again for each one.
        """
        hits = []
        for start, end in ranges:
            base = start
            while base < end:
                size = min(CHUNK, end - base)
                data = self.process.read(base, size)
                if data is not None:
                    for pattern in patterns:
                        at = data.find(pattern)
                        while at != -1:
                            if at % 8 == 0:
                                hits.append((base + at, pattern))
                            at = data.find(pattern, at + 1)
                base += size
        return hits

    def buckets_of(self, key, ranges):
        pattern = struct.pack("<Q", key_hash(key))
        return [hit - BUCKET_HASH for hit, _ in self.scan(ranges, [pattern])
                if self.is_bucket_of(hit - BUCKET_HASH, key)]

    def first_bucket(self, bucket):
        """Walks back while the buckets stay coherent: the entries.ptr."""
        while self.is_a_bucket(bucket - BUCKET_STRIDE):
            bucket -= BUCKET_STRIDE
        return bucket

    def owners(self, buckets, ranges):
        """The objects whose map holds these buckets."""
        firsts = {struct.pack("<Q", self.first_bucket(bucket)): bucket for bucket in buckets}
        owners = []
        for hit, pattern in self.scan(ranges, list(firsts)):
            first = struct.unpack("<Q", pattern)[0]
            obj = hit - OBJECT_ENTRIES_POINTER
            length = self.process.u64(obj + OBJECT_ENTRIES_LENGTH)
            capacity = self.process.u64(obj + OBJECT_ENTRIES_CAPACITY)
            index = (firsts[pattern] - first) // BUCKET_STRIDE
            if length is None or capacity is None:
                continue
            if index < length <= capacity and self.is_object(obj):
                owners.append(Obj(obj))
        return owners

    def game_mode(self, candidates):
        """The one candidate whose manager names it back as `current`."""
        for obj in candidates:
            manager = self.get(obj, K_MANAGER)
            if isinstance(manager, Obj) and self.get(manager, K_CURRENT) == obj:
                return obj
        return None


class Obj:
    def __init__(self, address):
        self.address = address

    def __eq__(self, other):
        return isinstance(other, Obj) and other.address == self.address

    def __hash__(self):
        return hash(self.address)

    def __repr__(self):
        return f"Object@{self.address:#x}"


class Clip(Obj):
    def __repr__(self):
        return f"MovieClip@{self.address:#x}"


def find_pid():
    found = subprocess.run(["pgrep", "-x", PROCESS_NAME], capture_output=True, text=True)
    pids = [int(pid) for pid in found.stdout.split()] or wine_pids()
    if not pids:
        sys.exit(f"no process named {PROCESS_NAME!r}: start a game in Eternalfest Desktop")
    return pids[0]


def wine_pids():
    """The processes that map `ruffle.exe`: Ruffle for Windows, under Wine."""
    pids = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/maps") as maps:
                if any(line.rstrip().lower().endswith(WINDOWS_EXECUTABLE) for line in maps):
                    pids.append(int(entry))
        except OSError:
            continue
    return pids


def dump(heap, obj, title):
    print(f"\n{title} {obj}")
    for key, bucket in heap.properties(obj).items():
        print(f"  {hfmap.clear(key):24} {key!r:12} {heap.value(bucket)!r}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--pid", type=int)
    parser.add_argument("--dump", action="store_true")
    arguments = parser.parse_args()

    process = Process(arguments.pid or find_pid())
    heap = RuffleHeap(process)
    ranges = process.heap()
    print(f"pid {process.pid}, module {heap.module[0]:#x}-{heap.module[1]:#x}, "
          f"{len(ranges)} heap ranges, {sum(end - start for start, end in ranges) >> 20} MiB")

    started = time.monotonic()
    buckets = heap.buckets_of(K_WORLD, ranges)
    print(f"pass A: {len(buckets)} buckets of {K_WORLD!r} (`world`)"
          f"  [{process.reads} reads, {process.bytes >> 20} MiB]")
    owners = heap.owners(buckets, ranges)
    print(f"pass B: {len(owners)} objects own them: {owners}"
          f"  [{process.reads} reads, {process.bytes >> 20} MiB]")
    game_mode = heap.game_mode(owners)
    print(f"search: {time.monotonic() - started:.2f} s")
    if game_mode is None:
        sys.exit("no GameMode: is a game running, past the black screen?")

    world = heap.get(game_mode, K_WORLD)
    chrono = heap.get(game_mode, K_CHRONO)
    set_name = heap.get(world, K_SET_NAME)
    stopped = heap.get(chrono, K_FL_STOP)
    clock = (heap.get(chrono, K_HALTED_TIMER) if stopped
             else heap.get(chrono, K_FRAME_TIMER) - heap.get(chrono, K_GAME_TIMER))
    print(f"\nGameMode {game_mode}")
    print(f"  world      {WORLDS.get(set_name, set_name)!r}")
    print(f"  level      {heap.get(world, K_CURRENT_ID)!r}   previous {heap.get(world, K_PREVIOUS_ID)!r}")
    print(f"  dimension  {heap.get(game_mode, K_CURRENT_DIM)!r}")
    print(f"  chrono     {clock!r} ms   stopped {stopped!r}")
    print(f"  duration   {heap.get(game_mode, K_DURATION)!r} cycles")
    print(f"  lock       {heap.get(game_mode, K_LOCK)!r}   game over {heap.get(game_mode, K_GAME_OVER)!r}")
    print(f"  end mode   {heap.get(game_mode, K_END_MODE)!r}")

    if arguments.dump:
        dump(heap, game_mode, "GameMode")
        dump(heap, world, "world")
        dump(heap, chrono, "Chrono")


if __name__ == "__main__":
    main()
