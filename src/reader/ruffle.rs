//! The heap of Ruffle 0.6.0, as the Hammerfest strategy asks for it.
//!
//! Ruffle keeps an AVM1 object as a map of entries. Each entry holds a value,
//! a pointer to its key, and the hash of that key. The hash can be computed
//! beforehand from the obfuscated name, so the search starts from it:
//!
//! ```text
//! sweep the hash of the key     each hit is an entry     check: its key
//!   -> the entries of a map     sweep the pointer to them, with the length
//!                               beside it                check: the object's type
//!   -> the object               the first one the strategy keeps
//! ```
//!
//! See `docs/concepts/ruffle-heap.md` for every offset, and where it was read.

use alloc::vec::Vec;

use crate::avm1::{read_u64, Memory};
use crate::heap::{Avm1Heap, FlashPlayer, Object, ObjectReference, Slot, StringReference, Value};
use crate::scan::{scan_qwords, scan_u64_any, u64_at, Scan};

const LAYOUT_NAME: &str = "ruffle-0.6.0";

/// A collected value is preceded by the vtable of its type. Its low bits are
/// flags of the collector.
const GC_VTABLE: u64 = 0x08;
const GC_FLAGS: u64 = 0xF;
/// What the vtable of an AVM1 object says of it: its alignment, its size.
const OBJECT_ALIGNMENT: u64 = 8;
const OBJECT_SIZE: u64 = 160;

/// The map of an object: the capacity, the pointer and the length of its
/// entries, one after the other.
const ENTRIES: u64 = 0x08;
const ENTRIES_POINTER: u64 = 0x10;
const ENTRY_SIZE: u64 = 56;
const ENTRY_KEY: usize = 0x28;
const ENTRY_HASH: usize = 0x30;
/// More entries than any Hammerfest object holds. A larger length is not a
/// map.
const MAX_ENTRIES: u64 = 4096;

const TAG_UNDEFINED: u8 = 0;
const TAG_NULL: u8 = 1;
const TAG_BOOL: u8 = 2;
const TAG_NUMBER: u8 = 3;
const TAG_STRING: u8 = 4;
const TAG_OBJECT: u8 = 5;
const TAG_MOVIE_CLIP: u8 = 6;
const VALUE_BOOL: usize = 0x01;
const VALUE_PAYLOAD: usize = 0x08;

/// A string: the pointer to its units, then its length, whose top bit says
/// the units are UTF-16 and not Latin-1.
const STRING_META: usize = 0x10;
const WIDE: u32 = 1 << 31;
/// Longer than any key or world name.
const MAX_STRING: usize = 64;

