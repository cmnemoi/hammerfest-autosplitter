//! Reading Ruffle in a browser: the linear memory, and the build that proves
//! it.
//!
//! The rules are in `docs/specs/ruffle-support.md#ruffle-in-a-browser`.

use hammerfest_reader::avm1::Memory;
use hammerfest_reader::linear_memory::LinearMemory;
use hammerfest_reader::ruffle::RuffleBuild;

use crate::memory_contract::contract::{cases, contents, BASE, LEN};
use crate::memory_contract::Heap;

/// Where the linear memory sits in the process: anywhere, as long as every
/// test reads through the offsets.
const LINEAR_BASE: u64 = 0x7f00_0000_0000;

/// A linear memory whose bytes, at the offsets the contract asks about, are
/// the ones of the contract.
fn a_linear_memory_over(process: &Heap) -> LinearMemory<'_> {
    LinearMemory::new(process, LINEAR_BASE, BASE + LEN as u64)
}

mod an_offset_from_the_base {
    use super::*;

    fn a_process() -> Heap {
        Heap::default().with_range(LINEAR_BASE + BASE, contents())
    }

    /** @spec browser::an-offset-from-the-base */
    #[test]
    fn reads_the_bytes_at_an_offset() {
        let process = a_process();
        cases::reads_the_bytes_at_an_address(&a_linear_memory_over(&process));
    }

    #[test]
    fn reads_up_to_the_last_byte_of_a_range() {
        let process = a_process();
        cases::reads_up_to_the_last_byte_of_a_range(&a_linear_memory_over(&process));
    }

    #[test]
    fn refuses_a_read_that_runs_past_the_end_of_a_range() {
        let process = a_process();
        cases::refuses_a_read_that_runs_past_the_end_of_a_range(&a_linear_memory_over(&process));
    }

    #[test]
    fn refuses_an_offset_that_is_in_no_range() {
        let process = a_process();
        cases::refuses_an_address_that_is_in_no_range(&a_linear_memory_over(&process));
    }

    /// The process has bytes after the end of the linear memory: they belong
    /// to something else, and are not read.
    ///
    /** @spec browser.read::past-the-end */
    #[test]
    fn refuses_a_read_past_the_end_of_the_linear_memory() {
        let process = Heap::default().with_range(LINEAR_BASE, vec![0xAA; 0x100]);
        let linear = LinearMemory::new(&process, LINEAR_BASE, 0x80);
        let mut buf = [0u8; 4];

        assert_eq!(linear.read_into(0x7E, &mut buf), None);
        assert_eq!(linear.read_into(0x7C, &mut buf), Some(()));
    }
}

mod a_known_build {
    use super::*;

    /// The size of a linear memory that holds the statics of every build.
    const SIZE: u64 = 0x40_0000;

    /// A linear memory with these vtables at these offsets, and zeros
    /// elsewhere.
    fn a_process_with(vtables: &[(u64, [u32; 4])]) -> Heap {
        let mut bytes = vec![0u8; SIZE as usize];
        for (offset, words) in vtables {
            for (index, word) in words.iter().enumerate() {
                let at = *offset as usize + index * 4;
                bytes[at..at + 4].copy_from_slice(&word.to_le_bytes());
            }
        }
        Heap::default().with_range(LINEAR_BASE, bytes)
    }

    fn recognised(process: &Heap) -> Option<RuffleBuild> {
        RuffleBuild::recognised_in(&LinearMemory::new(process, LINEAR_BASE, SIZE))
    }

    /** @spec browser.find::the-extensions-build */
    #[test]
    fn recognises_the_extensions_build() {
        let process = a_process_with(&[
            (0x2C69A0, [4, 80, 4417, 4418]),
            (0x315F30, [4, 20, 6388, 6389]),
        ]);

        assert_eq!(recognised(&process), Some(RuffleBuild::Extensions));
    }

    /** @spec browser.find::the-mvp-build */
    #[test]
    fn recognises_the_mvp_build() {
        let process = a_process_with(&[
            (0x253890, [4, 80, 2923, 2924]),
            (0x316310, [4, 20, 6373, 6374]),
        ]);

        assert_eq!(recognised(&process), Some(RuffleBuild::Mvp));
    }

    /// The object vtable of the extensions build, and nothing where its
    /// string vtable should be: another build, which moved one of them.
    ///
    /** @spec browser.find::an-unknown-build */
    #[test]
    fn recognises_no_build_when_one_vtable_is_elsewhere() {
        let process = a_process_with(&[(0x2C69A0, [4, 80, 4417, 4418])]);

        assert_eq!(recognised(&process), None);
    }

    /** @spec browser.find::an-unknown-build */
    #[test]
    fn recognises_no_build_in_another_wasm_module() {
        let process = a_process_with(&[]);

        assert_eq!(recognised(&process), None);
    }
}

mod blocks {
    use hammerfest_reader::linear_memory::blocks;

    const MIB: u64 = 1 << 20;

    /** @spec browser.find::blocks-of-a-linear-memory */
    #[test]
    fn cuts_a_linear_memory_into_blocks_of_one_mib() {
        assert_eq!(
            blocks(2 * MIB + MIB / 2),
            [(0, MIB), (MIB, 2 * MIB), (2 * MIB, 2 * MIB + MIB / 2)]
        );
    }

    /** @spec browser.find::blocks-of-a-linear-memory */
    #[test]
    fn keeps_the_blocks_it_had_when_the_memory_grows() {
        let before = blocks(2 * MIB);
        let after = blocks(3 * MIB);

        assert!(before.iter().all(|block| after.contains(block)));
    }

    /** @spec browser.find::an-empty-linear-memory */
    #[test]
    fn cuts_nothing_out_of_nothing() {
        assert_eq!(blocks(0), []);
    }
}
