# Hammerfest level and time, on Windows

> This document records the evidence. It assumes the vocabulary: heap, vtable,
> atom, property table. If that is new,
> [About AVM1 values](concepts/avm1-values.md) and
> [About AVM1 objects](concepts/avm1-objects.md) teach it first, with real
> bytes.

Reverse engineering notes. Every claim is marked **[PROVED]** (checked on the
live process, with the measurement that shows it) or **[ASSUMPTION]**.

Target: EternalTwin on Windows, `pepflashplayer.dll` win32-x64 **32.0.0.465**,
AVM1 / ActionScript 2, read only.

The earlier work (`cmnemoi/hammerfest-re`, done by Claude) had solved the
*score* on Linux. These notes are about the level and the time, on Windows.

This document covers what the game hides and how we find it. How the
autosplitter is built around that is in [the documentation hub](index.md).

---

## 1. What the game really tracks

Two time counters, both reported by the game at the end of a game
(`mode/Adventure.mt`, `onGameOver`):

```mt
manager.logAction( "$t=" + Math.round(duration/Data.SECOND) );
manager.history = [ "F="+$version, "T="+gameChrono.get() ];
```

| field | type | nature |
| --- | --- | --- |
| `GameMode.gameChrono` | `Chrono` | milliseconds, from the construction of the `GameMode` |
| `GameMode.duration` | `Float` | `duration += Timer.tmod` per frame, `Data.SECOND = 32`, from the moment level 0 appears |

Both stop at the same moments, pause and level transitions, because
`GameMode.lock()` stops `gameChrono` and makes `main()` return before the
increment of `duration`. They differ only in their origin, which is what makes
`duration` useful: see §9. **[PROVED]**: 26.2 s of pause give `duration`
+0.0 cycle and `gameChrono` +0 ms.

`Chrono` (`class/hammer/Chrono.mt`) does not store a duration but two instants:

```mt
function get() {
    if ( fl_stop )  return haltedTimer;
    else            return Math.floor( frameTimer-gameTimer );
}
```

`stop()` is called by `GameMode.lock()`, `start()` by `unlock()`. So it is a
game time that freezes on pause. It is what the game displays, and it is not a
real time. The run timer is built another way, see §9.

One detail of the constructor matters: `suspendTimer` is left at `null` there,
so the first `start()` does not shift `gameTimer`. `gameChrono` therefore
counts from the construction of the `Chrono`, level 0 loading included, and it
is already 0.55 s when the player sees the level. **[PROVED]**: 535, 539, 547
and 562 ms over four games.

The current level is `GameMode.world.currentId`, where `world : GameMechanics`
inherits from `SetManager` (`levels/SetManager.mt`):

```mt
function setCurrent(id:int) {
    _previous = current;  _previousId = currentId;
    current = levels[id]; currentId = id;
}
```

---

## 2. The obfuscated names come from a published table

The distributed SWF is obfuscated, but the mapping is public:
`eternalfest/game-types`, file `src/lib/hf.map.json`, MIT, 2952 entries. It
serves `eternalfest/project-phoenix`, the decompiler of the game. Copied into
`vendor/hf.map.json`.

Careful: the Eternalfest obfuscator (`eternalfest/obf`) did not produce this
SWF. It generates `md5(salt+name)[0:10]` names, purely
hexadecimal, while the Hammerfest SWF names look like `70dik` or `{8`. The
table, however, really describes the Motion Twin SWF.

It checks against the earlier reverse work, which observed these values in
memory without knowing the table: **[PROVED]**

| clear | obfuscated | source |
| --- | --- | --- |
| `realScores` | `70dik` | observed in 2025 in the heap |
| `fakeScores` | `[t}LJ(` | observed in 2025 in the heap |
| `gi` | `{8` | observed in 2025, the identification was still an assumption |

The keys used here:

