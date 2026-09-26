//! The extent of a Linux executable, read from its ELF header.
//!
//! LiveSplit gives a module the sum of the sizes of its mappings. That is its
//! extent only when the mappings follow each other, and the Flash projector
//! leaves a gap of 2 MiB between its code and its data. The ELF header at its
//! base says where each segment is loaded, so it says where the image ends.

use crate::avm1::Memory;

const MAGIC: &[u8; 4] = b"\x7fELF";
const CLASS_64_BITS: u8 = 2;
const LITTLE_ENDIAN: u8 = 1;
/// `ET_EXEC`: loaded at the addresses its segments name.
const FIXED_ADDRESS_EXECUTABLE: u16 = 2;
const PT_LOAD: u32 = 1;
/// A real executable has about a dozen program headers.
const MAX_PROGRAM_HEADERS: u16 = 64;

/// The executable at `base`, from `base` to the end of its last loadable
/// segment. Nothing when the bytes at `base` are not the ELF header of a
/// 64-bit executable loaded at fixed addresses.
///
/// @spec projector::the-module-spans-its-segments
pub fn loaded_image(memory: &dyn Memory, base: u64) -> Option<(u64, u64)> {
    let mut header = [0u8; 0x40];
    memory.read_into(base, &mut header)?;
    let is_fixed_address_executable = &header[..4] == MAGIC
        && header[4] == CLASS_64_BITS
        && header[5] == LITTLE_ENDIAN
        && u16_at(&header, 0x10) == FIXED_ADDRESS_EXECUTABLE;
    if !is_fixed_address_executable {
        return None;
    }
    let program_headers = base.checked_add(u64_at(&header, 0x20))?;
    let program_header_size = u64::from(u16_at(&header, 0x36));
    let program_header_count = u16_at(&header, 0x38);
    if program_header_count > MAX_PROGRAM_HEADERS || program_header_size < 0x38 {
        return None;
    }

    let mut end = None;
    for index in 0..u64::from(program_header_count) {
        let mut program_header = [0u8; 0x38];
        memory.read_into(
            program_headers + index * program_header_size,
            &mut program_header,
        )?;
        if u32_at(&program_header, 0) != PT_LOAD {
            continue;
        }
        let segment_end =
            u64_at(&program_header, 0x10).checked_add(u64_at(&program_header, 0x28))?;
        end = end.max(Some(segment_end));
    }
    end.filter(|&end| end > base).map(|end| (base, end))
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap_or_default())
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap_or_default())
}
