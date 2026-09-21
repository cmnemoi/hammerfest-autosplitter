# About why Flash is hard

**Read this when:** you wonder why this project needs a heap scan at all,
instead of one address written down once.

**You need:** nothing.

---

## How an autosplitter usually works

For a compiled game, the level counter is a variable. Variables live at fixed
places, or at the end of a fixed chain of pointers:

```text
   game.exe + 0x1A2B40          a pointer, always there
        |
        v
   the player struct
        + 0x28                  always that offset
        |
        v
   the level:  7
```

You find that chain once, write it in the code, and it works forever. The
whole autosplitter is a few dozen lines.

---

## Hammerfest is not compiled

Hammerfest ships as a `.swf` file. The file is downloaded and **interpreted**
by the Flash plugin, which contains a virtual machine called AVM1.

So the level is a property of an object that the virtual machine creates while
you play:

```text
   pepflashplayer.dll        the interpreter, a normal Windows module
        runs
   hammerfest.swf            ActionScript, downloaded at run time
        creates
   a GameMode object         somewhere in the heap
        which has a property
   world                     another object
        which has a property
   currentId  =  2           the level
```

Those objects land wherever the allocator had room. Between two games:

```text
   game 1     GameMode at 0x4793b413030
   game 2     GameMode somewhere else entirely
```

---

## No fixed chain either

A moving object is still usable if something fixed points at it. So we looked
for a chain from the plugin module down to the current game.

There is none. Four separate attempts are in section 10 of
[reverse-engineering.md](../reverse-engineering.md):

| question asked | answer |
| --- | --- |
| which addresses survive a game restart? | none |
| which slots does the player reuse? | none points at the current movie |
| how many module pointers are cited by code? | 109 out of 550, none useful |
| which globals do the AVM1 methods read? | three, two of them allocators |

The plugin keeps its current movie behind a PPAPI instance held in an indexed
structure, and indexed structures were exactly what the first question ruled
out.

---

## The names are obfuscated

The `.swf` was shipped obfuscated, so the property is not called `currentId` in
memory. It is called `-BBEO`.

The translation table is published, so this part is easy. See
[About the obfuscation](obfuscation.md).

---

## What follows from all this

Every time a game starts, we have to find the objects again, by looking at what
is in the heap rather than by following a map.

That search costs about a hundred megabytes of reading, and it has to finish
fast enough to be useful. Two pages carry the consequences:

* [About finding the game](../internals/finding-the-game.md) — how the search works
* [About speed](../internals/speed-matters.md) — why its cost decides everything
