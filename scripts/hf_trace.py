#!/usr/bin/env python3
"""Horodate le debut d'une partie, du process jusqu'au depart officiel.

La regle de course fixe le depart : *the timer begins when the loading text
disappears and fades in to level 0*. Ce fondu-la est celui du Loader, le SWF
exterieur, et le source dit exactement ou il commence et ou il finit :

    Loader.startGame()          le chargement est fini
        new GameManager(...)    -> fVersion, l'ancre du balayage
            new Adventure(...)  -> new Chrono()    gameChrono = 0
        attachLoading(6)        l'ecran de chargement, encore opaque

    Loader.mainGame()   a chaque image :
        loading._alpha -= 2     il s'efface de 2 % par image
        loading = null          <- DEPART OFFICIEL, au bout de 50 images
        gameInst.main()         le jeu tourne deja dessous

Le depart officiel est donc lisible : `Loader.loading` cesse de designer un
objet. Et le Loader porte `fVersion` comme le GameManager, donc le meme
balayage ramene les deux.

Ensuite, le temps reel se lit sans horloge exterieure :

    Chrono.update()  fait  frameTimer = Std.getTimer()
    et il tourne avant le test de pause et avant le return sur fl_lock,
    donc frameTimer avance toujours, en millisecondes reelles.

    temps reel = frameTimer - frameTimer(fin du fondu)

Le balayage du tas n'est donc plus sur le chemin critique : il suffit qu'il
finisse avant la fin du fondu pour que le depart soit date exactement.

Chronologie relevee, origine = premiere vue du process plugin :

    process       un process --type=ppapi apparait  (il nait avec la partie)
    module        pepflashplayer.dll y est charge
    GameManager   l'ancre est posee                 <- doit venir avant
    GameMode      la partie existe, gameChrono demarre
    fondu fini    Loader.loading passe a null       <- depart officiel
    controle      fl_lock retombe, le jeu repond aux touches

Usage:
    hf_trace.py                    attend une partie, trace, Ctrl-C pour finir
    hf_trace.py --out trace.csv    un releve complet, une ligne par echantillon

Une pause de dix secondes pendant la trace mesure d'un coup le comportement
de `duration`, de `gameChrono` et de `frameTimer` en pause.
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

# L'ancre du GameManager : `fVersion` est pose dans son constructeur et aucune
# autre classe ne le porte. Voir src/hammerfest.rs, meme chaine de resolution.
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

# Le Loader : l'objet du SWF exterieur, celui qui porte l'ecran de chargement.
# Il possede `fVersion` lui aussi, donc il sort du meme balayage que le
# GameManager, gratuitement. `gameInst` le distingue : le GameManager ne l'a
# pas. `loading` est le clip de l'ecran de chargement, mis a null par
# `Loader.mainGame` quand le fondu s'acheve -- c'est le depart officiel.
K_GAME_INST = hfmap.obf("gameInst")
K_LOADING = hfmap.obf("loading")

K_FRAME_TIMER = hfmap.obf("frameTimer")
K_GAME_TIMER = hfmap.obf("gameTimer")
K_HALTED_TIMER = hfmap.obf("haltedTimer")
K_FL_STOP = hfmap.obf("fl_stop")

SECOND = 32          # Data.SECOND : cycles de jeu par seconde
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


# Sans restype, ctypes ramene le HANDLE dans un int C 32 bits signe : le
# snapshot serait tronque et la comparaison a INVALID_HANDLE_VALUE fausse.
winmem.k32.CreateToolhelp32Snapshot.argtypes = [W.DWORD, W.DWORD]
winmem.k32.CreateToolhelp32Snapshot.restype = W.HANDLE
winmem.k32.Process32FirstW.argtypes = [W.HANDLE, C.POINTER(PROCESSENTRY32W)]
winmem.k32.Process32NextW.argtypes = [W.HANDLE, C.POINTER(PROCESSENTRY32W)]


def pids_by_name(names):
    """PIDs des process portant un de ces noms.

    Toolhelp32 plutot que `winmem.ppapi_pids`, qui passe par PowerShell : une
    requete WMI coute des centaines de millisecondes, donc elle ne peut pas
    dater la naissance d'un process. Ici le cout est de l'ordre de la
    milliseconde, ce qui autorise un sondage a 50 Hz -- et c'est exactement ce
    que fait l'autosplitter, qui liste les process a chaque tick.
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
    """Attend qu'une partie s'ouvre. -> (proc, module, t_process, t_module).

    Les process deja vivants sont ecartes d'emblee : l'application EternalTwin
    tourne avant la partie, seul le process plugin nait avec elle. C'est ce qui
    donne un instant de naissance exploitable comme origine des temps.
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
                # Pas encore charge, ou ce process ne portera jamais le plugin.
                # On ne peut pas les distinguer, donc on repasse : le module
                # apparait quelques dizaines de millisecondes apres le process.
                p.close()
                if now - seen[pid] > 5.0:
                    rejected.add(pid)
                continue
            return p, m, seen[pid], now
        time.sleep(poll)


# -- resolution --------------------------------------------------------------

def derive_so_tbl(av, tbl, min_votes=2):
    """Offset `ScriptObject -> table`, par vote sur les proprietes de `tbl`.

    La voie ordinaire derive cet offset d'une reference croisee :
    `GameManager.current` designe un mode dont le champ `manager` redesigne le
    GameManager. Elle prouve le candidat, mais elle exige qu'un mode existe
    deja -- donc elle interdit de poser l'ancre avant la partie, ce qui est
    precisement ce qu'on veut faire.

    Sans reference croisee, un seul objet ne suffit pas : plusieurs offsets
    peuvent mener a une table plausible. Mais le bon offset est le meme pour
    tous les ScriptObject du process, alors qu'un mauvais ne tombe juste que
    par accident. On fait donc voter toutes les proprietes objet de la table,
    et on exige une majorite nette.

    -> (offset, voix, objets examines), offset a None si la majorite manque.
    """
    votes = {}
    objets = 0
    for _, atom, _ in av.entries(tbl):
        so = atom & ~7
        if not so or not av.in_module(av.p.u64(so)):
            continue
        objets += 1
        for x in avm1.SO_TBL_CANDS:
            t = av.p.u64(so + x)
            if t and av.p.u64(t) == av.L.tbl_vt and av.capacity(t):
                votes[x] = votes.get(x, 0) + 1
    if not votes:
        return None, 0, objets
    best = max(votes, key=votes.get)
    return (best if votes[best] >= min_votes else None), votes[best], objets


def find_anchors(av):
    """Pose les deux ancres. -> (GameManager, Loader, commentaire).

    `fVersion` est pose dans le constructeur du GameManager, et le Loader le
    porte aussi : un seul balayage donne donc les deux objets. Le Loader est
    celui qui possede `gameInst`, l'instance de jeu qu'il a creee.

    Le GameManager mene a la partie. Le Loader, lui, mene au depart officiel :
    `Loader.mainGame` met `loading` a null quand l'ecran de chargement a fini
    de s'effacer.
    """
    tables = av.derive(K_F_VERSION)
    if not tables:
        return None, None, "aucune chaine fVersion dans le tas", []

    # Le Loader porte `fVersion` comme le GameManager : il sort du meme
    # balayage. `gameInst` -- l'instance de jeu qu'il a creee -- le distingue.
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
        x, voix, objets = derive_so_tbl(av, tbl)
        if x is not None:
            av.L.so_tbl = x
            return tbl, loader, ("vote +0x%02x, %d voix sur %d objets"
                                 % (x, voix, objets)), tables
        notes.append("table 0x%x : current %s, %d voix sur %d objets"
                     % (tbl, "pose" if current else "vide", voix, objets))
    return None, loader, "; ".join(notes), tables


def game_mode_of(av, manager):
    """`GameManager.current`, si c'est une partie et non un menu.

    Un GameMode se reconnait a son `gameChrono` : les modes de menu ont un
    `world`, mais aucun chrono de jeu.
    """
    mode = av.table_of(av.get(manager, K_CURRENT))
    if mode is None:
        return None
    chrono = av.child(mode, K_CHRONO)
    if chrono is None or av.get(chrono, K_FRAME_TIMER) is None:
        return None
    return mode if av.child(mode, K_WORLD) is not None else None


# -- echantillonnage ---------------------------------------------------------

def fade_done(av, loader):
    """L'ecran de chargement a-t-il fini de s'effacer ?

    `Loader.mainGame` retire le clip et met `loading` a null quand son alpha
    atteint zero. Le champ cesse donc de designer un objet a l'instant exact
    ou le fondu s'acheve -- c'est le depart officiel de la run.
    """
    if loader is None:
        return None
    atom = av.get(loader, K_LOADING)
    if atom is None:
        return True
    return (atom & 7) not in (avm1.TAG_OBJECT, avm1.TAG_NATIVE)


def sample(av, mode, loader=None):
    """Un releve complet, ou None si le mode n'est plus lisible."""
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
    # Chrono.get() : fl_stop ? haltedTimer : floor(frameTimer-gameTimer)
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
    """Les instants remarquables, dates depuis la naissance du process."""

    def __init__(self, t0):
        self.t0 = t0
        self.at = {}
        self.control = None      # le releve fait au moment du deverrouillage
        self.fade = None         # le releve fait a la fin du fondu
        # A-t-on vu le mode verrouille avant de le voir deverrouille ? Sinon
        # la transition est anterieure a la premiere lecture, et l'instant du
        # depart doit etre reconstruit par `duration`.
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
    print("attente d'une partie : ouvrir Hammerfest maintenant")
    proc, module, t_process, t_module = wait_for_plugin()
    base, end, path = module
    ev = Events(t_process)
    ev.mark("process", t_process)
    ev.mark("module", t_module)
    print("\npid            %d" % proc.pid)
    print("plugin         %s" % path)
    print("process -> module  %.3f s" % (t_module - t_process))

    av = avm1.Avm1(proc, (base, end), proc.regions())

    # -- poser l'ancre, en rebalayant tant qu'elle manque
    manager = None
    scans = 0
    connues = set()
    while manager is None:
        t = time.perf_counter()
        # Le tas grossit pendant que le SWF charge : relire les regions, et
        # commencer par les nouvelles. Les objets du SWF naissent dans de la
        # memoire qui vient d'etre engagee, donc c'est la qu'il faut regarder
        # d'abord -- relire les cent Mio deja vus retarderait la trouvaille de
        # tout le temps qu'il faut pour les lire.
        regions = proc.regions()
        neuves = [r for r in regions if r[0] not in connues]
        av.heaps = neuves + [r for r in regions if r[0] in connues]
        connues = {r[0] for r in regions}
        av._key_atoms.clear()
        mio = sum(b - a for a, b in av.heaps) / (1 << 20)
        try:
            manager, loader, how, tables = find_anchors(av)
        except OSError:
            print("  process disparu avant la resolution")
            return
        scans += 1
        print("  balayage %2d : %5.1f Mio (%d regions neuves), %.2f s -> %s%s"
              % (scans, mio, len(neuves), time.perf_counter() - t,
                 "GameManager 0x%x, %s" % (manager, how) if manager else how,
                 ", Loader 0x%x" % loader if loader else ""))
    ev.mark("manager")
    print("\nlayout AVM1 derive :\n%s" % av.L)

    rows = []
    mode = None
    last = None
    t_dead = None
    print("\ntrace en cours, Ctrl-C pour arreter\n")

    try:
        while True:
            now = time.perf_counter()
            if mode is None:
                try:
                    mode = game_mode_of(av, manager)
                except OSError:
                    break
                if mode is None:
                    # Pas de partie : un menu, ou elle n'existe pas encore.
                    # Attendre sans limite tant qu'on n'en a jamais vu une :
                    # l'ancre peut maintenant etre posee avant le lancement.
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
    """Annonce les transitions au fil de la trace."""
    if s["fl_lock"] is True:
        ev.saw_locked = True
    if s["fl_lock"] is False and ev.mark("control"):
        ev.control = s
        print("  CONTROLE       fl_lock=false  duration=%s  gameChrono=%s ms"
              % (s["duration"], s["chrono_ms"]))
    if s["fade_done"] and ev.mark("fondu"):
        ev.fade = s
        print("  FONDU FINI     depart officiel  gameChrono=%s ms  frameTimer=%s"
              % (s["chrono_ms"], s["frame_timer"]))
    if s["duration"] and ev.mark("duration"):
        print("  duration > 0   %.3f cycles" % s["duration"])
    if last is None:
        return
    if s["fl_pause"] != last["fl_pause"]:
        print("  %-14s t=%.3f s  duration=%s  gameChrono=%s ms"
              % ("PAUSE" if s["fl_pause"] else "reprise",
                 s["t"], round(s["duration"] or 0, 1), s["chrono_ms"]))
    if s["level"] != last["level"]:
        print("  niveau %s -> %-3s t=%.3f s  gameChrono=%s ms"
              % (last["level"], s["level"], s["t"], s["chrono_ms"]))


