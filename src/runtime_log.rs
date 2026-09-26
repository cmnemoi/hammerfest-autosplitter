//! What the search says, written to the LiveSplit log.
//!
//! Three lines ship in every build: they say where the game was found, and a
//! runner who sends a log sends them with it. The rest only measures, and
//! [`ScanTrace`] holds nothing without the `diagnostics` feature.

use crate::diagnostics::{self, ScanTrace};
use crate::search_log::SearchLog;

#[derive(Default)]
pub struct RuntimeLog {
    trace: ScanTrace,
}

impl SearchLog for RuntimeLog {
    fn search_started(&mut self) {
        self.trace.restart();
    }

    fn stage(&mut self, next: &'static str, requested: u64, calls: u64) {
        self.trace.stage(next, requested, calls);
    }

    fn block_read(&mut self, bytes: usize, succeeded: bool) {
        self.trace.read(bytes, succeeded);
    }

    fn tick_given_back(&mut self) {
        self.trace.paused();
    }

    fn outcome(&mut self, outcome: &'static str) {
        self.trace.outcome(outcome);
    }

    fn search_finished(&mut self, requested: u64, calls: u64) {
        self.trace.finish(requested, calls);
    }

    fn string_search(&mut self, key: &str, layout_proven: bool, cached: bool) {
        diagnostics::string_search(key, layout_proven, cached);
    }

    fn manager_found(&mut self, table: u64, profile: &'static str) {
        asr::print_message(&alloc::format!(
            "Hammerfest: GameManager 0x{table:x}, layout {profile}"
        ));
    }

    fn game_mode_found(&mut self, table: u64, world: &str, profile: &'static str) {
        asr::print_message(&alloc::format!(
            "Hammerfest: GameMode 0x{table:x}, world {world}, layout {profile}"
        ));
    }

    fn silent_anchor_dropped(&mut self) {
        asr::print_message("Hammerfest: silent anchor, searching again");
    }
}
