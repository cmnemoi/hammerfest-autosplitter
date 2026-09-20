#!/usr/bin/env python3
"""Lit le niveau courant et le chrono de Hammerfest dans la memoire du plugin.

Chaine de resolution, sans aucune adresse en dur :

    process --type=ppapi          le plugin Flash, cree au chargement du SWF
    pepflashplayer.dll            ASLR -> base du module
    scan "]=[]8" dans le tas      `world`, clef obfusquee connue par hf.map.json
    -> layout AVM1 derive         vtables, pas et offsets, mesures un par un
    -> table de GameMode          celle qui possede cette clef
    GameMode["]=[]8"]             -> world : GameMechanics
    world[" h;+A("]               -> setName == "]R;5E" = `xml_adventure`
    world["-BBEO"]                -> currentId : le niveau
    GameMode["8qkdA"]             -> gameChrono : Chrono
    Chrono[...]                   -> fl_stop ? haltedTimer : frameTimer-gameTimer

Usage:
    hf_state.py                   un releve
    hf_state.py --watch           suit les transitions de niveau
    hf_state.py --dump            affiche GameMode, world et Chrono en entier
"""
import argparse
import sys
import time

import avm1
import hfmap
import winmem

PLUGIN = r"pepflashplayer\.dll|libpepflashplayer\.so|PepperFlashPlayer"

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

SECOND = 32          # Data.SECOND : cycles de jeu par seconde
MAX_LEVEL = 256      # borne de plausibilite pour currentId


class Hammerfest:
    """Un etat Hammerfest resolu dans un process vivant.

    Rien n'est cache au-dela de la table GameMode : le jeu reconstruit ses
    objets entre deux parties, et un slot abandonne reste lisible en contenant
    une valeur parfaitement plausible. Chaque lecture re-parcourt
    GameMode -> world -> currentId, ce qui coute des microsecondes et ne peut
    pas devenir obsolete en silence.
    """

    def __init__(self, pid):
        self.pid = pid
        self.p = winmem.Proc(pid)
        m = self.p.module(PLUGIN)
        if m is None:
            raise RuntimeError("pepflashplayer.dll absent du pid %d" % pid)
        self.base, self.end, self.plugin_path = m
        self.heaps = self.p.regions()
        self.av = avm1.Avm1(self.p, (self.base, self.end), self.heaps)
        self.gm = None
        self.set_name = None

    # -- resolution --------------------------------------------------------
    def resolve(self, verbose=False):
        """Trouve la table GameMode. Coute un scan complet du tas."""
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
                log("  table 0x%x : `world` ne mene pas a un objet, ignoree" % tbl)
                continue
            raw = self.av.as_string(self.av.get(wtbl, K_SET_NAME))
            name = hfmap.clear(raw) if raw else None
            level = self.av.as_int(self.av.get(wtbl, K_CURRENT_ID), bound=MAX_LEVEL)
            # `world` ne suffit pas a identifier le GameMode : les objets View
            # en portent un aussi, et pointent vers le meme GameMechanics. Seul
            # le GameMode possede en plus un gameChrono.
            ctbl = self.av.child(tbl, K_CHRONO)
            has_chrono = ctbl is not None and self.av.get(ctbl, K_FRAME_TIMER) is not None
            log("  table 0x%x -> world 0x%x  setName=%r (%r)  currentId=%r"
                "  gameChrono=%s"
                % (tbl, wtbl, raw, name, level, "oui" if has_chrono else "non"))
            if name and name.startswith("xml_") and level is not None and has_chrono:
                found.append((tbl, name))

        if not found:
            return False
        if len(found) > 1:
            print("attention: %d GameMode vivants (%s), on prend le premier"
                  % (len(found), ", ".join(hex(t) for t, _ in found)),
                  file=sys.stderr)
        self.gm, self.set_name = found[0]
        return True

    # -- lectures ----------------------------------------------------------
    def world(self):
        return self.av.child(self.gm, K_WORLD)

    def chrono(self):
        return self.av.child(self.gm, K_CHRONO)

    def chrono_ms(self):
        """Chrono.get() : millisecondes de jeu, figees pendant les pauses."""
        c = self.chrono()
        if c is None:
            return None
        if self.av.as_bool(self.av.get(c, K_FL_STOP)):
            return self.av.as_int(self.av.get(c, K_HALTED_TIMER))
        frame = self.av.as_int(self.av.get(c, K_FRAME_TIMER))
        game = self.av.as_int(self.av.get(c, K_GAME_TIMER))
        return None if frame is None or game is None else frame - game

    def frame_timer(self):
        """Chrono.frameTimer : le battement de coeur du GameMode.

        `GameMode.main()` appelle `gameChrono.update()` inconditionnellement et
        avant le test de pause, donc ce compteur avance a chaque frame tant que
        ce GameMode est celui qui tourne. Fige, l'objet est mort -- et un objet
        mort reste lisible, avec un niveau et un chrono d'avant.
        """
        c = self.chrono()
        return None if c is None else self.av.as_int(self.av.get(c, K_FRAME_TIMER))

    def snapshot(self):
        w = self.world()
        if w is None:
            return None
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
        }

    # -- inspection --------------------------------------------------------
    def dump(self, tbl, title):
        print("\n%s : table 0x%x, capacite %s" % (title, tbl, self.av.capacity(tbl)))
        for name, atom, addr in self.av.entries(tbl):
            c = hfmap.clear(name)
            label = name if c == name else "%s -> %s" % (name, c)
            print("  0x%x  %-32s tag=%d  %s"
                  % (addr, label, self.av.tag(atom) or 0, self.av.describe(atom)))


