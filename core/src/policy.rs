//! When to start, split, reset, or drop a resolution.
//!
//! A pure state machine. Every tick it receives what the game says -- or
//! nothing, when no game was found -- and returns the actions to execute. It
//! reads no memory and knows nothing about LiveSplit.

use crate::end_sequence::EndSequence;

/// What the game says about itself at one instant.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct State {
    /// `GameMode.world.currentId`: the level, as the game displays it.
    pub level: i64,
    /// `SetManager._previousId`.
    pub previous: i64,
    /// `Chrono.get()`, in milliseconds.
    pub chrono_ms: i64,
    /// `Chrono.frameTimer`: it advances every frame while this GameMode runs.
    pub frame_timer: i64,
    /// `GameMode.currentDim`: 0 for the main world.
    pub dim: i64,
    /// `GameMode.fl_gameOver`.
    pub game_over: bool,
    /// `GameMode.fl_lock`: true during the black screen at the start, during
    /// level transitions, and during a pause. The game does not simulate.
    pub locked: bool,
    /// `GameMode.duration`, converted to milliseconds.
    ///
    /// `main()` returns on `fl_lock` **before** it increments this value. So
    /// the value is exactly zero until level 0 appears. After that, it
    /// measures the time during which the game ran.
    pub duration_ms: i64,
    /// `GameMode.endModeTimer`: the cinematic that follows the elevator.
    ///
    /// It says two things, and [`EndSequence`] carries both: that the run is
    /// over, and how long ago it ended.
    pub end_sequence: EndSequence,
}

/// `Data.SECOND`: game cycles per second.
const SECOND: f64 = 32.0;

/// `GameMode.duration` in milliseconds.
///
/// The game does `duration += Timer.tmod` every frame, and `Timer.tmod` is 1
/// per frame at the reference frame rate. The sum therefore follows real time.
/// Measured over a 67 s game, the difference is 0.1 %.
pub fn duration_ms(cycles: f64) -> i64 {
    (cycles * (1000.0 / SECOND)) as i64
}

/// What LiveSplit says about its own timer.
///
/// `Paused` is absent on purpose. A Hammerfest run has no legal pause: the
/// time counts whatever the player does, so a runner who pauses the LiveSplit
/// timer is still in a run. The adapter therefore reports `Running` for it,
/// and the core never has to think about it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TimerState {
    NotRunning,
    Running,
    Ended,
    /// The runtime reported something we do not know. We then do nothing,
    /// which is the only safe answer to "I cannot tell".
    Unknown,
}

/// What the caller must do, in this order.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct Actions {
    pub reset: bool,
    pub start: bool,
    pub split: bool,
    /// How many `skip_split` follow the `split`, for the levels a warp zone
    /// carried the player over and that they never played.
    ///
    /// A skipped segment records no time, so it produces no gold and stays out
    /// of the sum of best segments. The run then consumes the same number of
    /// segments whether the umbrella appeared or not.
    pub skips: u8,
    /// The current resolution is no longer valid. Drop it and search again.
    pub drop_resolution: bool,
    /// Real time since the official start, in milliseconds.
    ///
    /// The value is absolute. A late start corrects itself on the first read.
    /// That is what takes the heap scan off the critical path.
    pub real_time_ms: Option<i64>,
}

impl Actions {
    fn nothing() -> Self {
        Self::default()
    }
}

/// The first level of an adventure.
const FIRST_LEVEL: i64 = 0;
/// The widest move `SpecialManager.warpZone` can make. It advances by 1 to 3
/// through `forcedGoto`, stopping before a boss or an empty level.
const WARP_REACH: i64 = 3;
/// Ticks without a change in `frameTimer` before the GameMode is declared
/// dead. The value is generous: Flash runs at about thirty frames per second,
/// and a background window runs slower still.
const STALE_TICKS: u32 = 90;
/// Ticks without a valid read before a game counts as abandoned.
const LOST_BEFORE_RESET: u32 = 240;

#[derive(Default)]
pub struct Policy {
    /// The last confirmed state, the one we acted on.
    prev: Option<State>,
    /// The previous read, not confirmed yet.
    seen: Option<State>,
    /// Did we see the absence of a game since the last one?
    saw_no_game: bool,
    /// Did we see this game start, rather than find it already running?
    launched: bool,
    /// The origin of real time, in the clock of the Flash player.
    ///
    /// `Std.getTimer()` counts real milliseconds since the plugin started. We
    /// set the origin once. Everything after that is a subtraction, so nothing
    /// can drift.
    origin: Option<i64>,
    /// The last `duration` read. It only goes down at the next game.
    duration_seen: i64,
    /// Did we ever read a game at all?
    had_game: bool,
    lost: u32,
    heartbeat: i64,
    frozen: u32,
    /// The run is over: the player entered the elevator.
    ///
    /// The game keeps running for fourteen seconds after that, then it reaches
    /// game over and the plugin navigates away. Neither of those two events
    /// may reset a run that is already finished.
    finished: bool,
    /// Real time at the instant the run ended.
    ///
    /// Once it is set, it is the only value we publish. `frameTimer` keeps
    /// advancing during the end sequence, and the final time must not.
    finish_time_ms: Option<i64>,
}

