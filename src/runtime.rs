//! Which Flash player the game runs in, and where to look for it.

use alloc::vec::Vec;
use asr::Process;

use hammerfest_reader::linear_memory::blocks;

use crate::plugin;

/// A Flash player the autosplitter can read.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Runtime {
    /// The Pepper Flash plugin, inside EternalTwin.
    PepperFlash,
    /// Ruffle desktop, as Eternalfest Desktop starts it.
    Ruffle,
    /// Ruffle in a Firefox tab: a linear memory at `base`, whose reservation
    /// reaches `span` bytes. Every address the reader asks for is an offset
    /// in it.
    RuffleWeb { base: u64, span: u64 },
}

impl Runtime {
    /// For the log.
    pub fn name(self) -> &'static str {
        match self {
            Runtime::PepperFlash => "the Flash plugin of EternalTwin",
            Runtime::Ruffle => "Ruffle",
            Runtime::RuffleWeb { .. } => "Ruffle in Firefox",
        }
    }

    /// The ranges where the heap of this player lives, in the order to sweep
    /// them.
    pub fn heap_ranges(self, process: &Process) -> Vec<(u64, u64)> {
        match self {
            Runtime::PepperFlash => plugin::heap_ranges(process),
            Runtime::Ruffle => plugin::ruffle_heap_ranges(process),
            // Its committed part, as blocks of offsets.
            Runtime::RuffleWeb { base, .. } => plugin::committed_end(process, base)
                .map(|end| blocks(end - base))
                .unwrap_or_default(),
        }
    }

    /// Total committed bytes in the heap of this player.
    pub fn heap_size(self, process: &Process) -> u64 {
        match self {
            Runtime::PepperFlash => plugin::heap_size(process),
            Runtime::Ruffle | Runtime::RuffleWeb { .. } => self
                .heap_ranges(process)
                .iter()
                .map(|(start, end)| end - start)
                .sum(),
        }
    }

    /// Several Firefox tabs may run Ruffle, and only one plays Hammerfest.
    pub fn is_one_of_several(self) -> bool {
        matches!(self, Runtime::RuffleWeb { .. })
    }
}
