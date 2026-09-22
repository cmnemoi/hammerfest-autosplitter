# Memory reader

The layer that turns the bytes of a Flash heap into a `State`, or into nothing.

How the net around it is built is a separate page:
[About testing the memory reader](../internals/testing-the-memory-reader.md).

---

## Why

About 1400 lines of Rust stand between a process and a `State`. For a long
time none of it had a test, and none of it could: `cargo test` on that crate
failed to link, with 30 unresolved `asr` symbols.

The risk is not that the layer finds nothing. It is that it finds something
wrong. A missing `State` costs a few hundred milliseconds of display. A false
one starts the timer on a dead game, or reports a level the player is not on,
and says nothing while it does.

## Scope

From "here is a memory, and the range of the plugin module" to "here is a
`State`, or none".

Outside: finding the process, attaching to it, and every decision made from the
`State`.

---

## Rules

### A true state, or none

`{#reader::a-true-state-or-none}`

The layer produces a `State` whose every field comes from the object graph of
the `GameMode` the game is running, or it produces nothing. It never produces a
`State` that is partly true.

Three objects carry the fields:

```text
   GameMode           dim, game_over, locked, duration_ms, end_sequence
     .world           level, previous            a GameMechanics
     .gameChrono      chrono_ms, frame_timer     a Chrono
```

The `GameManager` carries none. It is a cheap route to the `GameMode`, and a
proof that the object found is one: `current` leads to a mode whose `manager`
points back. A game can be read without it, more slowly and on weaker evidence.

### It is a GameMode

`{#reader::it-is-a-game-mode}`

Other objects carry a `world` property and point at the same `GameMechanics`.
A `View` is one. The layer returns the `GameMode` and never one of them.

### Its world is a known world

`{#reader::a-known-world}`

`setName` must be one of the five worlds the game ships. This is checked when
the game is found and again on every reading, because memory is recycled.

### Its level is a level

`{#reader::a-level-in-range}`

`currentId` must be between 0 and 255. Levels above 103 exist, in the parallel
worlds.

### The bytes were decoded with the right layout

`{#reader::the-right-layout}`

The offsets of a String, a property table and a ScriptObject are measured at
run time. Under a layout that does not match the binary, neighbouring bytes
decode into plausible numbers. The layer must then find nothing.

A layout that has already read something is kept and trusted. So a player who
updates the plugin between two games gets nothing until the autosplitter is
started again. That is the decision this rule makes: no reading beats a wrong
level.

### The reading refuses rather than defaults

`{#reader::refuses-rather-than-defaults}`

When a field the `State` needs cannot be read, the whole reading fails. No
field is filled with a default value.

`duration` is the field that makes this a rule rather than a taste. Reading it
as zero would place the origin of the run at the instant of the resolution, so
the timer would be short by the whole delay of the search, silently.

---

## Acceptance criteria

### Finding the game

| id | given | then |
| --- | --- | --- |
| `reader.find::nothing-in-the-menus` | a heap with no `GameMode` | nothing is found |
| `reader.find::a-game-and-its-manager` | a game and its `GameManager` | the game is found |
| `reader.find::an-orphan-game` | a game and no `GameManager` | the game is found |
| `reader.find::rejects-a-game-already-over` | a game whose `fl_gameOver` is true | nothing is found |
| `reader.find::the-game-not-one-of-its-views` | a game and three `View` objects of it | the game is found, not a view |
| `reader.find::the-first-of-two-candidates` | two games that both pass every rule | the first is found |
| `reader.find::a-parallel-world` | a game in `xml_deepnight`, dimension 1 | the game is found, with dimension 1 |
| `reader.find::nothing-on-another-flash-build` | a game written by another Flash build, and a reader that has already proven the layout of the first build | nothing is found, and never a wrong level |
| `reader.find::the-same-game-when-looking-again` | a game, looked for twice, the heap unchanged | the same game is found |
| `reader.find::the-new-game-not-the-corpse` | a game replaced by another between two looks | the new game is found |

### Reading the state

| id | given | then |
| --- | --- | --- |
| `reader.read::the-nominal-state` | a game at level 2 in `xml_adventure` | the state carries the level, the world, the dimension and the clocks |
| `reader.read::rejects-an-unknown-world` | a game whose `setName` is no longer a known world | nothing is read |
| `reader.read::rejects-a-level-out-of-bounds` | a game whose level is above 255 | nothing is read |
| `reader.read::rejects-a-missing-duration` | a game whose `duration` cannot be read | nothing is read |
| `reader.read::rejects-a-missing-chrono` | a game whose `gameChrono` cannot be read | nothing is read |
| `reader.read::the-halted-clock-when-stopped` | a game whose `fl_stop` is true | the clock comes from `haltedTimer` |
| `reader.read::a-property-that-moved-slot` | a game whose `world` property changed slot between two readings | the value read is still right |

A behaviour counts as covered when its id appears in three places: this page, a
test, and the code that implements it.

```rust
/** @spec reader.find::the-game-not-one-of-its-views */
#[test]
fn finds_the_game_and_not_one_of_its_views() { ... }
```

---

## Out of scope

**Whether the object is still alive.** A corpse has bytes identical to a live
game. Only a frozen `frameTimer` gives it away, which takes two readings
separated in time. Time lives in `core`, which tests it with
`drops_the_resolution_when_the_heartbeat_freezes`.

**What it cost.** Read counts, byte counts, the number of passes and the
yielding budget are the subject of a benchmark, not of a test. Their effect is
that LiveSplit does not freeze, and there is no LiveSplit in a test.

**Which path found the game.** The fast path, the `fVersion` search and the
`world` fallback produce the same answer. A test that asserts the path asserts
an implementation detail, and the redesign will move all three.

**Finding the process.** Listing the EternalTwin processes and attaching to the
one that loaded the plugin stays outside this layer.

**A rule that lives here and belongs to `core`.** `validate` rejects a
`GameMode` whose `fl_gameOver` is true, which means the caller cannot tell "no
game" from "a game, and it is over". Keeping it was a decision, on the grounds
that attaching during a game over screen is rare and the time lost is
acceptable. It is listed here so that the redesign does not mistake it for an
accident.

**Whether the constants match what ships.** A test writes its heap with a
layout table of its own, and with the obfuscated names from `keys::`. It
therefore cannot see a `MEASURED` seed or an obfuscation table that is wrong
for the binary and the SWF in use. Only running the autosplitter on a real game
sees that.

**The number of situations above.** Seventeen is where the search stopped, not
where the defects end. The list grows with the bugs we find.