/// @spec crossing::warp-skips-the-levels-never-played
/// @spec crossing::a-large-jump-skips-nothing
///
/// How many segments a crossing of `jump` levels leaves behind.
///
/// A warp zone -- the umbrella -- carries the player over up to two levels
/// they never play. Those segments must record no time, so that a run consumes
/// the same number of segments whether the umbrella appeared or not: it spawns
/// at random, and a runner cannot plan a splits file around it.
///
/// Above `WARP_REACH` the move is not a warp zone. It is the level 0 shortcut,
/// which every attempt goes through and the runner plans as one segment; or
/// the three writes `Adventure.nextLevel` makes inside one frame, read in the
/// middle; or a misread. The bound is what keeps that last case cheap: without
/// it, one wrong read would skip its way through the rest of the splits file.
fn skips_after(jump: i64) -> u8 {
    if jump <= WARP_REACH {
        (jump - 1) as u8
    } else {
        0
    }
}

impl Policy {
    pub fn new() -> Self {
        Self::default()
    }

    /// One tick. `read` is None when no game could be read.
    pub fn tick(&mut self, timer: TimerState, read: Option<State>) -> Actions {
        let Some(now) = read else {
            return self.no_game(timer);
        };

        let mut actions = Actions::nothing();
        self.lost = 0;
        self.had_game = true;

        // A game appears where there was none: that is a launch. Having seen
        // the absence is what separates this case from an autosplitter that
        // starts while a game is already running.
        if self.saw_no_game {
            self.saw_no_game = false;
            self.launched = true;
        }

        // `fl_gameOver` is the exact end-of-game signal. We drop the
        // resolution at once: a finished GameMode stays readable for a long
        // time, and it holds plausible values.
        if now.game_over {
            // A finished run must survive its own end. Game over arrives
            // fourteen seconds after the elevator, and a reset here would
            // erase the run on the finish line.
            if timer == TimerState::Running && !self.finished {
                actions.reset = true;
            }
            self.forget();
            actions.drop_resolution = true;
            return actions;
        }

        // `frameTimer` advances every frame while this GameMode is the one
        // that runs. If it is frozen, the object is dead -- usually after a
        // return to the menu.
        if now.frame_timer == self.heartbeat {
            self.frozen = self.frozen.saturating_add(1);
            if self.frozen >= STALE_TICKS {
                self.forget();
                actions.drop_resolution = true;
                return actions;
            }
        } else {
            self.heartbeat = now.frame_timer;
            self.frozen = 0;
        }

        // Only two counters can go backwards, and only from one game to the
        // next: `duration`, which restarts at zero with the GameMode, and
        // `Std.getTimer()`, which restarts at zero with the plugin process.
        // When either one goes backwards, the origin belongs to the previous
        // game.
        //
        // The test looks at these two rather than at the loss of the
        // resolution. The resolution is dropped and taken again during a game,
        // and rebuilding the origin at that moment would place it too late, by
        // all the time spent between levels that `duration` does not count.
        if self.origin.is_some_and(|o| now.frame_timer < o)
            || now.duration_ms < self.duration_seen
        {
            self.origin = None;
        }
        self.duration_seen = now.duration_ms;

        // The official start is the frame where `fl_lock` goes false: the
        // black screen ends and level 0 appears. `GameMechanics.onViewReady`
        // calls `GameMode.onLevelReady`, which unlocks, in the same frame that
        // attaches the view.
        //
        // One formula covers both cases, because `duration` only runs from
        // that same unlock:
        //
        //   * resolved in time, we read the first unlocked state while
        //     `duration` is still zero: the origin is `frameTimer`, within one
        //     tick;
        //   * resolved late -- the usual case, the scan takes half a second
        //     too long -- `duration` says by how much, and the origin is
        //     rebuilt exactly.
        //
        // The scan therefore leaves the critical path. Its duration no longer
        // enters the timing. It only delays the display.
        if self.origin.is_none() && !now.locked {
            self.origin = Some(now.frame_timer - now.duration_ms);
            // A new origin is a new run. Whatever the last one ended as, it is
            // over.
            self.finished = false;
            self.finish_time_ms = None;
            if self.launched && timer == TimerState::NotRunning {
                actions.start = true;
            }
        }
        actions.real_time_ms = self.origin.map(|o| now.frame_timer - o);

        // The end of the run: the player enters the elevator.
        //
        // We act when the sequence starts, and only then. It stays started for
        // fourteen seconds, so acting on its mere presence would split four
        // hundred times. A resolution that lands inside it never saw the run
        // start either, and has no business ending it.
        //
        // One read is enough, unlike the level. `Adventure.nextLevel` writes
        // the level three times inside one frame; this sequence is written
        // once.
        //
        // The main world only, like a level crossing. The elevator belongs to
        // the last level of the adventure; a parallel dimension ending is not
        // the end of this run.
        if now.end_sequence.started()
            && now.dim == 0
            && !self.finished
            && self.seen.is_some_and(|s| !s.end_sequence.started())
        {
            self.finished = true;
            // The sequence dates itself. So the split lands where the run
            // ended, not where we noticed it.
            self.finish_time_ms = actions
                .real_time_ms
                .map(|ms| ms - now.end_sequence.since_the_elevator_ms());
            if timer == TimerState::Running {
                actions.split = true;
            }
        }
        if self.finished {
            actions.real_time_ms = self.finish_time_ms.or(actions.real_time_ms);
        }

        // We act only on a level read twice in a row.
        //
        // `Adventure.nextLevel` writes `currentId` three times in the same
        // frame -- 1, then 0, then 10 -- and nothing synchronises our read
        // with the game frame. Without this confirmation, a read that lands in
        // the middle would produce two splits instead of one, at random.
        if self.seen.map(|s| s.level) == Some(now.level) {
            self.decide(timer, &now, &mut actions);
            self.prev = Some(now);
        }
        self.seen = Some(now);
        actions
    }