def summary(ev, rows):
    """La chronologie, et le verdict qui commande l'architecture."""
    print("\nchronologie, origine = premiere vue du process :")
    for name, label in (("process", "process"), ("module", "module"),
                        ("manager", "GameManager"), ("gamemode", "GameMode"),
                        ("control", "controle"), ("fondu", "fondu fini"),
                        ("game over", "game over")):
        ev.show(name, label)

    # -- le depart officiel : la fin du fondu de l'ecran de chargement
    print("\ndepart officiel -- fin du fondu :")
    if ev.fade is None:
        print("  jamais observe : le Loader n'a pas ete trouve.")
    elif "fondu" in ev.at and ev.at["fondu"] > ev.at.get("manager", 0) + 0.05:
        f = ev.fade
        print("  observe a %.3f s." % ev.at["fondu"])
        print("  gameChrono     %s ms   (depuis la construction du Chrono)"
              % f["chrono_ms"])
        print("  frameTimer     %s ms   <- origine du temps reel"
              % f["frame_timer"])
        derniere = rows[-1]
        if derniere["frame_timer"] and f["frame_timer"]:
            reel = (derniere["frame_timer"] - f["frame_timer"]) / 1000
            mesure = derniere["t"] - ev.at["fondu"]
            print("  temps reel a la derniere lecture : %.3f s"
                  " (mesure de mon cote : %.3f s, ecart %+.0f ms)"
                  % (reel, mesure, (reel - mesure) * 1000))
    else:
        print("  deja fini a la premiere lecture : l'ancre est arrivee trop"
              " tard pour le dater.")
        if rows and rows[0]["chrono_ms"] is not None:
            print("  gameChrono valait deja %s ms." % rows[0]["chrono_ms"])

    if "control" in ev.at:
        # `duration` ne bouge qu'une fois le mode deverrouille : sa valeur a la
        # premiere lecture dit donc de combien on est arrive en retard.
        c = ev.control
        retard = (c["duration"] or 0) / SECOND
        chrono = c["chrono_ms"]
        if chrono is not None:
            chrono = round(chrono - retard * 1000)
        print("\nprise de controle -- fl_lock retombe :")
        print("  %.3f s%s" % (ev.at["control"] - retard,
                              "" if ev.saw_locked and retard < 0.1
                              else "  (reconstruite par duration)"))
        print("  gameChrono     %s ms" % chrono)

    pauses = [r for r in rows if r["fl_pause"] and r["duration"] is not None]
    if len(pauses) > 1:
        d = pauses[-1]["duration"] - pauses[0]["duration"]
        g = (pauses[-1]["chrono_ms"] or 0) - (pauses[0]["chrono_ms"] or 0)
        fr = (pauses[-1]["frame_timer"] or 0) - (pauses[0]["frame_timer"] or 0)
        reelle = pauses[-1]["t"] - pauses[0]["t"]
        print("\npendant la pause (%.1f s reelles) :" % reelle)
        print("  duration    %+.1f cycles = %+.2f s" % (d, d / SECOND))
        print("  gameChrono  %+d ms" % g)
        print("  frameTimer  %+d ms   <- doit suivre le temps reel" % fr)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", default="trace.csv", help="fichier CSV du releve")
    ap.add_argument("--interval", type=float, default=0.02,
                    help="periode d'echantillonnage, en secondes")
    args = ap.parse_args()
    if sys.platform != "win32":
        sys.exit("ce script utilise Toolhelp32 : Windows seulement")
    # Une trace se lit pendant qu'elle se fait : sans cela, Python met la
    # sortie en tampon des qu'elle n'est pas un terminal.
    sys.stdout.reconfigure(line_buffering=True)
    trace(args)


if __name__ == "__main__":
    main()
