# Resolution pacing

When the loop is allowed to scan the whole heap again.

---

## Why

A full scan reads every committed byte of the plugin heap, which is eighty MiB
once the SWF is loaded. It blocks the loop while it runs. Sixty scans per
second would freeze LiveSplit.

Waiting is not free either. The player clicks, and the game starts half a
second later. A scan that arrives late dates the start of the run late, and the
timer is short by that delay.

So the pacing sits between two costs, and one measurement separates them.
Loading the SWF takes the heap from two to eighty MiB, in jumps of several MiB.
Once the game runs, the heap moves by a few hundred KiB. The birth of the
objects is visible in the size of the heap, and nowhere else.

## Scope

One decision, from two numbers: may the loop scan now, given the committed size
of the heap and what the last attempts gave.

Outside: the scan itself, the fast path through the `GameManager`, and every
decision the policy makes.

---

## Rules

### The first scan waits for nothing

`{#pacing::the-first-scan-waits-for-nothing}`

A plugin process that has just appeared has no history. The first tick where
the fast path finds nothing scans at once. The game may already be running.

### A failed scan waits, and the wait grows

`{#pacing::a-failed-scan-waits-longer-each-time}`

A scan that finds nothing sets a wait of 20 ticks. The next failure waits 40,
then 60. The wait never exceeds 60 ticks, which is about one second.

While the player is in the menus, every scan fails. The wait is what stops the
autosplitter from reading eighty MiB per second for nothing.

### A heap that grows scans at once

`{#pacing::a-heap-that-grows-scans-at-once}`

Growth of 4 MiB or more since the last scan cancels the wait, whatever is left
of it. That growth is the SWF creating its objects, and it is the only window
where a new scan can learn something.

### Allocator noise is not growth

`{#pacing::allocator-noise-is-not-growth}`

Growth below 4 MiB leaves the wait alone. A game in progress moves the heap by
a few hundred KiB, and treating that as a birth would scan on every tick.

### A game found clears the wait

`{#pacing::a-game-found-clears-the-wait}`

When the game is found, the next failure waits 20 ticks again, and not the 60
the last failures had reached.

### A game lost scans at once

`{#pacing::a-game-lost-scans-at-once}`

When the resolution is dropped, the next tick scans with no wait. Losing a game
almost always announces the next one. The game builds new objects at every
launch, so the anchor dies with it and has to be found again.

---

## Out of scope

**What a scan costs.** Bytes read, runtime calls and the yielding budget are
the subject of a benchmark, not of a test.

**The fast path.** `resolve_via_manager` is a few reads. It runs on every tick
and asks nobody.

**Where the heap size comes from.** The loop either measures it or takes it
from the map of fresh regions. The pacing is given a number and does not care
which.

**Everything else in `run`.** The order of `start()` and `set_game_time()`, the
line printed once per game, and the attach loop stay outside this page. They
are the subject of
[the note on the loop](../internals/testing-the-memory-reader.md).
