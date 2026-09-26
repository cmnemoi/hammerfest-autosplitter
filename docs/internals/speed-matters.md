# About speed

**Read this when:** you are about to simplify the search, and you want to know
which of its mechanisms are load bearing.

**You need:** [About finding the game](finding-the-game.md).

---

## What the deadline really is

The objects of the SWF are not born one at a time. The heap goes from two to
eighty megabytes in a few seconds, and then it settles.

```text
   SWF starts loading
        |
        |<----- the heap grows, objects appear ----->|
                                                     |
                                              nothing to find
                                              before this point
                                                     |
                                                     |<-- 0.55 s -->| level 0
```

A full scan used to cost about 0.55 s. So the search and the start of the run
were in a dead heat.

[About the clocks](the-clocks.md) explains why we stopped trying to win that
race: the start is dated after the fact, so arriving late costs display delay,
not accuracy.

That changes what we optimise. A slow successful scan is only a nuisance. A
slow *failed* scan costs the runner, because while it runs, level 0 appears and
the timer shows nothing.

---

## The price of failure

A failed attempt reads about 272 MiB in four passes, and 76.5 % of that goes to
searching by content.

Every mechanism below exists to avoid those passes. None of them makes a
successful scan faster.

| mechanism | what it avoids |
| --- | --- |
| a layout seed with the offsets already measured, checked before use | the search by content, several passes on the first resolution of a session |
| candidates sorted inside the local buffer | one remote read per String object, and there are thousands |
| one pass for all candidates | eight re-reads of the heap when a string appears several times |
| no fallback once the layout is proven | reading for nothing: if the vtable is right and the string is absent, it does not exist yet |
| scanning only new or grown regions | re-reading a hundred MiB to find what is in the last four |
| a budget of 8 MiB or 128 reads before yielding | one pause per region when the map holds many small ones |

---

## The memory map cache

`livesplit-auto-splitting` caches the memory map for one second per attached
process. So the differential scan was comparing stale region lists:

```text
   the SWF commits new regions        <- the objects are born here
        |
        | the runtime still serves the map it cached a moment ago
        v
   the scan sees no new region  ->  "nothing can have been born"  ->  skip
        |
        v
   it waits for the cache to expire, up to a full second
```

A second, temporary access to the same process identifier returns a map that
is independent of that cache. The module takes one every hundred milliseconds
while it is looking for a game, and scans that.

Result: twelve starts, eleven of them displayed at 0 ms. The first start of a
fresh module, with every cache empty, fell to 135 ms.

---

## How to retire a mechanism

Every mechanism in the table is a hypothesis. To retire one:

1. say what it claims to save;
2. record a fixture of the situation it claims to help —
   [Capture a fixture](../how-to/capture-a-fixture.md);
3. compare with and without, counting reads and bytes rather than CPU time. A
   local `Vec<u8>` and a remote memory read do not cost the same;
4. confirm on a few live starts — [Measure the startup](../how-to/measure-the-startup.md);
5. keep it or delete it.

Correctness mechanisms are not in that table. Checking `setName`, rejecting a
finished GameMode and watching the heartbeat are not optimisations, so do not
measure them this way. See [About stale memory](stale-memory.md).

---

## What still costs

The two searches by content, on a failed attempt. The detail is in the git
history, commit `6be7eb9`.

---

## What the redesign cost

Nothing measurable. On 2026-09-26, the module of the 1.0.0 release and the
module after the redesign around `Avm1Heap` ran side by side in the end-to-end
harness, on the same EternalTwin game under Linux, for two minutes:

| | before | after |
| --- | --- | --- |
| update, p50 | 0.150 ms | 0.145 ms |
| update, p99 | 0.349 ms | 0.327 ms |
| update, max | 29.8 ms | 27.7 ms |

Both found the same `GameManager` at the same instant, dated the start the
same, and split on the same eight crossings. The small gap is noise: on each
tick the older module ran first, and paid for the cold caches.

The slow tick of the first search, near 30 ms, was already there. It is the
next thing worth measuring away. To compare two builds again:

```sh
mise run e2e -- 120 before.wasm target/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm
```

---

## The first search, in processor time

The reads are held by the baseline. The time the processor spends on them is
measured by hand, on the real captures:

```sh
cargo test --release -p hammerfest-reader times_the_first_search -- --ignored --nocapture
```

On 2026-09-26, once the sweeps of Pepper Flash compared the first unit of a
pattern in a tight loop, as those of Ruffle already did, and the whole
pattern only where it matched:

| player | before | after |
| --- | --- | --- |
| Pepper Flash | 22.2 ms | 6.0 ms |
| Ruffle desktop | 32.5 ms | 33.2 ms |
| Ruffle in Firefox | 24.8 ms | 25.2 ms |

The reads did not change. Ruffle was already swept in a tight loop; its
numbers moved only by the noise of the machine.
