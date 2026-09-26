//! The process, as the reader asks for it.

#![no_std]

use asr::{Address, Process};
use hammerfest_reader::avm1::Memory;

/// The memory of the plugin process.
///
/// A read that fails answers `None`, whatever the runtime left in the buffer.
/// That is what `reader::refuses-rather-than-defaults` asks of every `Memory`.
pub struct ProcessMemory<'a>(pub &'a Process);

impl Memory for ProcessMemory<'_> {
    #[inline]
    fn read_into(&self, address: u64, buf: &mut [u8]) -> Option<()> {
        self.0.read_into_slice(Address::new(address), buf).ok()
    }
}
