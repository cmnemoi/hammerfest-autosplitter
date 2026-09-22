# Run it on macOS

Three things are different on macOS. None of them is visible from LiveSplit,
and each one stops the autosplitter on its own.

---

## 1. Which LiveSplit

LiveSplit itself is Windows only. There is no official macOS build, and the
autosplitter cannot load itself into a browser tab.

The build tested here is release 0.9.1 of
[`AlexKnauth/livesplit-one-druid`](https://github.com/AlexKnauth/livesplit-one-druid),
a desktop build of LiveSplit One. It carries the auto splitting runtime, so it
loads the `.wasm` the same way LiveSplit does.

## 2. It must run as root

macOS refuses `task_for_pid` to an unsigned binary. That call is what opens
the memory of another process, so without it LiveSplit reads nothing at all:
no memory map, no module, no game. It fails silently.

```sh
sudo HOME="$HOME" /path/to/LiveSplitOne
```

`HOME` must be passed, so that LiveSplit finds your own splits and layout
rather than the ones of `root`.

The other cure is to sign LiveSplit with the `com.apple.security.cs.debugger`
entitlement. Nobody has tried it here.

## 3. The timer starts late

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

---

## When it does not work

LiveSplit writes a log, and the autosplitter writes into it. Turn it on in
`~/Library/Application Support/org.LiveSplit.LiveSplit-One/config.yml`:

```yaml
log:
  enable: true
  level: INFO
  clear: true
```

The log then holds one line per module load, one per process attached, and the
reason the module was stopped if it was.

```
Hammerfest: autosplitter 0.1.0 started (budget=true, diagnostics=false)
Hammerfest: Flash plugin attached
Hammerfest: GameMode 0x..., world xml_adventure, layout linux-x64
```

`timeout, no update in 5 seconds` means the module took longer than one tick
allows. LiveSplit gives a tick 8.3 ms and stops a module that falls five
seconds behind that rate.
