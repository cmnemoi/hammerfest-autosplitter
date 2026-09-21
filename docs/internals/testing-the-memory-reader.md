# About testing the memory reader

Read this when: you are about to add, move or delete a test of the layer that
turns heap bytes into a `State`.

You need: [About finding the game](finding-the-game.md).

This page is the specification of that layer. It says what the layer promises,
what a test of it may look at, and which situations the net covers. It came out
of a design interview, and the list of situations is a starting point: it will
grow with the bugs we find.

---

## What the layer promises

One sentence, and everything else follows from it.

> Produce a `State` that is true, or none. Never a false one.

A `State` is true when every field in it comes from the object graph of the
`GameMode` the game is running now. Not a copy, not a corpse, not a lookalike.

Three objects carry the fields:

```text
   GameMode           dim, game_over, locked, duration_ms, end_sequence
     .world           level, previous            a GameMechanics
     .gameChrono      chrono_ms, frame_timer     a Chrono
```

The `GameManager` supplies no field. It has two other parts:

```text
   a route    GameManager.current leads to the GameMode, in a handful of
              reads instead of a hundred megabytes. The same GameMode can
              be reached without it, by scanning for `world`.

   a proof    `current` leads to a mode whose `manager` points back at it.
              A coincidence does not survive that round trip, which makes
              it evidence for criterion 1.
```

That is why an orphan game is a legitimate situation: with no
`GameManager` the layer still produces a true `State`, only more slowly and on
weaker evidence.

The five criteria:

| | criterion | checked by |
| --- | --- | --- |
| 1 | it is a `GameMode`, not another object carrying `world` | this layer |
| 2 | it is the one running now | `core`, over time |
| 3 | its world is one of the five known ones | this layer |
| 4 | its level is between 0 and 255 | this layer |
| 5 | the bytes were decoded with the right layout | this layer |

Criterion 2 is not this layer's job, and that is not an oversight. A corpse has
bytes identical to a live game: the level is right, the world is right, the
clock is plausible. Only a frozen `frameTimer` gives it away, and that takes two
readings separated in time. Time lives in `core`, which already tests it with
`drops_the_resolution_when_the_heartbeat_freezes`.

---

## What a test may look at

The layer has exactly one client, `core::Policy`, and that client sees exactly
two things: a `State`, or nothing. So the observable surface is that, and
nothing else.

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

**Cost is not a test.** The scan budget decides when the scan yields to the
LiveSplit runtime, and its effect is that LiveSplit does not freeze. There is no
LiveSplit in a unit test. Read counts and byte counts belong to the benchmark
harness, where the `Scan` counters already live.

**There is no business code in this layer.** The rules live in `core`. One rule
leaked and stays for now: `validate` rejects a `GameMode` whose `fl_gameOver` is
true, which means the caller cannot tell "no game" from "a game, and it is
over". The decision was to keep it, because attaching during a game over screen
is rare and the time lost is acceptable.

---

## The situations

### Finding the game

`find_the_game` runs rarely and costs a hundred megabytes.

| id | situation | expected |
| --- | --- | --- |
| `reader.find::nothing-in-the-menus` | no `GameMode` at all | nothing found |
| `reader.find::a-game-and-its-manager` | a game and its `GameManager` | found |
| `reader.find::an-orphan-game` | no `GameManager` | found, through the fallback |
| `reader.find::rejects-a-game-already-over` | `fl_gameOver` is true | nothing found |
| `reader.find::the-game-not-one-of-its-views` | three `View` objects carry `world` too | the `GameMode` |
| `reader.find::the-first-of-two-candidates` | two valid candidates | the first |
| `reader.find::a-parallel-world` | `xml_deepnight`, dim 1 | found, dim 1 |
| `reader.find::nothing-on-another-flash-build` | offsets shifted | nothing found, never a wrong level |
| `reader.find::the-same-game-when-looking-again` | looking twice, the heap unchanged | the same game |
| `reader.find::the-new-game-not-the-corpse` | looking twice, a new game replaced the old | the new one |

`an-orphan-game` answers an open question of the project: is the direct `GameMode`
fallback a real safety net, or a scar left by an early wrong assumption? With
that test we will know.

`nothing-on-another-flash-build` turns the biggest open assumption of the project into a test. Until
now the claim was that a different Flash build "should fail cleanly rather than
report a wrong level, but that has not been tested".

### Reading the state

`game.read` runs on every tick and costs microseconds.

