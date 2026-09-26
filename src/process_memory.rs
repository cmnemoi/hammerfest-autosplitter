//! The process, as the reader asks for it.

use asr::{Address, Process};
use hammerfest_reader::avm1::Memory;

use crate::diagnostics;

/// The memory of the plugin process.
///
/// A read that fails answers `None`, whatever the runtime left in the buffer.
/// That is what `reader::refuses-rather-than-defaults` asks of every `Memory`.
pub struct ProcessMemory<'a>(pub &'a Process);

impl Memory for ProcessMemory<'_> {
    #[inline]
    fn read_into(&self, address: u64, buf: &mut [u8]) -> Option<()> {
        let result = self.0.read_into_slice(Address::new(address), buf).ok();
        diagnostics::count_read(buf.len(), result.is_some());
        result
    }
}
