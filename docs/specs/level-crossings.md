# Level crossings

What the autosplitter sends LiveSplit when the level number moves forward.

---

## Why

`currentId` does not advance by one each time. A warp zone advances by 1 to 3
in one step, and the level 0 shortcut goes straight to 10
([reverse-engineering.md, section 6](../reverse-engineering.md)).

Today every forward move produces exactly one split. That rule is right for
the level 0 shortcut and wrong for a warp zone, and the difference is not a
detail of taste:

| shortcut | present in every attempt? | one split is correct? |
| --- | --- | --- |
| level 0 to level 10 | yes, it is the route itself | yes |
| warp zone | no, the umbrella spawns at random | no |

A runner builds a splits file once. They can plan for the level 0 shortcut,
because it happens every time. They cannot plan for an umbrella. So a lucky
attempt produces fewer splits than an unlucky one, the splits file falls out
of step in the middle of the run, and every segment after that compares
against the wrong reference.

The run has to consume the same number of segments whatever the umbrella does.

## Scope

The decision the policy makes when a confirmed level is higher than the
previous confirmed level, in the main world, with the timer running.

Outside: starting, resetting, the elevator that ends the run, parallel
dimensions, and every rule that does not look at the level number.

---

## Rules

### One split per crossing

`{#crossing::one-split-per-crossing}`

A forward move of the level number produces one `split`, whatever the size of
the move. The split closes the segment of the level the player just left, and
that segment carries the real time the player spent there.

Nothing changes here. This rule already holds.

### A warp zone skips the levels it never played

`{#crossing::warp-skips-the-levels-never-played}`

A warp zone that moves the level by N also produces N-1 `skip_split`, after
the `split`.

A skipped segment records no time. So it produces no gold, and it does not
enter the sum of best segments. LiveSplit carries its time over to the next
segment, and that time is zero, so the next segment still measures the level
the player actually plays.

From level 42, an umbrella that lands on level 45:

| call | segment | what it records |
| --- | --- | --- |
| `split` | 42 | the real time of level 42 |
| `skip_split` | 43 | nothing |
| `skip_split` | 44 | nothing |
| | 45 | opens, and will hold the real time of level 45 |

The splits file consumes four segments whether the umbrella appeared or not.

### A jump larger than a warp skips nothing

`{#crossing::a-large-jump-skips-nothing}`

A forward move of more than 3 produces one `split` and no `skip_split`.

`SpecialManager.warpZone` advances by 1 to 3 and never more
([reverse-engineering.md, section 6](../reverse-engineering.md)). So a larger
move is not a warp zone. It is one of three other things:

- the level 0 shortcut, `0 -> 10`, which the runner plans for as one segment;
- the three writes `Adventure.nextLevel` makes inside one frame, read in the
  middle, which can show `1 -> 10`;
- a misread.

The bound is what makes the third case safe. Without it, one wrong read could
send dozens of `skip_split` and burn the rest of the splits file.

---

## Acceptance criteria

- Given the level moves from 3 to 4, when the move is confirmed, then the
  policy asks for 1 split and 0 skips.
- Given the level moves from 42 to 44, when the move is confirmed, then the
  policy asks for 1 split and 1 skip.
- Given the level moves from 42 to 45, when the move is confirmed, then the
  policy asks for 1 split and 2 skips.
- Given the level moves from 0 to 10, when the move is confirmed, then the
  policy asks for 1 split and 0 skips.
- Given the level moves from 1 to 10, when the move is confirmed, then the
  policy asks for 1 split and 0 skips.
- Given the level does not move, or moves backwards, then the policy asks for
  0 splits and 0 skips.
- Given the level moves forward in a parallel dimension, then the policy asks
  for 0 splits and 0 skips.
- Given the level moves forward while the timer is not running, then the
  policy asks for 0 splits and 0 skips.
- Given the player enters the elevator, when the run ends, then the policy
  asks for 1 split and 0 skips.

---

## Out of scope

**Naming the cause of a jump.** `currentId` alone cannot tell a warp zone from
the level 0 shortcut. The size of the move is the only evidence used. Reading
`fl_warpStart` would name the cause, and it would cost a new path through the
memory reader for no behaviour we need.

**The microsecond race.** `Adventure.nextLevel` writes 1, 0, then 10 as three
consecutive statements in one frame. If two consecutive reads both landed on
the same intermediate value, the policy would confirm it and act on it. The
window is microseconds wide, the confirmation over two reads already covers
it, and rule `{#crossing::a-large-jump-skips-nothing}` bounds the damage to
what today's rule already produces.

**Keeping the segment index equal to the level number.** The runtime offers
`timer::current_split_index`, so the policy could force the two to agree. That
would require one segment per level from 0, which the level 0 shortcut makes
wrong: nine segments would exist only to be skipped in every attempt.
