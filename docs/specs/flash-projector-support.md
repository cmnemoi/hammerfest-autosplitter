# Flash projector support

Why the autosplitter reads Adobe's Flash projector, how, and what is left. The
decisions were taken on 2026-09-26, with a live game in view: under Linux, and
the Windows projector under Wine 10.0. macOS followed on 2026-09-27, under
Rosetta 2 on an ARM Mac.

---

## Why

Eternalfest Desktop now plays contrées in Adobe's standalone Flash Player
32.0.0.465, the "projector", and keeps Ruffle as a fallback. Its reason: Ruffle
breaks some contrées, and the projector is the same Flash Player as on
eternalfest.net and in EternalTwin.

## Scope

In:

- the projector under Linux, x86-64, as Eternalfest Desktop starts it or as a
  runner starts it alone;
- the projector under Windows, a 32-bit program, as Eternalfest Desktop starts
  it (`flashplayer.exe`) or as Adobe ships it (`flashplayer_32_sa.exe`);
- the projector under macOS, an x86-64 program in `Flash Player.app`, that
  Rosetta 2 translates on an ARM Mac.

Out, for now:

- a check on a real Windows: the Windows projector was read under Wine only.

---

## Decisions

### The Pepper Flash reader reads the projector

Measured on a live game under Linux: the projector's AVM1 is the one of Pepper
Flash. Its String, its ScriptObject and its property table have the Linux
layout of the plugin, and the capture in `fixtures/replay/linux-projector`
is read by `PepperFlash` with no change.

**Refused: a reader of its own.** It would be a copy of the Pepper Flash one.

### A word of four bytes or eight, and nothing else changes

Measured under Wine: the Windows projector is a 32-bit build of the same
Flash Player. Its String, its ScriptObject and its property table are the ones
of Pepper Flash under Linux, with every offset counted in words of four bytes
instead of eight:

```text
                      64-bit (Linux)    32-bit (Windows projector)
  String   buffer     +0x08             +0x04        one word
           length     +0x30             +0x18        six words
  Object   table      +0x30             +0x18        six words
  table    capacity   +0x08             +0x04        one word
           first key  +0x20             +0x10        four words
           entry      16 bytes          8 bytes      two words: (value, key)
```

An atom is a word too, with the same tags. So the width of a word is data:
`avm1::Word`, carried by the layout, and the 32-bit table geometry is one more
profile, `windows-x86`. A float stays eight bytes, behind its atom.

**Refused: a 32-bit reader beside the 64-bit one.** The derivation of the
layout, the search of the key string and the walk back to a table header would
exist twice, and drift.

### A player of its own, beside EternalTwin

The projector is a `Runtime` of its own, with a `PepperFlash` of its own for
each width. What `PepperFlash` keeps about its binary is about
`pepflashplayer`, and each projector is another binary.

- **The process** is `flashplayer` under Linux, in words of eight bytes, and
  `flashplayer.exe` or `flashplayer_32_sa.exe` under Windows, in words of
  four.
