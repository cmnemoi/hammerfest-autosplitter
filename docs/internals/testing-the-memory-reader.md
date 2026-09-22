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

## The seam is a trait we own

`cargo test -p hammerfest-autosplitter` used to fail to link, with 30
unresolved `asr` symbols. That is where the net was blocked.

The first answer was to define those 30 symbols. It works, and it was wrong.

### Why a double over the runtime ABI was wrong

`asr` declares the runtime as one `extern "C"` block, in `src/runtime/sys.rs`.
A double that implements that block implements somebody else's contract, and
it has to decide what the contract means where the contract is silent.

Here is what that produced. The double said:

```rust
// A read that runs past the end of its range fails whole, as the runtime's
// does. It never returns the part that fitted.
```

`sys.rs` says this, and nothing else:

> Reads memory from a process at the address given. This will write the memory
> to the buffer given. Returns `false` if this fails.

Nothing about partial reads. Nothing about the edges of a range. The double had
invented a rule and dressed it as a promise. The tests would then hold the
reader to a rule the runtime never made, and `reader::refuses-rather-than-
defaults` is exactly the rule that turns on it.

### One method

The reader makes four kinds of raw read: a `u64`, the UTF-16 buffer of a
String, a scan block, and the PE headers behind the `known-flash` feature. All
four say "give me these bytes".

So the contract is one method, in `src/avm1.rs`:

```rust
pub trait Memory {
    fn read_into(&self, address: u64, buf: &mut [u8]) -> Option<()>;
}
```

It fills `buf` whole or returns `None`. That is our rule, stated once, and a
test double obeys it without guessing.

The adapter is three lines, and it is the only place in the project that
assumes anything about the runtime:

```rust
impl Memory for Process {
    fn read_into(&self, address: u64, buf: &mut [u8]) -> Option<()> {
        self.read_into_slice(Address::new(address), buf).ok()
    }
}
```

That assumption is not tested either. What changed is its size and its
address: three lines that a reader can check against `sys.rs`, instead of a
rule scattered through a double the tests trust.

### What it cost

Thirty-three signatures took a `&Process`. Thirty of them now take a
`&dyn Memory`. The change is mechanical, and the reader gained no type
parameter, because the trait is dyn safe by construction.

Three functions keep their `&Process`: `heap_iter`, `heap_ranges` and
`heap_size`. They list the regions of the process, which the spec puts outside
this layer. `resolve` now receives the ranges as a slice, and `src/lib.rs`
gathers them. That was `resolve`'s last tie to the runtime API.

### Both implementations answer the same questions

A trait we own moves the guessing out of the tests. It does not stop the fake
and the adapter drifting apart. If they answer differently, the reader's tests
prove nothing about production.

So `src/memory_contract.rs` states the contract as four questions, and runs
them against both implementations.

```text
   reads the bytes at an address                       simple, one
   reads up to the last byte of a range                boundary
   refuses a read that runs past the end of a range    boundary, exceptional
   refuses an address that is in no range              zero
```

Two more sit outside the shared set, each for one implementation only.

**The heap refuses a read that crosses from one range into the next.** The
adapter forwards such a read and the runtime decides, so the contract stays
silent on it. The heap is the stricter of the two, and stricter is the safe
direction: a reader that needed a straddling read would fail in a test and
work in production, never the other way round.

**The adapter refuses even when the host dirtied the buffer.** `sys.rs`
promises nothing about the buffer when `process_read` returns `false`, so a
runtime may write part of it and then fail. The adapter must still answer
`None`. That question is the one thing the adapter run asks which the fake
cannot.

### What the adapter run proves, and what it does not

It proves the three lines forward the address and the length unchanged, and
turn a failure into `None`.

It cannot prove what LiveSplit does. The bytes come from `src/asr_stubs.rs`, so
the region rule under test is ours on both sides. Only the hostile-host
question escapes that circle, because it asks how our adapter behaves under a
host we do not control.

### The red was checked, not assumed

Three mutations, each reddening only what it should:

