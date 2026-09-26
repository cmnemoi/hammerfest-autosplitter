//! The heap of Pepper Flash, as the Hammerfest strategy asks for it.
//!
//! Each answer is one of the layout's own reads, unchanged: the read cost
//! baseline holds that the seam costs nothing.

use alloc::{vec, vec::Vec};

use hammerfest_core::atom;

use crate::avm1::{read_u64, Layout, Memory, PROFILES, STR_BUF_CANDIDATES};
use crate::heap::{Avm1Heap, Object, ObjectReference, Slot, StringReference, Value};
use crate::scan::{
    give_the_tick_back, scan_bytes, scan_bytes_until, scan_u64_any, u64_at, Scan, CHUNK, OVERLAP,
};

/// A Pepper Flash heap, read with one layout.
#[derive(Copy, Clone, Debug)]
pub struct PepperFlashHeap {
    layout: Layout,
}

impl PepperFlashHeap {
    pub fn new(layout: Layout) -> Self {
        Self { layout }
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// An atom, as a value. Only a float costs a read: it does not fit in its
    /// atom.
    fn decode(&self, memory: &dyn Memory, value: u64) -> Option<Value> {
        let decoded = match value {
            atom::NULL => Value::Null,
            atom::TRUE => Value::Bool(true),
            atom::FALSE => Value::Bool(false),
            _ => match atom::tag(value) {
                atom::TAG_INT => Value::Number(atom::as_int(value)? as f64),
                atom::TAG_DOUBLE => Value::Number(atom::decode_double(read_u64(
                    memory,
                    atom::double_at(value)?,
                )?)),
                atom::TAG_STRING => Value::String(StringReference(atom::ptr(value))),
                atom::TAG_OBJECT => Value::Object(ObjectReference(atom::ptr(value))),
                _ => Value::Other,
            },
        };
        Some(decoded)
    }
}

impl Avm1Heap for PepperFlashHeap {
    fn property(
        &self,
        memory: &dyn Memory,
        object: Object,
        key: &str,
        slot: &mut Slot,
    ) -> Option<Value> {
        let value = self.layout.get_cached(memory, object.0, key, &mut slot.0)?;
        self.decode(memory, value)
    }

    fn object(&self, memory: &dyn Memory, reference: ObjectReference) -> Option<Object> {
        self.layout.table_of(memory, reference.0).map(Object)
    }

    fn object_owning(
        &mut self,
        memory: &dyn Memory,
        reference: ObjectReference,
        key: &str,
    ) -> Option<Object> {
        self.layout
            .derive_so_tbl(memory, reference.0, key)
            .map(Object)
    }

    fn string_is(&self, memory: &dyn Memory, string: StringReference, text: &str) -> bool {
        self.layout.string_eq(memory, string.0, text)
    }

    fn is_object(&self, memory: &dyn Memory, object: Object) -> Option<bool> {
        read_u64(memory, object.0).map(|vtable| vtable == self.layout.tbl_vt)
    }
}

// -- the binary ---------------------------------------------------------------

/// What we keep about the binary itself, from one plugin process to the next.
///
/// The plugin process comes and goes, but the DLL is always the same one: its
/// vtables sit at the same offset in the module, and only the base changes
/// with ASLR. Keeping them lets us find an interned string in **one** pass --
/// scanning the String object headers -- instead of two: find the buffer, then
/// find what points at it.
///
/// This is about the binary, not about the game. Nothing here can go stale
/// between two games, and everything is checked again at use.
#[derive(Copy, Clone, Default)]
pub struct Binary {
    layout: Option<Layout>,
}

/// Layout measured on `pepflashplayer.dll` win32-x64 32.0.0.465, as offsets
/// relative to the module.
///
/// This is not a hard coded address, it is a **seed**. It is checked exactly
/// like a derived value -- the string is read back and compared -- and dropped
/// without noise if the binary differs, in which case the full search takes
/// over.
///
/// What it saves: without it, the first resolution of a session searches the
/// string by its content, then makes one full pass over the heap **per
/// candidate** to find what points at it. With it, one pass over the object
/// headers is enough, from the first game on. The price on an unknown binary
/// is that one pass, lost once.
const MEASURED: Layout = Layout {
    module: (0, 0),
    str_vt: 0x1756db8,
    str_buf: 0x08,
    str_len: 0x30,
    tbl_vt: 0x174a460,
    profile: PROFILES[0],
    so_tbl: 0x30,
};

impl Binary {
    /// A binary whose layout is already proven, as it is after one successful
    /// reading. The search then trusts that layout and never questions it.
    ///
    /// `reader.find::nothing-on-another-flash-build` needs this state: a
    /// reader that has proven one build, and a heap written by another.
    pub fn proven_with(layout: Layout) -> Self {
        Self {
            layout: Some(layout),
        }
    }

