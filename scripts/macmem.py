"""Read only access to the memory of a macOS process. ctypes, no dependency.

The macOS counterpart of `procmem.py`, with the same interface:

    /proc/<pid>/maps   ->  mach_vm_region + proc_regionfilename
    /proc/<pid>/mem    ->  mach_vm_read_overwrite

`task_for_pid` needs root: run the scripts with `sudo`.

Nothing is written into the target process.
"""
import ctypes as C
import re
import subprocess

from procmem import Memory

libc = C.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)

KERN_SUCCESS = 0
VM_PROT_READ = 1
VM_PROT_WRITE = 2
VM_PROT_EXECUTE = 4
VM_REGION_BASIC_INFO_64 = 9
# `vm_region_basic_info_64`, counted in words of four bytes.
VM_REGION_BASIC_INFO_COUNT_64 = 9
PATH_MAX = 1024

# Under Rosetta 2, every page the guest allocates carries the name of the
# translator. Those pages are the heap, not a file: the reason is in
# `heap_iter`, in `src/plugin.rs`.
ROSETTA = re.compile(r"rosetta|/oah/", re.I)

mach_task_self = C.c_uint.in_dll(libc, "mach_task_self_")

libc.task_for_pid.argtypes = [C.c_uint, C.c_int, C.POINTER(C.c_uint)]
libc.mach_vm_read_overwrite.argtypes = [C.c_uint, C.c_uint64, C.c_uint64,
                                        C.c_uint64, C.POINTER(C.c_uint64)]
libc.mach_vm_region.argtypes = [C.c_uint, C.POINTER(C.c_uint64), C.POINTER(C.c_uint64),
                                C.c_int, C.POINTER(C.c_int), C.POINTER(C.c_uint),
                                C.POINTER(C.c_uint)]
libc.proc_regionfilename.argtypes = [C.c_int, C.c_uint64, C.c_char_p, C.c_uint32]


class Proc(Memory):
    """A live process, opened for reading only."""

    def __init__(self, pid):
        self.pid = pid
        self.task = C.c_uint()
        status = libc.task_for_pid(mach_task_self, pid, C.byref(self.task))
        if status != KERN_SUCCESS:
            raise OSError("task_for_pid(%d) failed with %d: run with sudo" % (pid, status))

    def close(self):
        pass

    def read(self, address, size):
        if not 0 < address < 1 << 47 or size <= 0:
            return None
        buffer = C.create_string_buffer(size)
        read = C.c_uint64()
        status = libc.mach_vm_read_overwrite(self.task, address, size,
                                             C.addressof(buffer), C.byref(read))
        if status != KERN_SUCCESS or read.value == 0:
            return None
        return buffer.raw[:read.value]

    def maps(self):
        address = C.c_uint64(0)
        size = C.c_uint64()
        info = (C.c_int * VM_REGION_BASIC_INFO_COUNT_64)()
        count = C.c_uint(VM_REGION_BASIC_INFO_COUNT_64)
        object_name = C.c_uint()
        while True:
            count.value = VM_REGION_BASIC_INFO_COUNT_64
            status = libc.mach_vm_region(self.task, C.byref(address), C.byref(size),
                                         VM_REGION_BASIC_INFO_64, info,
                                         C.byref(count), C.byref(object_name))
            if status != KERN_SUCCESS:
                return
            start, end = address.value, address.value + size.value
            protection = info[0]
            # `__PAGEZERO` and the guard pages: nothing to read, and a
            # `__PAGEZERO` with the name of the executable would move its base
            # to zero.
            if protection:
                yield start, end, permissions(protection), self.path_at(start)
            address.value = end

    def path_at(self, address):
        path = C.create_string_buffer(PATH_MAX)
        length = libc.proc_regionfilename(self.pid, address, path, PATH_MAX)
        name = path.raw[:length].decode(errors="replace") if length > 0 else ""
        return "" if ROSETTA.search(name) else name


def permissions(protection):
    return "".join((
        "r" if protection & VM_PROT_READ else "-",
        "w" if protection & VM_PROT_WRITE else "-",
        "x" if protection & VM_PROT_EXECUTE else "-",
        "p",
    ))


def flash_pids(pattern):
    """The processes whose executable matches `pattern`: a Flash projector.

    ponytail: the executable only, not every file a process maps. The plugin of
    EternalTwin lives in a helper, whose pid `--pid` gives.
    """
    rx = re.compile(pattern, re.I)
    listing = subprocess.run(["ps", "-axo", "pid=,comm="], capture_output=True,
                             text=True, check=True).stdout
    return [int(pid) for pid, command in
            (line.strip().split(maxsplit=1) for line in listing.splitlines() if line.strip())
            if rx.search(command)]
