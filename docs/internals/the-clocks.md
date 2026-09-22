# About the clocks

**Read this when:** you want to know how a run is timed to the frame by a
program that arrives half a second late.

**You need:** nothing.

---

## Four counters, four origins

The game keeps several counters. They all advance at the same speed. They
differ in when they start and when they stop.

| counter | starts at | stops on | unit |
| --- | --- | --- | --- |
| `Chrono.frameTimer` | the plugin process starts | never | ms |
| `GameMode.gameChrono` | the GameMode is built | pause, level change | ms |
| `GameMode.duration` | level 0 appears | pause, level change | cycles |
| `GameMode.endModeTimer` | the player enters the elevator | pause, level change | cycles |

One cycle is 31.25 ms, because `Data.SECOND` is 32.

```text
   plugin starts
        |
        |<-------------- frameTimer, never stops ----------------------->
        |
        |         GameMode built
        |              |
        |              |<------ gameChrono, stops on pause ------------->
        |              |
        |              |  0.55 s  |
        |              |<-------->| level 0 appears
        |                         |
        |                         |<--- duration, stops on pause ------->
        |                                                  |
        |                                            the elevator
        |                                                  |<-- 14 s -->|
        |                                                  endModeTimer
```

---

## The rule, and the frame it names

The speedrun rule says: *the timer begins when the loading text disappears and
fades in to level 0, and ends when the player enters the door and can no
longer control the character.*

The first instant is the frame where `GameMode.fl_lock` falls. The second is
the frame where `endModeTimer` leaves zero.

---

## Why we do not have to be there

The heap search takes about half a second. The window between the birth of the
game objects and the appearance of level 0 is 0.55 s. We would lose that race
about half the time.

So we do not race. `GameMode.main()` returns on `fl_lock` *before* it
increments `duration`. So `duration` is exactly zero for the whole black
screen, and it starts at the instant the rule names.

Which gives:

```text
   origin    =  frameTimer - duration     at the first read after the unlock
   run time  =  frameTimer - origin
```

Arriving on time, `duration` is zero and the origin is `frameTimer`. Arriving
late, `duration` says by how much, and the origin is rebuilt exactly.

End to end check: 6 ms of difference over 60.9 s.

The search is therefore off the critical path. Its duration no longer enters
the timing. It only delays the display.

---

## Two guards on the origin

The origin is set once and never drifts, because everything after it is a
subtraction. It is rebuilt in exactly one case, and deliberately not in
another:

**Rebuilt when a counter goes backwards.** Only two can, and only from one
game to the next: `duration` restarts with the GameMode, `frameTimer` restarts
with the plugin process.

**Not rebuilt when the resolution is lost and taken again.** Losing the
objects mid-game is normal. Rebuilding the origin there would place it too
late, by all the time between levels that `duration` does not count, and the
timer would jump backwards in front of the player.

---

## The end of the run

`endModeTimer` does at the end what `duration` does at the start.

It is zero for the whole game. One line of the adventure sets it, in the frame
where the player enters the elevator of the last level. It then counts down
fourteen seconds, to game over.

```text
   run ends  =  the frame endModeTimer leaves zero
   how late  =  14000 ms - endModeTimer in milliseconds
```

So the last split is dated exactly too, whichever read notices it. A gap above
500 ms is refused rather than applied. It would mean another version of the
game with another cinematic. A wrong correction would ruin the run, and a
missing one costs only one read.

After that the time freezes. Two events follow and neither may reset the
run: game over fourteen seconds later, and the loss of the plugin when the
page navigates away.

The proof is section 11 of
[reverse-engineering.md](../reverse-engineering.md). The code is
`src/core/end_sequence.rs`.

---

## Why the time travels in the *game time* channel

The runtime API offers `start`, `split`, `reset`, `set_game_time` and
`pause_game_time`. It cannot move a running timer backwards.

So the LiveSplit *real time* starts at the `start()` call, when the search
finishes, a few hundred milliseconds late. There is no way to correct it.

`set_game_time` takes an absolute value, so it carries the right one.

> In LiveSplit: **Compare Against → Game Time**.

`gameChrono`, the clock the game shows the player, goes in a variable beside
it. It excludes pauses and level transitions, so it is not a run time. It is
the number the player recognises.