def attach(pid=None, verbose=True):
    pids = [pid] if pid else winmem.ppapi_pids()
    if not pids:
        return None
    hf = Hammerfest(pids[0])
    if verbose:
        print("pid            %d" % hf.pid)
        print("plugin         %s" % hf.plugin_path)
        print("module         0x%x .. 0x%x" % (hf.base, hf.end))
        print("tas prive rw   %d regions, %.1f Mio"
              % (len(hf.heaps), sum(b - a for a, b in hf.heaps) / (1 << 20)))
        print("\nresolution, ancre %r = `world` :" % K_WORLD)
    t0 = time.time()
    ok = hf.resolve(verbose=verbose)
    if verbose:
        print("  -> %s en %.2f s"
              % ("GameMode 0x%x" % hf.gm if ok else "ECHEC", time.time() - t0))
        if ok:
            print("\nlayout AVM1 derive :\n%s" % hf.av.L)
    return hf if ok else None


def show(s):
    print("\nset            %s (dimension %s)" % (s["set"], s["dim"]))
    print("niveau         %s   (precedent %s)" % (s["level"], s["previous"]))
    print("chrono         %s ms%s"
          % (s["chrono_ms"], "   [en pause]" if s["paused"] else ""))
    print("frameTimer     %s%s" % (s["frame"], "   [game over]" if s["game_over"] else ""))
    d = s["duration"]
    print("duration       %s cycles%s"
          % (round(d, 1) if d else d, "  = %.1f s" % (d / SECOND) if d else ""))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pid", type=int)
    ap.add_argument("--watch", action="store_true")
    ap.add_argument("--dump", action="store_true")
    ap.add_argument("--interval", type=float, default=0.03)
    a = ap.parse_args()

    hf = attach(a.pid)
    if hf is None:
        sys.exit("\nnon resolu : le process --type=ppapi existe-t-il, et "
                 "es-tu bien dans une partie ?")

    if a.dump:
        hf.dump(hf.gm, "GameMode")
        w, c = hf.world(), hf.chrono()
        if w:
            hf.dump(w, "world (GameMechanics)")
        if c:
            hf.dump(c, "gameChrono (Chrono)")
        return

    show(hf.snapshot())
    if not a.watch:
        return

    print("\nsuivi des transitions, Ctrl-C pour arreter")
    last = None
    last_frame = None
    stalled = 0
    while True:
        try:
            s = hf.snapshot()
        except OSError:
            s = None
        if s is None or s["level"] is None:
            print("  perdu -> re-resolution")
            hf = attach(a.pid, verbose=False)
            if hf is None:
                time.sleep(1)
                continue
            last = None
            continue
        # `last` ne bouge qu'au changement de niveau : le battement de coeur se
        # suit donc a part, d'une lecture a l'autre.
        if s["frame"] == last_frame:
            stalled += 1
            if stalled == 60:
                print("  frameTimer fige depuis 60 lectures : GameMode mort ?")
        else:
            stalled = 0
        last_frame = s["frame"]
        if last is None:
            print("  niveau %-3s chrono %s ms" % (s["level"], s["chrono_ms"]))
            last = s
        elif s["level"] != last["level"]:
            print("  niveau %s -> %-3s chrono %s ms  (+%s ms)"
                  % (last["level"], s["level"], s["chrono_ms"],
                     (s["chrono_ms"] or 0) - (last["chrono_ms"] or 0)))
            last = s
        time.sleep(a.interval)


if __name__ == "__main__":
    main()