/// The hash Ruffle keeps beside a key.
///
/// FNV-1a 64 over each unit, lowered and written as a u16, then one byte
/// 0xff: `property_map.rs:198-201` in Ruffle.
pub fn key_hash(key: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let units = key
        .encode_utf16()
        .flat_map(|unit| lowered(unit).to_le_bytes());
    for byte in units.chain([0xff]) {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// `swf_to_lowercase`, for the keys the reader asks for: they are ASCII.
fn lowered(unit: u16) -> u16 {
    if (b'A' as u16..=b'Z' as u16).contains(&unit) {
        unit + (b'a' - b'A') as u16
    } else {
        unit
    }
}

/// A Ruffle heap, read with the vtable its AVM1 objects carry.
#[derive(Copy, Clone, Debug)]
pub struct RuffleHeap {
    object_vtable: u64,
}

impl RuffleHeap {
    /// Where the entries of an object are now, and how many.
    ///
    /// Read again at every call: Ruffle moves them when the map grows.
    ///
    /// @spec ruffle::the-entries-are-found-again
    fn entries(&self, memory: &dyn Memory, object: Object) -> Option<(u64, u64)> {
        let mut map = [0u8; 24];
        memory.read_into(object.0 + ENTRIES, &mut map)?;
        let (capacity, pointer, length) = (u64_at(&map, 0)?, u64_at(&map, 8)?, u64_at(&map, 16)?);
        (length <= capacity && length <= MAX_ENTRIES).then_some((pointer, length))
    }

    fn decode(&self, entry: &[u8]) -> Option<Value> {
        let payload = u64_at(entry, VALUE_PAYLOAD)?;
        let value = match *entry.first()? {
            TAG_UNDEFINED => Value::Undefined,
            TAG_NULL => Value::Null,
            TAG_BOOL => Value::Bool(*entry.get(VALUE_BOOL)? != 0),
            TAG_NUMBER => Value::Number(f64::from_bits(payload)),
            TAG_STRING => Value::String(StringReference(payload)),
            TAG_OBJECT => Value::Object(ObjectReference(payload)),
            TAG_MOVIE_CLIP => Value::Other,
            _ => return None,
        };
        Some(value)
    }
}

/// Does this entry hold the hash of `key`?
///
/// @spec ruffle::an-entry-answers-by-its-hash
fn answers_for(entry: &[u8], key: &str) -> bool {
    u64_at(entry, ENTRY_HASH) == Some(key_hash(key))
}

impl Avm1Heap for RuffleHeap {
    fn property(
        &self,
        memory: &dyn Memory,
        object: Object,
        key: &str,
        slot: &mut Slot,
    ) -> Option<Value> {
        let (entries, length) = self.entries(memory, object)?;
        let mut entry = [0u8; ENTRY_SIZE as usize];
        if slot.0 < length
            && memory
                .read_into(entries + slot.0 * ENTRY_SIZE, &mut entry)
                .is_some()
            && answers_for(&entry, key)
        {
            return self.decode(&entry);
        }

        let mut all = alloc::vec![0u8; (length * ENTRY_SIZE) as usize];
        memory.read_into(entries, &mut all)?;
        let (entries_read, _) = all.as_chunks::<{ ENTRY_SIZE as usize }>();
        let (index, entry) = entries_read
            .iter()
            .enumerate()
            .find(|(_, entry)| answers_for(*entry, key))?;
        *slot = Slot(index as u64);
        self.decode(entry)
    }

    fn object(&self, memory: &dyn Memory, reference: ObjectReference) -> Option<Object> {
        let object = Object(reference.0);
        self.is_object(memory, object)?.then_some(object)
    }

    fn object_owning(
        &mut self,
        memory: &dyn Memory,
        reference: ObjectReference,
        key: &str,
    ) -> Option<Object> {
        let object = self.object(memory, reference)?;
        self.property(memory, object, key, &mut Slot::default())?;
        Some(object)
    }

    fn string_is(&self, memory: &dyn Memory, string: StringReference, text: &str) -> bool {
        let mut header = [0u8; 24];
        if memory.read_into(string.0, &mut header).is_none() {
            return false;
        }
        let (Some(units), Some(meta)) = (u64_at(&header, 0), u64_at(&header, STRING_META)) else {
            return false;
        };
        let meta = meta as u32;
        let length = (meta & !WIDE) as usize;
        let mut bytes = [0u8; MAX_STRING * 2];
        if meta & WIDE == 0 {
            let latin1 = &mut bytes[..length.min(MAX_STRING)];
            length <= MAX_STRING
                && memory.read_into(units, latin1).is_some()
                && text
                    .chars()
                    .map(|character| character as u32)
                    .eq(latin1.iter().map(|&byte| byte as u32))
        } else {
            let utf16 = &mut bytes[..(length * 2).min(MAX_STRING * 2)];
            length <= MAX_STRING
                && memory.read_into(units, utf16).is_some()
                && text.encode_utf16().eq(utf16
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_le_bytes(*pair)))
        }
    }

    /// @spec ruffle::an-object-is-proven-by-its-type
    fn is_object(&self, memory: &dyn Memory, object: Object) -> Option<bool> {
        let tagged = read_u64(memory, object.0.checked_sub(GC_VTABLE)?)?;
        Some(tagged & !GC_FLAGS == self.object_vtable)
    }

    fn layout_name(&self) -> &'static str {
        LAYOUT_NAME
    }
}

/// Ruffle, as the search sees it.
#[derive(Default)]
pub struct Ruffle {
    module: (u64, u64),
    /// The vtable of an AVM1 object, once a game proved it. ASLR moves it
    /// with the module, so it lives as long as the process.
    object_vtable: Option<u64>,
}

