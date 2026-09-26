//! The four questions every `Memory` must answer, and the bytes they ask
//! about.
//!
//! Two test binaries ask them: the reader's, of its test heap, and the
//! module's, of the adapter that reads the process. Both include this file
//! with `#[path]`, so the questions cannot drift apart.

use hammerfest_reader::avm1::Memory;

// -- the fixture -------------------------------------------------------------

/// Where the range under test starts.
pub const BASE: u64 = 0x1000;
/// How long it is.
pub const LEN: usize = 16;

/// Sixteen bytes, each one different, so that a test can see an address off by
/// one and a buffer filled backwards.
pub fn contents() -> Vec<u8> {
    (0..LEN as u8).map(|i| 0xB0 | i).collect()
}

// -- the four questions ------------------------------------------------------

pub mod cases {
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
