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

### 1. A git hook, and the CI

`mise run check` says of itself that it is for a hook or a CI, and nothing
calls it. `.github/` exists and is empty. `.git/hooks` holds only samples.

A net nobody runs protects nothing. The hook comes first: it stops a fault
before it leaves the machine, in three seconds. The CI proves it publicly, and
it will run the replay test too, because its capture is the only one in git.

### 2. Whatever the redesign asks for next

The net is posed. The reader, the decisions, the pacing and the order of the
commands are all held by tests, and the parts that are not held say so in
[What the net does not hold](docs/internals/what-the-net-does-not-hold.md).

---

## What is deliberately not planned

Every gap, and the reason for it, lives on one page:
[What the net does not hold](docs/internals/what-the-net-does-not-hold.md).

In one line each: the runtime is a hard boundary; finding the process stays
outside; the Python of the reverse engineering is a second opinion and never
the reference; and performance is measured rather than tested, by the method
[Speed matters](docs/internals/speed-matters.md) prescribes.
