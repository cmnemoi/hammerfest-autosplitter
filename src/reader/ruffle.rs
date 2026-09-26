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
//! The same source runs on the desktop, compiled for x86-64, and in a
//! browser, compiled to wasm32. The logic is the same; the widths and the
//! offsets are not, and a [`RuffleLayout`] holds them. See
//! `docs/concepts/ruffle-heap.md` and `docs/concepts/ruffle-web-heap.md` for
//! every offset, and where it was read.

use alloc::vec::Vec;

use crate::avm1::Memory;
use crate::heap::{Avm1Heap, FlashPlayer, Object, ObjectReference, Slot, StringReference, Value};
use crate::scan::{scan_words, word_at, Scan};

/// Where Ruffle puts what the reader reads, for one target.
#[derive(Copy, Clone, Debug)]
pub struct RuffleLayout {
    pub name: &'static str,
    /// The bytes of a pointer, of a `usize`, and of the hash of a key.
    pub word: usize,
    /// How far in front of a collected value its vtable sits.
    pub gc_vtable: u64,
    /// What the vtable of an AVM1 object says of it.
    pub object_alignment: u64,
    pub object_size: u64,
    /// The capacity of the entries of an object; their pointer and their
    /// length follow, one word each.
    pub entries: u64,
    pub entry_size: usize,
    pub entry_key: usize,
    pub entry_hash: usize,
    /// Where a value holds a string or an object.
    pub value_pointer: usize,
    /// The length of a string, after the pointer to its units.
    pub string_meta: usize,
}

/// Ruffle desktop 0.6.0, x86-64: `docs/concepts/ruffle-heap.md`.
pub const DESKTOP: RuffleLayout = RuffleLayout {
    name: "ruffle-0.6.0",
    word: 8,
    gc_vtable: 0x08,
    object_alignment: 8,
    object_size: 160,
    entries: 0x08,
    entry_size: 56,
    entry_key: 0x28,
    entry_hash: 0x30,
    value_pointer: 0x08,
    string_meta: 0x10,
};

/// Ruffle web 0.6.0, wasm32: `docs/concepts/ruffle-web-heap.md`.
pub const WEB: RuffleLayout = RuffleLayout {
    name: "ruffle-web-0.6.0",
    word: 4,
    gc_vtable: 0x04,
    object_alignment: 4,
    object_size: 80,
    entries: 0x04,
    entry_size: 40,
    entry_key: 0x20,
    entry_hash: 0x24,
    value_pointer: 0x04,
    string_meta: 0x04,
};

impl RuffleLayout {
    /// The hash of `key`, as wide as this target keeps it: a 32-bit target
    /// keeps the low half of the same FNV-1a 64.
    fn hash_of(&self, key: &str) -> u64 {
        key_hash(key) & self.word_mask()
    }

    fn word_mask(&self) -> u64 {
        u64::MAX >> (64 - 8 * self.word)
    }

    fn word(&self, bytes: &[u8], offset: usize) -> Option<u64> {
        word_at(bytes, offset, self.word)
    }

    fn read_word(&self, memory: &dyn Memory, address: u64) -> Option<u64> {
        let mut bytes = [0u8; 8];
        memory.read_into(address, &mut bytes[..self.word])?;
        self.word(&bytes, 0)
    }

    /// Does this entry hold the hash of `key`?
    ///
    /// @spec ruffle::an-entry-answers-by-its-hash
    fn answers_for(&self, entry: &[u8], key: &str) -> bool {
        self.word(entry, self.entry_hash) == Some(self.hash_of(key))
    }
}

/// The collector keeps flags in the low bits of a vtable.
const GC_FLAGS: u64 = 0xF;
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
const VALUE_NUMBER: usize = 0x08;

/// The top bit of the length of a string says its units are UTF-16, and not
/// Latin-1.
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

