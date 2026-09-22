# TODO

The current goal: a test net over the whole autosplitter, so that a redesign or
a refactor is safe.

This page is the one place that says where that stands. Start here.

---

## Where we are

| layer | lines | tests | spec |
| --- | --- | --- | --- |
| `src/core/` the decisions | 1 760 | 71 | 10 of 10 rules traced |
| `src/avm1.rs`, `src/hammerfest.rs` the reader | 1 612 | 20 | 18 of 18 criteria, 6 of 6 rules |
| `src/lib.rs` the loop | 311 | 0 | the pacing left, and is tested |
| `src/asr_stubs.rs`, `src/memory_contract.rs`, `src/test_heap.rs`, `src/replay.rs` the scaffolding | 1 586 | 10 | the contract of `Memory` |

Those figures were true on 2026-09-22. Recompute them rather than trust them:

```sh
mise run spec-coverage    # which spec ids have no test yet
mise run test             # the tests themselves
```

Both layers can be refactored today. The reader is held by a synthetic heap
that a test writes byte by byte, and by the eighteen situations of
[the reader spec](docs/specs/memory-reader.md#acceptance-criteria).

One test is served bytes a Flash player wrote: `src/replay.rs` replays a
trimmed capture of a real game, 2.5 MiB in git. It is the only test that can
see a `MEASURED` seed or an obfuscation table that no longer matches what
ships.

The red was checked and not assumed. Fifteen mutations of the production code
were tried, and each one reddened the tests it should. The fifteenth changed
one obfuscated name in `vendor/hf.map.json`: only the two replay tests saw it,
and the twenty-eight tests on the synthetic heap stayed green. The list is on
[the reader test page](docs/internals/testing-the-memory-reader.md).

---

## What is next, in order

The list was agreed once and does not change on its own. It closes the net,
and then the net gets run by something other than memory.

### 1. The order of the commands

`apply`, in `src/lib.rs`, calls `timer::start()` then `timer::set_game_time()`,
and the order matters: starting a run resets the game time, so a time set
earlier is lost. That rule is a comment today.

`Policy` will emit an ordered list of commands, and `run` will execute it. The
order becomes a value, tested in `core` with the rest of the decisions. A fixed
size array, because `core` is `no_std` without `alloc`.

It also answers a question that has no answer today: what would it take to
drive another timer than LiveSplit. A list of commands supposes nothing about
LiveSplit.

### 2. Tests for `spec_coverage.py`

A coverage report that lies is worse than no report. It reads no memory, so it
is cheap to hold.

### 3. The written procedure, and the note on what stays out

What to run before a session, in five minutes. And one page that says what is
deliberately outside the net: the attachment to the process, the Python scripts
of the reverse engineering, and the performance, which is planned in another
form.

### 4. A git hook, and the CI

`mise run check` says of itself that it is for a hook or a CI, and nothing
calls it. The hook comes first: it stops a fault before it leaves the machine.
The CI proves it publicly.

The replay test will not run in CI, and that is accepted: the fixture in git is
enough for it to run, so it will. What will not run there is a capture of your
own game.

---

## What is deliberately not planned

**No test reads bytes a Flash player wrote.** Committing a trimmed capture
costs three to six megabytes in git and covers three criteria the synthetic
heap already covers. Add it the day the offset derivation itself changes. The
reasoning is on the
[reader test page](docs/internals/testing-the-memory-reader.md).

**Finding the process stays outside the net.** The reader spec puts it out of
scope, and `heap_iter`, `heap_ranges` and `heap_size` keep talking to `asr`
directly for that reason.

**No Python formatter.** Adding one means adding a dependency, which is a
separate decision.
