//! The extent of an executable, read from its ELF or its PE header.
//!
//! The rule is in `docs/specs/flash-projector-support.md`.

use alloc::vec::Vec;

use hammerfest_reader::{elf, pe};

use crate::memory_contract::Heap;

/// Where a Linux executable that is not position independent sits.
const BASE: u64 = 0x40_0000;

/// An ELF header and its program headers, as the loader leaves them at the
/// base of an executable.
struct ElfHeader {
    file_type: u16,
    segments: Vec<Segment>,
}

/// A loadable segment, at its address in memory.
struct Segment {
    address: u64,
    size_in_memory: u64,
}

impl ElfHeader {
    /// Bytes written with the offsets of the ELF specification, never with
    /// the reader's.
    fn bytes(&self) -> Vec<u8> {
        const HEADER_SIZE: usize = 0x40;
        const PROGRAM_HEADER_SIZE: usize = 0x38;
        const PT_LOAD: u32 = 1;
        const PT_NOTE: u32 = 4;

        let mut bytes = alloc::vec![0u8; HEADER_SIZE];
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2; // 64 bits
        bytes[5] = 1; // little endian
        bytes[0x10..0x12].copy_from_slice(&self.file_type.to_le_bytes());
        bytes[0x20..0x28].copy_from_slice(&(HEADER_SIZE as u64).to_le_bytes());
        bytes[0x36..0x38].copy_from_slice(&(PROGRAM_HEADER_SIZE as u16).to_le_bytes());
        // A segment that is not loaded comes first, far past the others: only
        // the loadable ones say where the image ends.
        let note = program_header(PT_NOTE, 0x7fff_0000, 0x100);
        let loads = self
            .segments
            .iter()
            .map(|segment| program_header(PT_LOAD, segment.address, segment.size_in_memory));
        let headers: Vec<[u8; PROGRAM_HEADER_SIZE]> = core::iter::once(note).chain(loads).collect();
        bytes[0x38..0x3a].copy_from_slice(&(headers.len() as u16).to_le_bytes());
        for header in headers {
            bytes.extend_from_slice(&header);
        }
        return bytes;

        fn program_header(
            kind: u32,
            address: u64,
            size_in_memory: u64,
        ) -> [u8; PROGRAM_HEADER_SIZE] {
            let mut header = [0u8; PROGRAM_HEADER_SIZE];
            header[..4].copy_from_slice(&kind.to_le_bytes());
            header[0x10..0x18].copy_from_slice(&address.to_le_bytes());
            header[0x28..0x30].copy_from_slice(&size_in_memory.to_le_bytes());
            header
        }
    }
}

const EXECUTABLE: u16 = 2;
const SHARED_OBJECT: u16 = 3;

/// The projector's shape: code, then a gap, then data.
fn an_executable_with_a_gap(file_type: u16) -> Heap {
    let header = ElfHeader {
        file_type,
        segments: alloc::vec![
            Segment {
                address: BASE,
                size_in_memory: 0xd8_7000,
            },
            Segment {
                address: 0x138_72e0,
                size_in_memory: 0xf_4d20,
            },
        ],
    };
    Heap::default().with_range(BASE, header.bytes())
}

/** @spec projector.find::segments-with-a-gap */
#[test]
fn the_module_runs_to_the_end_of_the_last_segment() {
    let process = an_executable_with_a_gap(EXECUTABLE);

    assert_eq!(elf::loaded_image(&process, BASE), Some((BASE, 0x147_c000)));
}

/** @spec projector.find::not-an-executable */
#[test]
fn bytes_that_are_not_an_elf_header_give_no_module() {
    let process = Heap::default().with_range(BASE, alloc::vec![0x90; 0x200]);

    assert_eq!(elf::loaded_image(&process, BASE), None);
}

/** @spec projector.find::a-position-independent-executable */
#[test]
fn a_position_independent_executable_gives_no_module() {
    let process = an_executable_with_a_gap(SHARED_OBJECT);

    assert_eq!(elf::loaded_image(&process, BASE), None);
}

// -- a Windows executable ----------------------------------------------------

/// A PE header, as the loader leaves it at the base of an executable, with the
/// offsets of the PE specification.
fn a_pe_image_of(size_of_image: u32) -> Heap {
    const NT_HEADERS: usize = 0x80;
    const SIZE_OF_IMAGE: usize = NT_HEADERS + 0x18 + 0x38;
    let mut bytes = alloc::vec![0u8; 0x200];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&(NT_HEADERS as u32).to_le_bytes());
    bytes[NT_HEADERS..NT_HEADERS + 4].copy_from_slice(b"PE\0\0");
    bytes[SIZE_OF_IMAGE..SIZE_OF_IMAGE + 4].copy_from_slice(&size_of_image.to_le_bytes());
    Heap::default().with_range(BASE, bytes)
}

/** @spec projector.find::a-pe-image */
#[test]
fn the_module_runs_to_the_end_of_its_image() {
    let process = a_pe_image_of(0x103_4000);

    assert_eq!(pe::loaded_image(&process, BASE), Some((BASE, 0x143_4000)));
}

/** @spec projector.find::not-a-pe-image */
#[test]
fn bytes_that_are_not_a_pe_header_give_no_module() {
    let process = an_executable_with_a_gap(EXECUTABLE);

    assert_eq!(pe::loaded_image(&process, BASE), None);
}
