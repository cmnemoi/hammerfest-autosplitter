# The Hammerfest autosplitter

What it does, how it is built, and where everything lives.

Read this page first. It covers the whole project in outline. Each other page
goes deeper on one part, and you do not need any of them to follow this one.

---

## What it does

A speedrunner races Hammerfest. LiveSplit is the stopwatch: it cuts the run
into slices, and the runner presses a key at the end of each one.

The autosplitter presses the key instead. It answers three questions:

| question | what it sends LiveSplit |
| --- | --- |
| has the run started? | `start` |
| has the player crossed a level, or changed dimension? | `split`, then one `skip split` per level a warp zone carried them over |
| is the run over, or abandoned? | `split`, then `reset` |

All three answers come from one number, the current level, plus a clock. So the
job is to read the level and the clock out of a running game, then decide.

We only read memory. The game runs exactly as it would without us.

---

## The pipeline

```text
   the Flash plugin process
            |
            |  1. find it, and read its memory
            v
   the AVM1 heap, 124 regions, 85 MiB
            |
            |  2. find the game objects in there
            v
   the GameMode object
            |
            |  3. read level, world, clocks, flags
            v
   a State: one snapshot of the game
            |
            |  4. compare with the last one, decide
            v
   start / split / reset / the time
            |
            v
        LiveSplit
```

Steps 1 to 3 know nothing about speedrunning. Step 4 knows nothing about
memory. The next section explains why that split is necessary.

---

## The four jobs

### 1. Find the process

EternalTwin starts half a dozen processes under one name. Only one loaded
`pepflashplayer.dll`. Finding out means attaching to each and listing its
modules.

That process lives only while a Flash instance lives. Its identifier is never
cached. → `src/hammerfest.rs`, `attach_plugin`

### 2. Find the game in the heap

This is the hard part. Hammerfest is ActionScript, interpreted by a virtual
machine, so the level is a property of an object created while you play. There
is no fixed address, and no fixed chain of pointers to one.

So we find the objects again every time, by scanning for a known string, then
for the tables that cite it. → [About finding the game](internals/finding-the-game.md),
[About why Flash is hard](concepts/why-flash-is-hard.md)

### 3. Read the state

From the `GameMode` object, follow the property graph and decode the values.
Everything is read again on every tick, and checked again, because an
abandoned object stays perfectly readable.
→ [About AVM1 values](concepts/avm1-values.md), [About AVM1 objects](concepts/avm1-objects.md),
[About stale memory](internals/stale-memory.md)

### 4. Decide

A pure state machine. It receives a `State` and returns `Actions`. It reads no
memory, talks to no runtime, and knows nothing about Flash.
→ [About the clocks](internals/the-clocks.md)

---

## Why the code is split in two crates

```text
src/           the infrastructure. It decides nothing.
src/core/      the decisions. No dependency, no memory, no runtime.
```

Every Rust file lives under `src/`. `src/core/` is its own crate, and its
sources sit beside its `Cargo.toml` rather than under a second `src/`.

The LiveSplit runtime symbols exist only inside the WebAssembly sandbox.
`core` receives a `State`, returns `Actions`, touches none of them, and carries
77 tests. It cannot depend on `asr`, and that is what proves the claim: the
compiler enforces it, not a convention.

`src/` can be tested too. The reader reads through one trait of our own,
`avm1::Memory`, so a test serves it a heap of its own making. Only the
three-line adapter below that trait touches the runtime.
See [About testing the memory reader](internals/testing-the-memory-reader.md).

---

## Two design choices

**The search is off the critical path.** A heap scan takes half a second, and
the window between the birth of the game objects and the appearance of level 0
is 0.55 s. We do not try to win that race. The game counts how long it has been
playing, so the start is dated after the fact. Measured: 6 ms of difference
over 60.9 s.

**A successful read does not prove the object is alive.** When a game ends, its
objects stay readable, with a plausible level and a plausible clock. They are
simply the old ones. Three separate checks guard against that.

---

## The code

| file | job |
| --- | --- |
| `src/core/policy.rs` | when to start, split, reset. The state machine. |
| `src/core/end_sequence.rs` | the end of the run, at the elevator |
| `src/core/atom.rs` | decoding one AVM1 value |
| `src/core/pacing.rs` | when the loop may scan the whole heap again |
| `src/core/command.rs` | what one tick sends the timer, in order |
| `src/lib.rs` | the main loop, and talking to LiveSplit |
| `src/hammerfest.rs` | finding the process, scanning, reading the game |
| `src/avm1.rs` | the AVM1 object model, measured at run time |
| `src/diagnostics.rs` | everything that measures. Absent from the normal build. |
| `build.rs` | turns the obfuscated names into Rust constants |
| `src/asr_stubs.rs` | 28 runtime symbols, so `cargo test` can link. Tests only. |
| `src/memory_contract.rs` | the contract of `avm1::Memory`, run against both implementations |
| `src/test_heap.rs` | a synthetic AVM1 heap, written byte by byte. Tests only. |
| `src/replay.rs` | a real capture, replayed. The only test served Flash bytes. |
| `scripts/` | reading and capturing memory, in Python |

