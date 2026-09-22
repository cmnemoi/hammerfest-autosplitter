# hammerfest-autosplitter

A LiveSplit autosplitter for [Hammerfest](https://eternalfest.net/). It starts
the timer when level 0 appears, splits on every level crossed, splits once more
at the elevator that ends the run, and resets when the game is over. There is
nothing to configure. It only reads the game's memory, and never writes to it.

## How to use it

Get `hammerfest_autosplitter.wasm`, or build it. Then, in LiveSplit:

1. **Edit Splits -> Activate**, and pick the file;
2. right click -> **Compare Against -> Game Time**.

Do not skip step 2. The big timer shows *real time*, which starts a few
hundred milliseconds late, when the autosplitter finds the game. The *game
time* channel carries the correct time.

Then start a game.

Tested on EternalTwin for Windows, `pepflashplayer.dll` win32-x64 32.0.0.465.

## What it does

| it sends | when |
| --- | --- |
| `start` | level 0 appears |
| `split` | the level number moves forward |
| `skip split` | for each level a warp zone carried you over |
| `split` | the player enters the elevator of the last level |
| `reset` | the game ends, or is abandoned |

A warp zone -- the umbrella -- advances by up to three levels at once. It
spawns at random, so no splits file can plan for it. The autosplitter splits
once, then skips one segment per level you never played. A skipped segment
records no time, so it takes no gold, and your run consumes the same number of
segments whether the umbrella appeared or not.

The level 0 shortcut is different. `0 -> 10` happens in every attempt, so it
counts as one split and skips nothing. Set up one segment for it. A level in a
parallel dimension splits nothing.

Three variables sit next to the timer:

| variable | content |
| --- | --- |
| `Level` | the number the game displays, with no offset |
| `World` | `xml_adventure`, or a parallel world |
| `Game clock (ms)` | the clock the game itself reports |

## How to contribute

Rust for the module, Python for the memory tools.
[mise](https://mise.jdx.dev) installs both.

```sh
mise run build    # -> target/*/release/hammerfest_autosplitter.wasm
mise run test     # the rules, with no runtime and no memory
mise run state    # read a running game: level, clock, world
```

On Windows you also need a C++ linker. The `.wasm` links by itself, but one
dependency builds a macro for the host machine:

```sh
winget install Microsoft.VisualStudio.2022.BuildTools \
  --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

[asr-debugger](https://github.com/LiveSplit/asr-debugger) reloads the module by
itself when the file changes. Leave it open, and build again:

```sh
tools/asr-debugger.exe \
  target/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm
```

Start at [docs/index.md](docs/index.md). It covers what the autosplitter does,
the four jobs it is made of, where each one lives, and what is still open.

## Credits

`vendor/hf.map.json` comes from [Eternalfest](https://gitlab.com/eternalfest),
MIT.

The reverse engineering, here and in `cmnemoi/hammerfest-re`, was done by
Claude.
