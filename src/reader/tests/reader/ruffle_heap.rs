//! The Ruffle driver: a world, written byte by byte as Ruffle 0.6.0 would
//! write it.
//!
//! The offsets below are this module's own, taken from
//! `docs/concepts/ruffle-heap.md`. They are not the reader's. A driver that
//! shared its offsets with the code under test could not see an error in
//! them. The replay of `fixtures/replay/ruffle-main-world` is what checks the
//! reader's offsets against bytes Ruffle really wrote.

use alloc::{string::String, vec, vec::Vec};
use core::marker::PhantomData;

use hammerfest_reader::keys;
use hammerfest_reader::ruffle::{Ruffle, RuffleBuild};

use crate::memory_contract::Heap;
use crate::scenarios::{World, WrittenHeap};

/// Where nothing is mapped at all.
const NOWHERE: u64 = 0x2000_0000;

/// Where one target of Ruffle puts what the driver writes. The driver's own
/// table: `docs/concepts/ruffle-heap.md` and `ruffle-web-heap.md`.
#[derive(Copy, Clone)]
pub struct DriverLayout {
    /// Where the heap starts: an address on the desktop, an offset in the
    /// linear memory in a browser.
    heap_base: u64,
    /// The bytes of a pointer, of a length, and of a stored hash.
    word: usize,
    /// A collected value is preceded by `next`, then its tagged vtable.
    gc_header: u64,
    gc_vtable: u64,
    object_vtable: u64,
    object_size: u64,
    vec_vtable: u64,
    string_vtable: u64,
    string_size: u64,
    /// Object: the borrow flag, then the map of its properties.
    entries_capacity: u64,
    entries_pointer: u64,
    entries_length: u64,
    /// Entry: the value, then the key and its hash.
    entry_size: u64,
    entry_key: u64,
    entry_hash: u64,
    /// Value: a tag, then the boolean, the pointer or the number.
    value_pointer: u64,
    /// String: the pointer to its units, then its length.
    string_length: u64,
}

/// The low bits of a vtable are flags of the collector.
const GC_FLAGS: u64 = 0b1010;
const TAG_BOOL: u8 = 2;
const TAG_NUMBER: u8 = 3;
const TAG_STRING: u8 = 4;
const TAG_OBJECT: u8 = 5;
const VALUE_BOOL: u64 = 0x01;
const VALUE_NUMBER: u64 = 0x08;

/// A target Ruffle is compiled for, and the player that reads it.
pub trait Target {
    const LAYOUT: DriverLayout;

    fn player() -> Ruffle;

    /// The memory, as the reader asks for it: the heap, and the statics.
    fn memory(heap: Vec<u8>) -> Heap;
}

/// Ruffle desktop 0.6.0, x86-64.
pub struct Desktop;

/// Where the fake module sits, and the vtables in it.
const MODULE: (u64, u64) = (0x4000_0000, 0x4010_0000);

impl Target for Desktop {
    const LAYOUT: DriverLayout = DriverLayout {
        heap_base: 0x1000_0000,
        word: 8,
        gc_header: 0x10,
        gc_vtable: 0x08,
        object_vtable: MODULE.0 + 0x1120,
        object_size: 160,
        vec_vtable: MODULE.0 + 0x1140,
        string_vtable: MODULE.0 + 0x1300,
        string_size: 32,
        entries_capacity: 0x08,
        entries_pointer: 0x10,
        entries_length: 0x18,
        entry_size: 56,
        entry_key: 0x28,
        entry_hash: 0x30,
        value_pointer: 0x08,
        string_length: 0x10,
    };

    fn player() -> Ruffle {
        Ruffle::attached_to(MODULE)
    }

    /// The module holds the vtables of the collected types: their alignment,
    /// then their size.
    fn memory(heap: Vec<u8>) -> Heap {
        let layout = Self::LAYOUT;
        let mut module = vec![0u8; 0x2000];
        for (vtable, size) in [
            (layout.object_vtable, layout.object_size),
            (layout.vec_vtable, 24),
            (layout.string_vtable, layout.string_size),
        ] {
            let at = (vtable - MODULE.0) as usize;
            module[at..at + 8].copy_from_slice(&8u64.to_le_bytes());
            module[at + 8..at + 16].copy_from_slice(&size.to_le_bytes());
        }
        Heap::default()
            .with_range(layout.heap_base, heap)
            .with_range(MODULE.0, module)
    }
}

/// Ruffle web 0.6.0, wasm32, the extensions build. Every address is an
/// offset in the linear memory.
pub struct Browser;

