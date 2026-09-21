# Architecture

How the autosplitter is built, and why it is built this way. What the game
hides in memory, and how we find it there, is the other document:
[reverse-engineering.md](reverse-engineering.md).

---

## Two crates, and the split is not decorative

```text
core/     pure logic: when to start, split, reset, drop a resolution; AVM1
          atom decoding. No dependency, no memory access. -> the tests are
          here.

src/      infrastructure: find the plugin process, scan the heap, read the
          AVM1 objects, talk to LiveSplit. It decides nothing.
```

The ASR runtime symbols exist only inside the WebAssembly sandbox.
**Everything that touches `asr` is therefore untestable on the development
machine**, and what must be tested has to be free of it. Hence the `core`
crate, which receives a `State` and returns `Actions`.

`src/lib.rs` only reads a state, hands it to `core::Policy`, and executes the
answer.

The tests cover the rules that actually broke: the level 0 shortcut that jumps
from 0 to 10, the three writes of `currentId` inside one frame, the clock that
is not zero at the start of a game, the reset that must not fire before a game
was ever seen, and the origin of real time in both its cases.

---

## The timing

The race rule sets the start: *the timer begins when the loading text
disappears and fades in to level 0*. That is the frame where
`GameMode.fl_lock` falls -- `onViewReady` attaches the view and calls
`onLevelReady`, which unlocks, in the same frame.

The heap scan finishes after that. **And there is no point in winning that
race**: the AVM1 objects of the SWF are all born together, half a second
before the level appears. The window is 0.55 s, and a full scan used to cost
as much.

We do not need to win it, because the game itself carries the start instant.
`GameMode.main()` returns on `fl_lock` **before** it increments `duration`,
which is therefore exactly zero during the whole black screen:

```text
origin    = frameTimer - duration     at the first unlocked read
real time = frameTimer - origin
```

Arriving in time, `duration` is zero and the origin is `frameTimer`. Arriving
late, `duration` says by how much. End to end check: **6 ms of difference over
60.9 s**.

`Policy` sets this origin once, then returns `real_time_ms` on every tick. Two
guards, both tested: the origin is rebuilt when `duration` or `frameTimer`
goes backwards -- so at the next game -- but **not** when the resolution is
lost and taken again during a game. Rebuilding it then would place it too
late, by all the time spent between levels that `duration` does not count, and
the timer would go backwards in front of the player.

### Why the exact time goes in the *game time* channel

The ASR API exposes only `start`, `split`, `reset`, `set_game_time` and
`pause_game_time`. **It cannot move a running timer backwards.** So the
LiveSplit *real time* starts at the `start()` call, that is, when the scan
finishes.

`set_game_time` accepts an absolute value. So it carries the correct time, and
`gameChrono` -- the clock the game displays, pauses and level transitions
excluded -- goes into a variable next to it.

> In LiveSplit: **Compare Against -> Game Time**.

### The end of the run

The rule ends the run when the player *enters the door and can no longer
control the character*. In the last level that door is an elevator.

`GameMode.endModeTimer` carries the cinematic that follows. It goes from zero
to fourteen seconds of cycles in the frame of the elevator, and nothing else in
an adventure writes it. `Policy` splits **when that timer starts** -- on the
one transition, not on its presence, which lasts four hundred reads.

**And the split is dated, not observed.** The game counts the timer down on the
line after `duration += Timer.tmod`, so what is left of it says how long ago
the run ended:

```text
finish time = real time at this read - (14000 ms - endModeTimer)
```

`EndSequence` holds that arithmetic. `Policy` only asks it how long ago. It is
the trick of the start, in reverse: the reader does not have to arrive in the
right frame. A correction above 500 ms is refused -- it would mean another
version of the game, with another cinematic, and a wrong correction costs the
run where a missing one costs one read.

The time then **freezes**: the game runs for fourteen more seconds, and
`frameTimer` with it.

Two events follow, and neither may reset the run -- game over at the end of the
cinematic, and the loss of the plugin when the page navigates to the end
screen. The `finished` flag of `Policy` outlives both, until a new origin is
set.

The controls are the wrong thing to read: the fruit release takes them away
too, 12.5 s earlier, and leaves this timer at zero. The proof is in section 11
of [reverse-engineering.md](reverse-engineering.md).

---

## The resolution, from the cheapest to the most expensive

```text
1. GameManager.current          a few reads, tried on every tick
2. scan: fVersion               -> the GameManager, born with the SWF
3. scan: world                  -> the GameMode directly, as a last resort
```

Once the anchor is set, a game that starts is seen in a few reads. Everything
is checked again before use: the game rebuilds its objects between two games,
and an abandoned slot stays readable while holding a perfectly plausible
value.

### What makes a scan acceptable

| mechanism | what it avoids |
| --- | --- |
| a layout seed with the offsets already measured, checked before use | the search by content, which cost several passes on the first resolution of a session |
| candidates sorted inside the local buffer | one remote read per String object, and there are thousands |
| one pass for all candidates | eight re-reads of the heap when the string appears several times |
| no fallback once the layout is proven | reading for nothing: if the vtable is right and the string is not there, it does not exist yet |
| scanning only the new or grown regions | re-reading a hundred MiB to find what is in the last four |
| a budget of 8 MiB or 128 reads before yielding | one pause per region when the map holds many small ones |

### The one second cache of the runtime

This is the least obvious mechanism, and the one that mattered most.