    fn no_game(&mut self, timer: TimerState) -> Actions {
        let mut actions = Actions::nothing();
        actions.drop_resolution = true;
        self.saw_no_game = true;
        self.prev = None;
        self.seen = None;

        // Count lost reads only after we have seen a game. Not finding one
        // yet is not the same as having lost one. The Flash plugin exists as
        // soon as the application opens, so long before there is anything to
        // resolve.
        if self.had_game {
            self.lost = self.lost.saturating_add(1);
            // Not after a finished run. The plugin navigates away seconds
            // after the elevator, so losing the game is the normal end of a
            // run that succeeded.
            // `==`, not `>=`: this fires once. Firing on every later tick
            // would send LiveSplit a reset per tick for as long as no game
            // comes back. The only way to miss it is an `Unknown` timer state
            // on that exact tick, which the runtime does not produce.
            if self.lost == LOST_BEFORE_RESET
                && timer == TimerState::Running
                && !self.finished
            {
                actions.reset = true;
            }
        }
        actions
    }

    /// Forgets the game, but **not** that the run is finished.
    ///
    /// `finished` outlives the game on purpose: game over and the loss of the
    /// plugin both arrive after the elevator, and both would otherwise reset a
    /// run that is already over. It is cleared when a new origin is set.
    fn forget(&mut self) {
        self.prev = None;
        self.seen = None;
        self.saw_no_game = true;
        // The origin belongs to the game that just ended. If we keep it, the
        // next game runs its timer from the wrong instant.
        self.origin = None;
        self.launched = false;
        self.duration_seen = 0;
    }

