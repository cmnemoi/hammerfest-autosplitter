# What the net does not hold

Read this when: a test seems missing, and you want to know whether it was
forgotten or refused.

Every line below is a decision. None of them is an accident, and each one says
what covers the gap instead.

---

## The runtime is a hard boundary

No test crosses into `asr`. Not `timer_start`, not `timer_split`, not
`process_list_by_name`, not `next_tick`.

**Why.** A double over the runtime ABI implements somebody else's contract, and
it has to invent rules where that contract is silent. The reader's own tests
were once held to a rule the runtime never made. The story is on
[the reader test page](testing-the-memory-reader.md).

**What covers it instead.** The order of what we send is a value now, decided
in `core` and held by [the timer commands spec](../specs/timer-commands.md).
What is left in `src/lib.rs` is `send`, one match arm per command, and each arm
is one call. A reader can check it against `asr::timer` in a minute.

The rest is the five-minute check before a session.

## Finding the process stays outside

Listing the EternalTwin processes, attaching to each one and asking for its
modules is out of the reader's scope, and the spec says so.

`heap_iter`, `heap_ranges` and `heap_size`, in `src/plugin.rs`, talk to `asr` directly for
that reason. `resolve` receives the ranges as a slice, which is what took the
reader off the runtime API.

## The loop keeps what only the runtime can answer

`run`, in `src/lib.rs`, still carries the attach loop, `process.is_open()`, the
line announced once per game, and the diagnostics prints.

The two decisions it used to carry have left: when to scan again is
[the pacing](../specs/resolution-pacing.md), and what to send is
[the commands](../specs/timer-commands.md). What remains asks the runtime a
question a test cannot answer.

## The Python of the reverse engineering is a second opinion

`scripts/avm1.py`, `scripts/hf_state.py` and `scripts/hf_trace.py` read a live
game and decode AVM1 themselves. They duplicate the Rust reader on purpose:
they are how the reader was written in the first place.

**They are never the reference.** When the autosplitter is silent,
`mise run state` says whether the game is readable at all, and that is all it
says.

**They are not untested either, in the direction that matters.**
`metadata.json` carries what the Python read at capture time, and
`src/replay.rs` checks the Rust against it on every run. The two implementations
share no constant, so their agreement is worth something.

A pytest suite for them would need a Python fixture replayer and a test
dependency, in a project whose `pyproject.toml` declares none. It buys the
agreement we already have.

## Performance is measured, not tested

Read counts, byte counts, the number of passes and the yielding budget are not
assertions. Their effect is that LiveSplit does not freeze, and there is no
LiveSplit in a test.

[Speed matters](speed-matters.md) prescribes the method instead: say what a
mechanism claims to save, replay a fixture of the situation, compare by
counting reads and bytes rather than CPU time, and confirm on a few real
starts.

That is planned, and it will not take the shape of a test.

## One thing nobody has checked

`MEASURED` and `vendor/hf.map.json` are checked against one capture of one
version of the player. If the player is updated, the replay test still passes,
because it replays the old bytes.

Only running the autosplitter on the new player sees that, which is the first
line of [the check before a session](../how-to/check-before-a-session.md).
