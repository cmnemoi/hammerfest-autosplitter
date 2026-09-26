//! The Ruffle driver: a world, written byte by byte as Ruffle 0.6.0 would
//! write it.
//!
//! The offsets below are this module's own, taken from
//! `docs/concepts/ruffle-heap.md`. They are not the reader's. A driver that
//! shared its offsets with the code under test could not see an error in
//! them. The replay of `fixtures/replay/ruffle-main-world` is what checks the
//! reader's offsets against bytes Ruffle really wrote.

use alloc::{string::String, vec, vec::Vec};

use hammerfest_reader::keys;
use hammerfest_reader::ruffle::Ruffle;

use crate::memory_contract::Heap;
use crate::scenarios::{World, WrittenHeap};

/// Where the fake module sits, and the vtables in it.
const MODULE: (u64, u64) = (0x4000_0000, 0x4010_0000);
/// Where the heap sits.
const HEAP_BASE: u64 = 0x1000_0000;
/// Where nothing is mapped at all.
const NOWHERE: u64 = 0x2000_0000;

/// Ruffle 0.6.0 under Linux, as `docs/concepts/ruffle-heap.md` draws it.
mod layout {
    use super::MODULE;

    /// A collected value is preceded by `next`, then its tagged vtable.
    pub const GC_HEADER: u64 = 0x10;
    pub const GC_VTABLE: u64 = 0x08;
    /// The low bits of the vtable are flags of the collector.
    pub const GC_FLAGS: u64 = 0b1010;

    /// The vtable of an AVM1 object, and the one of another type.
    pub const OBJECT_VTABLE: u64 = MODULE.0 + 0x1120;
    pub const OBJECT_SIZE: u64 = 160;
    pub const VEC_VTABLE: u64 = MODULE.0 + 0x1140;
    pub const VEC_SIZE: u64 = 24;
    pub const STRING_VTABLE: u64 = MODULE.0 + 0x1300;
    pub const STRING_SIZE: u64 = 32;

    /// Object: the borrow flag, then the map of its properties.
    pub const ENTRIES_CAPACITY: u64 = 0x08;
    pub const ENTRIES_POINTER: u64 = 0x10;
    pub const ENTRIES_LENGTH: u64 = 0x18;

    /// Entry: the value, then the key and its hash.
    pub const ENTRY_SIZE: u64 = 56;
    pub const ENTRY_KEY: u64 = 0x28;
    pub const ENTRY_HASH: u64 = 0x30;

    /// Value: a tag, then the boolean or the payload.
    pub const TAG_BOOL: u8 = 2;
    pub const TAG_NUMBER: u8 = 3;
    pub const TAG_STRING: u8 = 4;
    pub const TAG_OBJECT: u8 = 5;
    pub const VALUE_BOOL: u64 = 0x01;
    pub const VALUE_PAYLOAD: u64 = 0x08;

    /// String: the pointer to its units, then its length.
    pub const STRING_UNITS: u64 = 0x00;
    pub const STRING_LENGTH: u64 = 0x10;
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
    fn new() -> Self {
        Self {
            // Address zero means "no address", so nothing starts there.
            data: vec![0u8; 16],
            interned: Vec::new(),
        }
    }

    fn alloc(&mut self, len: u64) -> u64 {
        let address = HEAP_BASE + self.data.len() as u64;
        self.data
            .resize(self.data.len() + (len as usize).next_multiple_of(16), 0);
        address
    }

