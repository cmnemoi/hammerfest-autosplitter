# Run it on macOS

LiveSplit itself is Windows only, and the autosplitter cannot load itself into
a browser tab. The build tested here is release 0.9.1 of
[`AlexKnauth/livesplit-one-druid`](https://github.com/AlexKnauth/livesplit-one-druid),
a desktop build of LiveSplit One. It carries the auto splitting runtime, so it
loads the `.wasm` the same way LiveSplit does.

Its own README says how to give it permission to read the memory of another
process, and where it keeps its configuration and its log. Read that first.
Two things it does not say follow here.

---

## Pass your own `HOME`

macOS needs `sudo`, and `sudo` under it keeps `HOME`. Pass it anyway, because
one shell in ten does not:

```sh
sudo HOME="$HOME" /path/to/LiveSplitOne
```

Without it, LiveSplit reads the configuration of `root`, so it opens neither
your splits, nor your layout, nor the autosplitter you chose.

## The timer starts late

Half a second to four seconds, measured. The time it shows is right: it comes
from the clock of the game, so follow **Compare Against → Game Time** as on
every platform. Only the moment the timer appears is late.

The plugin is an x86-64 binary, and Rosetta 2 translates it. Every page the
game allocates is then attributed to `/usr/libexec/rosetta/runtime`, and the
memory to search is spread over 1119 MiB where Windows holds it in 84 MiB.
Finding the game there takes about 2.5 s.

See [About finding the game](../internals/finding-the-game.md) for what the
search does, and `heap_ranges` in `src/hammerfest.rs` for the order it reads
the ranges in.
