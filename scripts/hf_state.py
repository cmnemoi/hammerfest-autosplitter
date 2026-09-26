#!/usr/bin/env python3
"""Reads the current Hammerfest level and clock from the plugin memory.

The resolution chain, with no hard coded address:

    process --type=ppapi          the Flash plugin, born when the SWF loads
    pepflashplayer.dll            ASLR -> module base
    scan "]=[]8" in the heap      `world`, obfuscated key known from hf.map.json
    -> AVM1 layout derived        vtables, stride and offsets, measured one by one
    -> GameMode table             the one that owns this key
    GameMode["]=[]8"]             -> world: GameMechanics
    world[" h;+A("]               -> setName == "]R;5E" = `xml_adventure`
    world["-BBEO"]                -> currentId: the level
    GameMode["8qkdA"]             -> gameChrono: Chrono
    Chrono[...]                   -> fl_stop ? haltedTimer : frameTimer-gameTimer

The Flash projector carries the same AVM1: under Linux its executable is the
module, and `--pid` names it.

Usage:
    hf_state.py                   one reading
    hf_state.py --watch           follows the level transitions
    hf_state.py --dump            prints GameMode, world and Chrono in full
    hf_state.py --pid 1234        in this process: a projector, say
    hf_state.py --capture fixtures/linux-pepper-flash
                                  in a capture, with no game running
"""
import argparse
import sys
import time

import avm1
import hfmap
import platform_memory
import procmem

# The plugin of EternalTwin, and the projector: `flashplayer` under Linux,
# `flashplayer.exe` or `flashplayer_32_sa.exe` under Windows.
PLUGIN = (r"pepflashplayer\.dll|libpepflashplayer\.so|PepperFlashPlayer"
          r"|[/\\]flashplayer(_32_sa)?(\.exe)?$")

K_WORLD = hfmap.obf("world")
K_CURRENT_ID = hfmap.obf("currentId")
K_PREVIOUS_ID = hfmap.obf("_previousId")
K_SET_NAME = hfmap.obf("setName")
K_CHRONO = hfmap.obf("gameChrono")
K_DURATION = hfmap.obf("duration")
K_CURRENT_DIM = hfmap.obf("currentDim")
K_GAME_OVER = hfmap.obf("fl_gameOver")
K_PAUSE = hfmap.obf("fl_pause")

K_FRAME_TIMER = hfmap.obf("frameTimer")
K_GAME_TIMER = hfmap.obf("gameTimer")
K_HALTED_TIMER = hfmap.obf("haltedTimer")
K_FL_STOP = hfmap.obf("fl_stop")

# The end of the run. `ScriptEngine.codeTrigger` case 4 -- "sortie par
# l'ascenseur" -- writes `endModeTimer` in the frame where the player enters
# the elevator, and no other line of the adventure writes it. Case 3, the
# fruit release, also takes the controls away, but it leaves this timer at
# zero. That is what separates the end of the run from the cinematic before
# it.
K_FL_LOCK = hfmap.obf("fl_lock")
K_END_MODE = hfmap.obf("endModeTimer")
K_SCRIPT_ENGINE = hfmap.obf("scriptEngine")
K_ELEVATOR_OPEN = hfmap.obf("fl_elevatorOpen")

SECOND = 32          # Data.SECOND: game cycles per second
MAX_LEVEL = 256      # plausibility bound for currentId
END_MODE_CYCLES = SECOND * 14   # Data.SECOND*14, the value case 4 writes


