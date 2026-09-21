# Measure the startup

Find out how long after the real start of a run the timer appears.

This is the number the runner actually feels, and the only one that settles an
argument about the search. See [About speed](../internals/speed-matters.md).

---

## The measuring build

**The normal module measures nothing.** No trace, no counter, no timestamp,
and the `.wasm` does not even hold the matching strings:

```sh
grep -c HF_ target/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm   # 0
```

Everything that measures lives in `src/diagnostics.rs`, behind a Cargo feature
of the same name. Without the feature those calls have no body and the types
they handle are empty.

```sh
mise run build-diagnostics     # the traced module, in its own directory
```

It writes `HF_DIAG`, `HF_SCAN` and `HF_START` lines to the runtime log.

Two things live in `diagnostics.rs` that belong to the product, not to the
measurement: the WASI clock, because the runtime API exposes none, and
`FreshMap`, which the memory-map cache fix depends on.

---

## Capture some starts

```sh
mise run capture-startup
```

Records up to 25 starts over 15 minutes, with no manual log export. You play;
it watches.

```sh
mise run summarize-startup
```

Reads an exported log and prints the distribution:

```text
   N            how many starts
   zeros        how many were displayed at 0 ms
   median
   P95
   max
```

The last measured run: **twelve starts, eleven at 0 ms**, and the first start
of a fresh module at 135 ms.

---

## Two other variants

```sh
mise run build-cold-start      # diagnostics + known-flash
mise run build-scan-budget     # diagnostics + scan-budget
```

`known-flash` recognises the already measured Flash build by its PE headers,
before the first game, so the layout seed is proven from the start. It exists
to measure what the seed is worth.

`scan-budget` is on in the normal module. It yields to the runtime on a volume
of bytes rather than a number of blocks, which stops a map full of small
regions from producing one pause per region.

---

## The runtime cache

```sh
mise run probe-runtime
```

Measures how long the LiveSplit runtime caches a process memory map. The answer
was one second, and that measurement is the reason `FreshMap` exists. The story
is in [About speed](../internals/speed-matters.md).

---

## When to use which

```text
   changed anything at all        replay a fixture, count reads and bytes
                                  -> Capture a fixture

   changed the search             a few real starts

   about to release               25 real starts
```

Live starts are the reference, but they are slow and manual. Do not spend them
on a change a fixture can judge.
