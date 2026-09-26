//! When the loop may scan the whole heap again.
//!
//! See [the spec](../../docs/specs/resolution-pacing.md). In short: a scan
//! costs eighty MiB read and blocks the loop, so a failure buys a wait. A heap
//! that grows by several MiB is the SWF creating its objects, and that cancels
//! the wait.

/// Ticks to wait after the first scan that finds nothing.
const MIN_WAIT: u32 = 20;
/// The ceiling on that wait. About one second, at sixty ticks per second.
const MAX_WAIT: u32 = 60;
/// Heap growth that cancels a wait, in bytes.
const GROWTH: u64 = 4 << 20;

/// What the loop knows about its own scanning.
pub struct Pacing {
    /// Ticks left before a scan is allowed again.
    wait: u32,
    /// The wait the next failure will set.
    next_wait: u32,
    /// The size of the heap at the last scan.
    heap: u64,
    /// Is a game held since it was found?
    holds_a_game: bool,
}

impl Default for Pacing {
    fn default() -> Self {
        Self::new()
    }
}

impl Pacing {
    pub fn new() -> Self {
        Self {
            wait: 0,
            next_wait: MIN_WAIT,
            heap: 0,
            holds_a_game: false,
        }
    }

    /// May the loop scan the whole heap on this tick?
    ///
    /// `heap` is the committed size of the plugin heap now, in bytes. An
    /// answer of `false` spends one tick of the wait.
    pub fn may_scan(&mut self, heap: u64) -> bool {
        let grown = heap > self.heap + GROWTH;
        if self.wait > 0 && !grown {
            self.wait -= 1;
            return false;
        }
        self.heap = heap;
        true
    }

    /// The scan found nothing.
    pub fn scan_failed(&mut self) {
        self.wait = self.next_wait;
        self.next_wait = (self.next_wait * 2).min(MAX_WAIT);
    }

    /// The game is there, whichever path found it.
    pub fn game_found(&mut self) {
        self.next_wait = MIN_WAIT;
        self.holds_a_game = true;
    }

    /// The resolution was dropped.
    ///
    /// @spec pacing::a-game-lost-scans-at-once
    /// @spec pacing::only-a-game-held-can-be-lost
    pub fn game_lost(&mut self) {
        if !self.holds_a_game {
            return;
        }
        self.holds_a_game = false;
        self.wait = 0;
        self.next_wait = MIN_WAIT;
    }

    /// Ticks left before the next scan. For the trace, and nothing else.
    pub fn wait(&self) -> u32 {
        self.wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A heap that has finished loading. It stops growing, so the wait is
    /// what decides.
    const SETTLED: u64 = 80 << 20;

    /// How many ticks the pacing refuses before it allows a scan again.
    fn ticks_refused(p: &mut Pacing, heap: u64) -> u32 {
        let mut refused = 0;
        while !p.may_scan(heap) {
            refused += 1;
            assert!(refused < 1_000, "the wait never ends");
        }
        refused
    }

    /// @spec pacing::the-first-scan-waits-for-nothing
    #[test]
    fn the_first_scan_happens_on_the_first_tick() {
        assert!(Pacing::new().may_scan(SETTLED));
    }

    /// @spec pacing::a-failed-scan-waits-longer-each-time
    #[test]
    fn a_failed_scan_waits_before_the_next_one() {
        let mut p = Pacing::new();
        p.may_scan(SETTLED);
        p.scan_failed();

        assert_eq!(ticks_refused(&mut p, SETTLED), 20);
    }

    /// @spec pacing::a-failed-scan-waits-longer-each-time
    #[test]
    fn the_wait_grows_with_every_failure() {
        let mut p = Pacing::new();
        p.may_scan(SETTLED);

        let mut waits = [0u32; 3];
        for wait in &mut waits {
            p.scan_failed();
            *wait = ticks_refused(&mut p, SETTLED);
        }

        assert_eq!(waits, [20, 40, 60]);
    }

    /// @spec pacing::a-failed-scan-waits-longer-each-time
    #[test]
    fn the_wait_stops_at_its_ceiling() {
        let mut p = Pacing::new();
        p.may_scan(SETTLED);
        for _ in 0..10 {
            p.scan_failed();
            ticks_refused(&mut p, SETTLED);
        }

        p.scan_failed();

        assert_eq!(ticks_refused(&mut p, SETTLED), 60);
    }

    /// @spec pacing::a-heap-that-grows-scans-at-once
    #[test]
    fn a_heap_that_grows_scans_at_once() {
        let mut p = Pacing::new();
        p.may_scan(SETTLED);
        p.scan_failed();

        assert!(p.may_scan(SETTLED + (4 << 20) + 1));
    }

    /// @spec pacing::allocator-noise-is-not-growth
    #[test]
    fn allocator_noise_does_not_scan() {
        let mut p = Pacing::new();
        p.may_scan(SETTLED);
        p.scan_failed();

        assert!(!p.may_scan(SETTLED + (512 << 10)));
    }

    /// @spec pacing::a-game-found-clears-the-wait
    #[test]
    fn a_game_found_starts_the_wait_again_at_its_shortest() {
        let mut p = Pacing::new();
        p.may_scan(SETTLED);
        for _ in 0..5 {
            p.scan_failed();
            ticks_refused(&mut p, SETTLED);
        }

        p.game_found();
        p.scan_failed();

        assert_eq!(ticks_refused(&mut p, SETTLED), 20);
    }

    /// @spec pacing::a-game-lost-scans-at-once
    #[test]
    fn a_game_lost_scans_on_the_next_tick() {
        let mut p = Pacing::new();
        p.may_scan(SETTLED);
        p.game_found();
        p.scan_failed();

        p.game_lost();

        assert!(p.may_scan(SETTLED));
    }

    /// @spec pacing::only-a-game-held-can-be-lost
    #[test]
    fn a_resolution_dropped_in_the_menus_keeps_the_wait() {
        let mut p = Pacing::new();
        p.may_scan(SETTLED);
        p.scan_failed();

        p.game_lost();

        assert_eq!(ticks_refused(&mut p, SETTLED), 20);
    }

    /// @spec pacing::only-a-game-held-can-be-lost
    #[test]
    fn a_game_lost_twice_is_lost_once() {
        let mut p = Pacing::new();
        p.may_scan(SETTLED);
        p.game_found();
        p.game_lost();
        p.may_scan(SETTLED);
        p.scan_failed();

        p.game_lost();

        assert_eq!(ticks_refused(&mut p, SETTLED), 20);
    }
}