    /// A profile already measured, recognised by the PE headers and four
    /// methods. The usual fallback stays active if a single check fails.
    #[cfg(feature = "known-flash")]
    pub fn recognize(&mut self, mem: &dyn Memory, module: (u64, u64)) -> bool {
        let u32_at = |off| {
            let mut b = [0u8; 4];
            mem.read_into(module.0 + off, &mut b)
                .map(|()| u32::from_le_bytes(b))
        };
        let u16_at = |off| {
            let mut b = [0u8; 2];
            mem.read_into(module.0 + off, &mut b)
                .map(|()| u16::from_le_bytes(b))
        };
        if u16_at(0) != Some(0x5a4d)
            || u32_at(0x3c) != Some(0x158)
            || u32_at(0x158) != Some(0x4550)
            || u16_at(0x15c) != Some(0x8664)
            || u32_at(0x160) != Some(0x5fbd874b)
            || u16_at(0x170) != Some(0x20b)
            || u32_at(0x1a8) != Some(0x209e000)
            || u32_at(0x1b0) != Some(0x1f7c652)
        {
            return false;
        }
        for (slot, method) in [
            (MEASURED.str_vt, 0x4391d0),
            (MEASURED.str_vt + 8, 0x374620),
            (MEASURED.tbl_vt, 0x39ec60),
            (MEASURED.tbl_vt + 8, 0x3c2e10),
        ] {
            if read_u64(mem, module.0 + slot) != Some(module.0 + method) {
                return false;
            }
        }
        self.layout = Some(MEASURED);
        true
    }

    /// The layout, rebased on the module of this process.
    pub(crate) fn layout(&self, module: (u64, u64)) -> Option<Layout> {
        let mut l = self.layout.unwrap_or(MEASURED);
        l.str_vt += module.0;
        l.tbl_vt += module.0;
        l.module = module;
        Some(l)
    }

    /// Has the layout already read something in this binary?
    ///
    /// While it has not, we must keep the fallback: the seed may not hold for
    /// this version of the player. Once it has, the fallback can learn nothing
    /// more -- it would only re-read the heap for nothing, and that is exactly
    /// what cost seven hundred MiB per failed attempt while the SWF was
    /// loading.
    pub(crate) fn proven(&self) -> bool {
        self.layout.is_some()
    }

