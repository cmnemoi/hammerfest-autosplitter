# Ruffle support

Why the autosplitter will read Ruffle, how, and in what order. The decisions
were taken on 2026-09-26. Each one carries its reason.

The rules of the Ruffle reader are under [Rules](#rules). The scenarios the
two players share are in [Memory reader](memory-reader.md).

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
- Ruffle desktop, on Linux and Windows first;
- Ruffle in a browser, as the extension runs it on eternalfest.net: Firefox
  under Linux first, then Chrome and Windows, each checked on a live game. See
  [Ruffle in a browser](#ruffle-in-a-browser).

Out:

- other contrées of Eternalfest: their worlds and ends need rules of their own,
  in `core`, not in the reader;
- a site that hosts its own Ruffle: it may pin any version, and its layout;
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
of `fixtures/replay/linux-ruffle` is the only test that checks the
reader's offsets against bytes Ruffle wrote, as `windows-pepper-flash` does for Pepper
Flash.

The read cost baseline gains six situations: four on the Ruffle fixture, two
on the synthetic Ruffle heap.

### Step 5 keeps every read

Extracting `PepperFlashHeap` is a pure refactoring: the baseline does not
move. A saving comes after, in a commit of its own, with its baseline.

---

## Rules

What the Ruffle reader adds to the rules of [Memory reader](memory-reader.md).
Every scenario of that page runs on Ruffle too.

### An entry answers for its key by its hash

`{#ruffle::an-entry-answers-by-its-hash}`

Ruffle keeps, beside each key of a map, the FNV hash of that key. The reader
reads a property from an entry only when the hash it holds is the hash of the
key asked. An entry that holds another hash, or memory that was an entry once,
answers nothing.

### An object is proven by its type

`{#ruffle::an-object-is-proven-by-its-type}`

Every object Ruffle collects carries, in front of it, the vtable of its type,
and that vtable gives its size. A candidate is an AVM1 object only when that
vtable lies in the module and says 8 and 160. A map found in another kind of
allocation is not an object, and is never read.

### The entries are found again at every read

`{#ruffle::the-entries-are-found-again}`

Ruffle moves the entries of a map when the map grows. The reader keeps the
object, never the entries: it reads where they are at every read, and checks
that the entry it remembers still holds the key.

### A real game is read

`{#ruffle::a-real-game-is-read}`

The bytes Ruffle 0.6.0 wrote under Linux, in
`fixtures/replay/linux-ruffle`, give the level, the world and the
dimension the game showed.

### Windows is read as Linux is

`{#ruffle::windows-is-read}`

The Windows build of Ruffle 0.6.0 is the same Rust on the same processor as the
Linux one, and its objects, strings and maps have the same layout. The bytes
it wrote under Wine 10.0, in `fixtures/replay/windows-ruffle-wine`, give the
level, the world and the dimension the game showed, read with the desktop
layout.

Seen on the live game, 2026-09-26: the heap was 459 MiB, and 647 objects held
the key of `world`, against about ten under Linux.

## Acceptance criteria

| id | given | then |
| --- | --- | --- |
| `ruffle.read::an-entry-whose-hash-lies` | a game whose `currentId` entry holds the hash of another key | nothing is read |
| `ruffle.find::a-map-that-is-not-an-object` | a game whose object carries the vtable of another type | nothing is found |
| `ruffle.read::entries-that-moved` | a game whose entries were moved elsewhere, and reordered, after the first read | the level is still read |
| `ruffle.replay::the-windows-main-world` | the capture of a game at level 25 of `xml_adventure`, in Ruffle for Windows under Wine | the game is found, at level 25, in `xml_adventure`, dimension 0 |
| `ruffle.replay::the-main-world` | the capture of a game at level 2 of `xml_adventure` | the game is found, at level 2, in `xml_adventure`, dimension 0 |

---

## Ruffle in a browser

Hammerfest on eternalfest.net, played with the Ruffle extension, in Firefox
under Linux first. The layout is on
[About the heap of Ruffle in a browser](../concepts/ruffle-web-heap.md).

### Decisions

**One Ruffle reader, two layouts.** The logic of a Ruffle map -- an entry
answers by its hash, an object is proven by its type -- comes from the source
of Ruffle, the same for desktop and web. The widths and offsets come from the
target and the version, and change for other reasons: they are data, one table
per target. A third driver runs the shared scenarios on the web layout, so a
change to the shared logic is tested on every player.

**Refused: a separate web reader.** Two copies of the same logic would drift.
When the web code has to diverge for real, the fork is made then.

**Offsets, not addresses.** Every pointer in a linear memory is an offset from
its base. `LinearMemory` turns an offset into an address, as `ProcessMemory`
turns an address into bytes, and the reader works with offsets.

**The base is found by the shape of its range, then proven by a build.** A
linear memory is an `rw` range followed by a reserve of 4 GiB or more. The
vtables of a known build of Ruffle, at their offsets, prove it and name the
build.

**Several tabs.** The first process in which the strategy finds a
`GameManager` is read. The others are named in the log.

### Rules

#### A linear memory is proven by a known build

`{#browser::proven-by-a-known-build}`

A linear memory is read only when the vtables of the object and of the string
of a known build of Ruffle sit at their offsets in it. Any other build, or
another wasm module, is not read at all.

#### An offset is read from the base

`{#browser::an-offset-from-the-base}`

The reader asks for offsets. An offset is read at the base plus that offset,
and only inside the linear memory: a read that runs past its end reads
nothing.

#### A linear memory is swept by blocks

`{#browser::swept-by-blocks}`

A linear memory is one range, and it grows at its end. The search sweeps what
changed since the last one, and a single range would always look changed as a
whole. So it is cut into blocks of 1 MiB: when it grows, only the blocks that
grew are new, and the objects the game creates there are looked for first.

Found on 2026-09-26 with the live check: the game loaded 0.36 s before level 0,
and a search of the whole 97 MB took 1.24 s.

#### A real game in a browser is read

`{#browser::a-real-game-is-read}`

The bytes Ruffle 0.6.0 wrote in Firefox under Linux, in
`fixtures/replay/linux-ruffle-web`, give the level, the world and the
dimension the game showed.

### Acceptance criteria

| id | given | then |
| --- | --- | --- |
| `browser.find::the-extensions-build` | a linear memory with the vtables of the extensions build | the build is recognised |
| `browser.find::the-mvp-build` | a linear memory with the vtables of the MVP build | the build is recognised |
| `browser.find::an-unknown-build` | a linear memory whose vtables are not where a known build puts them | nothing is recognised, and no game is looked for |
| `browser.find::blocks-of-a-linear-memory` | a linear memory of 2.5 MiB | three blocks: two of 1 MiB, and one of 0.5 MiB at its end |
| `browser.find::an-empty-linear-memory` | a linear memory of no committed byte | no block |
| `browser.read::past-the-end` | a read that runs past the end of the linear memory | nothing is read |
| `browser.replay::the-main-world` | the capture of a game at level 2 of `xml_adventure`, in Firefox | the game is found, at level 2, in `xml_adventure`, dimension 0 |

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
Windows was checked under Wine on the same day: see
[the rule](#windows-is-read-as-linux-is).

- `docs/concepts/ruffle-heap.md`: strings, objects and property maps of Ruffle,
  drawn from a real capture;
- a trimmed fixture of a game under Ruffle 0.6.0, like
  `fixtures/replay/windows-pepper-flash`;
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