/// A Ruffle heap, read with one layout and the vtable its AVM1 objects carry.
#[derive(Copy, Clone, Debug)]
pub struct RuffleHeap {
    layout: RuffleLayout,
    object_vtable: u64,
}

impl RuffleHeap {
    /// Where the entries of an object are now, and how many.
    ///
    /// Read again at every call: Ruffle moves them when the map grows.
    ///
    /// @spec ruffle::the-entries-are-found-again
    fn entries(&self, memory: &dyn Memory, object: Object) -> Option<(u64, u64)> {
        let layout = self.layout;
        let mut map = [0u8; 24];
        let map = &mut map[..3 * layout.word];
        memory.read_into(object.0 + layout.entries, map)?;
        let (capacity, pointer, length) = (
            layout.word(map, 0)?,
            layout.word(map, layout.word)?,
            layout.word(map, 2 * layout.word)?,
        );
        (length <= capacity && length <= MAX_ENTRIES).then_some((pointer, length))
    }

    fn decode(&self, entry: &[u8]) -> Option<Value> {
        let pointer = self.layout.word(entry, self.layout.value_pointer)?;
        let value = match *entry.first()? {
            TAG_UNDEFINED => Value::Undefined,
            TAG_NULL => Value::Null,
            TAG_BOOL => Value::Bool(*entry.get(VALUE_BOOL)? != 0),
            TAG_NUMBER => Value::Number(f64::from_bits(word_at(entry, VALUE_NUMBER, 8)?)),
            TAG_STRING => Value::String(StringReference(pointer)),
            TAG_OBJECT => Value::Object(ObjectReference(pointer)),
            TAG_MOVIE_CLIP => Value::Other,
            _ => return None,
        };
        Some(value)
    }
}

impl Avm1Heap for RuffleHeap {
    fn property(
        &self,
        memory: &dyn Memory,
        object: Object,
        key: &str,
        slot: &mut Slot,
    ) -> Option<Value> {
        let layout = self.layout;
        let size = layout.entry_size;
        let (entries, length) = self.entries(memory, object)?;
        let mut entry = [0u8; 64];
        let entry = &mut entry[..size];
        if slot.0 < length
            && memory
                .read_into(entries + slot.0 * size as u64, entry)
                .is_some()
            && layout.answers_for(entry, key)
        {
            return self.decode(entry);
        }

        let mut all = alloc::vec![0u8; length as usize * size];
        memory.read_into(entries, &mut all)?;
        let (index, entry) = all
            .chunks_exact(size)
            .enumerate()
            .find(|(_, entry)| layout.answers_for(entry, key))?;
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
        let layout = self.layout;
        let mut header = [0u8; 24];
        // Up to the word after the length: the same read on every target.
        let header = &mut header[..layout.string_meta + 8];
        if memory.read_into(string.0, header).is_none() {
            return false;
        }
        let (Some(units), Some(meta)) = (
            layout.word(header, 0),
            word_at(header, layout.string_meta, 4),
        ) else {
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
        let tagged = self
            .layout
            .read_word(memory, object.0.checked_sub(self.layout.gc_vtable)?)?;
        Some(tagged & !GC_FLAGS == self.object_vtable)
    }

    fn layout_name(&self) -> &'static str {
        self.layout.name
    }
}

/// Ruffle, as the search sees it.
pub struct Ruffle {
    layout: RuffleLayout,
    /// Where the vtable of an AVM1 object may sit: in the executable on the
    /// desktop, in the static data of the linear memory in a browser.
    statics: (u64, u64),
    /// The vtable of an AVM1 object, once a game proved it, or once the
    /// build is known. It moves with the module, so it lives as long as the
    /// process.
    object_vtable: Option<u64>,
}

impl Default for Ruffle {
    fn default() -> Self {
        Self {
            layout: DESKTOP,
            statics: (0, 0),
            object_vtable: None,
        }
    }
}

