"""Read only access to the memory of a Linux process, or of a capture of one.

The Linux counterpart of `winmem.py`, with the same interface, so that the
scripts built on it run on both systems:

    /proc/<pid>/maps   the regions, their permissions and their files
    /proc/<pid>/mem    the bytes

`Recorded` serves a capture of `fixtures/<name>/` the same way, with no game
running: what a script reads live, it reads again off line.

Nothing is written into the target process.
"""
import gzip
import json
import os
import re
import struct

# One read per block while scanning, and an overlap so that a pattern across
# the edge of a block is not missed.
CHUNK = 8 << 20
OVERLAP = 64
# A cut-up block, when one page of it is gone.
RETRY = 64 << 10


class Memory:
    """What every script asks of a process: words, regions, scans.

    A subclass gives `read(address, size)` and `maps()`, the regions as
    `(start, end, permissions, path)`.
    """

    def read(self, address, size):
        raise NotImplementedError

    def maps(self):
        raise NotImplementedError

    # -- words -------------------------------------------------------------
    def u64(self, address):
        data = self.read(address, 8)
        return struct.unpack("<Q", data)[0] if data and len(data) == 8 else None

    def u32(self, address):
        data = self.read(address, 4)
        return struct.unpack("<I", data)[0] if data and len(data) == 4 else None

    def i32(self, address):
        data = self.read(address, 4)
        return struct.unpack("<i", data)[0] if data and len(data) == 4 else None

    # -- mapping -----------------------------------------------------------
    def module(self, pattern):
        """(base, end, path) of the first file whose path matches.

        Its extent runs over every mapping of that file. A Windows executable
        under Wine is mapped from its file for one page only, its sections
        copied into anonymous memory: its PE header then says how far it
        runs.
        """
        rx = re.compile(pattern, re.I)
        mapped = [(start, end, path) for start, end, _, path in self.maps()
                  if path and rx.search(path)]
        if not mapped:
            return None
        path = mapped[0][2]
        base = min(start for start, _, file in mapped if file == path)
        end = max(end for _, end, file in mapped if file == path)
        return base, max(end, base + (self.pe_image_size(base) or 0)), path

    def pe_image_size(self, base):
        """`SizeOfImage` of the PE header at `base`, or None."""
        if self.read(base, 2) != b"MZ":
            return None
        nt_headers = self.u32(base + 0x3C)
        if nt_headers is None or self.read(base + nt_headers, 4) != b"PE\0\0":
            return None
        return self.u32(base + nt_headers + 0x18 + 0x38)

    def region_info(self, writable_only=True, private_only=True, min_size=0):
        """Readable regions, with the permissions the kernel reports.

        `private_only` keeps the regions no file is behind: the AVM1 heap is
        allocated by the player, never mapped from a file.
        """
        out = []
        for start, end, permissions, path in self.maps():
            readable = permissions.startswith("r")
            writable = "w" in permissions
            if (not readable or (writable_only and not writable)
                    or (private_only and path) or end - start < min_size):
                continue
            out.append({"base": start, "end": end, "size": end - start,
                        "permissions": permissions})
        return out

    def regions(self, writable_only=True, private_only=True, min_size=0):
        return [(r["base"], r["end"])
                for r in self.region_info(writable_only, private_only, min_size)]

    # -- scan --------------------------------------------------------------
    def chunks(self, regions, chunk=CHUNK):
        for start, end in regions:
            at = start
            while at < end:
                size = min(chunk + OVERLAP, end - at)
                data = self.read(at, size)
                if data:
                    yield at, data
                elif size > RETRY:
                    # One page gone in the middle must not lose the block.
                    for sub in range(at, at + size, RETRY):
                        piece = self.read(sub, min(RETRY, at + size - sub))
                        if piece:
                            yield sub, piece
                at += chunk

    def scan(self, pattern, align=1, regions=None):
        """Every address of `pattern` in the given regions."""
        hits = set()
        for base, data in self.chunks(self.regions() if regions is None else regions):
            at = data.find(pattern)
            while at != -1:
                if (base + at) % align == 0:
                    hits.add(base + at)
                at = data.find(pattern, at + 1)
        return sorted(hits)

    def scan_tagged(self, pointer, regions=None, word=8):
        """The words of `word` bytes that hold `pointer` with any tag.

        An AVM1 atom is `(value << 3) | tag`: the variants differ only in the
        three low bits of the first byte, so we search the other bytes and
        check the first one afterwards.
        """
        tail = pointer.to_bytes(word, "little")[1:]
        low = pointer & 0xFF
        hits = set()
        for base, data in self.chunks(self.regions() if regions is None else regions):
            at = data.find(tail)
            while at != -1:
                if at and (base + at - 1) % word == 0 and data[at - 1] & ~7 == low:
                    hits.add(base + at - 1)
                at = data.find(tail, at + 1)
        return sorted(hits)


class Proc(Memory):
    """A live process, opened for reading only."""

    def __init__(self, pid):
        self.pid = pid
        self.mem = open(f"/proc/{pid}/mem", "rb", buffering=0)

    def close(self):
        self.mem.close()

    def read(self, address, size):
        if not 0 < address < 1 << 47 or size <= 0:
            return None
        try:
            return os.pread(self.mem.fileno(), size, address) or None
        except OSError:
            return None

    def maps(self):
        with open(f"/proc/{self.pid}/maps") as maps:
            for line in maps:
                fields = line.split(maxsplit=5)
                start, end = (int(bound, 16) for bound in fields[0].split("-"))
                path = fields[5].strip() if len(fields) > 5 else ""
                yield start, end, fields[1], path


class Recorded(Memory):
    """A capture of `fixtures/<name>/`, served as the process it was.

    Its regions are the captured ones, with no file behind them, and its
    module is the range the capture recorded, whatever pattern is asked.
    """

    def __init__(self, directory):
        with open(os.path.join(directory, "metadata.json"), encoding="utf-8") as stream:
            metadata = json.load(stream)
        path = os.path.join(directory, metadata["heap"]["file"])
        opener = gzip.open if path.endswith(".gz") else open
        with opener(path, "rb") as stream:
            self.bytes = stream.read()
        self.regions_recorded = sorted(
            (int(region["base"], 16), region["size"], region["offset"])
            for region in metadata["heap"]["regions"])
        plugin = metadata["plugin"]
        self.plugin = (int(plugin["base"], 16), int(plugin["base"], 16) + plugin["size"],
                       plugin.get("path", "recorded"))
        self.pid = 0

    def read(self, address, size):
        for base, length, offset in self.regions_recorded:
            if base <= address and address + size <= base + length:
                start = offset + address - base
                return self.bytes[start:start + size]
        return None

    def maps(self):
        for base, length, _ in self.regions_recorded:
            yield base, base + length, "rw-p", ""

    def module(self, pattern):
        return self.plugin


def flash_pids(pattern):
    """The processes that map a file matching `pattern`: the Flash plugin,
    or a Flash projector."""
    rx = re.compile(pattern, re.I)
    pids = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/maps") as maps:
                if any(rx.search(line) for line in maps):
                    pids.append(int(entry))
        except OSError:
            continue
    return pids