- **The module** is the executable itself. Neither projector is relocated: it
  sits at `0x400000`, and its vtables at fixed addresses in it. Its extent is
  read from its header, not asked of the runtime: see
  [the rule](#the-module-runs-to-the-end-of-its-image).
- **The heap** is anonymous and writable, like the one of the plugin: the
  `GameMode` of the live game sat in such a range, on both systems. The ranges
  of Pepper Flash are swept, and for a 32-bit build only below 4 GiB, since it
  can point nowhere else.
- **A seed of its own.** `MEASURED` holds for the Windows plugin only: on a
  projector, it cost one pass over the whole heap for nothing, 2.35 s on
  macOS. Each projector binary starts from its own seed instead, measured on
  its capture: `LINUX_PROJECTOR`, `MACOS_PROJECTOR` and `WINDOWS_PROJECTOR`.
  The host picks the one of its system. A seed is checked like the plugin's,
  and the search by content takes over when it does not hold.
- **A known build is trusted.** Each seed comes with the first method of its
  two vtables, read in the binary Adobe ships. When the module holds them, the
  seed is proven before the first search. A search while the SWF loads then
  costs one pass, and not four: measured on 2026-09-27 on macOS, a failed
  search took 9.3 s, and blinded the loop while the game started.
- **No large range under Rosetta 2.** The one pass left read 2 GB in 2.4 s,
  and 85 % of it sat in twelve ranges of 127 to 512 MiB. The game sits in
  ranges of 652 KiB at most, so ranges larger than 64 MiB are not swept.
- **The ranges that follow each other, as one.** Under Rosetta 2, the map
  holds thousands of ranges of a few KiB, and a read costs about 60 µs
  whatever its size: in EternalTwin, a sweep of 325 MiB made 9 499 reads and
  took 1 s. The ranges left are joined when one ends where the next starts:
  2 760 ranges made 166 runs on a real capture.

### Order

EternalTwin first, since the runs that count are made there. The projector
next, then Ruffle: Eternalfest Desktop starts one of the two, never both.

---

## Rules

### The module runs to the end of its image

`{#projector::the-module-spans-its-segments}`

The module is the executable as the header at its base loads it. Under Linux,
an ELF header: from that base to the end of its last loadable segment. Bytes
that are not the header of a 64-bit executable at a fixed address give no
module.

`{#projector::the-module-spans-its-image}`

Under Windows, a PE header: from that base, `SizeOfImage` bytes. Bytes that
are not a PE header give no module.

Found on 2026-09-26 under Wine: Wine maps the first page of `flashplayer.exe`
from the file and copies its sections into anonymous memory, so the runtime
sees a module of one page.

Found on 2026-09-26 with the live check: LiveSplit gave the projector a module
of `0x400000..0x127c000`, the sum of the sizes of its three mappings. The
mappings leave a gap of 2 MiB between the code and the data, so the data --
and the vtable of a property table at `0x140eb30` -- fell past that end, and no
table was ever proven.

### A real game in the projector is read

`{#projector::a-real-game-is-read}`

The bytes the projector 32.0.0.465 wrote under Linux, in
`fixtures/replay/linux-projector`, give the level, the world and the
dimension the game showed.

### A 32-bit build is read in words of four bytes

`{#projector::words-of-four-bytes}`

Every scenario of [Memory reader](memory-reader.md) runs on a heap written in
words of four bytes, and a negative integer keeps its sign. The bytes the
Windows projector 32.0.0.465 wrote under Wine, in
`fixtures/replay/windows-projector-wine`, give the level, the world and
the dimension the game showed.

### A real game in the macOS projector is read

`{#projector::macos-is-read}`

The macOS projector is a 64-bit build, like the Linux one: the process is
`Flash Player`, in words of eight bytes. Its segments follow each other with no
gap, so the module the runtime gives is its extent. The bytes the projector
32.0.0.465 wrote under macOS, in `fixtures/replay/macos-projector`, give the
level, the world and the dimension the game showed.

## Acceptance criteria

| id | given | then |
| --- | --- | --- |
| `projector.find::segments-with-a-gap` | an executable at `0x400000` whose two segments leave a gap between them | the module runs from `0x400000` to the end of the second segment |
| `projector.find::not-an-executable` | bytes at the base that are not an ELF header | no module |
| `projector.find::a-position-independent-executable` | an ELF header of a position independent executable | no module |
| `projector.find::a-pe-image` | a PE header at `0x400000` whose `SizeOfImage` is `0x1034000` | the module runs from `0x400000` to `0x1434000` |
| `projector.find::no-large-range-under-rosetta` | under Rosetta 2, a range of 512 MiB, one of 0.5 MiB and one of 4 KiB | the two small ones are swept, the smallest first |
| `projector.find::adjacent-ranges-under-rosetta` | under Rosetta 2, a range of one page, one of two pages that follows it, and one apart | the first two are swept as one range, after the one apart |
| `projector.find::no-seed` | a 64-bit build about which nothing is known yet | its first search never tries the seed of the plugin |
| `projector.seed::linux` | the capture of `linux-projector`, and `LINUX_PROJECTOR` | the String of `world` is found by its header, with no search by content |
| `projector.seed::macos` | the capture of `macos-projector`, and `MACOS_PROJECTOR` | the same |
| `projector.seed::windows` | the capture of `windows-projector-wine`, and `WINDOWS_PROJECTOR` | the same |
| `projector.find::not-a-pe-image` | bytes at the base that are not a PE header | no module |
| `projector.replay::the-windows-main-world` | the capture of a game at level 34 of `xml_adventure`, in the Windows projector under Wine | the game is found, at level 34, in `xml_adventure`, dimension 0 |
| `projector.replay::the-macos-main-world` | the capture of a game in the projector under macOS | the game is found, at the level, in the world and the dimension the game showed |
| `projector.replay::the-main-world` | the capture of a game at level 17 of `xml_adventure`, in the projector under Linux | the game is found, at level 17, in `xml_adventure`, dimension 0 |
