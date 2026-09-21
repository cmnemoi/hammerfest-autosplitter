# The Hammerfest autosplitter

What it does, how it is built, and where everything lives.

**Read this page first.** It tells the whole story at low resolution. Every
other page zooms in on one part of it, and none of them is required to follow
this one.

---

## What it does

A speedrunner races Hammerfest. LiveSplit is the stopwatch: it cuts the run
into slices, and the runner presses a key at the end of each one.

The autosplitter presses the key instead. To do that it answers three
questions, and nothing else:

| question | what it sends LiveSplit |
| --- | --- |
| has the run started? | `start` |
| has the player crossed a level? | `split` |
| is the run over, or abandoned? | `split`, then `reset` |

All three come from one number, the current level, plus a clock. So the whole
job is: **read the level and the clock out of a game that does not know we
exist**, and decide.

We only read memory. We never write, never patch. The game runs exactly as it
would without us.

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

Four jobs. Steps 1 to 3 know nothing about speedrunning. Step 4 knows nothing
about memory. That separation is the whole architecture, and the next section
says why it is not decorative.

---

## The four jobs

### 1. Find the process

EternalTwin starts half a dozen processes under one name. Only one loaded
`pepflashplayer.dll`. Finding out means attaching to each and listing its
modules.

That process lives only while a Flash instance lives. Its identifier is never
cached. → `src/hammerfest.rs`, `attach_plugin`

### 2. Find the game in the heap

The hard one. Hammerfest is ActionScript, interpreted by a virtual machine, so
the level is a property of an object created while you play. **There is no
fixed address and no fixed chain of pointers to one.**

Every time, we find the objects again: by scanning for a known string, then
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
core/    the decisions. No dependency, no memory, no runtime.  -> the tests
src/     the infrastructure. It decides nothing.
```

This is not taste. The LiveSplit runtime symbols exist **only inside the
WebAssembly sandbox**, so anything that touches them cannot run on a
development machine. Everything that must be tested has to be free of them.

Hence `core`, which receives a `State` and returns `Actions`, and carries all
46 tests.

---

## The two ideas worth knowing

**The search is off the critical path.** A heap scan takes half a second, and
the window between the birth of the game objects and the appearance of level 0
is 0.55 s. We stopped trying to win that race. The game itself counts how long
it has been playing, so the start is dated *after the fact*, exactly. Measured:
6 ms of difference over 60.9 s.

**A successful read proves nothing.** When a game ends, its objects stay
readable, with a plausible level and a plausible clock. They are simply the old
ones. Three separate checks exist for that, and they are the most important
correctness code in the project.

---

## The code

| file | job |
| --- | --- |
| `core/src/policy.rs` | when to start, split, reset. The state machine. |
| `core/src/end_sequence.rs` | the end of the run, at the elevator |
| `core/src/atom.rs` | decoding one AVM1 value |
| `src/lib.rs` | the main loop, and talking to LiveSplit |
| `src/hammerfest.rs` | finding the process, scanning, reading the game |
| `src/avm1.rs` | the AVM1 object model, measured at run time |
| `src/diagnostics.rs` | everything that measures. Absent from the normal build. |
| `build.rs` | turns the obfuscated names into Rust constants |
| `scripts/` | reading and capturing memory, in Python |

---

## The documents

| you want | read |
| --- | --- |
| to install and use it | [README](../README.md) |
| why there is no fixed address | [About why Flash is hard](concepts/why-flash-is-hard.md) |
| how a value is packed in 8 bytes | [About AVM1 values](concepts/avm1-values.md) |
| how to follow a pointer to a property | [About AVM1 objects](concepts/avm1-objects.md) |
| why the property names are gibberish | [About the obfuscation](concepts/obfuscation.md) |
| how the game is located, and how fast | [About finding the game](internals/finding-the-game.md) |
| why a valid read can lie | [About stale memory](internals/stale-memory.md) |
| how the run is timed to the frame | [About the clocks](internals/the-clocks.md) |
| why the search must stay cheap | [About speed](internals/speed-matters.md) |
| to read a live game yourself | [Read a live game](how-to/read-a-live-game.md) |
| to record memory for tests | [Capture a fixture](how-to/capture-a-fixture.md) |
| to measure the start delay | [Measure the startup](how-to/measure-the-startup.md) |
| the proof of any claim above | [reverse-engineering.md](../reverse-engineering.md) |

---

## What is still open

**The LiveSplit *real time* stays late** by the search delay. The runtime API
cannot move a running timer backwards, so the correct time travels in the
*game time* channel. In LiveSplit: **Compare Against → Game Time**.

**The final split has never been seen.** The rule that ends a run at the
elevator is proved by the game source and covered by tests, but no finished run
has confirmed it. Finishing Hammerfest takes a while.

**No setting is exposed.** The module applies the defaults: start, split on a
level crossed in the main world, split at the elevator, reset.

**Parallel dimensions are out of scope.** They are read and reported, and they
produce no split.

**One version, one machine.** The layout is derived at run time and a different
Flash build should fail cleanly rather than report a wrong level — but that has
not been tested.
