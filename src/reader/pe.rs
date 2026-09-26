//! The extent of a Windows executable, read from its PE header.
//!
//! Windows gives a module the size of its image. Wine does not: it maps the
//! first page of the Flash projector from its file and copies the sections
//! into anonymous memory, so the runtime sees a module of one page. The PE
//! header at its base says how large the image is.

use crate::avm1::Memory;

const DOS_MAGIC: &[u8; 2] = b"MZ";
const PE_MAGIC: &[u8; 4] = b"PE\0\0";
/// Where the DOS header says the NT headers are.
const NT_HEADERS_POINTER: u64 = 0x3c;
/// `SizeOfImage`, in the optional header that follows the file header. It
/// sits at the same place in a 32-bit and a 64-bit image.
const SIZE_OF_IMAGE: u64 = 0x18 + 0x38;
/// The DOS stub is small: the NT headers of a real executable are within
/// the first page.
const MAX_NT_HEADERS: u64 = 0x1000;

/// The executable at `base`, from `base` to the end of its image. Nothing when
/// the bytes at `base` are not a PE header.
///
/// @spec projector::the-module-spans-its-image
pub fn loaded_image(memory: &dyn Memory, base: u64) -> Option<(u64, u64)> {
    let mut dos_magic = [0u8; 2];
    memory.read_into(base, &mut dos_magic)?;
    if &dos_magic != DOS_MAGIC {
        return None;
    }
    let nt_headers = u64::from(u32_at(memory, base + NT_HEADERS_POINTER)?);
    if nt_headers > MAX_NT_HEADERS {
        return None;
    }
    let mut pe_magic = [0u8; 4];
    memory.read_into(base + nt_headers, &mut pe_magic)?;
    if &pe_magic != PE_MAGIC {
        return None;
    }
    let size = u64::from(u32_at(memory, base + nt_headers + SIZE_OF_IMAGE)?);
    (size > 0).then(|| (base, base + size))
}

fn u32_at(memory: &dyn Memory, address: u64) -> Option<u32> {
    let mut bytes = [0u8; 4];
    memory.read_into(address, &mut bytes)?;
    Some(u32::from_le_bytes(bytes))
}
