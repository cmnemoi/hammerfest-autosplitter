# TODO

The current goal: a test net over the whole autosplitter, so that a redesign or a
refactor is safe.

This page is the one place that says where that stands. Start here.

---

## Where we are

| layer | lines | tests | spec |
| --- | --- | --- | --- |
| `src/core/` the decisions | 1 572 | 63 | 4 of 4 rules traced |
| `src/avm1.rs`, `src/hammerfest.rs` the reader | 1 549 | 1 | 1 of 17 criteria |
| `src/lib.rs` the loop | 316 | 0 | none written |
| `src/asr_stubs.rs`, `src/memory_contract.rs` the scaffolding | 381 | 10 | the contract of `Memory` |

Those figures were true on 2026-09-22. Recompute them rather than trust them:

```sh
mise run spec-coverage    # which spec ids have no test yet
mise run test             # the tests themselves
```

`core` can be refactored today. The reader cannot: its one test says an empty
heap finds nothing, and protects nothing else.

What is built is the seam that makes the rest possible, and the proof that it
works: the `Memory` trait, its adapter, and the contract both implementations
answer. That is step 1 of
[About testing the memory reader](docs/internals/testing-the-memory-reader.md).

---

## What is next, in order

### 1. The heap builder and the sixteen remaining criteria

Step 2 of the reader plan. This is the work that moves "1 of 17".

The two entry points exist: `resolve` for finding, `Game::read` for reading.
The criteria are listed in
[the reader spec](docs/specs/memory-reader.md#acceptance-criteria), and
`mise run spec-coverage` names the ones still missing.

### 2. Identify the `GameMode` positively

Step 3 of the reader plan, and the change the net was built for. Today the
identification is negative: it is not a `View`, because it also owns a
`gameChrono`. The `manager` back-pointer proves it positively.

Judged by `reader.find::the-game-not-one-of-its-views` and
`reader.find::an-orphan-game`, so it waits for step 1 above.

### 3. Decide about the loop

`run`, in `src/lib.rs`, carries the cooldowns, the progressive backoff, the
heap-growth trigger and the announce-once. That is 136 lines of logic with no
test and no spec, and it sits outside every plan.

It needs a decision, not work: a spec of its own, or a written note that it
stays out of the net.

### 4. Format the code once

`mise run check` is red: `cargo fmt` wants to change 31 places, nearly all in
code no recent change touched. Run `mise run fmt` and commit that on its own,
so the reformat never hides a real change.

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