| clear | obfuscated |
| --- | --- |
| `world` | `]=[]8` |
| `currentId` | `-BBEO` |
| `_previousId` | `(VT Q` |
| `setName` | ` h;+A(` |
| `gameChrono` | `8qkdA` |
| `frameTimer` | `]);5(` |
| `gameTimer` | `{DfiG` |
| `haltedTimer` | `]IEAU` |
| `fl_stop` | `*9gvn` |
| `xml_adventure` | `]R;5E` |

`duration` is not renamed: it is an identifier of the standard AS2 API
(`Sound.duration`), so it is protected by the `as2.map.json` list. It keeps its
clear name in the SWF. **[PROVED]**: read as such in the GameMode table.

The world names are obfuscated too: `addWorld("xml_adventure")` passes a string
that looks like an identifier, so it is renamed to `]R;5E`. Searching for
`"xml_adventure"` in the heap finds nothing. **[PROVED]**

---

## 3. Memory layout: Windows differs from Linux

The Linux layout was known. Only half of it holds on Windows, and we measured
every offset: **[PROVED]**

```
                        Linux x86-64        Windows x86-64
String   vtable         +0x00               +0x00            same
         UTF-16 buffer  +0x08               +0x08            same
         length         +0x30               +0x30            same
ScriptObject -> table   +0x30               +0x30            same
table    vtable         +0x00               +0x00            same
         capacity       +0x08               +0x08            same
         entries        +0x18               +0x48            DIFFERENT
         stride         16 bytes            24 bytes         DIFFERENT
         entry          (value, key)        (value, _, key)  DIFFERENT
```

An entry is 24 bytes, and the key is the *third* qword, not the second. A port
that copied the Linux offsets would have read, for every property, the value of
the next field: plausible values, and wrong. That is the mistake I made first,
and the semantic check caught it.

Vtables observed in this session (with ASLR they are relative to the module
base; they are stable for this binary):

```
String        MODULE+0x1756db8
ScriptObject  MODULE+0x1749ed8
table         MODULE+0x174a460
```

The code does not hard code them: it derives them at run time
(`scripts/avm1.py`). Only the module base comes from the OS.

### Atom encoding **[PROVED]**

`atom = (value << 3) | tag`, the 3 low bits being the type:

| tag | meaning | decoding |
| --- | --- | --- |
| 0 | **signed** integer | `atom >> 3`, arithmetic shift |
| 1 | float | pointer to an 8 byte IEEE `double` |
| 2 | special | `0x0a` null, `0x12` false, `0x32` true |
| 3 | native object / MovieClip | pointer |
| 5 | String | pointer to a String object |
| 6 | object | pointer to a ScriptObject |

The sign matters: `portalId` was `0xfffffffffffffff8`, that is `-1`. An
unsigned decode would have read it as 2305843009213693951. **[PROVED]**

---

## 4. The resolution chain

There is no hard coded address, and no static pointer path, because the objects
are created at run time by a downloaded SWF. The anchor is an interned string
of the SWF:

```
process --type=ppapi         the Flash plugin, born when the SWF loads
pepflashplayer.dll           ASLR -> module base
scan "]=[]8" in the heap     `world`, key known from hf.map.json
  -> String object           the qword in front that points into the module
                             is the vtable
  -> length offset           the qword whose value is 5
  -> slots citing the string scan of the 8 atom encodings
  -> entry stride            measured on the neighbouring keys
  -> table base              first qword pointing into the module, before the
                             entries
  -> value offset            a vote (see below)
GameMode["]=[]8"]            -> world: GameMechanics
world[" h;+A("]              -> setName == "]R;5E" = xml_adventure   check
world["-BBEO"]               -> currentId: the level
GameMode["8qkdA"]            -> gameChrono
```

Two traps met on the way, both real:

**The vote on the value offset cannot rest on validity alone.** Every entry
holds an unused qword that is always zero, and zero is a valid integer atom. So
that column gets a perfect validity score while holding nothing. The column
must also show diversity.

