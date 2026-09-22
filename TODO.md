# TODO

The current goal: a test net over the whole autosplitter, so that a redesign or
a refactor is safe.

This page is the one place that says where that stands. Start here.

---

## Where we are

| layer | lines | tests | spec |
| --- | --- | --- | --- |
| `src/core/` the decisions | 1 570 | 63 | 4 of 4 rules traced |
| `src/avm1.rs`, `src/hammerfest.rs` the reader | 1 612 | 18 | 18 of 18 criteria, 6 of 6 rules |
| `src/lib.rs` the loop | 328 | 0 | none written |
| `src/asr_stubs.rs`, `src/memory_contract.rs`, `src/test_heap.rs` the scaffolding | 1 275 | 10 | the contract of `Memory` |

Those figures were true on 2026-09-22. Recompute them rather than trust them:

```sh
mise run spec-coverage    # which spec ids have no test yet
mise run test             # the tests themselves
```

Both layers can be refactored today. The reader is held by a synthetic heap
that a test writes byte by byte, and by the eighteen situations of
[the reader spec](docs/specs/memory-reader.md#acceptance-criteria).

The red was checked and not assumed. Fourteen mutations of the production code
were tried, and each one reddened the tests it should. The list is on
[the reader test page](docs/internals/testing-the-memory-reader.md).

---

## What is next, in order

### 1. Decide about the loop

`run`, in `src/lib.rs`, carries the cooldowns, the progressive backoff, the
heap-growth trigger and the announce-once. That is 136 lines of logic with no
test and no spec, and it sits outside every plan.

It needs a decision, not work: a spec of its own, or a written note that it
stays out of the net.

The case for leaving it out: those lines are timing against a runtime a test
cannot host. `next_tick`, `timer::*` and `process.is_open()` all come from the
sandbox. The upgrade path, if the redesign touches the pacing, is to move
`cooldown`, `backoff` and `HEAP_GROWTH` into `core` and test them there.

### 2. Whatever the redesign asks for next

The three steps the net was built for are done. The reader can now be changed
and the tests will say whether the change holds.

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
