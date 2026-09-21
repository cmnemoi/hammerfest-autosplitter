# hammerfest-autosplitter

A [Hammerfest](https://github.com/motion-twin/hammerfest) autosplitter for
LiveSplit. It starts the timer when level 0 appears, splits on every level
crossed, and resets at the end of a game. On its own.

The game is not modified: memory is read only, nothing is written, nothing is
patched.

## Install

- Download `hammerfest_autosplitter.wasm`, or build it (see
  [Contributing](#contributing)).
- In LiveSplit: **Edit Splits -> Activate**, then pick the file.
- Right click -> **Compare Against -> Game Time**.

That last step is not optional. The big LiveSplit timer shows *real time* by
default, and real time starts when the autosplitter finds the game -- a few
hundred milliseconds too late. The correct time is in the *game time* channel.

## Use

Nothing to set. Start a game on [EternalTwin](https://eternaltwin.org):

1. the timer starts the instant level 0 appears;
2. it splits on every level crossed, shortcuts included -- level 0 leads
   straight to level 10, warp zones skip up to three levels, and each one
   counts as a single split;
3. it resets when the game ends or is abandoned.

The time shown is a **real time**, measured from the official start of the
run: it counts pauses and loading. The internal game clock, which excludes
them, is published next to it in the `Game clock (ms)` variable.

| variable | content |
| --- | --- |
| `Level` | the number the game displays, with no offset |
| `World` | `xml_adventure` and the parallel worlds |
| `Game clock (ms)` | the clock Hammerfest itself reports at the end of a game |

Levels in parallel dimensions do not trigger a split.

## How this is possible

The level and the time are not C variables. They are properties of
ActionScript 2 objects, created at run time by a downloaded SWF, inside the
AVM1 heap. **No static pointer path leads to them**, and the SWF is
obfuscated.

Two things make the reading possible:

1. **The table of obfuscated names is public.** `eternalfest/project-phoenix`,
   the decompiler of the game, relies on `game-types/src/lib/hf.map.json` --
   2952 `clear -> obfuscated` entries, MIT. So we know that `currentId` is
   called `-BBEO` in the SWF, without having to look for it.
2. **A string known from the SWF serves as an anchor.** We look for it in the
   heap, which gives the String object, then the tables that cite it, then the
   `GameMode`.

The memory layout is measured at run time, never assumed: it differs between
Linux and Windows. The proofs and the traps are in
**[reverse-engineering.md](reverse-engineering.md)**.

The timing rests on one fact of the game: `GameMode.duration` runs only from
the moment level 0 appears. The start instant is therefore **rebuilt after the
fact**, and the time the memory search takes does not enter the timing.
Details in **[architecture.md](architecture.md)**.

## Contributing

Rust for the module, Python for the memory exploration.
[mise](https://mise.jdx.dev) installs the rest.

**Prerequisite**: a system linker. The `.wasm` links by itself, but `asr`
depends on a procedural macro, which must be built *for the host machine*. On
Windows that means the Visual Studio Build Tools with the C++ workload:

```sh
winget install Microsoft.VisualStudio.2022.BuildTools \
  --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

On Linux or macOS, the system linker is enough.

```sh
mise run build    # -> target/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm
mise run test     # the rules, in core/, with no runtime and no memory
```

[asr-debugger](https://github.com/LiveSplit/asr-debugger) **reloads the module
by itself** when the file changes: leave it open and run `mise run build`
again. It gives the logs, the variables and a fake timer.

```sh
tools/asr-debugger.exe target/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm
```

To read a game in progress without LiveSplit:

```sh
mise run state    # one reading: level, clock, world
mise run watch    # follows the level changes live
mise run dump     # the GameMode, world and Chrono tables, names in clear
```

A game must be running: the `--type=ppapi` process that carries Pepper Flash
exists only while a Flash instance lives.

**Before you touch the code**, read [architecture.md](architecture.md). It says
where the decisions are made, why the diagnostics do not live in the product
code, and what is still open.

## State

**Proved** on EternalTwin, `pepflashplayer.dll` win32-x64 32.0.0.465:
resolution with no hard coded address, reading of the level and the time,
detection of level changes, immediate reset at the end of a game.

**Measured**: the start is dated to the frame, even when the search finishes
late. Twelve games, eleven with an immediate display; 6 ms of difference over
one minute between the time shown and an outside clock.

**Out of scope**: the parallel dimensions, and the final split of the race
rule -- *enters the door and can no longer control the character*.

## Credits

The earlier reverse engineering (`cmnemoi/hammerfest-re`, reading the score on
Linux) and this one were done by Claude.

`vendor/hf.map.json` comes from [Eternalfest](https://gitlab.com/eternalfest),
MIT.
