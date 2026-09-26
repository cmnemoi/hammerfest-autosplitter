//! What a search tells whoever listens.
//!
//! The reader prints nothing. It says what happens, and the module that hosts
//! it decides what to do with that: print a line, measure, or nothing. Every
//! method does nothing unless a listener says otherwise.

pub trait SearchLog {
    /// A search starts to read the heap.
    fn search_started(&mut self) {}

    /// The search moves on to the stage `next`. `requested` bytes in `calls`
    /// block reads were asked for since the search started.
    fn stage(&mut self, _next: &'static str, _requested: u64, _calls: u64) {}

    /// One block of the heap was read, or could not be.
    fn block_read(&mut self, _bytes: usize, _succeeded: bool) {}

    /// The search let one tick pass.
    fn tick_given_back(&mut self) {}

    /// How the search will end, when it ends.
    fn outcome(&mut self, _outcome: &'static str) {}

    /// The search is over, whatever way it took out.
    fn search_finished(&mut self, _requested: u64, _calls: u64) {}

    /// The search looks for the String object of `key`.
    fn string_search(&mut self, _key: &str, _layout_proven: bool, _cached: bool) {}

    /// The `GameManager` was found, at this property table.
    fn manager_found(&mut self, _table: u64, _profile: &'static str) {}

    /// A `GameMode` was found without its manager, at this property table.
    fn game_mode_found(&mut self, _table: u64, _world: &str, _profile: &'static str) {}

    /// The anchor stayed silent too long, and was dropped.
    fn silent_anchor_dropped(&mut self) {}
}

/// A listener that ignores everything.
#[cfg(test)]
pub struct Silent;

#[cfg(test)]
impl SearchLog for Silent {}
