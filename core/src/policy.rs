//! When to start, split, reset, or drop a resolution.
//!
//! A pure state machine. Every tick it receives what the game says -- or
//! nothing, when no game was found -- and returns the actions to execute. It
//! reads no memory and knows nothing about LiveSplit.

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

/// The settings, as the core sees them: plain booleans.
#[derive(Copy, Clone, Debug)]
pub struct Rules {
    pub auto_start: bool,
    pub split_on_level: bool,
    pub main_world_only: bool,
    pub auto_reset: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            auto_start: true,
            split_on_level: true,
            main_world_only: true,
            auto_reset: true,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TimerState {
    NotRunning,
    Running,
    Paused,
    Ended,
    Unknown,
}

/// What the caller must do, in this order.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct Actions {
    pub reset: bool,
    pub start: bool,
    pub split: bool,
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
}

impl Policy {
    pub fn new() -> Self {
        Self::default()
    }

    /// One tick. `read` is None when no game could be read.
    pub fn tick(&mut self, timer: TimerState, rules: &Rules, read: Option<State>) -> Actions {
        let Some(now) = read else {
            return self.no_game(timer, rules);
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
            if rules.auto_reset && timer == TimerState::Running {
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
            if self.launched && rules.auto_start && timer == TimerState::NotRunning {
                actions.start = true;
            }
        }
        actions.real_time_ms = self.origin.map(|o| now.frame_timer - o);

        // We act only on a level read twice in a row.
        //
        // `Adventure.nextLevel` writes `currentId` three times in the same
        // frame -- 1, then 0, then 10 -- and nothing synchronises our read
        // with the game frame. Without this confirmation, a read that lands in
        // the middle would produce two splits instead of one, at random.
        if self.seen.map(|s| s.level) == Some(now.level) {
            self.decide(timer, rules, &now, &mut actions);
            self.prev = Some(now);
        }
        self.seen = Some(now);
        actions
    }

    fn no_game(&mut self, timer: TimerState, rules: &Rules) -> Actions {
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
            if self.lost == LOST_BEFORE_RESET
                && rules.auto_reset
                && timer == TimerState::Running
            {
                actions.reset = true;
            }
        }
        actions
    }

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

    fn decide(&self, timer: TimerState, rules: &Rules, now: &State, actions: &mut Actions) {
        let Some(prev) = self.prev else {
            return;
        };

        // A finished timer does not accept `start`. It must be reset first,
        // and only if we have permission to do so.
        if rules.auto_start
            && rules.auto_reset
            && timer == TimerState::Ended
            && now.level == FIRST_LEVEL
        {
            actions.reset = true;
            actions.start = true;
            return;
        }

        // The game clock goes backwards: another game started and we did not
        // see the transition.
        if now.chrono_ms + 2_000 < prev.chrono_ms {
            if rules.auto_reset && timer == TimerState::Running {
                actions.reset = true;
            }
            if rules.auto_start {
                actions.start = true;
            }
            return;
        }

        // Any forward progress counts, not only `+1`. Hammerfest skips
        // levels: the level 0 shortcut leads straight to level 10
        // (`Adventure.nextLevel` under `fl_warpStart`), and warp zones advance
        // by 1 to 3 (`SpecialManager.warpZone` -> `forcedGoto`).
        //
        // One split per crossing, whatever the number of levels skipped. The
        // route of a run goes through these shortcuts, so one segment matches
        // them. Going backwards -- death, restart -- is not progress.
        if rules.split_on_level
            && timer == TimerState::Running
            && now.level > prev.level
            && (!rules.main_world_only || now.dim == 0)
        {
            actions.split = true;
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
        rules: Rules,
        timer: TimerState,
    }

    impl Run {
        fn new() -> Self {
            Self {
                policy: Policy::new(),
                rules: Rules::default(),
                timer: TimerState::NotRunning,
            }
        }

        fn running(mut self) -> Self {
            self.timer = TimerState::Running;
            self
        }

        fn tick(&mut self, read: Option<State>) -> Actions {
            let actions = self.policy.tick(self.timer, &self.rules, read);
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
    fn does_not_split_backwards() {
        let mut r = Run::new().running();
        r.confirm(at(7, 30_000));
        assert!(!r.confirm(at(0, 31_000)).split);
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

    // -- settings ------------------------------------------------------------

    #[test]
    fn honours_the_settings_that_are_off() {
        let mut r = Run::new();
        r.rules = Rules {
            auto_start: false,
            split_on_level: false,
            main_world_only: true,
            auto_reset: false,
        };
        r.tick(None);
        assert!(!r.tick(Some(at(0, 1_000))).start);

        r.timer = TimerState::Running;
        r.confirm(at(3, 10_000));
        assert!(!r.confirm(at(4, 20_000)).split);

        let mut end = at(4, 21_000);
        end.game_over = true;
        assert!(!r.tick(Some(end)).reset);
    }
}
