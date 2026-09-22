# Timer commands

What the autosplitter sends the timer, and in which order.

---

## Why

The policy decides. Something else executes, and the order in which it executes
changes the result.

`timer::start()` puts the game time back to zero. So a game time set before it
is lost, and the timer shows zero for one frame. That is one line of order, and
today it is a comment in `src/lib.rs` rather than something a test can read.

The order also decides what a run looks like after a restart, and which
segments a warp zone skips.

A second reason. Today the decisions leave `core` as six booleans and a number,
and the parent crate turns them into calls to `asr::timer`. Anyone who wanted
to drive another timer would have to read that translation and copy it. A list
of commands supposes nothing about LiveSplit.

## Scope

The order of what one tick sends the timer, and the list itself.

Outside: what the policy decides, which is
[level crossings](level-crossings.md) and the rest of `policy.rs`; and how a
command reaches LiveSplit, which is three lines of `src/lib.rs`.

---

## Rules

### The reset comes before the start

`{#commands::the-reset-comes-before-the-start}`

A restart resets the timer and starts it again, in the same tick. The other
order would reset the run that has just started, and the player would see
nothing at all.

### The time is set after the start

`{#commands::the-time-is-set-after-the-start}`

`timer::start()` puts the game time back to zero. A time set before it is lost.

This is not cosmetic. The time we set dates the origin of the run, which is
what takes the heap scan off the critical path: a start found late corrects
itself on the first reading. Losing that value once means the timer shows zero
where it should show the delay already elapsed.

### The clock is paused before it is set

`{#commands::the-clock-is-paused-before-it-is-set}`

The value sent is absolute, read from the game itself. LiveSplit must not add
its own advance between two readings, so its game clock is paused first.

### The skips follow their split

`{#commands::the-skips-follow-their-split}`

A warp zone carries the player over levels they never play. Those segments must
record no time, so they are skipped, and a skip applies to the segment after
the split that precedes it.

### An idle tick sends nothing

`{#commands::an-idle-tick-sends-nothing}`

No game and no decision means no command. The timer is left alone.

---

## What this page does not settle

**The split takes the game time of the previous tick.** The split is sent
before the time of the current reading, so the segment is timed with the value
set one tick earlier, about sixteen milliseconds old at sixty ticks per second.

That is what the autosplitter has always done. It is written here so that the
redesign meets it as a decision and not as an accident. Changing it is a change
of behaviour, and it needs its own measurement.