**`world` is not enough to identify the GameMode.** `View` objects carry one
too, and they point at the same `GameMechanics`. I observed up to three
candidates at once, one of them pointing at `xml_deepnight`. The discriminator
is structural: only the GameMode also owns a `gameChrono` that holds a
`frameTimer`. **[PROVED]**

---

## 5. Cross check

The two counters of the game are independent: `gameChrono` counts real
milliseconds, `duration` accumulates `Timer.tmod` per frame. They must agree.
Read from the live process:

```
duration   14815.6 cycles / 32 = 463.0 s
gameChrono                       462898 ms = 462.9 s
```

**[PROVED]**: they agree to 0.1 %, over two entirely separate memory paths.

The `Chrono` model also checks itself. Game paused:

```
fl_stop      = true            (and fl_pause = true on the GameMode side)
frameTimer   = 280135
gameTimer    =  19419          frameTimer - gameTimer = 260716
suspendTimer = 274444          280135 - 274444 = 5691 ms since the stop
haltedTimer  = 255025          255025 + 5691  = 260716   OK
```

`stop()` sets `haltedTimer = get()` and `suspendTimer = frameTimer`: the
identity closes exactly. **[PROVED]**

---

## 6. The levels do not follow each other

`currentId` does not advance by 1 each time. **[PROVED]**: by the source, and
confirmed in game by the missing split at the level 0 shortcut.

`mode/Adventure.mt`:

```mt
function nextLevel() {
    super.nextLevel();          // goto(currentId+1)  ->  currentId = 1
    if ( fl_warpStart ) {
        world.currentId = 0;    // direct assignment, outside setCurrent
        unlock();
        world.view.detach();
        forcedGoto(10);         // ->  currentId = 10
    }
}
```

The level 0 shortcut therefore does `0 -> 10`. It is not an isolated case:
`SpecialManager.warpZone(w)` advances by 1 to 3 in one step through
`forcedGoto`, stopping before a boss or an empty level.

A split rule based on `currentId + 1` misses all of these. The correct test is
any forward progress, that is `currentId` strictly increasing.

Two less visible consequences:

**A read race.** These three writes happen in the same game frame, and nothing
synchronises a read made from another process with that frame. So we can
observe `0 -> 1`, then `1 -> 0`, then `0 -> 10`, and produce two splits instead
of one, at random. Hence the confirmation over two consecutive reads before we
act.

**`_previousId` is not reliable as a transition witness.** It is updated only
by `setCurrent`; the `world.currentId = 0` assignment bypasses it. It stays
useful to lift an ambiguity during the reverse work, but not to validate a
split.

## 7. Knowing that a GameMode is dead

An abandoned GameMode stays readable for a long time: the table is intact, the
world is still `xml_adventure`, and the level and the clock are plausible. They
are simply the old ones. No consistency check separates them from a live
object. That is the trap the score reverse work announced, and the symptom is
direct: the timer takes a long time to start, and it does not stop at the end
of a game.

Two signals solve it. **[PROVED]** by the source.

**`fl_gameOver`**, set by `GameMode.onGameOver()`, is the exact end-of-game
signal. In the `Adventure` override, the game sends `"T="+gameChrono.get()`
from there. A resolution that lands on a GameMode already in game
over must be rejected, otherwise we read a finished game instead of waiting for
the next one.

**`Chrono.frameTimer`** serves as a heartbeat. In `GameMode.main()`:

```mt
// Chrono
gameChrono.update();      // unconditional, and BEFORE the pause test

// Pause
if ( fl_pause ) { ... }
```

`Chrono.update()` does `frameTimer = Std.getTimer()`. So this counter advances
every frame while that GameMode is the one the game runs, during a pause too,
and with the clock stopped too. Frozen, the object is dead.

The threshold must stay generous: Flash runs at about thirty frames per second,
and Chromium slows a background window down further. A short threshold would
fire useless new resolutions, each one costing a full heap scan.

## 8. The stable anchor: GameManager.current

