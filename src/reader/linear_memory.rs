//! The linear memory of a WebAssembly module, read by offset.
//!
//! Every pointer inside a linear memory is an offset from its base. So the
//! reader asks for offsets, and this turns them into addresses of the
//! process, as `ProcessMemory` turns addresses into bytes.

use alloc::vec::Vec;

use crate::avm1::Memory;

/// The size of a block a linear memory is swept by.
const BLOCK: u64 = 1 << 20;

/// The committed part of a linear memory, as blocks of 1 MiB.
///
/// A linear memory grows at its end. Cut in blocks, the blocks it had stay the
/// same when it grows, and the search sees the new ones as the memory that
/// changed.
///
/// @spec browser::swept-by-blocks
pub fn blocks(committed: u64) -> Vec<(u64, u64)> {
    (0..committed)
        .step_by(BLOCK as usize)
        .map(|start| (start, (start + BLOCK).min(committed)))
        .collect()
}

/// A linear memory: `size` bytes of `process`, from `base`.
pub struct LinearMemory<'a> {
    process: &'a dyn Memory,
    base: u64,
    size: u64,
}

impl<'a> LinearMemory<'a> {
    pub fn new(process: &'a dyn Memory, base: u64, size: u64) -> Self {
        Self {
            process,
            base,
            size,
        }
    }

    /// The linear memory as one range of offsets, to sweep.
    pub fn range(&self) -> (u64, u64) {
        (0, self.size)
    }
}

impl Memory for LinearMemory<'_> {
    /// Past the end of the linear memory, the bytes belong to something else.
    ///
    /// @spec browser::an-offset-from-the-base
    fn read_into(&self, offset: u64, buf: &mut [u8]) -> Option<()> {
        let end = offset.checked_add(buf.len() as u64)?;
        if end > self.size {
            return None;
        }
        self.process.read_into(self.base.checked_add(offset)?, buf)
    }
}