---

## The documents

| you want | read |
| --- | --- |
| to install and use it | [README](../README.md) |
| to run it on macOS | [Run it on macOS](how-to/run-on-macos.md) |
| where the test net stands, and what is next | [TODO](../TODO.md) |
| why there is no fixed address | [About why Flash is hard](concepts/why-flash-is-hard.md) |
| how a value is packed in 8 bytes | [About AVM1 values](concepts/avm1-values.md) |
| how to follow a pointer to a property | [About AVM1 objects](concepts/avm1-objects.md) |
| why the property names are gibberish | [About the obfuscation](concepts/obfuscation.md) |
| how the game is located, and how fast | [About finding the game](internals/finding-the-game.md) |
| why a valid read can lie | [About stale memory](internals/stale-memory.md) |
| how the run is timed to the frame | [About the clocks](internals/the-clocks.md) |
| why the search must stay cheap | [About speed](internals/speed-matters.md) |
| what the memory reader must do | [Memory reader](specs/memory-reader.md) |
| what a level crossing sends LiveSplit | [Level crossings](specs/level-crossings.md) |
| when the loop may scan again | [Resolution pacing](specs/resolution-pacing.md) |
| why and how Ruffle will be read | [Ruffle support](specs/ruffle-support.md) |
| what we send the timer, and in what order | [Timer commands](specs/timer-commands.md) |
| how that is proved, and in what order | [About testing the memory reader](internals/testing-the-memory-reader.md) |
| to read a live game yourself | [Read a live game](how-to/read-a-live-game.md) |
| to record memory for tests | [Capture a fixture](how-to/capture-a-fixture.md) |
| to measure the start delay | [Measure the startup](how-to/measure-the-startup.md) |
| to check it still works, before a run | [Check before a session](how-to/check-before-a-session.md) |
| what the net refuses to hold, and why | [What the net does not hold](internals/what-the-net-does-not-hold.md) |
| the proof of any claim above | [reverse-engineering.md](reverse-engineering.md) |

---

## What the tests cover

`core` is under test. The reader is too: eighteen situations on a heap a test
writes byte by byte, plus one capture of a real game replayed from the bytes to
the split. The loop keeps only what asks the runtime a question.

The state of the net, and what comes next, live in one place:
[TODO](../TODO.md). It is computed by `mise run spec-coverage`, so read that
rather than trust a number on a page.

How the net is built, and in what order:
[About testing the memory reader](internals/testing-the-memory-reader.md).
What it deliberately leaves out:
[What the net does not hold](internals/what-the-net-does-not-hold.md).

---

## What is still open

**The LiveSplit *real time* stays late.** It carries the search delay. The
runtime API cannot move a running timer backwards, so the correct time travels
in the *game time* channel. In LiveSplit: **Compare Against → Game Time**.

**The final split has never been seen.** The rule that ends a run at the
elevator is proved by the game source and covered by tests, but no finished run
has confirmed it. Finishing Hammerfest takes a while.

**There are no settings.** The module always starts, splits on a level
crossed in the main world, splits at the elevator, and resets. A settings
struct existed, reached no LiveSplit control, and was removed. If a setting is
ever wanted, it arrives through the runtime settings API with a real path from
LiveSplit.

**A parallel dimension splits, but never ends the run.** The main route goes
through one, the one runners write `97.0`: level 97 opens it, and leaving it
lands on level 99. Entering and leaving are both crossings. The elevator of a
dimension ends nothing.

`world` follows `currentDim`, so a level number read inside a dimension belongs
to that dimension and can never be compared with a number from another. What
that number actually is has not been observed. The split rule never reads it.

**One version, one machine.** The layout is derived at run time. A different
Flash build fails cleanly rather than report a wrong level, and
`reader.find::nothing-on-another-flash-build` holds that. What no test can see
is the day the player itself is updated: the replay test then replays the old
bytes, and only a real game shows it.

A new build of the game is a separate problem. It renames every identifier, and
the cure is to regenerate the obfuscation table.
See [About the obfuscation](concepts/obfuscation.md).
