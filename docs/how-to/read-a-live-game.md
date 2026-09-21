# Read a live game

Read the level, the clocks and the flags out of a running game, without
LiveSplit and without building anything.

**Before you start:** a game must be running. The `--type=ppapi` process that
carries Pepper Flash exists only while a Flash instance is alive.

---

## One reading

```sh
mise run state
```

```text
set            xml_adventure (dimension 0)
level          2   (previous 1)
clock          10563 ms
frameTimer     23677
duration       321.6 cycles  = 10.0 s
fl_lock        True
endModeTimer   0 cycles
fl_elevatorOpen False
```

It prints the resolution first: which process, which module, how many
regions, and the AVM1 layout it measured. That part is the fastest way to check
that the memory layer still works.

---

## Follow the game

```sh
mise run watch
```

One line per level crossed, and one line when the run ends:

```text
  level 2 -> 3    clock 14002 ms  (+3439 ms)
  ELEVATOR  endModeTimer 0 -> 448.0 cycles (14.0 s)  level 103  clock ... ms
```

It also warns when `frameTimer` stops moving for sixty reads, which means the
object is dead and the resolution has to be taken again. See [About stale
memory](../internals/stale-memory.md).

---

## See every property

```sh
mise run dump
```

Prints four tables in full (`GameMode`, `world`, `gameChrono` and
`scriptEngine`), with every key translated from its obfuscated form:

```text
  0x4793b4136e8  ]=[]8 -> world              tag=6  object 0x4793b414560
  0x4793b413708  -BBEO -> currentId           tag=0  int 2
```

Use this tool to find a name from the game source in memory for the first
time.

---

## Time a start

```sh
mise run trace
```

Times one start, from the appearance of the process to the official start of
the run, and writes a CSV.

---

## What is underneath

Three modules, each usable on its own. They depend only on the standard
library, on purpose.

| file | what it gives |
| --- | --- |
| `scripts/winmem.py` | read-only access to a Windows process through ctypes: regions, modules, reads, scans |
| `scripts/avm1.py` | the AVM1 object model: atoms, strings, tables. The layout is derived at run time, never assumed |
| `scripts/hfmap.py` | source names to obfuscated names, from `vendor/hf.map.json` |

`scripts/hf_state.py` is a hundred lines on top of those three.

---

## Next

| you want | read |
| --- | --- |
| to understand what you are seeing | [About AVM1 objects](../concepts/avm1-objects.md) |
| to record it for later | [Capture a fixture](capture-a-fixture.md) |
| to measure the start delay | [Measure the startup](measure-the-startup.md) |
