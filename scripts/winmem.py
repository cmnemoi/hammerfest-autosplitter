"""Read only access to the memory of a Windows process. ctypes, no dependency.

The Windows equivalent of `memlib.py` / `heap.py` in hammerfest-re, which read
/proc/<pid>/mem and /proc/<pid>/maps:

    /proc/<pid>/maps   ->  VirtualQueryEx + EnumProcessModulesEx
    /proc/<pid>/mem    ->  ReadProcessMemory

Nothing is written into the target process.
"""
import ctypes as C
import ctypes.wintypes as W
import re
import struct
import subprocess

k32 = C.WinDLL("kernel32", use_last_error=True)
psapi = C.WinDLL("psapi", use_last_error=True)

PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_VM_READ = 0x0010

MEM_COMMIT = 0x1000
MEM_PRIVATE = 0x20000
PAGE_GUARD = 0x100
PAGE_NOACCESS = 0x01
READABLE = (0x02, 0x04, 0x08, 0x20, 0x40, 0x80)   # R, RW, WC, RX, RWX, WCX
WRITABLE = (0x04, 0x08, 0x40, 0x80)

LIST_MODULES_ALL = 0x03


class MEMORY_BASIC_INFORMATION64(C.Structure):
    _fields_ = [
        ("BaseAddress", C.c_ulonglong),
        ("AllocationBase", C.c_ulonglong),
        ("AllocationProtect", W.DWORD),
        ("__alignment1", W.DWORD),
        ("RegionSize", C.c_ulonglong),
        ("State", W.DWORD),
        ("Protect", W.DWORD),
        ("Type", W.DWORD),
        ("__alignment2", W.DWORD),
    ]


class MODULEINFO(C.Structure):
    _fields_ = [
        ("lpBaseOfDll", C.c_void_p),
        ("SizeOfImage", W.DWORD),
        ("EntryPoint", C.c_void_p),
    ]


k32.OpenProcess.restype = W.HANDLE
k32.OpenProcess.argtypes = [W.DWORD, W.BOOL, W.DWORD]
k32.ReadProcessMemory.argtypes = [W.HANDLE, C.c_void_p, C.c_void_p,
                                  C.c_size_t, C.POINTER(C.c_size_t)]
k32.VirtualQueryEx.argtypes = [W.HANDLE, C.c_void_p,
                               C.POINTER(MEMORY_BASIC_INFORMATION64), C.c_size_t]
k32.VirtualQueryEx.restype = C.c_size_t

# Without argtypes, ctypes passes a 64 bit HMODULE in a 32 bit C int and
# raises OverflowError as soon as the module is loaded high.
psapi.EnumProcessModulesEx.argtypes = [W.HANDLE, C.POINTER(C.c_void_p), W.DWORD,
                                       C.POINTER(W.DWORD), W.DWORD]
psapi.GetModuleFileNameExW.argtypes = [W.HANDLE, C.c_void_p, C.c_wchar_p, W.DWORD]
psapi.GetModuleInformation.argtypes = [W.HANDLE, C.c_void_p,
                                       C.POINTER(MODULEINFO), W.DWORD]


def ppapi_pids(exe_hint="Eternaltwin.exe"):
    """PIDs of the Flash plugin processes (--type=ppapi).

    The process exists only while a Flash instance lives: it appears when the
    SWF loads and disappears when you leave the page. Never cache its pid.
    """
    ps = subprocess.run(
        ["powershell", "-NoProfile", "-Command",
         "Get-CimInstance Win32_Process -Filter \"Name='%s'\" | "
         "Where-Object {$_.CommandLine -like '*--type=ppapi*'} | "
         "ForEach-Object {$_.ProcessId}" % exe_hint],
        capture_output=True, text=True,
    ).stdout
    return [int(x) for x in ps.split() if x.isdigit()]