    pub(crate) fn learn(&mut self, layout: &Layout) {
        let mut l = *layout;
        l.str_vt -= layout.module.0;
        l.tbl_vt -= layout.module.0;
        self.layout = Some(l);
    }
}

// -- the search ---------------------------------------------------------------

/// Finds the interned String object of a key, and the String layout with it.
///
/// The cache saves two full scans per attempt: these objects come from the
/// constant pool of the SWF, so they live as long as the plugin. It is checked
/// again by decoding the string, never assumed valid.
pub(crate) async fn find_string(
    mem: &dyn Memory,
    module: (u64, u64),
    ranges: &[(u64, u64)],
    key: &str,
    cache: &mut Option<u64>,
    binary: &Binary,
    cost: &mut Scan<'_>,
) -> Option<(Layout, u64)> {
    let units = key.encode_utf16().count() as u64;
    cost.log
        .string_search(key, binary.proven(), cache.is_some());
    if let Some(so) = *cache {
        if let Some(layout) = string_layout_at(mem, module, so, key, units) {
            return Some((layout, so));
        }
        *cache = None;
    }

    // Known vtable: one pass is enough, over the object headers.
    if let Some(seed) = binary.layout(module) {
        cost.stage("string_seed");
        let mut found = None;
        scan_bytes_until(
            mem,
            ranges,
            &seed.str_vt.to_le_bytes(),
            8,
            cost,
            |so, rest| {
                // The length first, and from the buffer when it fits there. It
                // rejects almost every String object, and the string itself is
                // only read back for the rare survivors.
                let len = u64_at(rest, seed.str_len as usize)
                    .or_else(|| read_u64(mem, so + seed.str_len));
                if len == Some(units) && seed.string_eq(mem, so, key) {
                    found = Some(so);
                    return true;
                }
                false
            },
        )
        .await;
        if let Some(so) = found {
            *cache = Some(so);
            return Some((seed, so));
        }
        if binary.proven() {
            // The vtable is the right one and the string is not there: it
            // does not exist yet. The SWF has not created it, and no other
            // search will make it appear.
            return None;
        }
    }

    // Fallback: the vtable is not known, or the seed does not hold for this
    // binary. Two passes, no more -- the bytes of the key first, then one
    // single pass for all the candidates at once.
    let needle: Vec<u8> = key.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    cost.stage("string_bytes");
    let buffers = scan_bytes(mem, ranges, &needle, 2, 8, cost).await;
    if buffers.is_empty() {
        return None;
    }

    let mut found = None;
    cost.stage("string_references");
    scan_u64_any(mem, ranges, &buffers, cost, |slot| {
        for buf_off in STR_BUF_CANDIDATES {
            let Some(so) = slot.checked_sub(buf_off) else {
                continue;
            };
            if let Some(layout) = string_layout_at(mem, module, so, key, units) {
                found = Some((layout, so));
                return true;
            }
        }
        false
    })
    .await;
    if let Some((_, so)) = found {
        *cache = Some(so);
    }
    found
}

/// Scans the tables that own `key` and returns the first one `accept` keeps.
///
/// The check happens as the scan goes. There are only about ten citations in
/// the whole heap, so collecting them before examining them would force us to
/// always read the hundred MiB, even when the right table is the first one we
/// meet.
pub(crate) async fn scan_tables<T>(
    mem: &dyn Memory,
    ranges: &[(u64, u64)],
    layout: Layout,
    strobj: u64,
    key: &str,
    cost: &mut Scan<'_>,
    mut accept: impl FnMut(Layout, u64) -> Option<T>,
) -> Option<T> {
    let mut layout = layout;
    let mut result = None;
    scan_atoms(mem, ranges, strobj, cost, |slot| {
        for &profile in PROFILES {
            layout.profile = profile;
            layout.tbl_vt = 0;
            layout.so_tbl = 0;
            if !layout.key_is(mem, slot, key) {
                continue;
            }
            let Some(tbl) = layout.table_base(mem, slot) else {
                continue;
            };
            if let Some(found) = accept(layout, tbl) {
                result = Some(found);
                return true;
            }
        }
        false
    })
    .await;
    result
}

/// Derives the String layout from the address of one String object.
///
/// @spec reader::the-right-layout
///
/// Three independent constraints: the leading qword points into the module (it
/// is the vtable), one qword holds the expected length, and the string decoded
/// that way is the one we look for.
fn string_layout_at(
    mem: &dyn Memory,
    module: (u64, u64),
    so: u64,
    key: &str,
    units: u64,
) -> Option<Layout> {
    let vt = read_u64(mem, so)?;
    if vt < module.0 || vt >= module.1 {
        return None;
    }
    for buf_off in STR_BUF_CANDIDATES {
        let mut len_off = 0x08;
        while len_off < 0x80 {
            if read_u64(mem, so + len_off) == Some(units) {
                let layout = Layout {
                    module,
                    str_vt: vt,
                    str_buf: buf_off,
                    str_len: len_off,
                    tbl_vt: 0,
                    profile: PROFILES[0],
                    so_tbl: 0,
                };
                if layout.string_eq(mem, so, key) {
                    return Some(layout);
                }
            }
            len_off += 8;
        }
    }
    None
}

/// Scans the slots that hold an atom pointing at `ptr`, whatever its tag, and
/// calls `on_hit` on each one. Returning `true` stops the scan.
///
/// An atom is `(value << 3) | tag`. The 8 variants differ only in the 3 low
/// bits of the first byte. So we walk the aligned positions, compare the 7
/// high bytes, then the first byte with the tag masked off.
///
/// Hits are delivered as they come, rather than collected. There are only
/// about ten citations in the whole heap, so waiting for the end of the scan
/// to examine them would mean always reading the hundred MiB, even when the
/// right table is the first one we meet.
async fn scan_atoms(
    mem: &dyn Memory,
    ranges: &[(u64, u64)],
    ptr: u64,
    cost: &mut Scan<'_>,
    mut on_hit: impl FnMut(u64) -> bool,
) {
    let bytes = ptr.to_le_bytes();
    let (lo, tail) = (bytes[0], &bytes[1..]);
    let mut buf = vec![0u8; CHUNK];
    let mut chunks = 0usize;

    for &(start, end) in ranges {
        let mut base = start;
        while base < end {
            let n = core::cmp::min(CHUNK as u64, end - base) as usize;
            if cost.read_block(mem, base, &mut buf[..n]) {
                let mut i = 0;
                while i + 8 <= n {
                    if buf[i] & !7 == lo && &buf[i + 1..i + 8] == tail && on_hit(base + i as u64) {
                        return;
                    }
                    i += 8;
                }
            }
            if n <= OVERLAP {
                break;
            }
            base += (n - OVERLAP) as u64;
            chunks += 1;
            if cost.should_yield(chunks) {
                cost.paused();
                give_the_tick_back().await;
            }
        }
    }
}
