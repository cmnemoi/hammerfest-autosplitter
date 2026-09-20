"""Lecture seule de la memoire d'un process Windows. ctypes, aucune dependance.

Equivalent Windows de `memlib.py` / `heap.py` de hammerfest-re, qui lisaient
/proc/<pid>/mem et /proc/<pid>/maps :

    /proc/<pid>/maps   ->  VirtualQueryEx + EnumProcessModulesEx
    /proc/<pid>/mem    ->  ReadProcessMemory

Rien n'est ecrit dans le process cible.
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

# Sans argtypes, ctypes passe un HMODULE 64 bits dans un int C 32 bits et
# leve OverflowError des que le module est charge haut.
psapi.EnumProcessModulesEx.argtypes = [W.HANDLE, C.POINTER(C.c_void_p), W.DWORD,
                                       C.POINTER(W.DWORD), W.DWORD]
psapi.GetModuleFileNameExW.argtypes = [W.HANDLE, C.c_void_p, C.c_wchar_p, W.DWORD]
psapi.GetModuleInformation.argtypes = [W.HANDLE, C.c_void_p,
                                       C.POINTER(MODULEINFO), W.DWORD]


def ppapi_pids(exe_hint="Eternaltwin.exe"):
    """PIDs des process plugin Flash (--type=ppapi).

    Le process n'existe que tant qu'une instance Flash est vivante : il
    apparait au chargement du SWF et disparait quand on quitte la page. Ne
    jamais mettre son pid en cache.
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
    """Un process ouvert en lecture seule."""

    def __init__(self, pid):
        self.pid = pid
        self.h = k32.OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
                                 False, pid)
        if not self.h:
            raise OSError("OpenProcess(%d) a echoue: erreur %d"
                          % (pid, C.get_last_error()))
        self._n = C.c_size_t()

    def close(self):
        if self.h:
            k32.CloseHandle(self.h)
            self.h = None

    # -- lecture -----------------------------------------------------------
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

    # -- cartographie ------------------------------------------------------
    def modules(self):
        """-> [(chemin, base, taille)] pour tous les modules charges."""
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
        """(base, fin, chemin) du premier module dont le chemin matche."""
        rx = re.compile(pattern, re.I)
        for name, base, size in self.modules():
            if rx.search(name):
                return base, base + size, name
        return None

    def regions(self, writable_only=True, private_only=True, min_size=0):
        """Regions engagees et lisibles.

        `writable_only` + `private_only` reproduit le filtre "rw anonyme" de la
        version Linux : le tas AVM1 est alloue par le plugin, jamais mappe
        depuis un fichier.
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
                out.append((mbi.BaseAddress, nxt))
            if nxt <= addr:
                break
            addr = nxt
        return out

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
                    # une page illisible au milieu ne doit pas faire sauter
                    # tout le bloc : on redecoupe.
                    for q in range(p, p + n, 65536):
                        sub = self.read(q, min(65536, p + n - q))
                        if sub:
                            yield q, sub
                p += n

    def scan(self, pat, align=1, regions=None):
        """Toutes les adresses de `pat` dans les regions donnees."""
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
        """Les 8 encodages d'atome possibles d'un pointeur aligne sur 8.

        Un atome AVM1 est `(valeur << 3) | tag` : les variantes ne different
        que par les 3 bits bas du premier octet, donc on cherche la queue de
        7 octets et on verifie l'octet de tete apres coup.
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