    fn decide(&self, timer: TimerState, now: &State, actions: &mut Actions) {
        // A finished run decides nothing more. The end sequence still reads as
        // a live game for fourteen seconds.
        if self.finished {
            return;
        }
        let Some(prev) = self.prev else {
            return;
        };

        // A finished timer does not accept `start`. It must be reset first.
        if timer == TimerState::Ended && now.level == FIRST_LEVEL {
            actions.reset = true;
            actions.start = true;
            return;
        }

        // The game clock goes backwards: another game started and we did not
        // see the transition.
        if now.chrono_ms + 2_000 < prev.chrono_ms {
            if timer == TimerState::Running {
                actions.reset = true;
            }
            actions.start = true;
            return;
        }

        // Any forward progress counts, not only `+1`. Hammerfest skips
        // levels: the level 0 shortcut leads straight to level 10
        // (`Adventure.nextLevel` under `fl_warpStart`), and warp zones advance
        // by 1 to 3 (`SpecialManager.warpZone` -> `forcedGoto`).
        //
        // One split per crossing, whatever the size of the move. The split
        // closes the segment of the level the player just left.
        //
        // Only forward counts, and nothing in a run moves backwards: a death
        // costs a life and keeps the same level, and a warp cannot arrive
        // below where it left. A lower number means a script jump, the writes
        // inside one frame, or a misread.
        if timer == TimerState::Running
            && now.level > prev.level
            && now.dim == 0
        {
            actions.split = true;
            actions.skips = skips_after(now.level - prev.level);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A live game state. Each test changes only what matters to it.
    ///
    /// `frameTimer` runs from the start of the plugin, `gameChrono` from the
    /// construction of the GameMode, and `duration` from the moment level 0
    /// appears. The 550 ms between the last two are measured: 535, 539, 547
    /// and 562 ms over four games.
    fn at(level: i64, chrono_ms: i64) -> State {
        State {
            level,
            previous: level - 1,
            chrono_ms,
            frame_timer: 1_000 + chrono_ms,
            dim: 0,
            game_over: false,
            locked: false,
            duration_ms: (chrono_ms - 550).max(0),
            end_sequence: EndSequence::NONE,
        }
    }

    /// The same state, read in the frame the player enters the elevator.
    ///
    /// The level does not change: the run ends inside the last level, not by
    /// leaving it.
    fn elevator(level: i64, chrono_ms: i64) -> State {
        elevator_since(level, chrono_ms, 0.0)
    }

    /// The same, read `elapsed` game cycles after the elevator. One cycle is
    /// 31.25 ms.
    fn elevator_since(level: i64, chrono_ms: i64, elapsed: f64) -> State {
        State {
            end_sequence: EndSequence::from_cycles(EndSequence::FULL_CYCLES - elapsed),
            ..at(level, chrono_ms)
        }
    }

    /// The same state, but locked: black screen, transition, or pause.
    fn locked(level: i64, chrono_ms: i64) -> State {
        State {
            locked: true,
            ..at(level, chrono_ms)
        }
    }

    /// Plays a series of reads and returns the actions of the last tick.
    ///
    /// The caller supplies the timer state. The policy knows it only from what
    /// we tell it, which saves us from simulating LiveSplit.
    struct Run {
        policy: Policy,
        timer: TimerState,
    }

    impl Run {
        fn new() -> Self {
            Self {
                policy: Policy::new(),
                timer: TimerState::NotRunning,
            }
        }

        fn running(mut self) -> Self {
            self.timer = TimerState::Running;
            self
        }

        fn tick(&mut self, read: Option<State>) -> Actions {
            let actions = self.policy.tick(self.timer, read);
            if actions.reset {
                self.timer = TimerState::NotRunning;
            }
            if actions.start {
                self.timer = TimerState::Running;
            }
            actions
        }

        /// Two identical reads: the policy acts only on a confirmed level.
        fn confirm(&mut self, s: State) -> Actions {
            self.tick(Some(s));
            self.tick(Some(s))
        }
    }

    // -- start -------------------------------------------------------------

    #[test]
    fn starts_when_a_game_appears_where_there_was_none() {
        let mut r = Run::new();
        r.tick(None);
        assert!(r.tick(Some(at(0, 3_000))).start);
    }

    #[test]
    fn does_not_start_if_a_game_was_already_running_when_we_attached() {
        // Autosplitter loaded in the middle of a run: the first read is not a
        // launch.
        let mut r = Run::new();
        assert!(!r.tick(Some(at(12, 90_000))).start);
    }

    #[test]
    fn starts_even_if_the_game_clock_is_not_zero() {
        // The game clock runs from the construction of the GameMode, loading
        // and intro included. Requiring a small value stopped every start.
        let mut r = Run::new();
        r.tick(None);
        assert!(r.tick(Some(at(0, 12_000))).start);
    }

    #[test]
    fn starts_even_if_level_0_is_already_behind_us() {
        // A runner leaves level 0 in two seconds. The resolution can take
        // longer than that.
        let mut r = Run::new();
        r.tick(None);
        assert!(r.tick(Some(at(10, 4_000))).start);
    }

    #[test]
    fn does_not_start_while_the_black_screen_lasts() {
        // Level 0 has not appeared yet, so the run has not started. The
        // GameMode exists and gameChrono already runs, but that is not the
        // start.
        let mut r = Run::new();
        r.tick(None);
        let actions = r.tick(Some(locked(0, 300)));
        assert!(!actions.start);
        assert_eq!(actions.real_time_ms, None);
    }

    // -- origin of real time -----------------------------------------------

    #[test]
    fn sets_the_origin_on_the_frame_of_the_unlock() {
        // Resolved before the level appears: we see the transition, and
        // `duration` is still zero.
        let mut r = Run::new();
        r.tick(None);
        r.tick(Some(locked(0, 300)));

        let mut start = at(0, 550);
        start.duration_ms = 0;
        let actions = r.tick(Some(start));
        assert!(actions.start);
        assert_eq!(actions.real_time_ms, Some(0));

        // Two seconds later, real time has counted them.
        let mut later = at(0, 2_550);
        later.frame_timer = start.frame_timer + 2_000;
        assert_eq!(r.tick(Some(later)).real_time_ms, Some(2_000));
    }

    #[test]
    fn rebuilds_the_origin_when_the_scan_arrives_late() {
        // The usual case: the scan finishes half a second too late.
        // `duration` runs only from the unlock, so it says by how much. The
        // real time shown is correct on the first read, with no visible catch
        // up.
        let mut r = Run::new();
        r.tick(None);

        let mut late = at(0, 1_150);
        late.duration_ms = 600;
        let actions = r.tick(Some(late));
        assert!(actions.start);
        assert_eq!(actions.real_time_ms, Some(600));
    }

    #[test]
    fn real_time_stops_neither_on_pause_nor_between_levels() {
        // `Chrono.update` runs before the pause test and before the `return`
        // on `fl_lock`. So `frameTimer` follows real time whatever happens.
        // Measured: +13843 ms for 13.9 s of pause.
        let mut r = Run::new().running();
        let start = at(0, 550);
        r.tick(Some(start));

        let mut pause = locked(3, 20_000);
        pause.frame_timer = start.frame_timer + 30_000;
        pause.duration_ms = 12_000; // frozen, this one
        assert_eq!(r.tick(Some(pause)).real_time_ms, Some(30_000));
    }

    #[test]
    fn keeps_the_origin_when_the_resolution_is_lost_then_taken_again() {
        // Dropping the resolution during a game is normal: the anchor dies and
        // we scan again. Rebuilding the origin then would place it too late,
        // by all the time spent between levels that `duration` does not count
        // -- and the timer would go backwards in front of the player.
        let mut r = Run::new().running();
        let start = at(0, 550);
        r.tick(Some(start));

        let mut later = at(6, 40_000);
        later.frame_timer = start.frame_timer + 60_000;
        later.duration_ms = 38_000; // 22 s of transitions, not counted
        assert_eq!(r.tick(Some(later)).real_time_ms, Some(60_000));

        r.tick(None); // resolution lost, scan again

        let mut again = later;
        again.frame_timer += 1_000;
        again.duration_ms += 1_000;
        assert_eq!(r.tick(Some(again)).real_time_ms, Some(61_000));
    }

    #[test]
    fn forgets_the_origin_when_the_clock_goes_backwards() {
        // Another plugin process, so another game: `Std.getTimer()` restarts
        // at zero. Keeping the old origin would give a negative time.
        let mut r = Run::new().running();
        r.tick(Some(at(5, 60_000)));

        let mut fresh = at(0, 550);
        fresh.frame_timer = 900; // the plugin has just started
        fresh.duration_ms = 0;
        assert_eq!(r.tick(Some(fresh)).real_time_ms, Some(0));
    }

    #[test]
    fn starts_from_a_fresh_origin_on_the_next_game() {
        let mut r = Run::new().running();
        r.tick(Some(at(4, 30_000)));
        let mut end = at(4, 31_000);
        end.game_over = true;
        r.tick(Some(end));

        r.tick(None);
        let mut fresh = at(0, 550);
        fresh.frame_timer = 400_000;
        fresh.duration_ms = 0;
        let actions = r.tick(Some(fresh));
        assert!(actions.start);
        assert_eq!(actions.real_time_ms, Some(0));
    }

    // -- splits ------------------------------------------------------------

    #[test]
    fn splits_on_a_level_crossed() {
        let mut r = Run::new().running();
        r.confirm(at(3, 10_000));
        assert!(r.confirm(at(4, 20_000)).split);
    }

    #[test]
    fn splits_on_the_level_0_shortcut() {
        // `Adventure.nextLevel` under `fl_warpStart`: 0 -> 10 in one step.
        let mut r = Run::new().running();
        r.confirm(at(0, 5_000));
        assert!(r.confirm(at(10, 9_000)).split);
    }

    #[test]
    fn one_split_only_despite_the_writes_inside_the_same_frame() {
        // The game writes `currentId` as 1, then 0, then 10 in the same frame.
        // Our reads are not synchronised with it. Without confirmation, we
        // would split twice.
        let mut r = Run::new().running();
        r.confirm(at(0, 5_000));
        let mut splits = 0;
        for level in [1, 0, 10, 10] {
            if r.tick(Some(at(level, 9_000))).split {
                splits += 1;
            }
        }
        assert_eq!(splits, 1);
    }

    #[test]
    fn a_level_number_going_backwards_splits_nothing_and_resets_nothing() {
        // Nothing in a run sends the player to a lower level. A death costs a
        // life and puts them back in the same level, without touching
        // `currentId`, and `SpecialManager.warpZone` can only move forward:
        // its arrival is `currentId + w`, and the loop that lowers it stops at
        // `currentId`.
        //
        // So a lower number means one of three things: a level script calling
        // `forcedGoto` backwards, the three writes `Adventure.nextLevel` makes
        // inside one frame, or a misread. None of them is progress, and none
        // of them ends the attempt.
        //
        // A new game is the fourth case, and the game clock catches that one:
        // see `restarts_when_the_game_clock_jumps_backwards`.
        let mut r = Run::new().running();
        r.confirm(at(7, 30_000));

        let back_one = r.confirm(at(6, 31_000));
        assert!(!back_one.split);
        assert!(!back_one.reset);

        let all_the_way = r.confirm(at(0, 32_000));
        assert!(!all_the_way.split);
        assert!(!all_the_way.reset);
    }

    #[test]
    fn does_not_split_in_a_parallel_dimension() {
        let mut r = Run::new().running();
        r.confirm(at(3, 10_000));
        let mut s = at(4, 20_000);
        s.dim = 1;
        assert!(!r.confirm(s).split);
    }

    #[test]
    fn does_not_split_when_the_timer_is_not_running() {
        let mut r = Run::new();
        r.confirm(at(3, 10_000));
        assert!(!r.confirm(at(4, 20_000)).split);
    }

    // -- warp zones ---------------------------------------------------------

    /// @spec crossing::warp-skips-the-levels-never-played
    #[test]
    fn a_crossing_of_one_level_skips_nothing() {
        let mut r = Run::new().running();
        r.confirm(at(3, 10_000));
        let actions = r.confirm(at(4, 20_000));
        assert!(actions.split);
        assert_eq!(actions.skips, 0);
    }

    /// @spec crossing::warp-skips-the-levels-never-played
    #[test]
    fn a_warp_of_two_skips_the_level_it_never_played() {
        // `SpecialManager.warpZone` -> `forcedGoto`: the umbrella. The player
        // never enters level 43, so its segment must record no time, and no
        // gold.
        let mut r = Run::new().running();
        r.confirm(at(42, 100_000));
        let actions = r.confirm(at(44, 110_000));
        assert!(actions.split);
        assert_eq!(actions.skips, 1);
    }

    /// @spec crossing::warp-skips-the-levels-never-played
    #[test]
    fn a_warp_of_three_skips_the_two_levels_it_never_played() {
        let mut r = Run::new().running();
        r.confirm(at(42, 100_000));
        let actions = r.confirm(at(45, 110_000));
        assert!(actions.split);
        assert_eq!(actions.skips, 2);
    }

    /// @spec crossing::a-large-jump-skips-nothing
    #[test]
    fn the_level_0_shortcut_skips_nothing() {
        // `0 -> 10` happens in every attempt, so the runner plans one segment
        // for it. Nine dead segments would exist only to be skipped.
        let mut r = Run::new().running();
        r.confirm(at(0, 5_000));
        let actions = r.confirm(at(10, 9_000));
        assert!(actions.split);
        assert_eq!(actions.skips, 0);
    }

    /// @spec crossing::a-large-jump-skips-nothing
    #[test]
    fn a_jump_wider_than_a_warp_skips_nothing() {
        // `warpZone` advances by at most 3. A wider move is the level 0
        // shortcut read in the middle of its three writes, or a misread.
        // Without the bound, one wrong read would burn the splits file.
        let mut r = Run::new().running();
        r.confirm(at(1, 5_000));
        let actions = r.confirm(at(10, 9_000));
        assert!(actions.split);
        assert_eq!(actions.skips, 0);
    }

    /// @spec crossing::warp-skips-the-levels-never-played
    #[test]
    fn nothing_that_does_not_split_ever_skips() {
        // A skip is never sent alone. Every path that refuses the split must
        // refuse the skips with it.
        // Each case starts from its own run: acting on a level makes it the
        // new reference, so chaining them would change what the next one
        // means.
        let mut backwards = Run::new().running();
        backwards.confirm(at(42, 100_000));
        assert_eq!(backwards.confirm(at(40, 101_000)).skips, 0);

        // A death costs a life and leaves `currentId` alone.
        let mut death = Run::new().running();
        death.confirm(at(42, 100_000));
        assert_eq!(death.confirm(at(42, 101_000)).skips, 0);

        let mut parallel = Run::new().running();
        parallel.confirm(at(42, 100_000));
        let mut elsewhere = at(45, 101_000);
        elsewhere.dim = 1;
        assert_eq!(parallel.confirm(elsewhere).skips, 0);

        let mut stopped = Run::new();
        stopped.confirm(at(42, 100_000));
        assert_eq!(stopped.confirm(at(45, 110_000)).skips, 0);
    }

    /// @spec crossing::one-split-per-crossing
    #[test]
    fn the_elevator_skips_nothing() {
        // The run ends inside the last level. Nothing is left to skip.
        let mut r = Run::new().running();
        r.confirm(at(103, 600_000));
        let actions = r.tick(Some(elevator(103, 601_000)));
        assert!(actions.split);
        assert_eq!(actions.skips, 0);
    }

    // -- end of game and dead objects ---------------------------------------

    #[test]
    fn resets_on_game_over_and_drops_the_resolution() {
        let mut r = Run::new().running();
        r.confirm(at(9, 40_000));
        let mut end = at(9, 41_000);
        end.game_over = true;
        let actions = r.tick(Some(end));
        assert!(actions.reset);
        assert!(actions.drop_resolution);
    }

    #[test]
    fn drops_the_resolution_when_the_heartbeat_freezes() {
        // An abandoned GameMode stays readable, with a plausible level and a
        // plausible clock. Only `frameTimer` gives it away.
        let mut r = Run::new().running();
        let dead = at(9, 40_000);
        let mut actions = Actions::default();
        for _ in 0..STALE_TICKS + 1 {
            actions = r.tick(Some(dead));
        }
        assert!(actions.drop_resolution);
    }

    #[test]
    fn drops_nothing_while_the_heartbeat_advances() {
        let mut r = Run::new().running();
        for i in 0..STALE_TICKS * 2 {
            let s = at(9, 40_000 + i as i64);
            assert!(!r.tick(Some(s)).drop_resolution);
        }
    }

    #[test]
    fn restarts_the_run_when_the_timer_has_already_ended() {
        // The runner reached the last split of their file, so LiveSplit is
        // finished. They launch another game. A finished timer refuses
        // `start`, so it has to be reset first, in that order.
        let mut r = Run::new();
        r.timer = TimerState::Ended;
        r.tick(Some(at(0, 3_000)));
        r.tick(Some(at(0, 3_050)));

        let actions = r.tick(Some(at(0, 3_100)));
        assert!(actions.reset);
        assert!(actions.start);
    }

    #[test]
    fn restarts_when_the_game_clock_jumps_backwards() {
        // A new game began and we never saw the gap: the GameMode was rebuilt
        // and its clock restarted from zero. More than two seconds backwards
        // is not a misread, it is another game.
        let mut r = Run::new().running();
        r.confirm(at(7, 40_000));

        let actions = r.confirm(at(0, 900));
        assert!(actions.reset);
        assert!(actions.start);
    }

    #[test]
    fn an_unknown_timer_state_makes_nothing_happen() {
        // The runtime reported something we cannot read. Doing nothing is the
        // only safe answer to "I cannot tell".
        let mut r = Run::new();
        r.timer = TimerState::Unknown;
        r.confirm(at(3, 10_000));

        let actions = r.confirm(at(4, 20_000));
        assert!(!actions.split);
        assert!(!actions.start);
        assert!(!actions.reset);
    }

    // -- the end of the run --------------------------------------------------

    #[test]
    fn splits_when_the_player_enters_the_elevator() {
        // The last split of the speedrun rule. The level stays at 103: the run
        // ends inside the last level, not by leaving it.
        let mut r = Run::new().running();
        r.confirm(at(103, 600_000));
        assert!(r.tick(Some(elevator(103, 601_000))).split);
    }

    #[test]
    fn splits_once_for_the_whole_end_sequence() {
        // `endModeTimer` stays above zero for fourteen seconds. We act on the
        // rise, so the reads that follow must add nothing.
        let mut r = Run::new().running();
        r.confirm(at(103, 600_000));
        let mut splits = 0;
        for i in 0..400 {
            if r.tick(Some(elevator(103, 601_000 + i))).split {
                splits += 1;
            }
        }
        assert_eq!(splits, 1);
    }

    #[test]
    fn the_time_stops_at_the_elevator() {
        // The game runs for fourteen more seconds, and `frameTimer` with it.
        // The final time must not follow.
        let mut r = Run::new().running();
        let start = at(0, 550);
        r.tick(Some(start));

        let mut end = elevator(103, 600_000);
        end.frame_timer = start.frame_timer + 600_000;
        let finish = r.tick(Some(end)).real_time_ms;
        assert_eq!(finish, Some(600_000));

        let mut later = end;
        later.frame_timer += 14_000;
        assert_eq!(r.tick(Some(later)).real_time_ms, finish);
    }

    #[test]
    fn dates_the_last_split_at_the_frame_of_the_elevator() {
        // We read 100 ms after the elevator. What is left of the cinematic
        // says so, and the final time goes back to the frame that ended the
        // run -- the same trick that dates the start.
        let mut r = Run::new().running();
        let start = at(0, 550);
        r.tick(Some(start));

        let mut late = elevator_since(103, 600_000, 3.2); // 3.2 cycles = 100 ms
        late.frame_timer = start.frame_timer + 600_100;
        let actions = r.tick(Some(late));
        assert!(actions.split);
        assert_eq!(actions.real_time_ms, Some(600_000));
    }

    #[test]
    fn dates_the_last_split_where_it_saw_it_when_the_gap_makes_no_sense() {
        // Another version, another cinematic: the value no longer means what
        // we think. We then keep the time of the read. One read late beats a
        // correction of ten seconds.
        let mut r = Run::new().running();
        let start = at(0, 550);
        r.tick(Some(start));

        let mut late = elevator_since(103, 600_000, 320.0); // 10 s
        late.frame_timer = start.frame_timer + 610_000;
        assert_eq!(r.tick(Some(late)).real_time_ms, Some(610_000));
    }

    #[test]
    fn the_elevator_of_a_parallel_dimension_ends_nothing() {
        // The elevator belongs to the last level of the adventure. Whatever a
        // parallel world does, it is not the end of this run.
        let mut r = Run::new().running();
        r.confirm(at(103, 600_000));

        let mut elsewhere = elevator(103, 601_000);
        elsewhere.dim = 1;
        let actions = r.tick(Some(elsewhere));
        assert!(!actions.split);

        // And the time still moves, because the run is not over.
        let mut later = elsewhere;
        later.frame_timer += 5_000;
        assert!(r.tick(Some(later)).real_time_ms > actions.real_time_ms);
    }

    #[test]
    fn does_not_reset_on_the_game_over_that_follows_the_elevator() {
        // Game over arrives fourteen seconds after the elevator. A reset there
        // would erase the run on the finish line.
        let mut r = Run::new().running();
        r.confirm(at(103, 600_000));
        r.tick(Some(elevator(103, 601_000)));

        let mut over = elevator(103, 615_000);
        over.game_over = true;
        let actions = r.tick(Some(over));
        assert!(!actions.reset);
        assert!(actions.drop_resolution);
    }

    #[test]
    fn does_not_reset_when_the_plugin_dies_after_the_elevator() {
        // `exitGame` navigates to the end page, so the SWF disappears. That is
        // the normal end of a run that succeeded, not an abandon.
        let mut r = Run::new().running();
        r.confirm(at(103, 600_000));
        r.tick(Some(elevator(103, 601_000)));
        for _ in 0..LOST_BEFORE_RESET * 2 {
            assert!(!r.tick(None).reset);
        }
    }

    #[test]
    fn does_not_end_a_run_it_joined_in_the_middle_of_the_sequence() {
        // Resolved during the fourteen seconds: we never saw the rise, and we
        // never saw the run start either. Ending it would invent a time.
        let mut r = Run::new().running();
        assert!(!r.tick(Some(elevator(103, 605_000))).split);
        assert!(!r.tick(Some(elevator(103, 605_100))).split);
    }

    #[test]
    fn the_last_level_alone_ends_nothing() {
        // Reaching level 103 is a crossing like any other. Only the elevator
        // ends the run, so the time must keep moving inside that level.
        let mut r = Run::new().running();
        let start = at(0, 550);
        r.tick(Some(start));
        r.confirm(at(102, 590_000));

        let mut arrive = at(103, 595_000);
        arrive.frame_timer = start.frame_timer + 595_000;
        assert!(r.confirm(arrive).split);

        let mut later = arrive;
        later.frame_timer += 5_000;
        assert_eq!(r.tick(Some(later)).real_time_ms, Some(600_000));
    }

    #[test]
    fn a_new_run_after_a_finished_one_starts_and_moves_again() {
        let mut r = Run::new().running();
        r.confirm(at(103, 600_000));
        r.tick(Some(elevator(103, 601_000)));

        let mut over = elevator(103, 615_000);
        over.game_over = true;
        r.tick(Some(over));
        r.tick(None);
        r.timer = TimerState::NotRunning;

        let mut fresh = at(0, 550);
        fresh.frame_timer = 900_000;
        fresh.duration_ms = 0;
        let actions = r.tick(Some(fresh));
        assert!(actions.start);
        assert_eq!(actions.real_time_ms, Some(0));

        let mut later = fresh;
        later.frame_timer += 3_000;
        later.duration_ms = 3_000;
        assert_eq!(r.tick(Some(later)).real_time_ms, Some(3_000));
    }

    // -- the invariant -------------------------------------------------------

    /// Plays a sequence and returns the last time published.
    ///
    /// Fails the moment the published time goes backwards. That is the one
    /// defect a runner sees at once, and no scenario test holds every guard
    /// against it at the same time.
    fn play(r: &mut Run, reads: &[Option<State>]) -> i64 {
        let mut last = -1;
        for read in reads {
            if let Some(ms) = r.tick(*read).real_time_ms {
                assert!(ms >= last, "the time went backwards: {ms} after {last}");
                last = ms;
            }
        }
        last
    }

    #[test]
    fn the_published_time_never_goes_backwards_inside_a_run() {
        // One whole run, through everything that has ever moved the origin:
        // the black screen, the start, a level shortcut, a game pause, the
        // resolution lost and taken again, then the elevator.
        //
        // `frame_timer` is the only clock that never stops, so it is the one
        // that drives this. `duration` freezes with the pause, as the game
        // does it.
        let read = |level: i64, frame: i64, duration: i64| State {
            level,
            previous: level - 1,
            chrono_ms: duration,
            frame_timer: frame,
            dim: 0,
            game_over: false,
            locked: false,
            duration_ms: duration,
            end_sequence: EndSequence::NONE,
        };
        let black_screen = State {
            locked: true,
            ..read(0, 100_000, 0)
        };
        let mut elevator_frame = read(12, 130_100, 12_100);
        elevator_frame.end_sequence = EndSequence::from_cycles(EndSequence::FULL_CYCLES);
        let mut after_the_end = elevator_frame;
        after_the_end.frame_timer += 5_000;

        let mut r = Run::new();
        let final_time = play(
            &mut r,
            &[
                None,                                  // no game yet
                Some(black_screen),                    // loading, level 0 hidden
                Some(read(0, 100_550, 0)),             // the start
                Some(read(0, 102_550, 2_000)),
                Some(read(10, 109_000, 8_450)),        // the level 0 shortcut
                Some(State { locked: true, ..read(10, 123_000, 8_450) }), // paused
                Some(read(10, 124_000, 9_450)),
                None,                                  // resolution lost
                None,
                Some(read(12, 130_000, 12_000)),       // taken again
                Some(elevator_frame),
                Some(after_the_end),                   // the cinematic runs on
            ],
        );

        // The origin is the frame of the unlock, 100_550. The elevator is at
        // 130_100, so the run lasted 29_550 ms -- pause included, as the rule
        // says -- and it must not move afterwards.
        assert_eq!(final_time, 29_550);
    }

    // -- no game ------------------------------------------------------------

    #[test]
    fn never_resets_before_a_game_was_seen() {
        // The Flash plugin exists as soon as the application opens. Counting
        // lost reads before the first game reset the timer during the loading
        // screen.
        let mut r = Run::new().running();
        for _ in 0..LOST_BEFORE_RESET * 2 {
            assert!(!r.tick(None).reset);
        }
    }

    #[test]
    fn resets_after_a_game_was_lost_for_long_enough() {
        let mut r = Run::new().running();
        r.confirm(at(5, 20_000));
        let mut reset = false;
        for _ in 0..LOST_BEFORE_RESET + 1 {
            reset |= r.tick(None).reset;
        }
        assert!(reset);
    }

    #[test]
    fn a_new_game_after_a_loss_starts_again() {
        let mut r = Run::new().running();
        r.confirm(at(5, 20_000));
        r.tick(None);
        r.timer = TimerState::NotRunning;
        assert!(r.tick(Some(at(0, 2_000))).start);
    }

}
