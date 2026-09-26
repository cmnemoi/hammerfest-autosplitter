# Capture a fixture

Record the memory of a running game, so it can be read again later with no
game and no Flash.

**Before you start:** a game must be running, in any state. The menus are a
useful capture too.

---

## One fixture per platform

Each platform the autosplitter supports, an OS and a Flash player, has one
trimmed capture in `fixtures/replay/<os>-<player>`. The CI replays every one
of them (`mise run test:slow`), so a change that breaks one platform is red on
every push.

| OS \ player | Pepper Flash (EternalTwin) | Flash projector | Ruffle desktop | Ruffle in a browser |
| --- | --- | --- | --- | --- |
| Windows | `windows-pepper-flash` | `windows-projector-wine`, under Wine | `windows-ruffle-wine`, under Wine | missing |
| Linux | `linux-pepper-flash` | `linux-projector` | `linux-ruffle` | `linux-ruffle-web`, Firefox |
| macOS | missing: needs a Mac | not shipped | missing: needs a Mac | missing: needs a Mac |

A missing cell is a platform the net does not hold yet.

---

## Take one

```sh
mise run capture-heap --name my-capture
```

It takes 0.3 to 0.6 s and writes:

```text
   fixtures/my-capture/
       metadata.json     every address, every size, every flag, and the state
       heap.bin.gz       the regions, concatenated in address order
       module.bin.gz     the plugin image
```

```text
124 regions, 118.0 MiB read in 0.6 s
stored         35.4 MiB  (30.0 % of the bytes read)
  heap.bin.gz   19.0 MiB of  85.4 MiB read
  module.bin.gz 18.0 MiB of  32.6 MiB read
level          2 -> 2
```

Useful flags:

| flag | effect |
| --- | --- |
| `--no-module` | halve the fixture. Only `Binary::recognize` reads those bytes |
| `--level 6` | smaller, and four times slower to take |
| `--raw` | no compression, to see what it saves |

Captures stay out of git. Their trimmed forms, under `fixtures/replay/`, do not:
see below.

---

## What a fixture holds, and why

Bytes alone are not enough. The search steers itself on the shape of the heap,
so the shape is recorded too:

```text
   the base address of every region      the differential scan compares these
   the size of every region              the same
   the flags Windows reported            so the selection can be reproduced
   the range of the plugin module        every "is this pointer in the module"
   the game state, before and after      what the fixture is a picture of
```

A flat dump loses all of it, and with it every address in the object graph.

One gzip stream rather than one file per region: there are 124 of them, and a
single stream shares its dictionary across all. `metadata.json` gives the
offset of each region inside the stream.

---

## A capture is not atomic

The game runs while we read, so the last region is younger than the first.

`state_before` and `state_after` bracket that, and they check each other. In a
real capture of 0.59 s, the two recorded game clocks were 599 ms apart.

```text
   the object graph   survives the smear     AVM1 objects do not move in a game
   the clocks         do not                 never assert an exact clock value
```

That is also why the default compression level is 1 rather than 6. Level 6
costs four times the time for 28 % fewer bytes, and more time means more
smear.

---

## Read it back

A fixture carries enough to replay the whole resolution off line. A recorded
process is the live one with three methods replaced: `read`, `regions` and
`module`. Everything else, including the scans, is the production code
unchanged.

On the capture above, the replay derived the same layout, found the same
`GameMode` address and read the same level, in 0.55 s, with no game running.

---

## Situations worth recording

```text
   in the menus            no GameMode exists yet
   the SWF loading         the heap growing from two to eighty MiB
   a game running          the ordinary case
   a parallel dimension    xml_deepnight, levels above 103
   right after a game      a stale GameMode, still readable
```

The last one is the hardest to get and the most valuable. See [About stale
memory](../internals/stale-memory.md).

---

## Make one a test

A capture can be replayed by `src/reader/tests/reader/replay.rs`, which is the only test served
bytes a Flash player wrote. That needs two files, and neither JSON nor gzip in
the crate:

```sh
mise run replay-fixture -- my-capture
cargo test -p hammerfest-reader smallest -- --ignored --nocapture
mise run replay-fixture -- my-capture --keep <the addresses it printed>
```

The first run writes every region, 85 MiB, for the tool to work on. The tool
then empties the biggest region, looks for the game again, and keeps it emptied
while the game is still found. The last run writes only what is left.

For `windows-pepper-flash`, what is left is 5 regions of 124, 33.3 MiB raw and 2.5 MiB
gzipped. That one lives in git, under `fixtures/replay/`. The captures
themselves stay out.

Why it is worth the 2.5 MiB is on
[the reader test page](../internals/testing-the-memory-reader.md).
