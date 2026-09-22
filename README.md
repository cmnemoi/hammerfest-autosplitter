# hammerfest-autosplitter

A LiveSplit autosplitter for [Hammerfest / Eternalfest](https://eternalfest.net/). It starts
the timer when level 0 appears, splits on every level crossed, splits once more
at the elevator that ends the run, and resets when the game is over.

## How to use it

Get [`hammerfest_autosplitter.wasm`](https://github.com/cmnemoi/hammerfest-r/releases/latest) (WIP : publish it in Livesplit registry). Then, in LiveSplit:

1. **Edit Splits -> Activate**, and pick the file;
2. right click -> **Compare Against -> Game Time**.

Do not skip step 2. The big timer shows *real time*, which starts a few
hundred milliseconds late. The *gametime* channel carries the correct time.

Then start a game.

The autosplitter has been tested on Eternaltwin application 1.0.0 for Windows.

## How to contribute

You need [mise](https://mise.jdx.dev) to interact with this repository.

```sh
mise run build    # Build the autosplitter for Livesplit
mise run test     # Run autosplitter tests
mist run check    # Check for code issues
```

On Windows you also need a C++ linker :

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

This autosplitter and retro-engineering code are published under the [Apache 2.0 License](LICENSE).

Some retro-engineering code and artefacts like `vendor/hf.map.json` comes from [Eternalfest](https://gitlab.com/eternalfest),
MIT.
