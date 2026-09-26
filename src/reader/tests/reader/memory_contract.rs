//! The contract of [`hammerfest_reader::avm1::Memory`], and the test heap
//! held to it.
//!
//! The reader's own tests run against `Heap`. If `Heap` and the production
//! adapter answer differently, those tests prove nothing about production. So
//! the four questions below say what any `Memory` must answer.
//!
//! Only the heap is asked here. The production adapter, `ProcessMemory` in
//! the module, lives beside the runtime, which no test crosses. See
//! `docs/internals/what-the-net-does-not-hold.md`.

use alloc::vec::Vec;

use hammerfest_reader::avm1::Memory;

/// The bytes a test serves the reader.
#[derive(Default)]
pub struct Heap {
    /// Base address and contents of each range.
    pub regions: Vec<(u64, Vec<u8>)>,
}

impl Heap {
    pub fn with_range(mut self, base: u64, bytes: Vec<u8>) -> Self {
        self.regions.push((base, bytes));
        self
    }
}

impl Memory for Heap {
    fn read_into(&self, address: u64, buf: &mut [u8]) -> Option<()> {
        let (base, bytes) = self
            .regions
            .iter()
            .find(|(base, bytes)| address >= *base && address - *base < bytes.len() as u64)?;
        let offset = (address - base) as usize;
        let source = bytes.get(offset..offset.checked_add(buf.len())?)?;
        buf.copy_from_slice(source);
        Some(())
    }
}

// -- the fixture -------------------------------------------------------------

/// Where the range under test starts.
const BASE: u64 = 0x1000;
/// How long it is.
const LEN: usize = 16;

/// Sixteen bytes, each one different, so that a test can see an address off by
/// one and a buffer filled backwards.
fn contents() -> Vec<u8> {
    (0..LEN as u8).map(|i| 0xB0 | i).collect()
}

fn a_heap() -> Heap {
    Heap::default().with_range(BASE, contents())
}

// -- the four questions ------------------------------------------------------

mod cases {
    use super::*;

    /// Simple, and one: the bytes asked for, in the order they lie in memory.
    pub fn reads_the_bytes_at_an_address(mem: &dyn Memory) {
        let mut buf = [0u8; 4];

        assert_eq!(mem.read_into(BASE + 2, &mut buf), Some(()));

        assert_eq!(buf, [0xB2, 0xB3, 0xB4, 0xB5]);
    }

    /// Boundary: a read that ends on the last byte of the range is inside it.
    pub fn reads_up_to_the_last_byte_of_a_range(mem: &dyn Memory) {
        let mut buf = [0u8; 4];

        assert_eq!(mem.read_into(BASE + 12, &mut buf), Some(()));

        assert_eq!(buf, [0xBC, 0xBD, 0xBE, 0xBF]);
    }

    /// Boundary: one byte further is outside it, and the whole read fails.
    ///
    /// This is the rule the reader depends on. A read that returned the part
    /// which fitted would hand `read_u64` four true bytes and four invented
    /// ones, and the level would be wrong without a word.
    pub fn refuses_a_read_that_runs_past_the_end_of_a_range(mem: &dyn Memory) {
        let mut buf = [0u8; 4];

        assert_eq!(mem.read_into(BASE + 13, &mut buf), None);
    }

    /// Zero, from the other side: an address that no range holds.
    pub fn refuses_an_address_that_is_in_no_range(mem: &dyn Memory) {
        let mut buf = [0u8; 1];

        assert_eq!(mem.read_into(BASE - 1, &mut buf), None);
        assert_eq!(mem.read_into(BASE + LEN as u64, &mut buf), None);
        assert_eq!(mem.read_into(0, &mut buf), None);
    }
}

// -- the implementations -----------------------------------------------------

mod the_test_heap {
    use super::*;

    #[test]
    fn reads_the_bytes_at_an_address() {
        cases::reads_the_bytes_at_an_address(&a_heap());
    }

    #[test]
    fn reads_up_to_the_last_byte_of_a_range() {
        cases::reads_up_to_the_last_byte_of_a_range(&a_heap());
    }

    #[test]
    fn refuses_a_read_that_runs_past_the_end_of_a_range() {
        cases::refuses_a_read_that_runs_past_the_end_of_a_range(&a_heap());
    }

    #[test]
    fn refuses_an_address_that_is_in_no_range() {
        cases::refuses_an_address_that_is_in_no_range(&a_heap());
    }

    /// The heap refuses a read that crosses out of one range and into the
    /// next. The adapter forwards such a read, and the runtime decides.
    ///
    /// So the contract stays silent on it, and the heap is the stricter of the
    /// two. Stricter is the safe direction: a reader that needed a straddling
    /// read would fail in a test and work in production, and never the other
    /// way round.
    #[test]
    fn refuses_a_read_that_crosses_from_one_range_into_the_next() {
        let heap = a_heap().with_range(BASE + LEN as u64, contents());
        let mut buf = [0u8; 4];

        assert_eq!(heap.read_into(BASE + 14, &mut buf), None);
    }
}