impl Ruffle {
    pub fn attached_to(module: (u64, u64)) -> Self {
        let mut player = Self::default();
        player.attach(module);
        player
    }

    /// A Ruffle process to search, whose executable sits at `module`.
    pub fn attach(&mut self, module: (u64, u64)) {
        self.module = module;
        self.object_vtable = None;
    }

    /// The vtable of the object at `object`, when it is the vtable of an AVM1
    /// object.
    ///
    /// @spec ruffle::an-object-is-proven-by-its-type
    fn object_vtable_of(&self, memory: &dyn Memory, object: u64) -> Option<u64> {
        let vtable = read_u64(memory, object.checked_sub(GC_VTABLE)?)? & !GC_FLAGS;
        if let Some(known) = self.object_vtable {
            return (vtable == known).then_some(vtable);
        }
        if !(self.module.0..self.module.1).contains(&vtable) {
            return None;
        }
        let mut layout = [0u8; 16];
        memory.read_into(vtable, &mut layout)?;
        (u64_at(&layout, 0)? == OBJECT_ALIGNMENT && u64_at(&layout, 8)? == OBJECT_SIZE)
            .then_some(vtable)
    }

    /// Is this an entry, of this key?
    fn is_entry_of(&self, memory: &dyn Memory, entry: u64, key: &str) -> bool {
        let mut bytes = [0u8; ENTRY_SIZE as usize];
        if memory.read_into(entry, &mut bytes).is_none() || bytes[0] > TAG_MOVIE_CLIP {
            return false;
        }
        let Some(key_string) = u64_at(&bytes, ENTRY_KEY) else {
            return false;
        };
        let heap = RuffleHeap { object_vtable: 0 };
        answers_for(&bytes, key) && heap.string_is(memory, StringReference(key_string), key)
    }
}

impl FlashPlayer for Ruffle {
    type Heap = RuffleHeap;

    /// Two sweeps. The first finds the entries of `key` by their hash, in the
    /// memory that changed. The second finds the maps that hold them, in all
    /// the memory: a qword that points at the start of the entries, followed
    /// by a length that reaches the entry.
    async fn objects_owning<T>(
        &mut self,
        memory: &dyn Memory,
        key: &str,
        fresh: &[(u64, u64)],
        all: &mut [(u64, u64)],
        cost: &mut Scan<'_>,
        mut accept: impl FnMut(RuffleHeap, Object) -> Option<T>,
    ) -> Option<T> {
        cost.stage("key_entries");
        let mut entries: Vec<u64> = Vec::new();
        scan_u64_any(memory, fresh, &[key_hash(key)], cost, |hash| {
            let entry = hash - ENTRY_HASH as u64;
            // Two blocks overlap, so a hit can come twice.
            if !entries.contains(&entry) && self.is_entry_of(memory, entry, key) {
                entries.push(entry);
            }
            false
        })
        .await;
        let (Some(&lowest), Some(&highest)) = (entries.iter().min(), entries.iter().max()) else {
            return None;
        };

        cost.stage("key_owners");
        let mut tried: Vec<u64> = Vec::new();
        let mut found = None;
        scan_qwords(memory, all, cost, |address, pointer, length| {
            let Some(length) = length else {
                return false;
            };
            let reach = length.saturating_mul(ENTRY_SIZE);
            if pointer > highest || pointer.saturating_add(reach) <= lowest || length > MAX_ENTRIES
            {
                return false;
            }
            let holds_an_entry = entries.iter().any(|&entry| {
                entry >= pointer && (entry - pointer) % ENTRY_SIZE == 0 && entry - pointer < reach
            });
            let object = address - ENTRIES_POINTER;
            if !holds_an_entry || tried.contains(&object) {
                return false;
            }
            tried.push(object);
            let Some(object_vtable) = self.object_vtable_of(memory, object) else {
                return false;
            };
            found = accept(RuffleHeap { object_vtable }, Object(object));
            found.is_some()
        })
        .await;
        found
    }

    fn learn(&mut self, heap: &RuffleHeap) {
        self.object_vtable = Some(heap.object_vtable);
    }
}
