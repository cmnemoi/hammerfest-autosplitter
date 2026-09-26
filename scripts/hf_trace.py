#!/usr/bin/env python3
"""Times the start of a game, from the process to the official start.

The speedrun rule sets the start: *the timer begins when the loading text
disappears and fades in to level 0*. In EternalTwin that instant is the frame
where `GameMode.fl_lock` falls: `GameMechanics.onViewReady` attaches the view
and calls `GameMode.onLevelReady`, which unlocks, in the same frame. The black
screen ends there, within one frame.

The fade of the original SWF loader is **not** observable here. This script
also looks for it, and that search is what proved it absent: no table carrying
`fVersion` also carries `gameInst`, so EternalTwin does not use that loader.
The check is kept because it is the evidence.

Real time then reads with no outside clock:

    Chrono.update()  does  frameTimer = Std.getTimer()
    and it runs before the pause test and before the return on fl_lock,
    so frameTimer always advances, in real milliseconds.

    real time = frameTimer - frameTimer(at the start)

The heap scan therefore leaves the critical path. `GameMode.duration` runs
only from that same start, so it says how late a scan arrived, and the origin
is rebuilt exactly.

Timeline recorded, origin = the first sight of the plugin process:

    process       a --type=ppapi process appears  (it is born with the game)
    module        pepflashplayer.dll is loaded in it
    GameManager   the anchor is set               <- should come first
    GameMode      the game exists, gameChrono starts
    fade done     Loader.loading becomes null     <- absent in EternalTwin
    control       fl_lock falls, the game answers the keys  <- official start

Usage:
    hf_trace.py                    waits for a game, traces, Ctrl-C to finish
    hf_trace.py --out trace.csv    a full reading, one line per sample

A ten second pause during the trace measures, in one go, how `duration`,
`gameChrono` and `frameTimer` behave while paused.
"""
import argparse
import csv
import ctypes as C
import ctypes.wintypes as W
import sys
import time

import avm1
import hfmap
import winmem

PROC_NAMES = ("Eternaltwin.exe", "etwin.exe")
PLUGIN = r"pepflashplayer\.dll|libpepflashplayer\.so|PepperFlashPlayer"

# The GameManager anchor: `fVersion` is set by its constructor and no other
# class carries it. See src/reader/hammerfest.rs, the same resolution chain.
K_F_VERSION = hfmap.obf("fVersion")
K_CURRENT = hfmap.obf("current")
K_MANAGER = hfmap.obf("manager")

K_WORLD = hfmap.obf("world")
K_SET_NAME = hfmap.obf("setName")
K_CURRENT_ID = hfmap.obf("currentId")
K_CHRONO = hfmap.obf("gameChrono")
K_DURATION = hfmap.obf("duration")
K_FL_LOCK = hfmap.obf("fl_lock")
K_FL_PAUSE = hfmap.obf("fl_pause")
K_GAME_OVER = hfmap.obf("fl_gameOver")

# The Loader: the object of the outer SWF, the one that holds the loading
# screen. It owns `fVersion` too, so the same scan brings it back for free.
# `gameInst` tells them apart: the GameManager does not have it. `loading` is
# the clip of the loading screen, set to null by `Loader.mainGame` when the
# fade ends. In EternalTwin this object is absent, which is what we check.
K_GAME_INST = hfmap.obf("gameInst")
K_LOADING = hfmap.obf("loading")

K_FRAME_TIMER = hfmap.obf("frameTimer")
K_GAME_TIMER = hfmap.obf("gameTimer")
K_HALTED_TIMER = hfmap.obf("haltedTimer")
K_FL_STOP = hfmap.obf("fl_stop")

SECOND = 32          # Data.SECOND: game cycles per second
MAX_LEVEL = 256

FIELDS = ("t", "level", "fl_lock", "fl_stop", "fl_pause", "duration",
          "duration_s", "chrono_ms", "frame_timer", "game_timer",
          "halted_timer", "game_over", "fade_done")


# -- process ----------------------------------------------------------------

TH32CS_SNAPPROCESS = 0x02
INVALID_HANDLE_VALUE = C.c_void_p(-1).value


