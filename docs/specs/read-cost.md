# Read cost

What the reader costs, counted, and held to a baseline.

---

## Why

The redesign will move every line of the reader. A search that reads more
starts the timer later, and a failed search that reads more freezes the loop
while level 0 appears. See [About speed](../internals/speed-matters.md).

A benchmark in CPU time says little: a local `Vec<u8>` and a remote memory read
do not cost the same, and a machine is never quiet. What costs is the number of
reads the runtime serves, and the bytes they carry. Both are exact, so a change
in them is a fact, not noise.

## Scope

A fixed list of situations, each one run against the reader, and the cost of
each one compared with a baseline kept in git.

Outside: CPU time, which stays measured by hand, and the time a live runtime
takes to serve one read.

---

## Rules

### Cost is counted, not timed

`{#cost::counted-not-timed}`

The cost of a situation is three numbers:

- **reads**: the calls the reader makes to `Memory`;
- **bytes**: the bytes those calls ask for;
- **yields**: the ticks the reader gives back to the runtime before it answers.

Running a situation twice gives the same three numbers.

### Any change fails

`{#cost::any-change-fails}`

A situation whose cost differs from the baseline fails the net, even when the
cost is lower. A lower cost can hide a search that stopped too early.

A change that is wanted updates the baseline, in the same commit:

```sh
mise run read-cost-update
```

The diff of `fixtures/read-cost.txt` then shows the change to whoever reads
the commit.

### A missing measure fails

`{#cost::a-missing-measure-fails}`

A situation that cannot run fails the net. It is never skipped. A situation
with no line in the baseline fails, and so does a line that no situation
writes.

---

## Acceptance criteria

| id | given | then |
| --- | --- | --- |
| `cost.compare::the-same-cost` | a cost equal to the baseline | nothing fails |
| `cost.compare::a-higher-cost` | one more read than the baseline | the net fails, and names the situation and the two costs |
| `cost.compare::a-lower-cost` | one byte less than the baseline | the net fails |
| `cost.compare::a-situation-with-no-baseline` | a situation absent from the baseline | the net fails |
| `cost.compare::a-baseline-with-no-situation` | a baseline line no situation writes | the net fails |
| `cost.count::every-read-and-its-bytes` | a reader that asks two reads, of 8 and 16 bytes | the cost is 2 reads and 24 bytes, whether the reads succeed or not |
| `cost.count::every-yield` | a search that gives back three ticks | the cost is 3 yields |

The situations measured:

| situation | what it stands for |
| --- | --- |
| `real-game/first-search` | the first search in a real game: the capture in `fixtures/replay/windows-pepper-flash`, nothing learned yet |
| `real-game/second-search` | the same search again, with what the first one learned |
| `real-game/first-read` | the first read of the state of that game, when nothing is learned about where its properties sit |
| `real-game/next-read` | the read that follows, as every tick does |
| `game-over/first-search` | a written heap whose only game is over: a search that finds nothing, at full cost |
| `game-over/second-search` | the same failed search again, the heap unchanged |

---

## Out of scope

**CPU time.** It is measured by hand, as [About speed](../internals/speed-matters.md)
prescribes.

**The features `scan-budget` and `known-flash`.** They change the cost on
purpose, and they are measurement builds. The baseline is the build that ships.

**How long a read takes in a live runtime.** The runtime caches its memory
map, and a real read crosses a process boundary. Only a live start shows that:
[Measure the startup](../how-to/measure-the-startup.md).