| mutation | what reddened |
| --- | --- |
| the adapter adds 1 to the address | 3 tests, adapter suite only |
| the adapter swallows the error and returns `Some` | 3 tests, adapter suite only |
| the heap fills what fits instead of refusing | 2 tests, heap suite only |

### What still has to be stubbed

Twenty-eight symbols, measured and not guessed. `src/asr_stubs.rs` holds them.

| how many | which | body |
| --- | --- | --- |
| 24 | `settings_*`, `setting_value_*`, `timer_*`, `process_list_by_name` | `unimplemented!()` |
| 4 | `process_read`, `process_attach_by_pid`, `process_detach`, `runtime_print_message` | a real one |

The twenty-four are `libasr`'s own object files, for APIs this project never
calls. `/OPT:REF` keeps them because they sit in codegen units it retains. No
design of ours removes them. A stub that fires is a message: the reader grew a
tie to the runtime that the trait does not cover.

Three of the four exist only to let the adapter suite build a `Process` and
serve it bytes. The reader's tests never reach them, because the reader is
served a `Memory` of its own. `runtime_print_message` is the fourth, and it
must return, because the reader prints while it searches.

---

---

## The builder's layout

The builder writes bytes, the reader reads them, and both need the same table
of offsets. Where the builder takes its own decides what the tests can see.

It takes a table written in the test module, in `mod layout`. Not `MEASURED`,
the seed the production code uses.

A test double that shares its constants with the code under test cannot see an
error in those constants. If someone mistypes `str_buf` from `0x08` to `0x10`
and the builder follows, the test stays green while production finds nothing.

The obfuscated names are the same story, and it has no clean answer. The
builder must write the name the reader searches for, so it writes `keys::`, and
it shares those constants with the code under test.

`build.rs` breaks when a wanted identifier leaves `vendor/hf.map.json`, which
catches most of it. A table that is right for the old SWF and wrong for the new
one still passes every test. See
[About the obfuscation](../concepts/obfuscation.md).

One number is shared of necessity: the geometry of a property table. The reader
tries the two profiles of `avm1::PROFILES` and no third, so a heap written in a
third geometry could not be read at all. The builder writes the `windows-x64`
one. Every other offset is the test's own, and they differ from `MEASURED`.

---

## No test reads bytes a Flash player wrote

A characterization test on a real capture was planned, and dropped.

A capture holds 19 MiB of heap. Committing one means trimming it, porting the
Python fixture reader to Rust, and carrying three to six megabytes in git for
ever. It would cover three of the seventeen criteria.

The gap it leaves is narrower than it looks. The synthetic heap already covers
"the layout does not match, so nothing is found": that is
`reader.find::nothing-on-another-flash-build`.

What no test covers is whether the `MEASURED` seed and `vendor/hf.map.json`
match the binary and the SWF that ship. Running the autosplitter on a real
game covers that, and you do it anyway.

Add the trimmed fixture the day you change the derivation code itself. Not
before.

---

## The DSL

It lives in `src/test_heap.rs`, with the builder and the seventeen tests.
Three phases, and each one has a name.

```rust
let mut heap = given_a_heap()
    .with_a_manager_and_a_game()
    .in_world("xml_deepnight")
    .in_dimension(1)
    .build();

let found = when_we_look_for_the_game(&mut heap);

let mut game = then_the_game_is_found(&heap, found);
then_the_world_is(&game, "xml_deepnight");
then_the_state(when_we_read_it(&heap, &mut game)).dimension(1);
```

What the heap may hold:

```text
with_a_manager_and_a_game()          the nominal heap
with_a_game()                        no GameManager, so the fallback runs
that_is_over()                       fl_gameOver is true
and_three_views_of_that_game()       three objects that carry the anchor key
and_a_second_game()                  two candidates, both valid
written_by_another_flash_build()     the String vtable is elsewhere
in_world(name) in_dimension(n) at_level(n)
with_frame_timer(t) with_game_timer(t) with_duration(cycles)
without_a_duration() stopped_at(ms)
```