| id | situation | expected |
| --- | --- | --- |
| `reader.read::the-nominal-state` | the nominal reading | level, world, dimension, clocks |
| `reader.read::rejects-an-unknown-world` | `setName` is no longer a known world | nothing read |
| `reader.read::rejects-a-level-out-of-bounds` | the level goes above 255 | nothing read |
| `reader.read::rejects-a-missing-duration` | `duration` cannot be read | nothing read |
| `reader.read::rejects-a-missing-chrono` | `gameChrono` cannot be read | nothing read |
| `reader.read::the-halted-clock-when-stopped` | `fl_stop` is true | the clock comes from `haltedTimer` |
| `reader.read::a-property-that-moved-slot` | a property moved slot between two readings | the value is still right |

`rejects-a-missing-duration` deserves a note. Reading `duration` as zero by default would place
the origin at the instant of the resolution, so the timer would be short by the
whole delay of the scan, silently. Refusing the reading is the deliberate
choice, and this test pins it.

`a-property-that-moved-slot` is the only observation of the hints. A hint says "`world` was at
slot 68, try there first". It is an optimisation, never a source of truth, and
the value must stay right when it misses.

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

- **Values a test asserts on are arguments.** The name goes in the method, since
  Rust has no named arguments. Defaults stay allowed for values the test does
  not care about.
- **Domain facts, not mechanisms.** `written_by_another_flash_build()`, not
  `laid_out_like(WINDOWS_LAYOUT.shifted_by(8))`.
- **The layout belongs to the heap**, not to one game. Every object in a heap is
  written by the same Flash build.
- **Finding and reading stay two steps**, because they are two contracts with
  two costs. `.and_look_again()` and `.when_we_read_it_again()` carry the
  situations that happen over time.
- **`with_a_game()` and `with_a_manager_and_a_game()` are both real.** A heap
  always has a manager in production, so a test that omits it is testing the
  fallback on purpose.

---

## How the net is built

### Traceability

Every situation carries a stable id. A behaviour counts as covered only when
the id appears in three places: this page, at least one test, and the code that
implements it.

```rust
/** @spec reader.find::the-game-not-one-of-its-views */
#[test]
fn finds_the_game_and_not_one_of_its_views() { ... }
```

### The seam

The reading code cannot be tested where it is: `cargo test -p
hammerfest-autosplitter` fails to link, with 30 unresolved `asr` symbols,
`process_read` among them.

So the AVM1 model and the resolution move to a third crate,
`hammerfest-memory-reader`, which does not depend on `asr`. `build.rs` and
`vendor/hf.map.json` move with them, because `keys::` is used there and nowhere
else.

The seam carries five things: read bytes at an address, list the regions, the
module range, a yield that does nothing in a test, and a way to say a message.
The fifth appeared when the move was surveyed: `hammerfest.rs` calls
`print_message` four times to tell the runner what it found. The reader must
not talk to LiveSplit itself, so it says, and the host decides what that means. The scan stays `async`;
making it synchronous is a redesign, and a net is not the moment for one.

`diagnostics.rs` stays where it is, behind a trace trait the new crate defines
and `src/` implements.

### The builder's layout

The builder writes bytes; the reader reads them. Both need the same table of
offsets, and where the builder gets it decides what the tests can see.

It gets its own table, written in the test module. Not `MEASURED`, the seed the
production code uses.

The reason: a test double that shares its constants with the code under test
cannot see an error in those constants. If someone mistypes `str_buf` from
`0x08` to `0x10` and the builder follows, the test stays green and production
finds nothing.

So the duties split. The synthetic heap tests the reading, against a fixed
layout. The real capture tests the derivation, against a real binary.

### The fixture

A characterization test on a real capture pins the current behaviour before
`validate` changes.

Captures are 19 MiB of heap, which does not belong in git. But most of it does
not matter: zeroing the 124 regions of a real capture one at a time and
replaying the resolution showed that 5 regions carry the answer. Trimmed and
recompressed, the fixture weighs 2.58 MiB.

That measurement ran against the `world` path only. The trimming must be redone
against the Rust algorithm, which tries `fVersion` and the `GameManager` first,
so expect three to six megabytes for a complete one.

The trimmed fixture lives with the tests, in
`hammerfest-memory-reader/tests/fixtures/`. The `fixtures/` directory at the
root keeps its single meaning: captures taken on a machine, too big for git.

### The order

1. Create the crate. Move `avm1.rs` and the resolution half of `hammerfest.rs`.
   Add the seam and the trace trait. No behaviour changes. Proof: the `.wasm`
   builds and the core tests still pass.
2. Port the fixture reader to Rust, trim a capture, commit it, and pin the
   current behaviour on it.
3. Write the heap builder and the seventeen situations.
4. Change `validate` to identify the `GameMode` positively, through the
   `manager` back-pointer, and see what the tests of steps 2 and 3 say.

Step 4 is the only one that changes the product, and it arrives with three nets
watching it. The current identification is negative — "it is not a `View`,
because it also owns a `gameChrono`" — while the `GameManager` path already
proves itself positively, through a pointer that comes back.
