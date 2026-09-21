# About stale memory

**Read this when:** you are tempted to cache an address, or you wonder why
every tick re-reads the whole chain.

**You need:** [About AVM1 objects](../concepts/avm1-objects.md).

---

## The trap

When a game ends, nothing wipes its objects. The allocator frees them, so it
may reuse the memory. Often it does not.

So this happens:

```text
   during the game        0x4793b413030  ->  a live GameMode, level 7
   the player quits
   ten seconds later      0x4793b413030  ->  the same bytes, still readable
                                             world still xml_adventure
                                             level still 7
                                             clock still plausible
```

Nothing in those bytes says the object is dead. Every check on the content
passes, because the content is genuine. It is simply old.

An autosplitter that trusts it shows a timer that starts late and never stops.

---

## Defence 1: the heartbeat

`Chrono.frameTimer` is set to the current time on every frame, by the mode the
game is actually running:

```text
   GameMode.main()
       gameChrono.update()      ->  frameTimer = now       unconditional
       if (fl_pause) { ... }                               and before this
```

A live object changes that number constantly. A dead one never does.

Frozen for 90 ticks, the object is declared dead and the search starts again.
The threshold is generous on purpose: Flash runs at about thirty frames per
second, and a background window runs slower still.

---

## Defence 2: the end-of-game flag

`fl_gameOver` is the game's own signal, set by `GameMode.onGameOver()`.

Two rules follow. A search that lands on an object already in game over
**rejects it**, rather than reporting a finished game as a running one. And a
game over that arrives while we are watching drops the resolution at once.

---

## Defence 3: read everything, every time

We never keep the address of the level. We keep the address of the `GameMode`,
and on every tick we walk the whole chain again:

```text
   GameMode  ->  world  ->  setName    is it one of the five known worlds?
                        ->  currentId  is it below 256?
```

The walk costs microseconds. Caching the final address would break the second
game of a session, silently, and that is the case nobody tests.

We do remember the slot indexes, as hints: "`world` was at slot 68, try that
first". We check a hint before every use, and never trust it on its own.

---

## The rule

> A successful read proves that memory was readable. It proves nothing about
> what the memory means.

The proofs are in section 7 of
[reverse-engineering.md](../reverse-engineering.md). The code is
`src/hammerfest.rs`, `validate` and `Game::read`.