    fn put(&mut self, address: u64, value: u64) {
        let at = (address - HEAP_BASE) as usize;
        self.data[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn put_byte(&mut self, address: u64, value: u8) {
        self.data[(address - HEAP_BASE) as usize] = value;
    }

    /// A value the collector owns: its header, then the value itself.
    fn collected(&mut self, vtable: u64, size: u64) -> u64 {
        let header = self.alloc(layout::GC_HEADER + size);
        self.put(header + layout::GC_VTABLE, vtable | layout::GC_FLAGS);
        header + layout::GC_HEADER
    }

    /// A string, written once and shared, as the pool of constants does.
    fn string(&mut self, text: &str) -> u64 {
        if let Some((_, address)) = self.interned.iter().find(|(name, _)| name == text) {
            return *address;
        }
        let units = self.alloc(text.len() as u64);
        let at = (units - HEAP_BASE) as usize;
        self.data[at..at + text.len()].copy_from_slice(text.as_bytes());
        let string = self.collected(layout::STRING_VTABLE, layout::STRING_SIZE);
        self.put(string + layout::STRING_UNITS, units);
        self.put(string + layout::STRING_LENGTH, text.len() as u64);
        self.interned.push((String::from(text), string));
        string
    }

    fn object(&mut self) -> Obj {
        Obj {
            address: self.collected(layout::OBJECT_VTABLE, layout::OBJECT_SIZE),
            properties: Vec::new(),
        }
    }

    /// Writes the entries of an object in a new buffer, and points its map at
    /// them. That is also what Ruffle does when a map grows.
    fn write_entries(&mut self, object: &Obj) {
        let count = object.properties.len() as u64;
        let entries = self.alloc(count * layout::ENTRY_SIZE);
        for (index, (key, value)) in object.properties.iter().enumerate() {
            let entry = entries + index as u64 * layout::ENTRY_SIZE;
            self.write_value(entry, *value);
            let key_string = self.string(key);
            self.put(entry + layout::ENTRY_KEY, key_string);
            self.put(entry + layout::ENTRY_HASH, key_hash(key));
        }
        self.put(object.address + layout::ENTRIES_CAPACITY, count);
        self.put(object.address + layout::ENTRIES_POINTER, entries);
        self.put(object.address + layout::ENTRIES_LENGTH, count);
    }

    fn write_value(&mut self, at: u64, value: Value) {
        match value {
            Value::Bool(flag) => {
                self.put_byte(at, layout::TAG_BOOL);
                self.put_byte(at + layout::VALUE_BOOL, flag as u8);
            }
            Value::Number(number) => {
                self.put_byte(at, layout::TAG_NUMBER);
                self.put(at + layout::VALUE_PAYLOAD, number.to_bits());
            }
            Value::String(string) => {
                self.put_byte(at, layout::TAG_STRING);
                self.put(at + layout::VALUE_PAYLOAD, string);
            }
            Value::Object(object) => {
                self.put_byte(at, layout::TAG_OBJECT);
                self.put(at + layout::VALUE_PAYLOAD, object);
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
        let at = (object.address + layout::ENTRIES_POINTER - HEAP_BASE) as usize;
        let entries = u64::from_le_bytes(self.data[at..at + 8].try_into().unwrap());
        entries + index as u64 * layout::ENTRY_SIZE
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

/// A world, in the bytes of Ruffle, and what the driver knows of them.
pub struct RuffleHeapWriter {
    bytes: Bytes,
    mode: Obj,
    manager: Obj,
    mechanics: Obj,
    chrono: Obj,
}

impl WrittenHeap for RuffleHeapWriter {
    type Player = Ruffle;

    fn write(world: &World<Self>) -> (Self, Ruffle) {
        let mut b = Bytes::new();
        let player = Ruffle::attached_to(MODULE);
        if world.empty {
            let written = Self {
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
            bytes: b,
            mode: game_mode,
            manager,
            mechanics,
            chrono,
        };
        (written, player)
    }

    fn memory(&self) -> Heap {
        Heap::default()
            .with_range(HEAP_BASE, self.bytes.data.clone())
            .with_range(MODULE.0, module_bytes())
    }

    fn ranges(&self) -> Vec<(u64, u64)> {
        vec![(HEAP_BASE, HEAP_BASE + self.bytes.data.len() as u64)]
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

/// The module, where the vtables of the collected types sit: their alignment,
/// then their size.
fn module_bytes() -> Vec<u8> {
    let mut module = vec![0u8; 0x2000];
    for (vtable, size) in [
        (layout::OBJECT_VTABLE, layout::OBJECT_SIZE),
        (layout::VEC_VTABLE, layout::VEC_SIZE),
        (layout::STRING_VTABLE, layout::STRING_SIZE),
    ] {
        let at = (vtable - MODULE.0) as usize;
        module[at..at + 8].copy_from_slice(&8u64.to_le_bytes());
        module[at + 8..at + 16].copy_from_slice(&size.to_le_bytes());
    }
    module
}

/// The traps only Ruffle sets.
impl RuffleHeapWriter {
    /// The entry of `currentId` holds the hash of another key, as memory that
    /// was once another entry would.
    pub fn the_level_entry_holds_another_hash(&mut self) {
        let entry = self.bytes.entry_of(&self.mechanics, keys::CURRENT_ID);
        self.bytes
            .put(entry + layout::ENTRY_HASH, key_hash(keys::PREVIOUS_ID));
    }

    /// The `GameMode` carries the vtable of another type: its map was found
    /// in an allocation that is not an AVM1 object.
    pub fn the_game_is_not_an_object(&mut self) {
        let header = self.mode.address - layout::GC_HEADER;
        self.bytes.put(
            header + layout::GC_VTABLE,
            layout::VEC_VTABLE | layout::GC_FLAGS,
        );
    }
}