What may happen to it afterwards, between two looks or two readings:

```text
the_game_is_replaced()               a new GameMode, and the corpse still there
the_world_is_no_longer_known()       the memory was recycled
the_level_becomes(n)
the_chrono_is_lost()
the_world_property_moves_slot()      the table was rebuilt
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
  two costs. The fixture keeps the `Anchor`, so a second
  `when_we_look_for_the_game` is the same search looking again.
- **Both `with_a_game()` and `with_a_manager_and_a_game()` exist.** A heap
  always has a manager in production, so a test that omits it is exercising the
  fallback on purpose.

### What a find test asserts, and why it is the address

`then_the_game_is_found` compares the address of the table the reader returned
with the address the builder wrote. Not the level, not the world.

Two criteria need exactly that. `the-game-not-one-of-its-views` fails if the
reader returns a view, and a view answers every value question the same way,
because it points at the same `GameMechanics`. `the-first-of-two-candidates`
fails if the reader returns the second game, and the two are identical in
every field.

---

## The plan, and what each step covers

### Step 1, the seam. Done

The `Memory` trait, its adapter, the twenty-eight stubs, the contract of the
trait run against both implementations, and the first criterion.

| covered | against |
| --- | --- |
| `reader.find::nothing-in-the-menus` | a `Memory` with no ranges |
| the contract of `Memory`, four questions | the test heap and the adapter |

Eleven tests. The `.wasm` still builds, under every feature, and the 63 core
tests still pass.

### Step 2, the builder and the rest. Done

`src/test_heap.rs` writes a synthetic heap byte by byte, and the sixteen
remaining situations are written against it.

| covered | against |
| --- | --- |
| the ten `reader.find::` criteria | a synthetic heap |
| the seven `reader.read::` criteria | a synthetic heap |

Seventeen tests, and `mise run spec-coverage` says 17 of 17.

The builder is three primitives -- `alloc`, `put`, `intern` -- and two things
built on them: `object()` writes a ScriptObject and its property table, and
`set()` writes one entry. Every object of a heap is allocated before any entry
is written, because the manager cites the game and the game cites the manager
back.

One thing the builder must do, and it is easy to miss: `alloc` keeps every
address 8-aligned. The reader scans aligned qwords and nothing else, so an
object on an odd address is invisible to it. The first version did not align,
and the reader found nothing in a heap that looked right.

### The red was checked, a second time

Twelve mutations of the production code, each one reddening only what it
should.

| mutation | what reddened |
| --- | --- |
| `key_addr` adds 8 to the offset of an entry | every test of the layer |
| `validate` stops requiring a `gameChrono` | the-game-not-one-of-its-views |
| `validate` stops rejecting a game over | rejects-a-game-already-over |
| a proven binary still falls back to the full search | nothing-on-another-flash-build |
| the manager path reuses the last address found | the-new-game-not-the-corpse |
| the dimension is read as zero | a-parallel-world |
| the world is no longer checked on reading | rejects-an-unknown-world |
| the level bound is dropped on reading | rejects-a-level-out-of-bounds |
| a missing `duration` reads as zero | rejects-a-missing-duration, the-nominal-state |
| a missing `gameChrono` reads as zero | rejects-a-missing-chrono |
| `gameTimer` is not subtracted from `frameTimer` | the-nominal-state |
| the kept entry index is trusted without checking | every test of the layer |

The last one is coarse on purpose. Every reading goes through
`get_cached`, so a mutation there cannot redden one test alone.

### Step 3, the change the net was for

Change `validate` to identify the `GameMode` positively, through the `manager`
back-pointer, and see what step 2 says.

Today the identification is negative: it is not a `View`, because it also owns
a `gameChrono`. The `GameManager` path already proves itself positively,
through a pointer that comes back. The asymmetry is the defect, and
`reader.find::the-game-not-one-of-its-views` and `reader.find::an-orphan-game`
are the two criteria that will judge the fix.