Scanning the heap to find the GameMode costs about a hundred MiB of reads.
Doing it again at every game is already unpleasant. Doing it in a loop between
two games, which is what any autosplitter waiting for the next one does, stirs
the process for nothing.

The game offers the right anchor, though. `GameManager` is created once when
the SWF loads and survives across games, and it points at the current mode
(`GameManager.mt`): **[PROVED]**

```mt
var current : Mode;

function transition(prev:Mode,next:Mode) {
    next.init();
    if ( prev==null )  current = next;
    else            {  prev.destroy(); current = next; }
}
```

The other way round, every `Mode` carries `manager : GameManager`. This cross
reference is enough to identify the GameManager without knowing any name of its
own: it is the object whose `current` points at a mode which, through its
`manager` field, points back at that object. `current` alone is not a test,
because every `SetManager` has one.

```text
scan, once              -> GameManager          (kept)
GameManager.current     -> the running mode     (read again at will)
```

We look for the GameManager rather than the GameMode because it exists as soon
as the SWF loads. So the scan already succeeds in the menus, before any game,
and the game that starts next is found by following a pointer. A search for a
GameMode can only succeed once the game has started, which is the worst moment,
the one where the delay shows.

The AVM1 objects die with the game: the SWF builds a new GameManager and new
interned strings at every launch, so no heap address survives from one game to
the next. The plugin process, though,
can carry several: four games in a row were observed in one process. The scan
is unavoidable once per game; all we can choose is to do it early.

The shape to use is a stable anchor, then resolution through the object graph.
It is not a cached final address: `current` is read again every time, and the
GameMode we get goes through every check again (known world, plausible level,
`gameChrono` present, not in game over).

What can invalidate the anchor: the property table of an AVM1 object is
reallocated when it grows. The `GameManager` fields are all set in the
constructor, so it should not move, but that is not guaranteed. Hence the
check of its vtable before every use, and the fallback to a full scan if it
stops answering.

## 9. Dating the start of the run, without winning a race

The speedrun rule sets the start: *the timer begins when the loading text
disappears and fades in to level 0*. On the memory side, that instant is the
one where `GameMode.fl_lock` falls.

```mt
GameMechanics.onViewReady()      the level view is attached
    game.onLevelReady()
        unlock()                 fl_lock = false
            gameChrono.start()
```

The view is attached and the mode is unlocked in the same frame, so the black
screen ends at `fl_lock = false` within one frame, that is 31 ms.

### The race cannot be won **[PROVED]**

We tried to set the anchor before the start, to see the transition live. It
cannot be done, and not for lack of optimisation.

| | run 1 | run 2 | run 3 | run 4 |
| --- | --- | --- | --- | --- |
| anchor set, after the start | +0.374 s | +0.601 s | +0.553 s | +0.657 s |
| `gameChrono` at the unlock | 535 ms | 539 ms | 562 ms | 547 ms |

Measurements from `scripts/hf_trace.py`. What the successive scans show:

1. **Nothing to find before.** Until the last second, the heap holds no
   `fVersion` string, neither the one of the `GameManager` nor the one of the
   `Loader`. The AVM1 objects of the SWF are all born in one burst, at the end
   of the initialisation. So there is no earlier window.
2. **The useful window is 0.55 s**, between the construction of the `GameMode`
   and the unlock.
3. **A full scan costs 0.5 s** over the 80 MiB of the heap. The cost is the
   copy across the process boundary, not the comparison, so we cannot make it
   instant.

So the race comes down to a few tens of milliseconds, and we lose it.

### The game carries the start instant **[PROVED]**

`GameMode.main()` returns on `fl_lock` **before** it increments `duration`:

```mt
gameChrono.update();          // frameTimer = Std.getTimer()
if ( fl_pause ) { ... }
if ( fl_lock ) return;        // <- black screen, transitions, pause
...
duration += Timer.tmod;       // <- runs only from the official start
```

So `duration` is exactly zero during the whole black screen, and then it
measures the time during which the game ran. One formula covers both cases:

