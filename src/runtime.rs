//! Which Flash player the game runs in, and where to look for it.

use alloc::vec::Vec;
use asr::Process;

use crate::plugin;

/// A Flash player the autosplitter can read.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Runtime {
    /// The Pepper Flash plugin, inside EternalTwin.
    PepperFlash,
    /// Ruffle desktop, as Eternalfest Desktop starts it.
    Ruffle,
}

impl Runtime {
    /// For the log.
    pub fn name(self) -> &'static str {
        match self {
            Runtime::PepperFlash => "the Flash plugin of EternalTwin",
            Runtime::Ruffle => "Ruffle",
        }
    }

    /// The ranges where the heap of this player lives, in the order to sweep
    /// them.
    pub fn heap_ranges(self, process: &Process) -> Vec<(u64, u64)> {
        match self {
            Runtime::PepperFlash => plugin::heap_ranges(process),
            Runtime::Ruffle => plugin::ruffle_heap_ranges(process),
        }
    }

    /// Total committed bytes in the heap of this player.
    pub fn heap_size(self, process: &Process) -> u64 {
        match self {
            Runtime::PepperFlash => plugin::heap_size(process),
            Runtime::Ruffle => plugin::ruffle_heap_ranges(process)
                .iter()
                .map(|(start, end)| end - start)
                .sum(),
        }
    }
}