class Hammerfest:
    """A Hammerfest state resolved in a live process, or in a capture.

    Nothing is cached beyond the GameMode table. The game rebuilds its objects
    between two games, and an abandoned slot stays readable while holding a
    perfectly plausible value. Every read walks GameMode -> world -> currentId
    again, which costs microseconds and cannot go stale in silence.
    """

    def __init__(self, memory):
        self.p = memory
        self.pid = memory.pid
        m = self.p.module(PLUGIN)
        if m is None:
            raise RuntimeError("no Flash player in pid %d" % self.pid)
        self.base, self.end, self.plugin_path = m
        self.heaps = self.p.regions()
        self.av = avm1.Avm1(self.p, (self.base, self.end), self.heaps)
        self.gm = None
        self.set_name = None

    # -- resolution --------------------------------------------------------
    def resolve(self, verbose=False):
        """Finds the GameMode table. Costs one full heap scan."""
        log = print if verbose else (lambda *_: None)
        self.heaps = self.p.regions()
        self.av.heaps = self.heaps
        self.av._key_atoms.clear()

        found = []
        for tbl in self.av.derive(K_WORLD, verbose=verbose):
            world_atom = self.av.get(tbl, K_WORLD)
            wtbl = (self.av.table_of(world_atom)
                    or self.av.derive_script_object(world_atom, K_SET_NAME))
            if wtbl is None:
                log("  table 0x%x: `world` leads to no object, skipped" % tbl)
                continue
            raw = self.av.as_string(self.av.get(wtbl, K_SET_NAME))
            name = hfmap.clear(raw) if raw else None
            level = self.av.as_int(self.av.get(wtbl, K_CURRENT_ID), bound=MAX_LEVEL)
            # `world` is not enough to identify the GameMode: View objects
            # carry one too, and they point at the same GameMechanics. Only
            # the GameMode also owns a gameChrono.
            ctbl = self.av.child(tbl, K_CHRONO)
            has_chrono = ctbl is not None and self.av.get(ctbl, K_FRAME_TIMER) is not None
            log("  table 0x%x -> world 0x%x  setName=%r (%r)  currentId=%r"
                "  gameChrono=%s"
                % (tbl, wtbl, raw, name, level, "yes" if has_chrono else "no"))
            if name and name.startswith("xml_") and level is not None and has_chrono:
                found.append((tbl, name))

        if not found:
            return False
        if len(found) > 1:
            print("warning: %d live GameModes (%s), taking the first one"
                  % (len(found), ", ".join(hex(t) for t, _ in found)),
                  file=sys.stderr)
        self.gm, self.set_name = found[0]
        return True

    # -- reads ---------------------------------------------------------------
    def world(self):
        return self.av.child(self.gm, K_WORLD)

    def chrono(self):
        return self.av.child(self.gm, K_CHRONO)

    def script_engine(self):
        """GameMechanics.scriptEngine: the script of the level in progress.

        `GameMechanics.buildLevel` builds a new one for every level. So
        `fl_elevatorOpen` only means something inside the last level, and it
        goes back to false in the same frame that ends the run.
        """
        w = self.world()
        return None if w is None else self.av.child(w, K_SCRIPT_ENGINE)

    def chrono_ms(self):
        """Chrono.get(): game milliseconds, frozen during pauses."""
        c = self.chrono()
        if c is None:
            return None
        if self.av.as_bool(self.av.get(c, K_FL_STOP)):
            return self.av.as_int(self.av.get(c, K_HALTED_TIMER))
        frame = self.av.as_int(self.av.get(c, K_FRAME_TIMER))
        game = self.av.as_int(self.av.get(c, K_GAME_TIMER))
        return None if frame is None or game is None else frame - game

    def frame_timer(self):
        """Chrono.frameTimer: the heartbeat of the GameMode.

        `GameMode.main()` calls `gameChrono.update()` unconditionally, and
        before the pause test. So this counter advances every frame while this
        GameMode is the one that runs. Frozen, the object is dead -- and a dead
        object stays readable, with an old level and an old clock.
        """
        c = self.chrono()
        return None if c is None else self.av.as_int(self.av.get(c, K_FRAME_TIMER))

    def snapshot(self):
        w = self.world()
        if w is None:
            return None
        se = self.script_engine()
        return {
            "level": self.av.as_int(self.av.get(w, K_CURRENT_ID), bound=MAX_LEVEL),
            "previous": self.av.as_int(self.av.get(w, K_PREVIOUS_ID), bound=MAX_LEVEL),
            "set": hfmap.clear(self.av.as_string(self.av.get(w, K_SET_NAME)) or ""),
            "chrono_ms": self.chrono_ms(),
            "frame": self.frame_timer(),
            "duration": self.av.as_number(self.av.get(self.gm, K_DURATION)),
            "dim": self.av.as_int(self.av.get(self.gm, K_CURRENT_DIM), bound=64),
            "paused": self.av.as_bool(self.av.get(self.gm, K_PAUSE)),
            "game_over": self.av.as_bool(self.av.get(self.gm, K_GAME_OVER)),
            "locked": self.av.as_bool(self.av.get(self.gm, K_FL_LOCK)),
            "end_mode": self.av.as_number(self.av.get(self.gm, K_END_MODE)),
            "elevator_open": None if se is None else self.av.as_bool(
                self.av.get(se, K_ELEVATOR_OPEN)),
        }

    # -- inspection --------------------------------------------------------
    def dump(self, tbl, title):
        print("\n%s: table 0x%x, capacity %s" % (title, tbl, self.av.capacity(tbl)))
        for name, atom, addr in self.av.entries(tbl):
            c = hfmap.clear(name)
            label = name if c == name else "%s -> %s" % (name, c)
            print("  0x%x  %-32s tag=%d  %s"
                  % (addr, label, self.av.tag(atom) or 0, self.av.describe(atom)))