`livesplit-auto-splitting` caches the memory map **for one second per attached
process** (`refresh_memory_ranges`, livesplit-core `377f598`). So the
differential scan compared stale regions: it could not see the birth of the
ones where the SWF had just created its objects, and it waited for the next
expiry.

A temporary access to the same PID gives a map that is independent of that
cache. `diagnostics::FreshMap` takes one every hundred milliseconds while the
module looks for the game, and `resolve` scans that one.

Result: **twelve starts, eleven at 0 ms** of display delay. The first one of a
fresh module, with empty caches, falls to 135 ms.

### What still costs

A failed attempt reads about 272 MiB in four passes, of which 76.5 % go to the
two searches by content. That is where the rare delays live. The detail is in
the git history, commit `6be7eb9`.

---

## The diagnostics do not live in the product code

**The normal build measures nothing**: no trace, no counter, and the `.wasm`
does not even hold the matching strings.

```sh
grep -c HF_ target/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm   # 0
```

Everything that measures lives in `src/diagnostics.rs`, behind the feature of
the same name. The product code only calls into it: without the feature, those
calls have no body, and the types they handle are empty.

Two exceptions, which belong to the product: the WASI clock -- imported
directly, because the ASR API exposes none and the `asr` `Instant` exists only
on the wasi target -- and `FreshMap`, which the fix above depends on.

### Measuring

```sh
mise run build-diagnostics     # traced module, in its own directory
mise run capture-startup       # runs games and records their delay
mise run summarize-startup     # summarises an exported log
mise run probe-runtime         # measures the cache of the LiveSplit ASR DLL
```

The `diagnostics` build writes `HF_DIAG`, `HF_SCAN` and `HF_START` lines,
which these scripts read. A `known-flash` build also exists: it recognises the
player already measured by its PE headers, before the first game.

---

## The scripts

Three families, which do not read the same way.

**Libraries** -- they do not run on their own, everything else builds on them.

| file | role |
| --- | --- |
| `winmem.py` | read only access to a Windows process through ctypes, with no dependency. The Windows equivalent of `memlib.py` in the `hammerfest-re` repository, which read `/proc/<pid>/mem` |
| `avm1.py` | the AVM1 object model: atoms, strings, tables. The layout is derived at run time, never assumed |
| `hfmap.py` | source names to obfuscated names, from `vendor/hf.map.json` |

**Tools** -- they are still in use.

| command | what it does |
| --- | --- |
| `mise run state` / `watch` / `dump` | read a running game without LiveSplit |
| `mise run trace` | time a start, from the process to the official start, and write a CSV |
| `mise run capture-heap` | record the plugin memory into `fixtures/<name>/`, for the documents and for off line tests |
| `mise run capture-startup` / `summarize-startup` / `probe-runtime` | measure the display delay |

### Memory captures

`mise run capture-heap` writes a replayable fixture into `fixtures/<name>/`:
`metadata.json`, one gzip stream for the heap, one for the plugin image.

Measured on a live game, `pepflashplayer.dll` 32.0.0.465:

| | regions | read | stored |
| --- | --- | --- | --- |
| heap | 124 | 85.4 MiB | 19 MiB, 22 % |
| plugin image | 1 | 32.6 MiB | 18 MiB, 52 % |

**The whole capture takes 0.3 to 0.6 s.** That duration is the error it
carries: the game runs while we read, so the last region is younger than the
first. `state_before` and `state_after` bracket it, and they check each other
-- a capture of 0.59 s recorded a game clock 599 ms apart. The object graph
survives the smear, because AVM1 objects do not move inside a game. The clocks
do not, so a fixture must never assert an exact clock value.

`--no-module` halves the fixture. Only `Binary::recognize` reads those bytes;
the layout seed and the module range test need the range alone, which
`metadata.json` always carries.

Fixtures stay out of git until a measurement says otherwise. Compression uses
level 1 on purpose: level 6 costs four times the time for 28 % fewer bytes,
and here time is accuracy.

**Readings** -- they ran once, to establish that no static pointer path leads
to the game objects. Their conclusion is in section 10 of
[reverse-engineering.md](reverse-engineering.md), and their code does not have
to be pretty.

| file | question | answer |
| --- | --- | --- |
| `anchors.py` | which addresses survive a game restart? | none |
| `stable_slots.py` | which slots does the player reuse? | none points at the current movie |
| `xrefs.py` | how many module pointers are cited by code? | 109 out of 550 |
| `vtable_globals.py` | which globals do the AVM1 methods read? | three, two of them allocators |
| `ptrscan.py`, `findchain.py`, `checkchain.py` | is there a short chain from the module to the current movie? | nothing found; section 10 concludes without them |

They depend on the libraries above, so they do not move without them. That is
why they stay here rather than join the `hammerfest-re` repository, which
carries a different piece of work: reading the **score** on Linux.

---

## What is still open

**The LiveSplit real time stays late** by the resolution delay. The API does
not allow a correction; the correct timer is the one in the *game time*
channel.

**The end of the run is implemented, not observed.** The final split fires on
the rise of `GameMode.endModeTimer`, which the source proves to be the frame of
the elevator. No finished run has confirmed it yet: the level script that
triggers it is encrypted in the Motion Twin repository, so only a real run
can.

**No setting is exposed.** The module applies `Rules::default()`: start, split
on level change in the main world, reset. Settings saved by older versions are
ignored.

**The open assumptions** are listed at the end of
[reverse-engineering.md](reverse-engineering.md): one version of the Flash
player, one machine, and the parallel dimensions out of scope.
