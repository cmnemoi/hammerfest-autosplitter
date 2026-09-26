# Flash projector support

Why the autosplitter reads Adobe's Flash projector, how, and what is left. The
decisions were taken on 2026-09-26, with a live game in view: under Linux, and
the Windows projector under Wine 10.0.

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
  it (`flashplayer.exe`) or as Adobe ships it (`flashplayer_32_sa.exe`).

Out, for now:

- a check on a real Windows: the Windows projector was read under Wine only.

---

## Decisions

### The Pepper Flash reader reads the projector

Measured on a live game under Linux: the projector's AVM1 is the one of Pepper
Flash. Its String, its ScriptObject and its property table have the Linux
layout of the plugin, and the capture in `fixtures/replay/projector-main-world`
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
- **No seed.** `MEASURED` holds for the 64-bit plugin. A 32-bit build starts
  with no seed, and learns its layout from its first search.

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
`fixtures/replay/projector-main-world`, give the level, the world and the
dimension the game showed.

### A 32-bit build is read in words of four bytes

`{#projector::words-of-four-bytes}`

Every scenario of [Memory reader](memory-reader.md) runs on a heap written in
words of four bytes, and a negative integer keeps its sign. The bytes the
Windows projector 32.0.0.465 wrote under Wine, in
`fixtures/replay/projector-win32-main-world`, give the level, the world and
the dimension the game showed.

## Acceptance criteria

| id | given | then |
| --- | --- | --- |
| `projector.find::segments-with-a-gap` | an executable at `0x400000` whose two segments leave a gap between them | the module runs from `0x400000` to the end of the second segment |
| `projector.find::not-an-executable` | bytes at the base that are not an ELF header | no module |
| `projector.find::a-position-independent-executable` | an ELF header of a position independent executable | no module |
| `projector.find::a-pe-image` | a PE header at `0x400000` whose `SizeOfImage` is `0x1034000` | the module runs from `0x400000` to `0x1434000` |
| `projector.find::not-a-pe-image` | bytes at the base that are not a PE header | no module |
| `projector.replay::the-windows-main-world` | the capture of a game at level 34 of `xml_adventure`, in the Windows projector under Wine | the game is found, at level 34, in `xml_adventure`, dimension 0 |
| `projector.replay::the-main-world` | the capture of a game at level 17 of `xml_adventure`, in the projector under Linux | the game is found, at level 17, in `xml_adventure`, dimension 0 |
