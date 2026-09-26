//! The Pepper Flash driver: a world, written byte by byte as Pepper Flash
//! would write it.
//!
//! The reader is given bytes and nothing else. So a scenario describes a
//! world, this driver writes the bytes Pepper Flash would write, and the
//! scenario asks the reader what it sees. See `scenarios.rs`.
//!
//! The layout table below is this module's own. It is not `MEASURED`, the seed
//! the production code uses. A builder that shared its offsets with the code
//! under test could not see an error in those offsets: the two would move
//! together, and the test would stay green while production found nothing.
//!
//! See `docs/internals/testing-the-memory-reader.md`.

use alloc::{string::String, vec, vec::Vec};

use hammerfest_core::atom;

use crate::memory_contract::Heap;
use crate::scenarios::{World, WrittenHeap};
use hammerfest_reader::avm1::{Layout, PROFILES};
use hammerfest_reader::keys;
use hammerfest_reader::pepper_flash::{Binary, PepperFlash};

/// Where the fake module sits. No bytes are served for it: only `in_module`
/// looks at this range, and it only compares.
pub const MODULE: (u64, u64) = (0x4000_0000, 0x4010_0000);

/// Where the heap sits.
const HEAP_BASE: u64 = 0x1000_0000;

/// The layout this module writes. The reader derives its own, and it must land
/// on these numbers.
///
/// The table geometry is the one `avm1::PROFILES` calls `windows-x64`. That
/// one is shared of necessity: the reader tries two geometries and no third,
/// so a heap written in a third could not be read at all. Every other offset
/// here is free, and differs from `MEASURED`.
mod layout {
    use super::MODULE;

    /// String object: vtable, buffer pointer, length in UTF-16 units.
    pub const STR_VT: u64 = MODULE.0 + 0x120;
    pub const STR_BUF: u64 = 0x08;
    pub const STR_LEN: u64 = 0x10;
    pub const STR_SIZE: usize = 0x18;

    /// ScriptObject: vtable, then the property table.
    pub const SO_VT: u64 = MODULE.0 + 0x200;
    pub const SO_TBL: u64 = 0x30;
    pub const SO_SIZE: usize = 0x38;

    /// The same vtable on another build of the plugin. The binary is not the
    /// same one, so its classes are not at the same place.
    pub const OTHER_STR_VT: u64 = MODULE.0 + 0x900;

    /// Property table: vtable, capacity, then the entries.
    pub const TBL_VT: u64 = MODULE.0 + 0x340;
    pub const TBL_CAPACITY: u64 = 0x08;
    /// Offset of the first key. One entry is (value, pad, key).
    pub const KEYS: u64 = 0x58;
    pub const STRIDE: u64 = 24;
    pub const VALUE: i64 = -0x10;
}

// -- writing the bytes -------------------------------------------------------

/// A bump allocator over one region, and the strings already interned.
struct Bytes {
    data: Vec<u8>,
    interned: Vec<(String, u64)>,
    /// The String vtable this heap was written with. Another Flash build puts
    /// it somewhere else in the module.
    str_vt: u64,
}

/// An object the builder wrote: its ScriptObject, its table, and how many
/// properties the table already holds.
#[derive(Copy, Clone, Default)]
struct Obj {
    so: u64,
    tbl: u64,
    next: u64,
}

impl Obj {
    /// The atom that points at this object.
    fn atom(&self) -> u64 {
        self.so | atom::TAG_OBJECT
    }
}

impl Bytes {
    fn new(str_vt: u64) -> Self {
        Self {
            // Address zero means "no address" to the reader, so nothing is
            // ever written at the very start of the region.
            data: vec![0u8; 8],
            interned: Vec::new(),
            str_vt,
        }
    }

    /// Reserves `len` bytes, and keeps the next address 8-aligned.
    ///
    /// The reader scans aligned qwords, and nothing else. An object on an odd
    /// address is invisible to it, exactly as it would be in a real heap.
    fn alloc(&mut self, len: usize) -> u64 {
        let addr = HEAP_BASE + self.data.len() as u64;
        self.data
            .resize(self.data.len() + len.next_multiple_of(8), 0);
        addr
    }

