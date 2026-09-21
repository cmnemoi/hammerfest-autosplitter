# About testing the memory reader

Read this when: you are about to build, extend or move the test net around the
layer that turns heap bytes into a `State`.

You need: [About finding the game](finding-the-game.md).

What that layer must do is not here. It is the spec:
[Memory reader](../specs/memory-reader.md). This page says how the net that
proves it is built, and in what order.

---

## What a test of this layer may look at

The layer has one client, `core::Policy`, and that client sees two things: a
`State`, or nothing. The observable surface is that, and nothing else.

```text
observable, because the core sees it

   is there a State?
   what is in it?
   does it survive the next reading?
   does it come back after a loss?

implementation detail, because the core cannot tell the difference

   which of the three paths found it
   whether the anchor was reused
   whether the scan was differential
   how many reads it cost
   whether a hint was hit
   the order of the candidates
   the layout seed rather than the derivation
```

The spec lists the same line under Out of scope, from the behaviour side.

---

## The seam

The reading code cannot be tested where it lives. `cargo test -p
hammerfest-autosplitter` fails to link, with 30 unresolved `asr` symbols,
`process_read` among them.

So the AVM1 model and the resolution move to a crate that does not depend on
`asr`: `hammerfest-memory-reader`. `build.rs` moves with them, because `keys::`
is used there and nowhere else. `vendor/hf.map.json` stays at the workspace
root, where `scripts/hfmap.py` also reads it.

A survey of the code that moves gives the exact surface it needs from outside:

```text
   process.read_into_slice        one call      block reads
   process.read::<T>              three calls   typed reads, 2 to 8 bytes
   process.memory_ranges          one call      the regions
   asr::print_message             four calls    telling the runner what was found
   next_tick().await              four calls    yielding during a scan
   diagnostics::validation_read   two calls
   ScanTrace                      six calls     the scan counters
```

That becomes two traits.

```text
   Memory   read_into, regions, module, say, diag, now_us, validation_read
            dyn safe, so avm1 takes a &dyn Memory and carries no type parameter

   Host     Memory, plus yield_now
            used only by the scans, which are the only async code
```

`say` was not in the original design. It appeared in the survey: the reader
prints four messages for the runner. The reader must not talk to LiveSplit, so
it says, and the host decides what a message means. `diag` is the same for the
diagnostics build, which removes every feature flag from the moved code.

The scan stays `async`. Making it synchronous is a redesign, and a net is not
the moment for one.

---

## The builder's layout

The builder writes bytes, the reader reads them, and both need the same table
of offsets. Where the builder takes its own decides what the tests can see.

It takes a table written in the test module. Not `MEASURED`, the seed the
production code uses.

A test double that shares its constants with the code under test cannot see an
error in those constants. If someone mistypes `str_buf` from `0x08` to `0x10`
and the builder follows, the test stays green while production finds nothing.

So the duties split. The synthetic heap tests the reading, against a fixed
layout. The real capture tests the derivation, against a real binary.

---

## The fixture

A characterization test on a real capture pins today's behaviour before
`validate` changes.

Captures hold 19 MiB of heap, which does not belong in git. Most of it does not
matter: zeroing the 124 regions of a real capture one at a time and replaying
the resolution showed that 5 regions carry the answer. Trimmed and
recompressed, the fixture weighs 2.58 MiB.

That measurement ran against the `world` path only. The trimming has to be
redone against the Rust algorithm, which tries `fVersion` and the `GameManager`
first, so expect three to six megabytes.

The trimmed fixture lives with the tests, in `memory-reader/tests/fixtures/`.
The `fixtures/` directory at the root keeps its single meaning: captures taken
on a machine, too big for git.

---

## The DSL

Three phases, each nameable. The chain is sugar over the same functions, so a
test can break it apart when that reads better.

```rust
// the chain
given_a_heap()
    .with_a_manager_and_a_game()
        .in_world("xml_adventure")
        .at_level(2)
        .with_frame_timer(23_677)
    .and_three_views_of_that_game()
.when_we_look_for_the_game()
.then_it_is_found()
    .at_level(2)
    .with_frame_timer(23_677);

// the same, one phase per variable
let heap  = given_a_heap().with_a_manager_and_a_game().at_level(2).build();
let found = when_we_look_for_the_game(&heap);
then_it_is_found(found).at_level(2);
```

Rules the vocabulary follows:

- **Values a test asserts on are arguments.** The name goes in the method,
  since Rust has no named arguments. Defaults stay allowed for values the test
  does not care about.
- **Domain facts, not mechanisms.** `written_by_another_flash_build()`, not
  `laid_out_like(WINDOWS_LAYOUT.shifted_by(8))`.
- **The layout belongs to the heap**, not to one game. Every object in a heap
  is written by the same Flash build.
- **Finding and reading stay two steps**, because they are two contracts with
  two costs. `.and_look_again()` and `.when_we_read_it_again()` carry the
  situations that happen over time.
- **Both `with_a_game()` and `with_a_manager_and_a_game()` exist.** A heap
  always has a manager in production, so a test that omits it is exercising the
  fallback on purpose.

---

## The plan, and what each step covers

### Step 1, the crate and the seam

Create `memory-reader`. Move `avm1.rs` and the resolution half of
`hammerfest.rs`. Add `Memory` and `Host`. Move `build.rs`.

No behaviour changes, so no acceptance criterion is covered. The proof is that
the `.wasm` still builds and the 49 core tests still pass.

### Step 2, a real capture under test

Port the fixture reader to Rust, trim a capture, commit it, and pin today's
behaviour against it.

| covered | against |
| --- | --- |
| `reader.find::a-game-and-its-manager` | real memory |
| `reader.find::the-game-not-one-of-its-views` | real memory, which holds four candidates |
| `reader.read::the-nominal-state` | real memory |

Three of the seventeen, and the only three that will ever run against bytes a
Flash player actually wrote.

### Step 3, the builder and the rest

Write the heap builder and the remaining situations.

| covered | against |
| --- | --- |
| the ten `reader.find::` criteria | a synthetic heap |
| the seven `reader.read::` criteria | a synthetic heap |

The three of step 2 are written twice on purpose: once against real memory,
once against a synthetic heap. They detect through independent paths, which is
the case where overlap pays.

### Step 4, the change the net was for

Change `validate` to identify the `GameMode` positively, through the `manager`
back-pointer, and see what steps 2 and 3 say.

Today the identification is negative: it is not a `View`, because it also owns
a `gameChrono`. The `GameManager` path already proves itself positively,
through a pointer that comes back. The asymmetry is the defect, and
`reader.find::the-game-not-one-of-its-views` and `reader.find::an-orphan-game`
are the two criteria that will judge the fix.
