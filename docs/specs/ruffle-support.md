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
