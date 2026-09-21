//! What the game plays once the player enters the elevator.

use crate::policy::duration_ms;

/// The end sequence of an adventure.
///
/// The race rule ends the run when the player *enters the door and can no
/// longer control the character*. In the last level that door is an elevator.
/// The game then plays a cinematic, reaches game over, and navigates away.
///
/// `GameMode.endModeTimer` carries that cinematic.
/// `ScriptEngine.codeTrigger` case 4 -- *sortie par l'ascenseur* -- sets it to
/// fourteen seconds of game cycles in the frame of the elevator, and nothing
/// else in an adventure writes it. `GameMode.main()` then counts it down by
/// `Timer.tmod`, on the line that follows `duration += Timer.tmod`.
///
/// Two things follow from that one line.
///
/// **It says the run is over.** The fruit release, case 3, also takes the
/// controls away, 12.5 s earlier. It leaves this timer at zero. So the timer
/// separates the two, where the controls themselves cannot.
///
/// **It says how long ago.** What is left of the cinematic dates the last
/// split exactly, the same way `duration` dates the first one. The reader
/// does not have to arrive in the right frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct EndSequence {
    /// What is left of the cinematic. Zero means it has not started.
    remaining_ms: i64,
}

impl EndSequence {
    /// `Data.SECOND * 14`: the length the game gives the cinematic.
    pub const FULL_CYCLES: f64 = 448.0;

    /// The same length in milliseconds, at 31.25 ms per cycle.
    const FULL_MS: i64 = 14_000;

    /// The largest correction we accept.
    ///
    /// We see the start of the sequence within one read, so the real gap is a
    /// few tens of milliseconds. A larger one means the value does not mean
    /// what we think -- another version of the game, another cinematic. We
    /// then date the split where we saw it. A missing correction costs one
    /// read; a wrong one costs the run.
    const MAX_CORRECTION_MS: i64 = 500;

    /// The sequence has not started. The run is still going.
    pub const NONE: Self = Self { remaining_ms: 0 };

    /// From `GameMode.endModeTimer`, in game cycles.
    pub fn from_cycles(cycles: f64) -> Self {
        Self {
            remaining_ms: duration_ms(cycles).max(0),
        }
    }

    /// Has the player entered the elevator?
    pub fn started(self) -> bool {
        self.remaining_ms > 0
    }

    /// Milliseconds between the elevator and this reading.
    ///
    /// Zero when the sequence has not started, and zero when the gap is too
    /// large to be trusted.
    ///
    /// The count is a play time: the game freezes this timer and `duration`
    /// together, on the same `fl_lock`. A pause between the elevator and our
    /// reading would therefore shorten it. One read cannot hold a pause, so
    /// that case does not arise.
    pub fn since_the_elevator_ms(self) -> i64 {
        if !self.started() {
            return 0;
        }
        let gap = Self::FULL_MS - self.remaining_ms;
        if (0..=Self::MAX_CORRECTION_MS).contains(&gap) {
            gap
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_not_started_by_default() {
        assert!(!EndSequence::NONE.started());
        assert!(!EndSequence::default().started());
        assert_eq!(EndSequence::NONE.since_the_elevator_ms(), 0);
    }

    #[test]
    fn starts_at_the_full_length_with_no_gap() {
        let s = EndSequence::from_cycles(EndSequence::FULL_CYCLES);
        assert!(s.started());
        assert_eq!(s.since_the_elevator_ms(), 0);
    }

    #[test]
    fn measures_the_gap_from_what_is_left() {
        // 3.2 cycles at 31.25 ms each: 100 ms of cinematic already played.
        let s = EndSequence::from_cycles(EndSequence::FULL_CYCLES - 3.2);
        assert_eq!(s.since_the_elevator_ms(), 100);
    }

    #[test]
    fn refuses_a_gap_that_is_too_large_to_be_real() {
        // Ten seconds in. Another game, another cinematic: we correct nothing.
        let s = EndSequence::from_cycles(EndSequence::FULL_CYCLES - 320.0);
        assert!(s.started());
        assert_eq!(s.since_the_elevator_ms(), 0);
    }

    #[test]
    fn a_negative_timer_is_not_a_sequence() {
        // `main()` takes the timer below zero on the frame it calls game over.
        assert!(!EndSequence::from_cycles(-1.0).started());
    }
}
