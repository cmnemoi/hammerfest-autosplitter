# About finding the game

**Read this when:** you want to know how one address in a hundred megabytes of
heap gets found, several times per second.

**You need:** [About why Flash is hard](../concepts/why-flash-is-hard.md).

---

## The three paths

Nothing points at the game, so we look for it. There are three ways, from cheap
to expensive. Every tick takes the cheapest one that can still work.

```text
                          every tick
                               |
                               v
                    +----------------------+
                    |   anchor known?      |
                    +----------------------+
                     yes |            | no
                         v            v
            +---------------+   +--------------------------+
            | FAST PATH     |   | has the heap grown 4 MiB |
            | a few reads   |   +--------------------------+
            |               |     yes |             | no
            | GameManager   |         v             v
            |   .current    |   +------------+  +---------+
            |      |        |   | SEARCH     |  | wait    |
            |      v        |   | for        |  | 20 to   |
            |  GameMode     |   | "fVersion" |  | 60      |
            +---------------+   |     |      |  | ticks   |
                     |          |     v      |  +---------+
                     |          | GameManager|
                     |          +------------+
                     |                 | not found
                     |                 v
                     |          +--------------+
                     |          | LAST RESORT  |
                     |          | search for   |
                     |          | "world"      |
                     |          |      |       |
                     |          |      v       |
                     |          |  GameMode    |
                     |          +--------------+
                     |                 |
                     +--------+--------+
                              v
                    +------------------------+
                    | CHECK                  |
                    |  world name known?     |
                    |  level below 256?      |
                    |  gameChrono present?   |
                    |  not already over?     |
                    +------------------------+
                              |
                              v
                          the level
```

---

## The anchor: an interned string

A `.swf` keeps its string constants in a pool, and AVM1 **interns** them:
there is exactly one `String` object for the text `]=[]8`, and it lives as long
as the plugin does.

That gives us a landmark that does not move during a session. The search is
two passes:

```text
   pass 1    scan the heap for the ten bytes of "]=[]8" in UTF-16
             ->  the String object

   pass 2    scan for tables whose key slot points at that String
             ->  the objects that own a `world` property
```

Two anchors are used, for two different jobs:

| anchor | finds | why that one |
| --- | --- | --- |
| `fVersion` | the `GameManager` | only its constructor sets it, so it is cited once |
| `world` | a `GameMode` directly | the last resort, when the first fails |

A common key such as `current` would be cited dozens of times, and each
candidate costs the rebuild of a table.

---

## The fast path

The `GameManager` is born with the SWF and lives as long as it. Once we have
found it, it hands us the running mode for a handful of reads:

```text
   GameManager  ->  current  ->  the GameMode  ->  world  ->  currentId
```

A few reads against a hundred megabytes. That is what lets us look for the game
on every tick, so a game that starts is seen almost at once rather than half a
second later.

The search only runs to learn the anchor, or when the anchor has died.

---

## The differential scan

The objects of a new game are born in memory that has just been committed. So
a region absent from the previous list is new, or it grew.

Scanning only those turns a hundred megabytes into a few. Two guards make that
safe:

* if no region changed, nothing can have been born, so the scan is skipped
  entirely;
* every eighth skipped attempt does a full pass anyway, because an object can
  be born inside memory that was already committed.

---

## The check at the end

A table with a `world` property is not always the game. `View` objects carry a
`world` too, and they point at the same object.

So every candidate, and every later read, is checked four ways:

```text
   setName is one of the five known worlds       xml_adventure, xml_deepnight, ...
   currentId is between 0 and 255
   the object also owns a gameChrono             only a GameMode does
   fl_gameOver is not already true               a finished game is not the one we want
```

[About stale memory](stale-memory.md) explains why these checks are
necessary.

---

## The numbers

| constant | value | what it decides |
| --- | --- | --- |
| `HEAP_GROWTH` | 4 MiB | growth that allows an immediate new search |
| `RESOLVE_MIN_COOLDOWN` | 20 ticks | wait after the first failure |
| `RESOLVE_MAX_COOLDOWN` | 60 ticks | the wait doubles up to here |
| `FULL_SWEEP` | 8 | skipped attempts before a full pass anyway |
| `MANAGER_IDLE_LIMIT` | 30 | silent attempts before an unproven anchor is dropped |
| `MAX_LEVEL` | 256 | the plausibility bound on a level |

The code is `src/hammerfest.rs`, `resolve` and `resolve_via_manager`.
