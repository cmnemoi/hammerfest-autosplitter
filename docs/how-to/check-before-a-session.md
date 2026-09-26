# Check before a session

Five minutes, before a run that counts. The tests say the code still does what
it did. They do not say the autosplitter still works with the player, the SWF
and LiveSplit that are installed today.

---

## The five minutes

```sh
mise run ci       # The format, the lint, every test.
mise run build    # the .wasm LiveSplit loads
```

`mise run ci` is what the pre-push hook and the CI run too, so the three can
never drift apart. Install the hooks once per clone:

```sh
mise run hooks
```

Then start a game, in EternalTwin or in Ruffle, and play past the black
screen. With the game running:

```sh
mise run e2e      # 30 s. The real module, in LiveSplit's own runtime.
```

It prints what the module logs and asks of the timer, then judges two
things: the module found the game and published its level, and one update in
a hundred stays within one tick (8.3 ms). `mise run e2e -- 120` watches for
two minutes, long enough to cross a level. The first run builds the runtime,
which takes a few minutes; the next ones start at once.

Then, with the game and LiveSplit open:

1. start a game, and watch the timer start when level 0 appears;
2. play one level, and watch one split arrive at the crossing;
3. look at the variables beside the timer: `Level`, `World`, `Game clock (ms)`.

If those three happen, the whole chain works: the process, the plugin, the
heap, the object graph, the policy and the runtime.

---

## When nothing happens

The question is always the same: does the autosplitter fail to read the game,
or does it read it and decide nothing?

**Ask the second opinion.** `mise run state` reads the game with the Python
tools of the reverse engineering, which share no constant with the Rust.

| what it says | what it means |
| --- | --- |
| a level and a clock | the game is readable. The fault is in the Rust reader, or later |
| nothing | the game is not readable at all: the plugin, the process, or the SWF has changed |

**Read the log.** LiveSplit prints every line the module sends. The first one
names the build. `Hammerfest: GameManager 0x…` means the anchor was found, and
`Hammerfest: GameMode 0x…` means the fallback search found the game.

**Suspect the constants last.** If `mise run test` was green, then
`src/reader/tests/reader/replay.rs` replayed a real capture, and `MEASURED` and
`vendor/hf.map.json` still decode bytes a Flash player wrote. They are not the
problem unless the player itself was updated. See
[the reader test page](../internals/testing-the-memory-reader.md).

---

## Before a release, not before a session

Twenty-five real starts, measured, take fifteen minutes:

```sh
mise run capture-startup
mise run summarize-startup
```

[Measure the startup](measure-the-startup.md) says when that is worth it, and
what the numbers mean.