```text
origin = frameTimer - duration           at the first unlocked read
real time = frameTimer - origin
```

Arriving in time, `duration` is zero and the origin is `frameTimer`. Arriving
late, which is the usual case, `duration` says by how much.

The two counters that make this possible:

| counter | what it measures | measurement |
| --- | --- | --- |
| `frameTimer` | `Std.getTimer()`, real milliseconds since the plugin started. `Chrono.update()` runs before the pause test and before the `return` on `fl_lock`, so it is never stopped | +13 843 ms for 13.9 s of pause |
| `duration` | the time during which the game ran, in cycles of `Data.SECOND` = 32 | 67.20 s for 67.12 real s, that is 0.1 % |

End to end check: origin set at the first read, then compared with an outside
clock one minute later. **6 ms of difference over 60.9 s.** **[PROVED]**

So the heap scan leaves the critical path. Its duration now only delays the
*display* of the timer; it no longer enters the timing.

### What stays late

The LiveSplit *real time* starts at the `timer_start()` call, and the ASR API
cannot move a running timer backwards. It exposes only `start`, `split`,
`reset`, `set_game_time` and `pause_game_time`. So it carries the delay of the
scan, 0.4 to 0.7 s according to the measurements above.

That is why the exact real time goes into the *game time* channel, which
accepts an absolute value. `gameChrono` goes into a variable next to the timer:
it is the number the game itself reports at the end of a game
(`"T="+gameChrono.get()`), but it excludes pauses and level transitions, so it
cannot serve as a real time.

### Why not the other tests

| test | fault |
| --- | --- |
| the plugin process appears | 2.8 to 4.5 s before the start, depending on the SWF load time, so unusable |
| `clock < 5 s` | `gameChrono` runs from the construction of the `GameMode`: it is already 0.55 s when the level appears |
| `currentId == 0` | a fast player leaves level 0 before the scan ends |
| end of the `Loader` fade | the `Loader` of the original SWF does not exist in EternalTwin: no table carrying `fVersion` carries `gameInst` |

## 10. Why there is no static pointer path

An ordinary autosplitter follows `module + offset -> +offset -> +offset`. That
does not work here, and not for lack of searching. The reason is structural.

**What was measured**, in order:

1. Everything AVM1 is built again at every game launch, inside the same plugin
   process: GameMode, GameManager, the class object carrying the `SELF` static,
   and even the interned strings of the SWF constant pool. No heap address can
   serve as an anchor from one game to the next. **[PROVED]** (`anchors.py`, two
   consecutive readings)

2. The addresses that point at the current movie are rebuilt too: the player
   keeps no fixed address pointer to the movie at that level. **[PROVED]**
   (`stable_slots.py`)

3. Out of 550 pointers in the module data, only 109 are referenced by code. No
   instruction in relative addressing references the other 441; they are arrays
   and allocator buckets. **[PROVED]** (`xrefs.py`, disassembly of the 22 MiB of
   `.text`)

4. The globals read by the methods of the AVM1 objects, that is those listed
   in the String, ScriptObject, table and MovieClip vtables, come down to three
   things: **[PROVED]** (`vtable_globals.py`, 141 functions)

   | global | role |
   | --- | --- |
   | `module+0x1e02e90` | `__security_cookie`, confirmed by the PE `LoadConfig` |
   | `module+0x1e583a8/b0/c0` | MMgc allocators, they point at the bases of the heap regions |
   | `module+0x1f2b058` | a non pointer value, disassembly noise |

**Conclusion.** The AVM1 interpreter context is not a global. It is passed as a
parameter, or reached from the object itself. That is the usual way to write a
VM, and it explains why no static root appears. The chain searches failed for
that reason, not for lack of depth or of filters.

What would remain to try, in another setting: identify the context as a field
of the AVM1 object itself, then look for what holds that context on the player
side, most likely the PPAPI instance, registered in an indexed structure,
which is exactly the kind of array that point 3 ruled out.

