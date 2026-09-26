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
    /// Adobe's Flash projector under Linux, as Eternalfest Desktop starts it.
    /// Its AVM1 is the one of Pepper Flash.
    FlashProjector,
    /// Ruffle desktop, as Eternalfest Desktop starts it when the projector is
    /// missing.
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
            Runtime::FlashProjector => "the Flash projector",
            Runtime::Ruffle => "Ruffle",
            Runtime::RuffleWeb { .. } => "Ruffle in Firefox",
        }
    }

    /// The ranges where the heap of this player lives, in the order to sweep
    /// them.
    pub fn heap_ranges(self, process: &Process) -> Vec<(u64, u64)> {
        match self {
            Runtime::PepperFlash | Runtime::FlashProjector => plugin::heap_ranges(process),
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
            Runtime::PepperFlash | Runtime::FlashProjector => plugin::heap_size(process),
            Runtime::Ruffle | Runtime::RuffleWeb { .. } => self
                .heap_ranges(process)
                .iter()
                .map(|(start, end)| end - start)
                .sum(),
        }
    }

    /// Are the objects of this player born in memory already committed? The
    /// allocator of a linear memory reuses the room it freed, so a new object
    /// does not grow the memory, and a search of what changed misses it.
    pub fn births_in_reused_memory(self) -> bool {
        matches!(self, Runtime::RuffleWeb { .. })
    }

    /// Several Firefox tabs may run Ruffle, and only one plays Hammerfest.
    pub fn is_one_of_several(self) -> bool {
        matches!(self, Runtime::RuffleWeb { .. })
    }
}