impl Target for Browser {
    const LAYOUT: DriverLayout = DriverLayout {
        // Above the stack and the statics, where dlmalloc starts.
        heap_base: 0x49_0000,
        word: 4,
        gc_header: 0x08,
        gc_vtable: 0x04,
        object_vtable: 0x2C_69A0,
        object_size: 80,
        vec_vtable: 0x2D_C490,
        string_vtable: 0x31_5F30,
        string_size: 20,
        entries_capacity: 0x04,
        entries_pointer: 0x08,
        entries_length: 0x0C,
        entry_size: 40,
        entry_key: 0x20,
        entry_hash: 0x24,
        value_pointer: 0x04,
        string_length: 0x04,
    };

    fn player() -> Ruffle {
        Ruffle::in_browser(RuffleBuild::Extensions)
    }

    /// The build names the vtable of an object: no static is read.
    fn memory(heap: Vec<u8>) -> Heap {
        Heap::default().with_range(Self::LAYOUT.heap_base, heap)
    }
}

/// A value, as the driver writes it in an entry.
#[derive(Copy, Clone)]
enum Value {
    Bool(bool),
    Number(f64),
    String(u64),
    Object(u64),
}

/// The FNV-1a hash Ruffle keeps beside a key: each unit, lowered, as a u16,
/// then one byte 0xff.
fn key_hash(key: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let units = key.encode_utf16().flat_map(|unit| {
        let lowered = if (b'A' as u16..=b'Z' as u16).contains(&unit) {
            unit + 32
        } else {
            unit
        };
        lowered.to_le_bytes()
    });
    for byte in units.chain([0xff]) {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

// -- writing the bytes -------------------------------------------------------

/// A bump allocator over one region, and the strings already interned.
struct Bytes {
    layout: DriverLayout,
    data: Vec<u8>,
    interned: Vec<(String, u64)>,
}

/// An object the driver wrote, and the properties it holds, in order.
#[derive(Clone, Default)]
struct Obj {
    address: u64,
    properties: Vec<(String, Value)>,
}

impl Bytes {
    fn new(layout: DriverLayout) -> Self {
        Self {
            layout,
            // Address zero means "no address", so nothing starts there.
            data: vec![0u8; 16],
            interned: Vec::new(),
        }
    }

    fn alloc(&mut self, len: u64) -> u64 {
        let address = self.layout.heap_base + self.data.len() as u64;
        self.data
            .resize(self.data.len() + (len as usize).next_multiple_of(16), 0);
        address
    }

    /// A word of the target: a pointer, a length, a hash.
    fn put(&mut self, address: u64, value: u64) {
        let width = self.layout.word;
        self.put_bytes(address, &value.to_le_bytes()[..width]);
    }

    fn put_number(&mut self, address: u64, value: f64) {
        self.put_bytes(address, &value.to_bits().to_le_bytes());
    }

    fn put_bytes(&mut self, address: u64, bytes: &[u8]) {
        let at = (address - self.layout.heap_base) as usize;
        self.data[at..at + bytes.len()].copy_from_slice(bytes);
    }

    fn put_byte(&mut self, address: u64, value: u8) {
        self.put_bytes(address, &[value]);
    }

    fn word(&self, address: u64) -> u64 {
        let at = (address - self.layout.heap_base) as usize;
        let mut bytes = [0u8; 8];
        bytes[..self.layout.word].copy_from_slice(&self.data[at..at + self.layout.word]);
        u64::from_le_bytes(bytes)
    }

    /// A value the collector owns: its header, then the value itself.
    fn collected(&mut self, vtable: u64, size: u64) -> u64 {
        let header = self.alloc(self.layout.gc_header + size);
        self.put(header + self.layout.gc_vtable, vtable | GC_FLAGS);
        header + self.layout.gc_header
    }

    /// A string, written once and shared, as the pool of constants does.
    fn string(&mut self, text: &str) -> u64 {
        if let Some((_, address)) = self.interned.iter().find(|(name, _)| name == text) {
            return *address;
        }
        let units = self.alloc(text.len() as u64);
        self.put_bytes(units, text.as_bytes());
        let string = self.collected(self.layout.string_vtable, self.layout.string_size);
        self.put(string, units);
        // The length is a u32 on every target.
        self.put_bytes(
            string + self.layout.string_length,
            &(text.len() as u32).to_le_bytes(),
        );
        self.interned.push((String::from(text), string));
        string
    }

    fn object(&mut self) -> Obj {
        Obj {
            address: self.collected(self.layout.object_vtable, self.layout.object_size),
            properties: Vec::new(),
        }
    }

    /// Writes the entries of an object in a new buffer, and points its map at
    /// them. That is also what Ruffle does when a map grows.
    fn write_entries(&mut self, object: &Obj) {
        let layout = self.layout;
        let count = object.properties.len() as u64;
        let entries = self.alloc(count * layout.entry_size);
        for (index, (key, value)) in object.properties.iter().enumerate() {
            let entry = entries + index as u64 * layout.entry_size;
            self.write_value(entry, *value);
            let key_string = self.string(key);
            self.put(entry + layout.entry_key, key_string);
            // `put` keeps the low half of the hash on a 32-bit target, as
            // Ruffle does.
            self.put(entry + layout.entry_hash, key_hash(key));
        }
        self.put(object.address + layout.entries_capacity, count);
        self.put(object.address + layout.entries_pointer, entries);
        self.put(object.address + layout.entries_length, count);
    }

    fn write_value(&mut self, at: u64, value: Value) {
        match value {
            Value::Bool(flag) => {
                self.put_byte(at, TAG_BOOL);
                self.put_byte(at + VALUE_BOOL, flag as u8);
            }
            Value::Number(number) => {
                self.put_byte(at, TAG_NUMBER);
                self.put_number(at + VALUE_NUMBER, number);
            }
            Value::String(string) => {
                self.put_byte(at, TAG_STRING);
                self.put(at + self.layout.value_pointer, string);
            }
            Value::Object(object) => {
                self.put_byte(at, TAG_OBJECT);
                self.put(at + self.layout.value_pointer, object);
            }
        }
    }

    /// The entry of `key` in the entries an object points at now.
    fn entry_of(&self, object: &Obj, key: &str) -> u64 {
        let index = object
            .properties
            .iter()
            .position(|(name, _)| name == key)
            .expect("the object carries no property under that key");
        let entries = self.word(object.address + self.layout.entries_pointer);
        entries + index as u64 * self.layout.entry_size
    }

    /// Writes another value under a key the object already carries.
    fn replace(&mut self, object: &mut Obj, key: &str, value: Value) {
        let entry = self.entry_of(object, key);
        self.write_value(entry, value);
        let property = object
            .properties
            .iter_mut()
            .find(|(name, _)| name == key)
            .unwrap();
        property.1 = value;
    }
}

fn set(object: &mut Obj, key: &str, value: Value) {
    object.properties.push((String::from(key), value));
}

// -- the driver --------------------------------------------------------------

/// A world, in the bytes of Ruffle for one target, and what the driver knows
/// of them.
pub struct RuffleHeapWriter<T> {
    target: PhantomData<T>,
    bytes: Bytes,
    mode: Obj,
    manager: Obj,
    mechanics: Obj,
    chrono: Obj,
}

/// The Ruffle desktop driver.
pub type RuffleDesktopHeap = RuffleHeapWriter<Desktop>;
/// The Ruffle web driver: offsets in a linear memory.
pub type RuffleWebHeap = RuffleHeapWriter<Browser>;

impl<T: Target> WrittenHeap for RuffleHeapWriter<T> {
    type Player = Ruffle;

    fn write(world: &World<Self>) -> (Self, Ruffle) {
        let mut b = Bytes::new(T::LAYOUT);
        let player = T::player();
        if world.empty {
            let written = Self {
                target: PhantomData,
                bytes: b,
                mode: Obj::default(),
                manager: Obj::default(),
                mechanics: Obj::default(),
                chrono: Obj::default(),
            };
            return (written, player);
        }

        // Every object first, so that each one can cite the others. The views
        // come before the game, so that the search meets a view first.
        let mut manager = b.object();
        let mut views: Vec<Obj> = (0..world.views).map(|_| b.object()).collect();
        let mut corpse = world.corpse.then(|| b.object());
        let mut game_mode = b.object();
        let mut second = world.second_game.then(|| b.object());
        let mut mechanics = b.object();
        let mut chrono = b.object();

        let obfuscated = keys::WORLDS
            .iter()
            .find(|(_, clear)| *clear == world.set_name)
            .map_or(world.set_name, |(obf, _)| obf);
        let set_name = b.string(obfuscated);
        set(&mut mechanics, keys::SET_NAME, Value::String(set_name));
        set(
            &mut mechanics,
            keys::CURRENT_ID,
            Value::Number(world.level as f64),
        );
        set(
            &mut mechanics,
            keys::PREVIOUS_ID,
            Value::Number(world.previous as f64),
        );

        set(
            &mut chrono,
            keys::FRAME_TIMER,
            Value::Number(world.frame_timer as f64),
        );
        set(
            &mut chrono,
            keys::GAME_TIMER,
            Value::Number(world.game_timer as f64),
        );
        if let Some(ms) = world.halted {
            set(&mut chrono, keys::FL_STOP, Value::Bool(true));
            set(&mut chrono, keys::HALTED_TIMER, Value::Number(ms as f64));
        }

        for view in &mut views {
            set(view, keys::WORLD, Value::Object(mechanics.address));
        }

        for mode in [corpse.as_mut(), Some(&mut game_mode), second.as_mut()]
            .into_iter()
            .flatten()
        {
            set(mode, keys::WORLD, Value::Object(mechanics.address));
            set(mode, keys::GAME_CHRONO, Value::Object(chrono.address));
            if let Some(cycles) = world.duration {
                set(mode, keys::DURATION, Value::Number(cycles));
            }
            set(mode, keys::FL_LOCK, Value::Bool(false));
            set(mode, keys::CURRENT_DIM, Value::Number(world.dim as f64));
            if world.game_over {
                set(mode, keys::FL_GAME_OVER, Value::Bool(true));
            }
        }

        if world.manager || world.corpse {
            if world.manager {
                let version = b.string("1.0");
                set(&mut manager, keys::F_VERSION, Value::String(version));
            }
            set(
                &mut manager,
                keys::CURRENT,
                Value::Object(game_mode.address),
            );
            set(
                &mut game_mode,
                keys::MANAGER,
                Value::Object(manager.address),
            );
            if let Some(dead) = corpse.as_mut() {
                set(dead, keys::MANAGER, Value::Object(manager.address));
            }
        }

        let every_object = [&manager, &game_mode, &mechanics, &chrono]
            .into_iter()
            .chain(&views)
            .chain(corpse.as_ref())
            .chain(second.as_ref());
        for object in every_object {
            b.write_entries(object);
        }

        let written = Self {
            target: PhantomData,
            bytes: b,
            mode: game_mode,
            manager,
            mechanics,
            chrono,
        };
        (written, player)
    }

    fn memory(&self) -> Heap {
        T::memory(self.bytes.data.clone())
    }

    fn ranges(&self) -> Vec<(u64, u64)> {
        let base = T::LAYOUT.heap_base;
        vec![(base, base + self.bytes.data.len() as u64)]
    }

    fn game_mode(&self) -> u64 {
        self.mode.address
    }

    fn the_world_is_no_longer_known(&mut self) {
        let unknown = self.bytes.string("not_a_world");
        self.bytes
            .replace(&mut self.mechanics, keys::SET_NAME, Value::String(unknown));
    }

    fn the_level_becomes(&mut self, level: i64) {
        self.bytes.replace(
            &mut self.mechanics,
            keys::CURRENT_ID,
            Value::Number(level as f64),
        );
    }

    fn the_chrono_is_lost(&mut self) {
        self.bytes
            .replace(&mut self.mode, keys::GAME_CHRONO, Value::Object(NOWHERE));
    }

    /// Ruffle moves the entries of a map when it grows: they land in another
    /// buffer, and here `world` changes place in it too.
    fn the_world_property_moves(&mut self) {
        self.mode.properties.swap(0, 1);
        self.bytes.write_entries(&self.mode);
    }

    fn the_game_is_replaced(&mut self) {
        let mut new_mode = self.bytes.object();
        set(
            &mut new_mode,
            keys::WORLD,
            Value::Object(self.mechanics.address),
        );
        set(
            &mut new_mode,
            keys::GAME_CHRONO,
            Value::Object(self.chrono.address),
        );
        set(&mut new_mode, keys::DURATION, Value::Number(0.0));
        set(&mut new_mode, keys::FL_LOCK, Value::Bool(false));
        set(
            &mut new_mode,
            keys::MANAGER,
            Value::Object(self.manager.address),
        );
        self.bytes.write_entries(&new_mode);
        self.bytes.replace(
            &mut self.manager,
            keys::CURRENT,
            Value::Object(new_mode.address),
        );
        self.mode = new_mode;
    }
}

/// The traps only Ruffle sets.
impl<T: Target> RuffleHeapWriter<T> {
    /// The entry of `currentId` holds the hash of another key, as memory that
    /// was once another entry would.
    pub fn the_level_entry_holds_another_hash(&mut self) {
        let entry = self.bytes.entry_of(&self.mechanics, keys::CURRENT_ID);
        self.bytes
            .put(entry + T::LAYOUT.entry_hash, key_hash(keys::PREVIOUS_ID));
    }

    /// The `GameMode` carries the vtable of another type: its map was found
    /// in an allocation that is not an AVM1 object.
    pub fn the_game_is_not_an_object(&mut self) {
        let layout = T::LAYOUT;
        let header = self.mode.address - layout.gc_header;
        self.bytes
            .put(header + layout.gc_vtable, layout.vec_vtable | GC_FLAGS);
    }
}