impl Ruffle {
    /// Ruffle desktop, whose executable sits at `module`.
    pub fn attached_to(module: (u64, u64)) -> Self {
        let mut player = Self::default();
        player.attach(module);
        player
    }

    /// A Ruffle desktop process to search, whose executable sits at `module`.
    pub fn attach(&mut self, module: (u64, u64)) {
        self.layout = DESKTOP;
        self.statics = module;
        self.object_vtable = None;
    }

    /// The vtable of the object at `object`, when it is the vtable of an AVM1
    /// object.
    ///
    /// @spec ruffle::an-object-is-proven-by-its-type
    fn object_vtable_of(&self, memory: &dyn Memory, object: u64) -> Option<u64> {
        let layout = self.layout;
        let vtable = layout.read_word(memory, object.checked_sub(layout.gc_vtable)?)? & !GC_FLAGS;
        if let Some(known) = self.object_vtable {
            return (vtable == known).then_some(vtable);
        }
        if !(self.statics.0..self.statics.1).contains(&vtable) {
            return None;
        }
        let mut described = [0u8; 16];
        let described = &mut described[..2 * layout.word];
        memory.read_into(vtable, described)?;
        (layout.word(described, 0)? == layout.object_alignment
            && layout.word(described, layout.word)? == layout.object_size)
            .then_some(vtable)
    }

    /// Is this an entry, of this key?
    fn is_entry_of(&self, memory: &dyn Memory, entry: u64, key: &str) -> bool {
        let layout = self.layout;
        let mut bytes = [0u8; 64];
        let bytes = &mut bytes[..layout.entry_size];
        if memory.read_into(entry, bytes).is_none() || bytes[0] > TAG_MOVIE_CLIP {
            return false;
        }
        let Some(key_string) = layout.word(bytes, layout.entry_key) else {
            return false;
        };
        let heap = RuffleHeap {
            layout,
            object_vtable: 0,
        };
        layout.answers_for(bytes, key) && heap.string_is(memory, StringReference(key_string), key)
    }
}

impl FlashPlayer for Ruffle {
    type Heap = RuffleHeap;

    /// Two sweeps. The first finds the entries of `key` by their hash, in the
    /// memory that changed. The second finds the maps that hold them, in all
    /// the memory: a word that points at the start of the entries, followed
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
        let layout = self.layout;
        let hash = layout.hash_of(key);
        let entry_size = layout.entry_size as u64;

        cost.stage("key_entries");
        let mut entries: Vec<u64> = Vec::new();
        scan_words(memory, fresh, layout.word, cost, |address, value, _| {
            let entry = address.wrapping_sub(layout.entry_hash as u64);
            // Two blocks overlap, so a hit can come twice.
            if value == hash && !entries.contains(&entry) && self.is_entry_of(memory, entry, key) {
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
        let pointer_offset = layout.entries + layout.word as u64;
        scan_words(
            memory,
            all,
            layout.word,
            cost,
            |address, pointer, length| {
                let Some(length) = length else {
                    return false;
                };
                let reach = length.saturating_mul(entry_size);
                if pointer > highest
                    || pointer.saturating_add(reach) <= lowest
                    || length > MAX_ENTRIES
                {
                    return false;
                }
                let holds_an_entry = entries.iter().any(|&entry| {
                    entry >= pointer
                        && (entry - pointer) % entry_size == 0
                        && entry - pointer < reach
                });
                let object = address.wrapping_sub(pointer_offset);
                if !holds_an_entry || tried.contains(&object) {
                    return false;
                }
                tried.push(object);
                let Some(object_vtable) = self.object_vtable_of(memory, object) else {
                    return false;
                };
                found = accept(
                    RuffleHeap {
                        layout,
                        object_vtable,
                    },
                    Object(object),
                );
                found.is_some()
            },
        )
        .await;
        found
    }

    fn learn(&mut self, heap: &RuffleHeap) {
        self.object_vtable = Some(heap.object_vtable);
    }
}