class PROCESSENTRY32W(C.Structure):
    _fields_ = [
        ("dwSize", W.DWORD),
        ("cntUsage", W.DWORD),
        ("th32ProcessID", W.DWORD),
        ("th32DefaultHeapID", C.POINTER(C.c_ulong)),
        ("th32ModuleID", W.DWORD),
        ("cntThreads", W.DWORD),
        ("th32ParentProcessID", W.DWORD),
        ("pcPriClassBase", C.c_long),
        ("dwFlags", W.DWORD),
        ("szExeFile", W.WCHAR * 260),
    ]


# Without restype, ctypes brings the HANDLE back in a signed 32 bit C int:
# the snapshot would be truncated and the comparison to INVALID_HANDLE_VALUE
# would be wrong.
winmem.k32.CreateToolhelp32Snapshot.argtypes = [W.DWORD, W.DWORD]
winmem.k32.CreateToolhelp32Snapshot.restype = W.HANDLE
winmem.k32.Process32FirstW.argtypes = [W.HANDLE, C.POINTER(PROCESSENTRY32W)]
winmem.k32.Process32NextW.argtypes = [W.HANDLE, C.POINTER(PROCESSENTRY32W)]


def pids_by_name(names):
    """PIDs of the processes carrying one of these names.

    Toolhelp32 rather than `winmem.ppapi_pids`, which goes through PowerShell.
    A WMI query costs hundreds of milliseconds, so it cannot date the birth of
    a process. Here the cost is about one millisecond, which allows polling at
    50 Hz -- and that is exactly what the autosplitter does, since it lists
    the processes on every tick.
    """
    low = {n.lower() for n in names}
    snap = winmem.k32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    if snap == INVALID_HANDLE_VALUE:
        return []
    out = []
    try:
        e = PROCESSENTRY32W()
        e.dwSize = C.sizeof(e)
        ok = winmem.k32.Process32FirstW(snap, C.byref(e))
        while ok:
            if e.szExeFile.lower() in low:
                out.append(e.th32ProcessID)
            ok = winmem.k32.Process32NextW(snap, C.byref(e))
    finally:
        winmem.k32.CloseHandle(snap)
    return out


def wait_for_plugin(poll=0.02):
    """Waits for a game to open. -> (proc, module, t_process, t_module).

    Processes that are already alive are set aside at once: the EternalTwin
    application runs before the game, and only the plugin process is born with
    it. That is what gives a birth instant we can use as the time origin.
    """
    known = set(pids_by_name(PROC_NAMES))
    rejected = set()
    seen = {}

    while True:
        now = time.perf_counter()
        for pid in pids_by_name(PROC_NAMES):
            if pid in known or pid in rejected:
                continue
            seen.setdefault(pid, now)
            try:
                p = winmem.Proc(pid)
            except OSError:
                continue
            m = p.module(PLUGIN)
            if m is None:
                # Not loaded yet, or this process will never carry the
                # plugin. We cannot tell the two apart, so we come back: the
                # module appears a few tens of milliseconds after the
                # process.
                p.close()
                if now - seen[pid] > 5.0:
                    rejected.add(pid)
                continue
            return p, m, seen[pid], now
        time.sleep(poll)


# -- resolution --------------------------------------------------------------

def derive_so_tbl(av, tbl, min_votes=2):
    """The `ScriptObject -> table` offset, by a vote on the `tbl` properties.

    The ordinary way derives this offset from a cross reference:
    `GameManager.current` points at a mode whose `manager` field points back at
    the GameManager. That proves the candidate, but it needs a mode to exist
    already -- so it forbids setting the anchor before the game, which is
    exactly what we want to do.

    Without a cross reference, one object is not enough: several offsets can
    lead to a plausible table. But the right offset is the same for every
    ScriptObject in the process, while a wrong one is right only by accident.
    So we let every object property of the table vote, and we demand a clear
    majority.

    -> (offset, votes, objects examined), offset is None if the majority is
    missing.
    """
    votes = {}
    objects = 0
    for _, atom, _ in av.entries(tbl):
        so = atom & ~7
        if not so or not av.in_module(av.p.u64(so)):
            continue
        objects += 1
        for x in avm1.SO_TBL_CANDS:
            t = av.p.u64(so + x)
            if t and av.p.u64(t) == av.L.tbl_vt and av.capacity(t):
                votes[x] = votes.get(x, 0) + 1
    if not votes:
        return None, 0, objects
    best = max(votes, key=votes.get)
    return (best if votes[best] >= min_votes else None), votes[best], objects


