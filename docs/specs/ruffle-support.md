# Ruffle support

Why the autosplitter will read Ruffle, how, and in what order. The decisions
were taken on 2026-09-26. Each one carries its reason.

This page has no rule ids yet. A rule gets an id when a test holds it.

---

## Why

Hammerfest also runs in [Ruffle](https://ruffle.rs) desktop, a Flash player
written in Rust. Eternalfest Desktop starts the official Ruffle 0.6.0 as a
child process: `ruffle` on Linux, `ruffle.exe` on Windows.

Pepper Flash stays supported. The runs that count are still made on
EternalTwin.

## Scope

In:

- Hammerfest, the main world and its dimensions, with the same rules as today;
- Ruffle desktop, on Linux and Windows first.

Out:

- other contrées of Eternalfest: their worlds and ends need rules of their own,
  in `core`, not in the reader;
- Ruffle in a browser: the VM lives in the wasm memory of a browser process,
  which is another problem;
- macOS: best effort, after the release.

---

## Decisions

### The autosplitter reads Ruffle memory, and asks nothing of the host

An autosplitter observes from outside. It does not ask its host to change.
So it works with Ruffle started alone, or by any launcher.

**Refused: a hook that publishes the state.** A small AS2 wrapper around
`loader.swf` would trace the state every frame, and Eternalfest Desktop would
copy it to a native buffer. It removes all reverse engineering. It also puts
game knowledge in another repository, couples the versions of two
applications, and makes a chain of six links in three languages.

**Why the cost is acceptable.** Ruffle is open source. The layout of a string,
of an object and of its property map is known from the source. The reverse
engineering checks what is known. It does not discover the unknown, as it did
for Pepper Flash.

**Fallback.** If the spike below fails, the hook comes back. Its design is
researched: a page-aligned block, a magic built at run time plus its own
address, a sequence counter read in three separate calls, a CRC32 and a
heartbeat.

### One game model, two heaps

The way to the level is the same in both players: find the obfuscated key of
`world`, find the object that owns it, then read `GameMode`. Only the layout
of a string, of an object and of a property table changes.

```text
Hammerfest       GameMode.world.currentId, Chrono...      once
    |
Avm1Heap         find the object that owns key K,         trait
    |            read property P
    +-- PepperFlashHeap    the layout of today
    +-- RuffleHeap         the layout of Ruffle 0.6.0
    |
Memory           read_into                                 trait
```

A `Runtime` sits above: process names, the module, the heap regions, and which
heap to use. One `.wasm` finds either player, and the runner sets nothing.

### Ruffle versions are probed, then validated

Rust does not promise a layout, and Ruffle publishes a nightly almost every
day. The reader tries candidate layouts, as `PROFILES` does for Pepper Flash,
and keeps the one that gives a valid `GameMode`: a known `setName`, a level in
bounds.

**Refused: a strict list of known builds.** The validation is already strict,
so a false positive is unlikely. A list would block a runner on a build we did
not test, for nothing.

The versions that were tested are listed on this page. The first one is 0.6.0.

### Speed is held by a baseline, to the read

A scan that arrives late dates the run late. So the redesign cannot cost
reads.

A bench counts reads, bytes and ticks: to the first resolution, to a
resolution again, and per tick. It runs on the replayed capture and on the
synthetic heap. These counts are deterministic, so the baseline is exact: the
CI fails on any change. A change that is wanted updates the baseline in the
same commit.

Processor time stays measured by hand, as [About speed](../internals/speed-matters.md)
says.

---

## Design

Decided on 2026-09-26, with the code and the spike in view.

### The seam is the object graph

A trait, `Avm1Heap`, answers two questions: which objects own a key, found by
a sweep that yields its candidates one by one and stops when the caller keeps
one; and what a property of an object holds. `PepperFlashHeap` and
`RuffleHeap` implement it.

The Hammerfest strategy exists once, above it: the `GameManager` first,
`world` as a fallback, the validation of a candidate, the reading of the
`State`. It is generic over the heap, so it costs nothing at run time. An
`enum Runtime` picks the heap once per attached process, and gives the process
names and the ordered list of regions to sweep.

**Refused: one `resolve` and one `read` per runtime.** Simpler to write, and
the strategy would exist twice.

### One value, for both players

```rust
enum Value { Undefined, Null, Bool(bool), Number(f64), String(..), Object(..) }
```

No integer variant: Ruffle has none, and a Pepper Flash integer is exact in an
f64 up to 2^53. A level is read as a number, and kept only when it is whole
and in bounds. A string is compared by the heap, with no allocation.

### State: the strategy's, and each heap's

`Anchor` keeps what the strategy learns: the `GameManager`, whether it was
proven, how long it stayed silent, the regions already seen. Each heap keeps
its own: the Pepper Flash layout, its string caches and slot hints; whatever
`RuffleHeap` needs.

### Every read is checked, and a doubt reads nothing

The game can change an object while we read it, and Ruffle reallocates the
entries of a map when it grows. No lock is possible from outside. So every
read is validated, the key of a Ruffle entry by its hash, the key of a Pepper
Flash slot by its string, and a doubt gives `None`. The policy of `core`
confirms a level on two reads in a row, so one wrong read never splits.

### One Ruffle layout, strictly checked

Only 0.6.0 under Linux is known. The reader checks it strictly: the vtable
says 8 and 160, the hash of an entry is the FNV of its key. Profiles come with
a second real layout, as the Pepper Flash ones did.

### Which process, which regions

Pepper Flash is looked for first, then Ruffle. A runner must start one game,
in one player. The README says so, and the log says when the module sees
several candidates.

Ruffle under Linux: `[heap]` first, through `get_module_range("[heap]")`, then
the anonymous regions. **The `PATH` filter of Pepper Flash must not apply**:
LiveSplit sets `PATH` on `[heap]`, and the whole game lives there. Ruffle
under Windows: every private RW region, until a capture says better.

### Tests: one scenario, several drivers

The scenarios of the reader are a DSL, and the synthetic Pepper Flash heap is
its first driver. A synthetic Ruffle heap is the second. A macro writes each
scenario once and makes one test per driver, so a red names the player.

Seventeen scenarios run on both. Two stay on Pepper Flash, because they are
its own traps: another Flash build, and a key that is not a String object.

The Ruffle driver writes with its own offsets, never the reader's. The replay
of `fixtures/replay/ruffle-main-world` is the only test that checks the
reader's offsets against bytes Ruffle wrote, as `main-world` does for Pepper
Flash.

The read cost baseline gains six situations: four on the Ruffle fixture, two
on the synthetic Ruffle heap.

### Step 5 keeps every read

Extracting `PepperFlashHeap` is a pure refactoring: the baseline does not
move. A saving comes after, in a commit of its own, with its baseline.

---

## Order

Each step stays green, and the bench measures it.

1. **The bench.** Counters, an exact baseline, in the CI.
2. **The reader leaves the wasm crate.** A crate with no `asr`, like `core`.
   Its tests and doubles live in its `tests/`. The stubs of the runtime
   serve only the adapter's tests, in `src/process/tests/`. The
   diagnostic `cfg(feature)` become an injected observer with an empty
   production implementation.
3. **The Ruffle spike.** Research, so it can run beside steps 1 and 2. Two or
   three sessions, then a review.
4. **Design `Avm1Heap` and `Runtime`**, with the code and the spike in view.
5. **Extract `Avm1Heap` from the Pepper Flash reader.** A pure refactoring.
6. **`RuffleHeap`, and finding the player.** The `feat:` that ships.
7. **The rest of the object redesign.**

The work goes straight to `main`, in small commits: trunk-based development,
no pull request. Steps 1 to 5 are `refactor:` or `test:` and make no release.
Step 6 is the `feat:` that does.

### What the spike delivers

**Where it stands, 2026-09-26.** Under Linux, all three are delivered, and a
live game read a complete and correct `State` on the first try.
`GameMode.duration` follows real time to 0.01 %. What is not met yet is the
cost: the Ruffle heap is 300 MiB, and the first search reads 601 MiB. See
[About the Ruffle heap](../concepts/ruffle-heap.md#seen-on-a-live-game).
Windows is not checked.

- `docs/concepts/ruffle-heap.md`: strings, objects and property maps of Ruffle,
  drawn from a real capture;
- a trimmed fixture of a game under Ruffle 0.6.0, like
  `fixtures/replay/main-world`;
- a Python script that reads a live game, as a second opinion.

It succeeds when a complete and correct `State` is read from a live Ruffle,
with a first scan of the same order of cost as for Pepper Flash. It fails when
the layout cannot be found in a stable way, or when the scan is much slower.

It also checks one fact: that `GameMode.duration` still follows real time
under Ruffle. Under Pepper Flash the difference is 0.1 %.

### Open for step 4

One `read` gives no order between the bytes it copies. The game can change an
object while we read it. The Pepper Flash reader already lives with this.
The design of `Avm1Heap` must say how it does.