def attach(pid=None, verbose=True, capture=None):
    if capture:
        memory = procmem.Recorded(capture)
    else:
        pids = [pid] if pid else platform_memory.flash_pids(PLUGIN)
        if not pids:
            return None
        memory = platform_memory.Proc(pids[0])
    hf = Hammerfest(memory)
    if verbose:
        print("pid            %d" % hf.pid)
        print("plugin         %s" % hf.plugin_path)
        print("module         0x%x .. 0x%x" % (hf.base, hf.end))
        print("private rw heap %d regions, %.1f MiB"
              % (len(hf.heaps), sum(b - a for a, b in hf.heaps) / (1 << 20)))
        print("\nresolution, anchor %r = `world`:" % K_WORLD)
    t0 = time.time()
    ok = hf.resolve(verbose=verbose)
    if verbose:
        print("  -> %s en %.2f s"
              % ("GameMode 0x%x" % hf.gm if ok else "FAILED", time.time() - t0))
        if ok:
            print("\nAVM1 layout derived:\n%s" % hf.av.L)
    return hf if ok else None


def show(s):
    print("\nset            %s (dimension %s)" % (s["set"], s["dim"]))
    print("level          %s   (previous %s)" % (s["level"], s["previous"]))
    print("clock          %s ms%s"
          % (s["chrono_ms"], "   [paused]" if s["paused"] else ""))
    print("frameTimer     %s%s" % (s["frame"], "   [game over]" if s["game_over"] else ""))
    d = s["duration"]
    print("duration       %s cycles%s"
          % (round(d, 1) if d else d, "  = %.1f s" % (d / SECOND) if d else ""))
    print("fl_lock        %s" % s["locked"])
    e = s["end_mode"]
    print("endModeTimer   %s cycles%s"
          % (round(e, 1) if e is not None else e,
             "   <- ELEVATOR: the run is over" if e and e > 0 else ""))
    print("fl_elevatorOpen %s" % s["elevator_open"])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int)
    ap.add_argument("--watch", action="store_true")
    ap.add_argument("--dump", action="store_true")
    ap.add_argument("--interval", type=float, default=0.03)
    ap.add_argument("--capture", help="a capture directory, read instead of a process")
    a = ap.parse_args()

    hf = attach(a.pid, capture=a.capture)
    if hf is None:
        sys.exit("\nnot resolved: does a Flash player run, and are you in a "
                 "game?")

    if a.dump:
        hf.dump(hf.gm, "GameMode")
        w, c = hf.world(), hf.chrono()
        if w:
            hf.dump(w, "world (GameMechanics)")
        if c:
            hf.dump(c, "gameChrono (Chrono)")
        se = hf.script_engine()
        if se:
            hf.dump(se, "scriptEngine (ScriptEngine)")
        return

    show(hf.snapshot())
    if not a.watch:
        return

    print("\nfollowing the transitions, Ctrl-C to stop")
    last = None
    last_frame = None
    last_end = None
    stalled = 0
    while True:
        if hf is None:
            hf = attach(a.pid, verbose=False)
            if hf is None:
                time.sleep(1)
                continue
        try:
            s = hf.snapshot()
        except OSError:
            s = None
        if s is None or s["level"] is None:
            print("  lost -> resolving again")
            hf = None
            last = None
            continue
        # `last` only moves on a level change, so the heartbeat is followed
        # separately, from one read to the next.
        if s["frame"] == last_frame:
            stalled += 1
            if stalled == 60:
                print("  frameTimer frozen for 60 reads: dead GameMode?")
        else:
            stalled = 0
        last_frame = s["frame"]
        # The end of the run. Announce the frame, not the countdown: the value
        # only tells us how long the cinematic that follows still has to run.
        end = s["end_mode"] or 0
        if end > 0 and (last_end or 0) <= 0:
            print("  ELEVATOR  endModeTimer 0 -> %.1f cycles (%.1f s)"
                  "  level %s  clock %s ms  elevatorOpen %s"
                  % (end, end / SECOND, s["level"], s["chrono_ms"],
                     s["elevator_open"]))
        last_end = end
        # The dimension and the level come from two objects: `currentDim`
        # from the GameMode, `currentId` from the world. Follow all three, to
        # see whether a read can catch one of them ahead of the others.
        where = (s["set"], s["dim"], s["level"])
        if last is None or where != last_where:
            print("  %-13s dim %-3s level %-3s frame %s  clock %s ms"
                  % (where + (s["frame"], s["chrono_ms"])))
            last, last_where = s, where
        time.sleep(a.interval)


if __name__ == "__main__":
    main()