def find_anchors(av):
    """Sets both anchors. -> (GameManager, Loader, comment).

    `fVersion` is set by the GameManager constructor, and the Loader carries it
    too, so one scan gives both objects. The Loader is the one that owns
    `gameInst`, the game instance it created.

    The GameManager leads to the game. The Loader, if it exists, leads to the
    end of the loading fade. In EternalTwin it does not exist.
    """
    tables = av.derive(K_F_VERSION)
    if not tables:
        return None, None, "no fVersion string in the heap", []

    # The Loader carries `fVersion` like the GameManager, so it comes out of
    # the same scan. `gameInst` -- the game instance it created -- tells them
    # apart.
    loader = next((t for t in tables if av.slot(t, K_GAME_INST) is not None),
                  None)

    notes = []
    for tbl in tables:
        if tbl == loader:
            continue
        current = av.get(tbl, K_CURRENT)
        if current:
            mode = av.derive_script_object(current, K_MANAGER)
            if mode is not None and av.child(mode, K_MANAGER) == tbl:
                return tbl, loader, "reference croisee", tables
        x, votes, objects = derive_so_tbl(av, tbl)
        if x is not None:
            av.L.so_tbl = x
            return tbl, loader, ("vote +0x%02x, %d votes over %d objects"
                                 % (x, votes, objects)), tables
        notes.append("table 0x%x: current %s, %d votes over %d objects"
                     % (tbl, "set" if current else "empty", votes, objects))
    return None, loader, "; ".join(notes), tables


def game_mode_of(av, manager):
    """`GameManager.current`, if it is a game and not a menu.

    A GameMode is known by its `gameChrono`: menu modes have a `world`, but no
    game clock.
    """
    mode = av.table_of(av.get(manager, K_CURRENT))
    if mode is None:
        return None
    chrono = av.child(mode, K_CHRONO)
    if chrono is None or av.get(chrono, K_FRAME_TIMER) is None:
        return None
    return mode if av.child(mode, K_WORLD) is not None else None


# -- sampling ----------------------------------------------------------------

def fade_done(av, loader):
    """Has the loading screen finished fading out?

    `Loader.mainGame` removes the clip and sets `loading` to null when its
    alpha reaches zero. So the field stops pointing at an object at the exact
    instant the fade ends. This is only observable when the original loader is
    present, which is not the case in EternalTwin.
    """
    if loader is None:
        return None
    atom = av.get(loader, K_LOADING)
    if atom is None:
        return True
    return (atom & 7) not in (avm1.TAG_OBJECT, avm1.TAG_NATIVE)


def sample(av, mode, loader=None):
    """One full reading, or None if the mode is no longer readable."""
    world = av.child(mode, K_WORLD)
    chrono = av.child(mode, K_CHRONO)
    if world is None or chrono is None:
        return None

    frame = av.as_int(av.get(chrono, K_FRAME_TIMER))
    game = av.as_int(av.get(chrono, K_GAME_TIMER))
    halted = av.as_int(av.get(chrono, K_HALTED_TIMER))
    stop = av.as_bool(av.get(chrono, K_FL_STOP))
    if frame is None:
        return None
    # Chrono.get(): fl_stop ? haltedTimer : floor(frameTimer-gameTimer)
    chrono_ms = halted if stop else (None if game is None else frame - game)

    duration = av.as_number(av.get(mode, K_DURATION))
    return {
        "fade_done": fade_done(av, loader),
        "level": av.as_int(av.get(world, K_CURRENT_ID), bound=MAX_LEVEL),
        "fl_lock": av.as_bool(av.get(mode, K_FL_LOCK)),
        "fl_stop": stop,
        "fl_pause": av.as_bool(av.get(mode, K_FL_PAUSE)),
        "duration": duration,
        "duration_s": None if duration is None else duration / SECOND,
        "chrono_ms": chrono_ms,
        "frame_timer": frame,
        "game_timer": game,
        "halted_timer": halted,
        "game_over": av.as_bool(av.get(mode, K_GAME_OVER)),
    }


