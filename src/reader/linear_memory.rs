//! The linear memory of a WebAssembly module, read by offset.
//!
//! Every pointer inside a linear memory is an offset from its base. So the
//! reader asks for offsets, and this turns them into addresses of the
//! process, as `ProcessMemory` turns addresses into bytes.

use crate::avm1::Memory;

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