class Proc:
    """A process opened for reading only."""

    def __init__(self, pid):
        self.pid = pid
        self.h = k32.OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
                                 False, pid)
        if not self.h:
            raise OSError("OpenProcess(%d) failed: error %d"
                          % (pid, C.get_last_error()))
        self._n = C.c_size_t()

    def close(self):
        if self.h:
            k32.CloseHandle(self.h)
            self.h = None

    # -- reading -----------------------------------------------------------
    def read(self, addr, n):
        if not (0 < addr < (1 << 47)) or n <= 0:
            return None
        buf = (C.c_char * n)()
        if not k32.ReadProcessMemory(self.h, C.c_void_p(addr), buf, n,
                                     C.byref(self._n)):
            return None
        return buf.raw[: self._n.value] if self._n.value else None

    def u64(self, addr):
        b = self.read(addr, 8)
        return struct.unpack("<Q", b)[0] if b and len(b) == 8 else None

    def u32(self, addr):
        b = self.read(addr, 4)
        return struct.unpack("<I", b)[0] if b and len(b) == 4 else None

    def i32(self, addr):
        b = self.read(addr, 4)
        return struct.unpack("<i", b)[0] if b and len(b) == 4 else None

    # -- mapping -----------------------------------------------------------
    def modules(self):
        """-> [(path, base, size)] for every loaded module."""
        needed = W.DWORD()
        arr = (C.c_void_p * 2048)()
        if not psapi.EnumProcessModulesEx(self.h, arr, C.sizeof(arr),
                                          C.byref(needed), LIST_MODULES_ALL):
            return []
        out = []
        name = C.create_unicode_buffer(1024)
        mi = MODULEINFO()
        count = min(len(arr), needed.value // C.sizeof(C.c_void_p))
        for i in range(count):
            hmod = arr[i]
            if not hmod:
                continue
            psapi.GetModuleFileNameExW(self.h, hmod, name, len(name))
            psapi.GetModuleInformation(self.h, hmod, C.byref(mi), C.sizeof(mi))
            out.append((name.value, mi.lpBaseOfDll or 0, mi.SizeOfImage))
        return out

    def module(self, pattern):
        """(base, end, path) of the first module whose path matches."""
        rx = re.compile(pattern, re.I)
        for name, base, size in self.modules():
            if rx.search(name):
                return base, base + size, name
        return None

    def region_info(self, writable_only=True, private_only=True, min_size=0):
        """Committed and readable regions, with the flags Windows reports.

        `writable_only` + `private_only` reproduces the "anonymous rw" filter
        of the Linux version: the AVM1 heap is allocated by the plugin, never
        mapped from a file.

        The flags are not used to select anything. They are here so that a
        capture can record what Windows said, which is what makes a memory
        fixture reproducible rather than a bag of bytes.
        """
        out = []
        addr = 0
        mbi = MEMORY_BASIC_INFORMATION64()
        while addr < (1 << 47):
            if not k32.VirtualQueryEx(self.h, C.c_void_p(addr), C.byref(mbi),
                                      C.sizeof(mbi)):
                break
            nxt = mbi.BaseAddress + mbi.RegionSize
            prot = mbi.Protect & 0xFF
            ok = (
                mbi.State == MEM_COMMIT
                and not (mbi.Protect & (PAGE_GUARD | PAGE_NOACCESS))
                and prot in (WRITABLE if writable_only else READABLE)
                and mbi.RegionSize >= min_size
                and (mbi.Type == MEM_PRIVATE or not private_only)
            )
            if ok:
                out.append({
                    "base": mbi.BaseAddress,
                    "end": nxt,
                    "size": mbi.RegionSize,
                    "protect": mbi.Protect,
                    "state": mbi.State,
                    "type": mbi.Type,
                })
            if nxt <= addr:
                break
            addr = nxt
        return out

    def regions(self, writable_only=True, private_only=True, min_size=0):
        """The same regions as `region_info`, as `(base, end)` pairs."""
        return [
            (r["base"], r["end"])
            for r in self.region_info(writable_only, private_only, min_size)
        ]

    # -- scan --------------------------------------------------------------
    def chunks(self, regions, chunk=8 << 20):
        for a, b in regions:
            p = a
            while p < b:
                n = min(chunk, b - p)
                buf = self.read(p, n)
                if buf:
                    yield p, buf
                elif n > 65536:
                    # One unreadable page in the middle must not lose the
                    # whole block, so we cut it up again.
                    for q in range(p, p + n, 65536):
                        sub = self.read(q, min(65536, p + n - q))
                        if sub:
                            yield q, sub
                p += n

    def scan(self, pat, align=1, regions=None):
        """Every address of `pat` in the given regions."""
        hits = []
        regs = self.regions() if regions is None else regions
        for base, buf in self.chunks(regs):
            i = buf.find(pat)
            while i != -1:
                if (base + i) % align == 0:
                    hits.append(base + i)
                i = buf.find(pat, i + 1)
        return hits

    def scan_tagged(self, ptr, regions=None):
        """The 8 possible atom encodings of a pointer aligned on 8.

        An AVM1 atom is `(value << 3) | tag`. The variants differ only in the
        3 low bits of the first byte, so we search the 7 byte tail and check
        the leading byte afterwards.
        """
        tail = struct.pack("<Q", ptr)[1:]
        lo = ptr & 0xFF
        hits = []
        regs = self.regions() if regions is None else regions
        for base, buf in self.chunks(regs):
            i = buf.find(tail)
            while i != -1:
                if i and (base + i - 1) % 8 == 0 and buf[i - 1] & ~7 == lo:
                    hits.append(base + i - 1)
                i = buf.find(tail, i + 1)
        return hits