    fn put(&mut self, addr: u64, value: u64) {
        let at = (addr - HEAP_BASE) as usize;
        self.data[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn get(&self, addr: u64) -> u64 {
        let at = (addr - HEAP_BASE) as usize;
        u64::from_le_bytes(self.data[at..at + 8].try_into().unwrap())
    }

    /// Writes another value under a key the object already carries.
    fn replace(&mut self, o: &Obj, key: &str, value: u64) {
        let wanted = self.string(key);
        for i in 0..o.next {
            let key_addr = o.tbl + layout::KEYS + i * layout::STRIDE;
            if self.get(key_addr) == wanted {
                self.put((key_addr as i64 + layout::VALUE) as u64, value);
                return;
            }
        }
        panic!("the object carries no property under that key");
    }

    /// Swaps two entries of a table, key and value together.
    ///
    /// The game does this when it rebuilds a table. The reader keeps the index
    /// of an entry from one reading to the next, so the entry must be able to
    /// move under it.
    fn swap_entries(&mut self, o: &Obj, a: u64, b: u64) {
        let (ka, kb) = (
            o.tbl + layout::KEYS + a * layout::STRIDE,
            o.tbl + layout::KEYS + b * layout::STRIDE,
        );
        let (va, vb) = (
            (ka as i64 + layout::VALUE) as u64,
            (kb as i64 + layout::VALUE) as u64,
        );
        let (key_a, key_b) = (self.get(ka), self.get(kb));
        let (value_a, value_b) = (self.get(va), self.get(vb));
        self.put(ka, key_b);
        self.put(va, value_b);
        self.put(kb, key_a);
        self.put(vb, value_a);
    }

    /// The String object of `s`, written once and shared, as the SWF interns
    /// the names of its properties.
    fn intern(&mut self, s: &str) -> u64 {
        if let Some((_, addr)) = self.interned.iter().find(|(name, _)| name == s) {
            return *addr;
        }
        let units: Vec<u16> = s.encode_utf16().collect();
        let buf = self.alloc(units.len() * 2);
        for (i, unit) in units.iter().enumerate() {
            let at = (buf - HEAP_BASE) as usize + i * 2;
            self.data[at..at + 2].copy_from_slice(&unit.to_le_bytes());
        }
        let so = self.alloc(layout::STR_SIZE);
        self.put(so, self.str_vt);
        self.put(so + layout::STR_BUF, buf);
        self.put(so + layout::STR_LEN, units.len() as u64);
        self.interned.push((String::from(s), so));
        so
    }

    /// The atom of an interned string.
    fn string(&mut self, s: &str) -> u64 {
        self.intern(s) | atom::TAG_STRING
    }

    /// A float, which does not fit inside an atom and so lives beside it.
    fn double(&mut self, v: f64) -> u64 {
        let addr = self.alloc(8);
        self.put(addr, v.to_bits());
        addr | atom::TAG_DOUBLE
    }

    /// An empty object, with room for `capacity` properties.
    fn object(&mut self, capacity: u64) -> Obj {
        let tbl = self.alloc((layout::KEYS + capacity * layout::STRIDE) as usize);
        self.put(tbl, layout::TBL_VT);
        self.put(tbl + layout::TBL_CAPACITY, capacity);
        let so = self.alloc(layout::SO_SIZE);
        self.put(so, layout::SO_VT);
        self.put(so + layout::SO_TBL, tbl);
        Obj { so, tbl, next: 0 }
    }

    /// Adds one entry whose key is `key_atom` as it is, not an interned
    /// String.
    fn set_raw_key(&mut self, o: &mut Obj, key_atom: u64, value: u64) {
        let key_addr = o.tbl + layout::KEYS + o.next * layout::STRIDE;
        o.next += 1;
        self.put(key_addr, key_atom);
        self.put((key_addr as i64 + layout::VALUE) as u64, value);
    }

    /// Adds one property. The entries are written from index zero upward, as
    /// a table the player filled would be.
    fn set(&mut self, o: &mut Obj, key: &str, value: u64) {
        let key_addr = o.tbl + layout::KEYS + o.next * layout::STRIDE;
        o.next += 1;
        let key_atom = self.string(key);
        self.put(key_addr, key_atom);
        self.put((key_addr as i64 + layout::VALUE) as u64, value);
    }
}

/// An integer, as the player stores it.
fn int(v: i64) -> u64 {
    (v as u64) << 3
}

fn boolean(v: bool) -> u64 {
    if v {
        atom::TRUE
    } else {
        atom::FALSE
    }
}

// -- the driver --------------------------------------------------------------

/// A world, in the bytes of Pepper Flash, and what the driver knows of them.
pub struct PepperFlashHeapWriter {
    bytes: Bytes,
    game_mode: u64,
    mode: Obj,
    manager: Obj,
    mechanics: Obj,
    chrono: Obj,
}

impl WrittenHeap for PepperFlashHeapWriter {
    type Player = PepperFlash;

    fn write(world: &World<Self>) -> (Self, PepperFlash) {
        // What the reader already knows. On another build, it knows the
        // layout of the first one, and the search never questions it.
        let known = Layout {
            module: (0, 0),
            str_vt: layout::STR_VT - MODULE.0,
            str_buf: layout::STR_BUF,
            str_len: layout::STR_LEN,
            tbl_vt: layout::TBL_VT - MODULE.0,
            profile: PROFILES[0],
            so_tbl: layout::SO_TBL,
        };
        let (str_vt, binary) = if world.another_build {
            (layout::OTHER_STR_VT, Binary::proven_with(known))
        } else {
            (layout::STR_VT, Binary::default())
        };
        let mut b = Bytes::new(str_vt);
        let mut player = PepperFlash::new(binary);
        player.attach(MODULE);

        if world.empty {
            // No object: nothing a mutation could change either.
            let nothing = Obj::default();
            let written = Self {
                bytes: b,
                game_mode: 0,
                mode: nothing,
                manager: nothing,
                mechanics: nothing,
                chrono: nothing,
            };
            return (written, player);
        }

        // Every object first, so that each one can cite the others.
        //
        // The order is the order of the addresses, and the search reads the
        // heap from the low addresses upward. The views come before the game
        // so that the search meets a view first.
        let mut manager = b.object(4);
        let mut views: Vec<Obj> = (0..world.views).map(|_| b.object(4)).collect();
        let mut corpse = world.corpse.then(|| b.object(12));
        let mut game_mode = b.object(12);
        let mut second = world.second_game.then(|| b.object(12));
        let mut mechanics = b.object(4);
        let mut chrono = b.object(4);

        if world.foreign_keys {
            let foreign = b.object(1).so | atom::TAG_STRING;
            for o in [&mut manager, &mut game_mode] {
                b.set_raw_key(o, foreign, int(0));
            }
        }

        let obfuscated = keys::WORLDS
            .iter()
            .find(|(_, clear)| *clear == world.set_name)
            .map_or(world.set_name, |(obf, _)| obf);
        let set_name = b.string(obfuscated);
        b.set(&mut mechanics, keys::SET_NAME, set_name);
        b.set(&mut mechanics, keys::CURRENT_ID, int(world.level));
        b.set(&mut mechanics, keys::PREVIOUS_ID, int(world.previous));

        b.set(&mut chrono, keys::FRAME_TIMER, int(world.frame_timer));
        b.set(&mut chrono, keys::GAME_TIMER, int(world.game_timer));
        if let Some(ms) = world.halted {
            b.set(&mut chrono, keys::FL_STOP, boolean(true));
            b.set(&mut chrono, keys::HALTED_TIMER, int(ms));
        }

        // A view carries the anchor key and the same mechanics. It owns no
        // `gameChrono`, and that is the only thing that tells it apart.
        for view in &mut views {
            b.set(view, keys::WORLD, mechanics.atom());
        }

        for mode in [corpse.as_mut(), Some(&mut game_mode), second.as_mut()]
            .into_iter()
            .flatten()
        {
            b.set(mode, keys::WORLD, mechanics.atom());
            b.set(mode, keys::GAME_CHRONO, chrono.atom());
            if let Some(cycles) = world.duration {
                let duration = b.double(cycles);
                b.set(mode, keys::DURATION, duration);
            }
            b.set(mode, keys::FL_LOCK, boolean(false));
            b.set(mode, keys::CURRENT_DIM, int(world.dim));
            if world.game_over {
                b.set(mode, keys::FL_GAME_OVER, boolean(true));
            }
        }

        if world.manager || world.corpse {
            if world.manager {
                // `fVersion` is what the search looks for: the GameManager is
                // the only class that carries it.
                let version = b.string("1.0");
                b.set(&mut manager, keys::F_VERSION, version);
            }
            b.set(&mut manager, keys::CURRENT, game_mode.atom());
            b.set(&mut game_mode, keys::MANAGER, manager.atom());
            // The corpse still names the manager. The manager no longer names
            // it, and that is what tells the two apart.
            if let Some(dead) = corpse.as_mut() {
                b.set(dead, keys::MANAGER, manager.atom());
            }
        }

        let written = Self {
            bytes: b,
            game_mode: game_mode.tbl,
            mode: game_mode,
            manager,
            mechanics,
            chrono,
        };
        (written, player)
    }

    fn memory(&self) -> Heap {
        Heap::default().with_range(HEAP_BASE, self.bytes.data.clone())
    }

    fn ranges(&self) -> Vec<(u64, u64)> {
        vec![(HEAP_BASE, HEAP_BASE + self.bytes.data.len() as u64)]
    }

    fn game_mode(&self) -> u64 {
        self.game_mode
    }

    fn the_world_is_no_longer_known(&mut self) {
        let unknown = self.bytes.string("not_a_world");
        self.bytes.replace(&self.mechanics, keys::SET_NAME, unknown);
    }

    fn the_level_becomes(&mut self, level: i64) {
        self.bytes
            .replace(&self.mechanics, keys::CURRENT_ID, int(level));
    }

    fn the_chrono_is_lost(&mut self) {
        let nowhere = (HEAP_BASE + 0x100_0000) | atom::TAG_OBJECT;
        self.bytes.replace(&self.mode, keys::GAME_CHRONO, nowhere);
    }

    /// Pepper Flash rebuilds a table in place: `world` changes entry, and the
    /// index the last reading kept is now wrong.
    fn the_world_property_moves(&mut self) {
        self.bytes.swap_entries(&self.mode, 0, 1);
    }

    fn the_game_is_replaced(&mut self) {
        let mut new_mode = self.bytes.object(12);
        let duration = self.bytes.double(0.0);
        let world = self.mechanics.atom();
        let chrono = self.chrono.atom();
        self.bytes.set(&mut new_mode, keys::WORLD, world);
        self.bytes.set(&mut new_mode, keys::GAME_CHRONO, chrono);
        self.bytes.set(&mut new_mode, keys::DURATION, duration);
        self.bytes.set(&mut new_mode, keys::FL_LOCK, boolean(false));
        let manager = self.manager.atom();
        self.bytes.set(&mut new_mode, keys::MANAGER, manager);
        self.bytes
            .replace(&self.manager, keys::CURRENT, new_mode.atom());
        self.mode = new_mode;
        self.game_mode = new_mode.tbl;
    }
}

/// The traps only Pepper Flash sets.
impl World<PepperFlashHeapWriter> {
    /// A player who has another version of the plugin.
    ///
    /// The String vtable is elsewhere in the module, because the binary is
    /// another one. The reader has already proven the layout of the first
    /// build, so it trusts that layout and must find nothing here. Finding
    /// nothing is the rule: a layout that does not match decodes neighbouring
    /// bytes into plausible numbers, and a plausible level is worse than none.
    pub fn written_by_another_flash_build(mut self) -> Self {
        self.another_build = true;
        self
    }

    /// Each table starts with a key that is not a String object.
    ///
    /// The Linux player writes such keys: measured on a running game, the
    /// entry in front of `world` held a pointer to an object of another
    /// class. The base of a table must be found all the same.
    pub fn with_a_key_that_is_not_a_string_first(mut self) -> Self {
        self.foreign_keys = true;
        self
    }
}