class Events:
    """The instants that matter, dated from the birth of the process."""

    def __init__(self, t0):
        self.t0 = t0
        self.at = {}
        self.control = None      # the reading taken at the unlock
        self.fade = None         # the reading taken at the end of the fade
        # Did we see the mode locked before we saw it unlocked? If not, the
        # transition happened before the first read, and the start instant
        # must be rebuilt from `duration`.
        self.saw_locked = False

    def mark(self, name, t=None):
        if name in self.at:
            return False
        self.at[name] = (t or time.perf_counter()) - self.t0
        return True

    def show(self, name, label, extra=""):
        if name in self.at:
            print("  %-14s %7.3f s  %s" % (label, self.at[name], extra))


def trace(args):
    print("waiting for a game: open Hammerfest now")
    proc, module, t_process, t_module = wait_for_plugin()
    base, end, path = module
    ev = Events(t_process)
    ev.mark("process", t_process)
    ev.mark("module", t_module)
    print("\npid            %d" % proc.pid)
    print("plugin         %s" % path)
    print("process -> module  %.3f s" % (t_module - t_process))

    av = avm1.Avm1(proc, (base, end), proc.regions())

    # -- set the anchor, scanning again while it is missing
    manager = None
    scans = 0
    seen_starts = set()
    while manager is None:
        t = time.perf_counter()
        # The heap grows while the SWF loads. Read the regions again, and
        # start with the new ones. The SWF objects are born in memory that has
        # just been committed, so that is where to look first. Re-reading the
        # hundred MiB already seen would delay the find by all the time needed
        # to read them.
        regions = proc.regions()
        fresh = [r for r in regions if r[0] not in seen_starts]
        av.heaps = fresh + [r for r in regions if r[0] in seen_starts]
        seen_starts = {r[0] for r in regions}
        av._key_atoms.clear()
        mio = sum(b - a for a, b in av.heaps) / (1 << 20)
        try:
            manager, loader, how, tables = find_anchors(av)
        except OSError:
            print("  process gone before the resolution")
            return
        scans += 1
        print("  scan %2d: %5.1f MiB (%d new regions), %.2f s -> %s%s"
              % (scans, mio, len(fresh), time.perf_counter() - t,
                 "GameManager 0x%x, %s" % (manager, how) if manager else how,
                 ", Loader 0x%x" % loader if loader else ""))
    ev.mark("manager")
    print("\nlayout AVM1 derive :\n%s" % av.L)

    rows = []
    mode = None
    last = None
    t_dead = None
    print("\ntrace running, Ctrl-C to stop\n")

    try:
        while True:
            now = time.perf_counter()
            if mode is None:
                try:
                    mode = game_mode_of(av, manager)
                except OSError:
                    break
                if mode is None:
                    # No game: a menu, or it does not exist yet. Wait with
                    # no limit while we have never seen one: the anchor can
                    # now be set before the launch.
                    if t_dead and "gamemode" in ev.at and now - t_dead > 3.0:
                        break
                    time.sleep(args.interval)
                    continue
                t_dead = None
                ev.mark("gamemode")
                print("  GameMode 0x%x" % mode)

            try:
                s = sample(av, mode, loader)
            except OSError:
                break
            if s is None:
                mode, t_dead = None, now
                continue

            s["t"] = now - ev.t0
            rows.append(s)
            report(ev, last, s)
            if s["game_over"]:
                ev.mark("game over")
                break
            last = s
            time.sleep(args.interval)
    except KeyboardInterrupt:
        print("\n  arret demande")

    summary(ev, rows)
    if args.out:
        with open(args.out, "w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, FIELDS)
            w.writeheader()
            w.writerows({k: r.get(k) for k in FIELDS} for r in rows)
        print("\n%d echantillons -> %s" % (len(rows), args.out))


def report(ev, last, s):
    """Announces the transitions as the trace goes."""
    if s["fl_lock"] is True:
        ev.saw_locked = True
    if s["fl_lock"] is False and ev.mark("control"):
        ev.control = s
        print("  CONTROL        fl_lock=false  duration=%s  gameChrono=%s ms"
              % (s["duration"], s["chrono_ms"]))
    if s["fade_done"] and ev.mark("fondu"):
        ev.fade = s
        print("  FADE DONE      official start  gameChrono=%s ms  frameTimer=%s"
              % (s["chrono_ms"], s["frame_timer"]))
    if s["duration"] and ev.mark("duration"):
        print("  duration > 0   %.3f cycles" % s["duration"])
    if last is None:
        return
    if s["fl_pause"] != last["fl_pause"]:
        print("  %-14s t=%.3f s  duration=%s  gameChrono=%s ms"
              % ("PAUSE" if s["fl_pause"] else "resume",
                 s["t"], round(s["duration"] or 0, 1), s["chrono_ms"]))
    if s["level"] != last["level"]:
        print("  level %s -> %-3s t=%.3f s  gameChrono=%s ms"
              % (last["level"], s["level"], s["t"], s["chrono_ms"]))


def summary(ev, rows):
    """The timeline, and the verdict that drives the architecture."""
    print("\ntimeline, origin = the first sight of the process:")
    for name, label in (("process", "process"), ("module", "module"),
                        ("manager", "GameManager"), ("gamemode", "GameMode"),
                        ("control", "control"), ("fondu", "fade done"),
                        ("game over", "game over")):
        ev.show(name, label)

    # -- the end of the loading fade, absent in EternalTwin
    print("\nend of the loading fade:")
    if ev.fade is None:
        print("  never observed: the Loader was not found.")
    elif "fondu" in ev.at and ev.at["fondu"] > ev.at.get("manager", 0) + 0.05:
        f = ev.fade
        print("  observed at %.3f s." % ev.at["fondu"])
        print("  gameChrono     %s ms   (since the Chrono was built)"
              % f["chrono_ms"])
        print("  frameTimer     %s ms   <- origin of real time"
              % f["frame_timer"])
        last_row = rows[-1]
        if last_row["frame_timer"] and f["frame_timer"]:
            real = (last_row["frame_timer"] - f["frame_timer"]) / 1000
            measured = last_row["t"] - ev.at["fondu"]
            print("  real time at the last read: %.3f s"
                  " (measured on my side: %.3f s, difference %+.0f ms)"
                  % (real, measured, (real - measured) * 1000))
    else:
        print("  already over at the first read: the anchor arrived too late"
              " to date it.")
        if rows and rows[0]["chrono_ms"] is not None:
            print("  gameChrono was already %s ms." % rows[0]["chrono_ms"])

    if "control" in ev.at:
        # `duration` only moves once the mode is unlocked, so its value at
        # the first read says how late we arrived.
        c = ev.control
        late = (c["duration"] or 0) / SECOND
        chrono = c["chrono_ms"]
        if chrono is not None:
            chrono = round(chrono - late * 1000)
        print("\nofficial start -- fl_lock falls:")
        print("  %.3f s%s" % (ev.at["control"] - late,
                              "" if ev.saw_locked and late < 0.1
                              else "  (rebuilt from duration)"))
        print("  gameChrono     %s ms" % chrono)

    pauses = [r for r in rows if r["fl_pause"] and r["duration"] is not None]
    if len(pauses) > 1:
        d = pauses[-1]["duration"] - pauses[0]["duration"]
        g = (pauses[-1]["chrono_ms"] or 0) - (pauses[0]["chrono_ms"] or 0)
        fr = (pauses[-1]["frame_timer"] or 0) - (pauses[0]["frame_timer"] or 0)
        real_seconds = pauses[-1]["t"] - pauses[0]["t"]
        print("\nduring the pause (%.1f real s):" % real_seconds)
        print("  duration    %+.1f cycles = %+.2f s" % (d, d / SECOND))
        print("  gameChrono  %+d ms" % g)
        print("  frameTimer  %+d ms   <- must follow real time" % fr)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", default="trace.csv", help="CSV file of the reading")
    ap.add_argument("--interval", type=float, default=0.02,
                    help="periode d'echantillonnage, en secondes")
    args = ap.parse_args()
    if sys.platform != "win32":
        sys.exit("ce script utilise Toolhelp32 : Windows seulement")
    # A trace is read while it happens. Without this, Python buffers the
    # output as soon as it is not a terminal.
    sys.stdout.reconfigure(line_buffering=True)
    trace(args)


if __name__ == "__main__":
    main()