## 11. The end of the run: the elevator

The speedrun rule ends the run when the player *enters the door and can no longer
control the character*. In the last level of the adventure, that door is an
elevator.

**[PROVED]** by the source, `class/hammer/levels/ScriptEngine.mt`:

```mt
var fl_elevatorOpen : bool;   // flag fin de jeu          line 67

function codeTrigger(id:int) {
    case 3:                   // "liberation des fruits"  line 1010
        fl_elevatorOpen = true;
        lockControls( Data.SECOND*12.5 );   // 12.5 s
        // endModeTimer is NOT touched

    case 4:                   // "sortie par l'ascenseur" line 1021
        if ( fl_elevatorOpen ) {
            lockControls( 99999 );                  // line 1026
            game.endModeTimer = Data.SECOND*14;     // line 1036
            fl_elevatorOpen = false;
        }
}
```

Then, in `class/hammer/mode/GameMode.mt`:

```mt
endModeTimer = 0;                      // line 170, at initialisation
...
duration += Timer.tmod;                // line 2317
if ( endModeTimer>0 ) {                // line 2320
    endModeTimer -= Timer.tmod;
    if ( endModeTimer<=0 ) {
        onGameOver();                  // line 2327  ->  fl_gameOver = true
    }
}
```

`Data.SECOND` is 32 (`class/hammer/Data.mt:17`), so the timer starts at 448
cycles: fourteen seconds of cinematic, then game over, then `saveScore` and a
redirect that destroys the SWF.

### The controls are the wrong thing to read

Case 3 and case 4 both take the controls away. Only case 4 ends the run.

A reader that watched `Player.fl_lockControls` would stop the timer 12.5 s too
early, on the fruit release. `endModeTimer` separates the two, because case 3
leaves it at zero.

### The transition ends the run, the value dates it

Nothing else in an adventure writes `endModeTimer`. The only other assignments
in the whole source are the initialisation at zero and `mode/Soccer.mt:541`,
another game mode that our `setName` check already rejects.

So the transition from zero is the frame where the player enters the elevator,
and that transition ends the run. Acting on the value being positive instead
would split four hundred times: the window is fourteen seconds wide, which is
four hundred reads at thirty per second. The event cannot be missed.

The value is not thrown away. `main()` counts it down by `Timer.tmod`, on the
line after `duration += Timer.tmod`. So:

```text
milliseconds since the elevator = 14000 - endModeTimer in milliseconds
```

That dates the last split at the frame of the elevator, whichever read sees
it. It mirrors §9: there, `duration` dates the first split; here, what is left
of the cinematic dates the last one.

**[ASSUMPTION]** The count is a play time, not a real time: `fl_lock` freezes
this timer and `duration` together. A pause between the elevator and our
reading would shorten it. One read cannot hold a pause, so the case does not
arise, and a gap above 500 ms is refused rather than applied.

### What is not proved

**[ASSUMPTION]** that the last level triggers case 4. `codeTrigger` is called
from the level script (`ScriptEngine.mt:537`), and the scripts live in
`xml/levels/adventure.xml`, which is encrypted in the Motion Twin repository.

Only a finished run confirms it. `mise run watch` prints an `ELEVATOR` line on
the rise, which is what that run has to show.

---

## 12. What is still open

- **[ASSUMPTION]** The relative vtables are stable for this exact binary. They
  are derived at run time, so another version would fail cleanly (`None`)
  rather than return a wrong level, but that has not been tested on another
  version.
- **[ASSUMPTION]** The layout measured here holds for every
  `pepflashplayer.dll` win32-x64 32.0.0.465. Measured on one machine.
- Tags 4 and 7 have not been identified. Tag 7 appears on objects whose vtable
  is `MODULE+0x178a300`, different from the ordinary ScriptObject.
- The parallel dimensions are not handled: we read `world`, which follows
  `currentDim`. `currentDim` is published in the reading so that splits outside
  the main world can be ignored later.
