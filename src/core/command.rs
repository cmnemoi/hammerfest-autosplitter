//! What one tick sends the timer, in order.
//!
//! See [the spec](../../docs/specs/timer-commands.md). The order is the whole
//! point: `Start` puts the game time back to zero, so a `SetGameTime` before
//! it is lost.
//!
//! The list is a value, so a test reads it. Nothing here knows what LiveSplit
//! is: driving another timer means writing a function that consumes these
//! commands.

use crate::policy::Actions;

/// One order for the timer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Reset,
    Start,
    Split,
    /// The segment of a level the player never entered. It records no time.
    SkipSplit,
    /// Stop the timer's own game clock. The value we send is absolute.
    PauseGameTime,
    /// Real time since the official start, in milliseconds.
    SetGameTime(i64),
}

/// The most commands one tick can carry.
///
/// One reset, one start, one split, two skips for the longest warp, and the
/// pair that sets the clock.
const MAX: usize = 8;

/// The commands of one tick, in the order they must be sent.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Commands {
    items: [Option<Command>; MAX],
    len: usize,
}

impl Commands {
    fn push(&mut self, command: Command) {
        // A full list would drop an order silently, and the timer would be
        // left in a state nobody decided.
        debug_assert!(self.len < MAX, "more commands than one tick can carry");
        if self.len < MAX {
            self.items[self.len] = Some(command);
            self.len += 1;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = Command> + '_ {
        self.items[..self.len].iter().flatten().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Actions {
    /// The commands this decision becomes, in the order they must be sent.
    ///
    /// @spec commands::the-reset-comes-before-the-start
    /// @spec commands::the-time-is-set-after-the-start
    /// @spec commands::the-clock-is-paused-before-it-is-set
    /// @spec commands::the-skips-follow-their-split
    /// @spec commands::an-idle-tick-sends-nothing
    pub fn commands(&self) -> Commands {
        let mut out = Commands::default();
        if self.reset {
            out.push(Command::Reset);
        }
        if self.start {
            out.push(Command::Start);
        }
        if self.split {
            out.push(Command::Split);
            for _ in 0..self.skips {
                out.push(Command::SkipSplit);
            }
        }
        if let Some(ms) = self.real_time_ms {
            out.push(Command::PauseGameTime);
            out.push(Command::SetGameTime(ms));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Command::*;
    use super::*;
    extern crate alloc;
    use alloc::vec::Vec;

    fn sent(actions: Actions) -> Vec<Command> {
        actions.commands().iter().collect()
    }

    /// @spec commands::an-idle-tick-sends-nothing
    #[test]
    fn a_tick_that_decides_nothing_sends_nothing() {
        assert!(Actions::default().commands().is_empty());
        assert_eq!(sent(Actions::default()), []);
    }

    /// @spec commands::the-reset-comes-before-the-start
    #[test]
    fn a_restart_resets_before_it_starts() {
        let restart = Actions {
            reset: true,
            start: true,
            ..Actions::default()
        };

        assert_eq!(sent(restart), [Reset, Start]);
    }

    /// @spec commands::the-time-is-set-after-the-start
    /// @spec commands::the-clock-is-paused-before-it-is-set
    #[test]
    fn the_time_is_set_after_the_start_and_on_a_paused_clock() {
        let start = Actions {
            start: true,
            real_time_ms: Some(1_200),
            ..Actions::default()
        };

        assert_eq!(sent(start), [Start, PauseGameTime, SetGameTime(1_200)]);
    }

    /// @spec commands::the-skips-follow-their-split
    #[test]
    fn the_skips_follow_their_split() {
        let warp = Actions {
            split: true,
            skips: 2,
            ..Actions::default()
        };

        assert_eq!(sent(warp), [Split, SkipSplit, SkipSplit]);
    }

    #[test]
    fn a_skip_without_a_split_sends_nothing() {
        // `skips` counts the levels a warp carried the player over, and it
        // only ever comes with the split it belongs to. A skip alone would
        // consume a segment nobody crossed.
        let stray = Actions {
            skips: 2,
            ..Actions::default()
        };

        assert_eq!(sent(stray), []);
    }

    #[test]
    fn a_full_tick_stays_inside_the_list() {
        let everything = Actions {
            reset: true,
            start: true,
            split: true,
            skips: 2,
            drop_resolution: true,
            real_time_ms: Some(42),
        };

        assert_eq!(
            sent(everything),
            [
                Reset,
                Start,
                Split,
                SkipSplit,
                SkipSplit,
                PauseGameTime,
                SetGameTime(42)
            ]
        );
    }
}
