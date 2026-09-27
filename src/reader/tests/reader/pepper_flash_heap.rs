//! The Pepper Flash driver: a world, written byte by byte as Pepper Flash
//! would write it, in words of eight bytes, or as the 32-bit Windows projector
//! would, in words of four.
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
use hammerfest_reader::avm1::{Layout, Word, PROFILES, PROFILES_32_BITS};
use hammerfest_reader::keys;
use hammerfest_reader::pepper_flash::{Binary, PepperFlash};

/// Where the fake module sits. No bytes are served for it: only `in_module`
/// looks at this range, and it only compares.
pub const MODULE: (u64, u64) = (0x4000_0000, 0x4010_0000);

/// Where the heap sits.
const HEAP_BASE: u64 = 0x1000_0000;

/// The layout this module writes, for one width of word. The reader derives
/// its own, and it must land on these numbers.
///
/// The table geometry is one of `avm1::PROFILES` or `avm1::PROFILES_32_BITS`,
/// and the capacity sits right after the vtable. Those are shared of
/// necessity: the reader tries these geometries and no other, so a heap
/// written in another could not be read at all. Every other offset here is
/// free, and differs from what was measured on a real player.
struct DriverLayout {
    word: Word,
    /// String object: vtable, buffer pointer, length in UTF-16 units.
    str_vt: u64,
    str_buf: u64,
    str_len: u64,
    str_size: usize,
    /// ScriptObject: vtable, then the property table.
    so_vt: u64,
    so_tbl: u64,
    so_size: usize,
    /// The same vtable on another build of the player. The binary is not the
    /// same one, so its classes are not at the same place.
    other_str_vt: u64,
    /// Property table: vtable, capacity, then the entries.
    tbl_vt: u64,
    /// Offset of the first key.
    keys: u64,
    stride: u64,
    value: i64,
}

impl DriverLayout {
    const fn of(word_bytes: u64) -> &'static DriverLayout {
        match word_bytes {
            4 => &FOUR_BYTES,
            _ => &EIGHT_BYTES,
        }
    }

    fn capacity(&self) -> u64 {
        self.word.bytes()
    }
}

/// One entry is (value, pad, key): the geometry `windows-x64`.
const EIGHT_BYTES: DriverLayout = DriverLayout {
    word: Word::Eight,
    str_vt: MODULE.0 + 0x120,
    str_buf: 0x08,
    str_len: 0x10,
    str_size: 0x18,
    so_vt: MODULE.0 + 0x200,
    so_tbl: 0x30,
    so_size: 0x38,
    other_str_vt: MODULE.0 + 0x900,
    tbl_vt: MODULE.0 + 0x340,
    keys: 0x58,
    stride: 24,
    value: -0x10,
};

/// One entry is (value, key): the geometry `windows-x86`.
const FOUR_BYTES: DriverLayout = DriverLayout {
    word: Word::Four,
    str_vt: MODULE.0 + 0x160,
    str_buf: 0x08,
    str_len: 0x0c,
    str_size: 0x10,
    so_vt: MODULE.0 + 0x240,
    so_tbl: 0x14,
    so_size: 0x18,
    other_str_vt: MODULE.0 + 0x980,
    tbl_vt: MODULE.0 + 0x380,
    keys: 0x10,
    stride: 8,
    value: -0x04,
};

// -- writing the bytes -------------------------------------------------------

