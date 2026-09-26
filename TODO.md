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

### 1. Support Ruffle, and redesign the reader on the way

The need, the decisions and their reasons live on one page:
[Ruffle support](docs/specs/ruffle-support.md). In short: the autosplitter
reads the heap of Ruffle as it reads the heap of Pepper Flash, behind one
`Avm1Heap` trait, and a bench holds the reads to an exact baseline.

The steps, in order:

1. the bench, in the CI;
2. the reader leaves the wasm crate, and `asr_stubs` goes;
3. the Ruffle spike, beside steps 1 and 2;
4. design `Avm1Heap` and `Runtime`;
5. extract `Avm1Heap` from the Pepper Flash reader;
6. `RuffleHeap`, and finding the player;
7. the rest of the object redesign.

### Before any step

The net is posed, and something other than memory runs it. `mise run ci` is
the one name the git hook and the CI both call, so they cannot drift apart.

Install the hooks once per clone:

```sh
mise run hooks
```

What the net holds: the reader, the decisions, the pacing of the scans and the
order of the commands. What it does not hold says so, on one page:
[What the net does not hold](docs/internals/what-the-net-does-not-hold.md).

---

## What is deliberately not planned

Every gap, and the reason for it, lives on one page:
[What the net does not hold](docs/internals/what-the-net-does-not-hold.md).

In one line each: the runtime is a hard boundary; finding the process stays
outside; the Python of the reverse engineering is a second opinion and never
the reference; and performance is measured rather than tested, by the method
[Speed matters](docs/internals/speed-matters.md) prescribes.
