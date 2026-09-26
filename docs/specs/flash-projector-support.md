# Flash projector support

Why the autosplitter reads Adobe's Flash projector, how, and what is left. The
decisions were taken on 2026-09-26, with a live game in view.

---

## Why

Eternalfest Desktop now plays contrées in Adobe's standalone Flash Player
32.0.0.465, the "projector", and keeps Ruffle as a fallback. Its reason: Ruffle
breaks some contrées, and the projector is the same Flash Player as on
eternalfest.net and in EternalTwin.

## Scope

In:

- the projector under Linux, x86-64, as Eternalfest Desktop starts it or as a
  runner starts it alone.

Out, for now:

- the projector under Windows. `flashplayer.exe` is a 32-bit program: its
  atoms and its pointers are four bytes wide, and every read of the reader
  assumes eight. It needs a layout of its own, and a capture to hold it.

---

## Decisions

### The Pepper Flash reader reads the projector

Measured on a live game under Linux: the projector's AVM1 is the one of Pepper
Flash. Its String, its ScriptObject and its property table have the Linux
layout of the plugin, and the capture in `fixtures/replay/projector-main-world`
is read by `PepperFlash` with no change.

**Refused: a reader of its own.** It would be a copy of the Pepper Flash one.

### A player of its own, beside EternalTwin

The projector is a `Runtime` of its own, with a `PepperFlash` of its own. What
`PepperFlash` keeps about its binary is about `pepflashplayer`, and the
projector is another binary.

- **The process** is `flashplayer`, the name of the projector under Linux. The
  name of the Windows projector is not looked for: its heap is not the one the
  reader reads.
- **The module** is the executable itself. It is not position independent: it
  sits at `0x400000`, and its vtables at fixed addresses in it. Its extent is
  read from its ELF header, not asked of the runtime: see
  [the rule](#the-module-runs-to-the-end-of-its-last-segment).
- **The heap** is anonymous and writable, like the one of the plugin: the
  `GameMode` of the live game sat in such a range. The ranges of Pepper Flash
  are swept.

### Order

EternalTwin first, since the runs that count are made there. The projector
next, then Ruffle: Eternalfest Desktop starts one of the two, never both.

---

## Rules

### The module runs to the end of its last segment

`{#projector::the-module-spans-its-segments}`

The module is the executable as the ELF header at its base loads it: from that
base to the end of its last loadable segment. Bytes that are not the header of
a 64-bit executable at a fixed address give no module.

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

## Acceptance criteria

| id | given | then |
| --- | --- | --- |
| `projector.find::segments-with-a-gap` | an executable at `0x400000` whose two segments leave a gap between them | the module runs from `0x400000` to the end of the second segment |
| `projector.find::not-an-executable` | bytes at the base that are not an ELF header | no module |
| `projector.find::a-position-independent-executable` | an ELF header of a position independent executable | no module |
| `projector.replay::the-main-world` | the capture of a game at level 17 of `xml_adventure`, in the projector under Linux | the game is found, at level 17, in `xml_adventure`, dimension 0 |