/// A bump allocator over one region, and the strings already interned.
struct Bytes {
    layout: &'static DriverLayout,
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
    fn new(layout: &'static DriverLayout, str_vt: u64) -> Self {
        Self {
            layout,
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

    /// Writes one word. A negative integer keeps its low bytes, as the player
    /// writes it: -1 is `0xfffffff8` in four bytes.
    fn put(&mut self, addr: u64, value: u64) {
        let width = self.layout.word.bytes() as usize;
        let at = (addr - HEAP_BASE) as usize;
        self.data[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
    }

    fn get(&self, addr: u64) -> u64 {
        let width = self.layout.word.bytes() as usize;
        let at = (addr - HEAP_BASE) as usize;
        let mut bytes = [0u8; 8];
        bytes[..width].copy_from_slice(&self.data[at..at + width]);
        u64::from_le_bytes(bytes)
    }

    /// Writes another value under a key the object already carries.
    fn replace(&mut self, o: &Obj, key: &str, value: u64) {
        let wanted = self.string(key);
        for i in 0..o.next {
            let key_addr = o.tbl + self.layout.keys + i * self.layout.stride;
            if self.get(key_addr) == wanted {
                self.put((key_addr as i64 + self.layout.value) as u64, value);
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
            o.tbl + self.layout.keys + a * self.layout.stride,
            o.tbl + self.layout.keys + b * self.layout.stride,
        );
        let (va, vb) = (
            (ka as i64 + self.layout.value) as u64,
            (kb as i64 + self.layout.value) as u64,
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
        let so = self.alloc(self.layout.str_size);
        self.put(so, self.str_vt);
        self.put(so + self.layout.str_buf, buf);
        self.put(so + self.layout.str_len, units.len() as u64);
        self.interned.push((String::from(s), so));
        so
    }

    /// The atom of an interned string.
    fn string(&mut self, s: &str) -> u64 {
        self.intern(s) | atom::TAG_STRING
    }

    /// A float, which does not fit inside an atom and so lives beside it, in
    /// eight bytes whatever the width of a word.
    fn double(&mut self, v: f64) -> u64 {
        let addr = self.alloc(8);
        let at = (addr - HEAP_BASE) as usize;
        self.data[at..at + 8].copy_from_slice(&v.to_bits().to_le_bytes());
        addr | atom::TAG_DOUBLE
    }

    /// An empty object, with room for `capacity` properties.
    fn object(&mut self, capacity: u64) -> Obj {
        let tbl = self.alloc((self.layout.keys + capacity * self.layout.stride) as usize);
        self.put(tbl, self.layout.tbl_vt);
        self.put(tbl + self.layout.capacity(), capacity);
        let so = self.alloc(self.layout.so_size);
        self.put(so, self.layout.so_vt);
        self.put(so + self.layout.so_tbl, tbl);
        Obj { so, tbl, next: 0 }
    }

    /// Adds one entry whose key is `key_atom` as it is, not an interned
    /// String.
    fn set_raw_key(&mut self, o: &mut Obj, key_atom: u64, value: u64) {
        let key_addr = o.tbl + self.layout.keys + o.next * self.layout.stride;
        o.next += 1;
        self.put(key_addr, key_atom);
        self.put((key_addr as i64 + self.layout.value) as u64, value);
    }

    /// Adds one property. The entries are written from index zero upward, as
    /// a table the player filled would be.
    fn set(&mut self, o: &mut Obj, key: &str, value: u64) {
        let key_addr = o.tbl + self.layout.keys + o.next * self.layout.stride;
        o.next += 1;
        let key_atom = self.string(key);
        self.put(key_addr, key_atom);
        self.put((key_addr as i64 + self.layout.value) as u64, value);
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
///
/// Words of eight bytes unless told otherwise: see [`ThirtyTwoBitHeapWriter`].
pub struct PepperFlashHeapWriter<const WORD_BYTES: u64 = 8> {
    bytes: Bytes,
    game_mode: u64,
    mode: Obj,
    manager: Obj,
    mechanics: Obj,
    chrono: Obj,
}

/// A world, in the bytes of the Windows projector: a 32-bit build.
pub type ThirtyTwoBitHeapWriter = PepperFlashHeapWriter<4>;

impl<const WORD_BYTES: u64> WrittenHeap for PepperFlashHeapWriter<WORD_BYTES> {
    type Player = PepperFlash;

    fn write(world: &World<Self>) -> (Self, PepperFlash) {
        let layout = DriverLayout::of(WORD_BYTES);
        let profile = match layout.word {
            Word::Four => PROFILES_32_BITS[0],
            Word::Eight => PROFILES[0],
        };
        // What the reader already knows. On another build, it knows the
        // layout of the first one, and the search never questions it.
        let known = Layout {
            module: (0, 0),
            str_vt: layout.str_vt - MODULE.0,
            str_buf: layout.str_buf,
            str_len: layout.str_len,
            tbl_vt: layout.tbl_vt - MODULE.0,
            profile,
            so_tbl: layout.so_tbl,
            word: layout.word,
        };
        let (str_vt, binary) = if world.another_build {
            (layout.other_str_vt, Binary::proven_with(known))
        } else {
            (layout.str_vt, Binary::default())
        };
        let mut b = Bytes::new(layout, str_vt);
        let mut player = PepperFlash::new(layout.word, binary);
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
        // Every game runs under a `GameManager` whose constructor set
        // `fVersion` before it started any mode: `hf/GameManager.hx` in
        // `eternalfest/hammerfest-haxe`. When the world holds no manager the
        // search can prove, the key still lives on an object it cannot prove.
        if !world.manager {
            let mut unproven = b.object(1);
            let version = b.string("1.0");
            b.set(&mut unproven, keys::F_VERSION, version);
        }

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

/// The traps only Pepper Flash sets, at either width.
impl<const WORD_BYTES: u64> World<PepperFlashHeapWriter<WORD_BYTES>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_cost::{measure, Metered};
    use crate::scenarios::block_on;
    use hammerfest_reader::hammerfest::{resolve, Anchor};
    use hammerfest_reader::pepper_flash::{under_rosetta, KnownBuild};
    use hammerfest_reader::search_log::{SearchLog, Silent};

    /// The stages a search went through, in order.
    #[derive(Default)]
    struct Stages(Vec<&'static str>);

    impl SearchLog for Stages {
        fn stage(&mut self, next: &'static str, _requested: u64, _calls: u64) {
            self.0.push(next);
        }
    }

    /** @spec reader.find::the-first-ranges-first */
    #[test]
    fn a_key_in_the_first_ranges_is_found_without_the_rest_of_the_heap() {
        const THE_REST: u64 = 256 << 20;
        const REST_BASE: u64 = 0x1_0000_0000;
        let fixture = World::<PepperFlashHeapWriter>::default()
            .with_a_manager_and_a_game()
            .build();
        let memory = fixture
            .written
            .memory()
            .with_range(REST_BASE, vec![0; THE_REST as usize]);
        let mut ranges = fixture.written.ranges();
        ranges.push((REST_BASE, REST_BASE + THE_REST));
        let mut player = PepperFlash::with_words(Word::Eight);
        player.attach(MODULE);

        let metered = Metered::new(&memory);
        let (found, cost) = measure(
            &metered,
            resolve(
                &metered,
                &mut player,
                &mut Anchor::default(),
                &ranges,
                &mut Silent,
            ),
        );

        assert!(found.is_some(), "the game was not found");
        assert!(
            cost.bytes < THE_REST,
            "the search read the rest of the heap: {cost}"
        );
    }

    /// The first methods of the driver's vtables, as offsets in the module.
    const STRING_METHOD: u64 = 0x1_2340;
    const TABLE_METHOD: u64 = 0x5_6780;

    /// The build this driver writes, known with its seed and its methods.
    fn the_driver_build() -> KnownBuild {
        let layout = DriverLayout::of(8);
        KnownBuild {
            seed: Layout {
                module: (0, 0),
                str_vt: layout.str_vt - MODULE.0,
                str_buf: layout.str_buf,
                str_len: layout.str_len,
                tbl_vt: layout.tbl_vt - MODULE.0,
                profile: PROFILES[0],
                so_tbl: layout.so_tbl,
                word: layout.word,
            },
            string_method: STRING_METHOD,
            table_method: TABLE_METHOD,
        }
    }

    /// The menus of a player whose vtables start with these methods: the SWF
    /// has created nothing the reader looks for yet.
    fn the_menus_of_a_build(string_method: u64, table_method: u64) -> Stages {
        let fixture = World::<PepperFlashHeapWriter>::default()
            .with_nothing_in_it()
            .build();
        let memory = with_the_methods(fixture.written.memory(), string_method, table_method);
        let mut stages = Stages::default();

        let found = block_on(resolve(
            &memory,
            &mut the_driver_build_attached(),
            &mut Anchor::default(),
            &fixture.written.ranges(),
            &mut stages,
        ));

        assert!(found.is_none(), "a game was found in the menus");
        stages
    }

    /// The module, as far as the recognition of a build reads it: the first
    /// method of each vtable of the driver.
    fn with_the_methods(memory: Heap, string_method: u64, table_method: u64) -> Heap {
        let layout = DriverLayout::of(8);
        memory
            .with_range(
                layout.str_vt,
                (MODULE.0 + string_method).to_le_bytes().to_vec(),
            )
            .with_range(
                layout.tbl_vt,
                (MODULE.0 + table_method).to_le_bytes().to_vec(),
            )
    }

    fn the_driver_build_attached() -> PepperFlash {
        let mut player = PepperFlash::new(Word::Eight, Binary::of_build(the_driver_build()));
        player.attach(MODULE);
        player
    }

    /// A proven layout does not make the manager mandatory: the key of the
    /// manager is there, and the game is found without it.
    ///
    /** @spec reader.find::an-orphan-game */
    #[test]
    fn a_known_build_still_finds_a_game_whose_manager_it_cannot_prove() {
        let fixture = World::<PepperFlashHeapWriter>::default()
            .with_a_game()
            .build();
        let memory = with_the_methods(fixture.written.memory(), STRING_METHOD, TABLE_METHOD);

        let found = block_on(resolve(
            &memory,
            &mut the_driver_build_attached(),
            &mut Anchor::default(),
            &fixture.written.ranges(),
            &mut Silent,
        ));

        assert_eq!(
            found.map(|game| game.game_mode),
            Some(fixture.written.game_mode()),
            "the orphan game was not found"
        );
    }

    /** @spec reader.find::a-known-build-is-trusted */
    #[test]
    fn a_known_build_does_not_search_by_content() {
        let stages = the_menus_of_a_build(STRING_METHOD, TABLE_METHOD);

        assert!(
            !stages.0.contains(&"string_bytes"),
            "a known build searched by content: {:?}",
            stages.0
        );
    }

    /** @spec reader.find::another-build-is-not-trusted */
    #[test]
    fn another_build_still_searches_by_content() {
        let stages = the_menus_of_a_build(STRING_METHOD + 0x10, TABLE_METHOD);

        assert!(
            stages.0.contains(&"string_bytes"),
            "another build was trusted: {:?}",
            stages.0
        );
    }

    /** @spec reader.find::no-manager-no-game */
    #[test]
    fn without_the_key_of_the_manager_the_world_is_not_searched() {
        let stages = the_menus_of_a_build(STRING_METHOD, TABLE_METHOD);

        let world = stages.0.iter().position(|&stage| stage == "world");
        let search_of_world = world.map_or(&[][..], |world| &stages.0[world..]);
        assert!(
            !search_of_world.contains(&"string_seed"),
            "the world was searched: {:?}",
            stages.0
        );
    }

    /** @spec projector.find::no-large-range-under-rosetta */
    #[test]
    fn under_rosetta_the_large_ranges_are_not_swept() {
        const MIB: u64 = 1 << 20;
        let malloc_zone = (0x6000_0000_0000, 0x6000_0000_0000 + 512 * MIB);
        let object_block = (0x3c77_5958_f000, 0x3c77_5958_f000 + MIB / 2);
        let small_block = (0x1340_328f_8000, 0x1340_328f_8000 + 4096);

        let swept = under_rosetta(vec![malloc_zone, object_block, small_block]);

        assert_eq!(swept, vec![small_block, object_block]);
    }

    /** @spec projector.find::adjacent-ranges-under-rosetta */
    #[test]
    fn under_rosetta_the_ranges_that_follow_each_other_are_swept_as_one() {
        const PAGE: u64 = 4096;
        let base = 0x3c77_5958_f000;
        let first_page = (base, base + PAGE);
        let second_page = (base + PAGE, base + 3 * PAGE);
        let apart = (base + 8 * PAGE, base + 9 * PAGE);

        let swept = under_rosetta(vec![second_page, apart, first_page]);

        assert_eq!(swept, vec![apart, (base, base + 3 * PAGE)]);
    }

    /** @spec projector.find::no-seed */
    #[test]
    fn a_projector_never_tries_the_seed_of_the_plugin() {
        let fixture = World::<PepperFlashHeapWriter>::default()
            .with_a_game()
            .build();
        let mut projector = PepperFlash::with_words(Word::Eight);
        projector.attach(MODULE);
        let mut stages = Stages::default();

        let found = block_on(resolve(
            &fixture.written.memory(),
            &mut projector,
            &mut Anchor::default(),
            &fixture.written.ranges(),
            &mut stages,
        ));

        assert!(found.is_some(), "the projector found no game");
        assert!(
            !stages.0.contains(&"string_seed"),
            "the projector tried the seed of the plugin: {:?}",
            stages.0
        );
    }
}
