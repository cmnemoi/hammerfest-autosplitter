//! A synthetic AVM1 heap, written byte by byte, for the reader's tests.
//!
//! The reader is given bytes and nothing else. So a test writes the bytes a
//! Flash player would write, and asks the reader what it sees.
//!
//! The layout table below is this module's own. It is not `MEASURED`, the seed
//! the production code uses. A builder that shared its offsets with the code
//! under test could not see an error in those offsets: the two would move
//! together, and the test would stay green while production found nothing.
//!
//! See `docs/internals/testing-the-memory-reader.md`.

use alloc::{string::String, vec, vec::Vec};

use hammerfest_core::atom;

use crate::avm1::{Layout, PROFILES};
use crate::hammerfest::{resolve, Anchor, Binary, Game, State};
use crate::keys;
use crate::memory_contract::Heap;

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
#[derive(Copy, Clone)]
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

// -- the DSL -----------------------------------------------------------------

pub fn given_a_heap() -> World {
    World::default()
}

/// What the heap must hold. The bytes are written once, by [`World::build`].
///
/// A description first, and the bytes at the end, because the objects cite
/// each other: the manager points at the game, and the game points back. One
/// single pass would mean patching what was already written.
pub struct World {
    manager: bool,
    game_over: bool,
    views: usize,
    second_game: bool,
    corpse: bool,
    another_build: bool,
    foreign_keys: bool,
    set_name: &'static str,
    dim: i64,
    level: i64,
    previous: i64,
    frame_timer: i64,
    game_timer: i64,
    /// Absent when the game does not carry the property at all.
    duration: Option<f64>,
    /// `Chrono.haltedTimer`, and `fl_stop` with it.
    halted: Option<i64>,
}

impl Default for World {
    fn default() -> Self {
        Self {
            manager: false,
            game_over: false,
            views: 0,
            second_game: false,
            corpse: false,
            another_build: false,
            foreign_keys: false,
            set_name: "xml_adventure",
            dim: 0,
            level: 2,
            previous: -1,
            frame_timer: 23_677,
            game_timer: 1_000,
            duration: Some(0.0),
            halted: None,
        }
    }
}

impl World {
    /// A game, and the `GameManager` that points at it.
    pub fn with_a_manager_and_a_game(mut self) -> Self {
        self.manager = true;
        self
    }

    /// A game alone. In production a heap always holds a `GameManager`, so a
    /// test that leaves it out exercises the fallback search on purpose.
    pub fn with_a_game(self) -> Self {
        self
    }

    /// The player has lost. `fl_gameOver` is true.
    pub fn that_is_over(mut self) -> Self {
        self.game_over = true;
        self
    }

    /// Three `View` objects of the same game.
    ///
    /// A view carries a `world` and points at the same `GameMechanics`, so it
    /// answers the anchor key exactly as the game does. It owns no
    /// `gameChrono`, which is the one difference.
    ///
    /// They are written before the game, so the search meets them first.
    pub fn and_three_views_of_that_game(mut self) -> Self {
        self.views = 3;
        self
    }

    /// A second game, which passes every rule the first one passes.
    pub fn and_a_second_game(mut self) -> Self {
        self.second_game = true;
        self
    }

    /// A game that is over, still in memory, and the manager that has moved
    /// on to the game which replaced it.
    ///
    /// The corpse comes first in memory, so the search meets it first. The
    /// manager carries no `fVersion`, so the search cannot find it by itself
    /// and has to fall back on the `world` key.
    pub fn and_a_corpse_the_manager_has_left(mut self) -> Self {
        self.corpse = true;
        self
    }

    pub fn in_world(mut self, name: &'static str) -> Self {
        self.set_name = name;
        self
    }

    pub fn at_level(mut self, level: i64) -> Self {
        self.level = level;
        self
    }

    pub fn with_frame_timer(mut self, ticks: i64) -> Self {
        self.frame_timer = ticks;
        self
    }

    pub fn with_game_timer(mut self, ticks: i64) -> Self {
        self.game_timer = ticks;
        self
    }

    /// `GameMode.duration`, in game cycles. It dates the start of the run.
    pub fn with_duration(mut self, cycles: f64) -> Self {
        self.duration = Some(cycles);
        self
    }

    /// A game that carries no `duration` at all.
    pub fn without_a_duration(mut self) -> Self {
        self.duration = None;
        self
    }

    /// The clock is stopped, and `haltedTimer` holds the time it stopped at.
    pub fn stopped_at(mut self, ms: i64) -> Self {
        self.halted = Some(ms);
        self
    }

    pub fn in_dimension(mut self, dim: i64) -> Self {
        self.dim = dim;
        self
    }

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

    pub fn build(self) -> Fixture {
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
        let (str_vt, binary) = if self.another_build {
            (layout::OTHER_STR_VT, Binary::proven_with(known))
        } else {
            (layout::STR_VT, Binary::default())
        };
        let mut b = Bytes::new(str_vt);

        // Every object first, so that each one can cite the others.
        //
        // The order is the order of the addresses, and the search reads the
        // heap from the low addresses upward. The views come before the game
        // so that the search meets a view first.
        let mut manager = b.object(4);
        let mut views: Vec<Obj> = (0..self.views).map(|_| b.object(4)).collect();
        let mut corpse = self.corpse.then(|| b.object(12));
        let mut game_mode = b.object(12);
        let mut second = self.second_game.then(|| b.object(12));
        let mut mechanics = b.object(4);
        let mut chrono = b.object(4);

        if self.foreign_keys {
            let foreign = b.object(1).so | atom::TAG_STRING;
            for o in [&mut manager, &mut game_mode] {
                b.set_raw_key(o, foreign, int(0));
            }
        }

        let obfuscated = keys::WORLDS
            .iter()
            .find(|(_, clear)| *clear == self.set_name)
            .map_or(self.set_name, |(obf, _)| obf);
        let set_name = b.string(obfuscated);
        b.set(&mut mechanics, keys::SET_NAME, set_name);
        b.set(&mut mechanics, keys::CURRENT_ID, int(self.level));
        b.set(&mut mechanics, keys::PREVIOUS_ID, int(self.previous));

        b.set(&mut chrono, keys::FRAME_TIMER, int(self.frame_timer));
        b.set(&mut chrono, keys::GAME_TIMER, int(self.game_timer));
        if let Some(ms) = self.halted {
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
            if let Some(cycles) = self.duration {
                let duration = b.double(cycles);
                b.set(mode, keys::DURATION, duration);
            }
            b.set(mode, keys::FL_LOCK, boolean(false));
            b.set(mode, keys::CURRENT_DIM, int(self.dim));
            if self.game_over {
                b.set(mode, keys::FL_GAME_OVER, boolean(true));
            }
        }

        if self.manager || self.corpse {
            if self.manager {
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

        Fixture {
            bytes: b,
            game_mode: game_mode.tbl,
            mode: game_mode,
            manager,
            mechanics,
            chrono,
            anchor: Anchor::default(),
            binary,
        }
    }
}

/// The bytes, and what the builder knows about them.
pub struct Fixture {
    bytes: Bytes,
    /// The address of the `GameMode` property table the reader must return.
    pub game_mode: u64,
    mode: Obj,
    manager: Obj,
    mechanics: Obj,
    chrono: Obj,
    pub anchor: Anchor,
    pub binary: Binary,
}

impl Fixture {
    /// The heap as it stands. A mutation changes the bytes, so the heap is
    /// built again for every look and every reading.
    pub fn heap(&self) -> Heap {
        Heap::default().with_range(HEAP_BASE, self.bytes.data.clone())
    }

    pub fn ranges(&self) -> Vec<(u64, u64)> {
        vec![(HEAP_BASE, HEAP_BASE + self.bytes.data.len() as u64)]
    }

    /// The memory under the game was recycled: `setName` no longer names a
    /// world the game ships.
    pub fn the_world_is_no_longer_known(&mut self) {
        let unknown = self.bytes.string("not_a_world");
        self.bytes.replace(&self.mechanics, keys::SET_NAME, unknown);
    }

    pub fn the_level_becomes(&mut self, level: i64) {
        self.bytes
            .replace(&self.mechanics, keys::CURRENT_ID, int(level));
    }

    /// The `gameChrono` of the game points at memory that cannot be read.
    pub fn the_chrono_is_lost(&mut self) {
        let nowhere = (HEAP_BASE + 0x100_0000) | atom::TAG_OBJECT;
        self.bytes.replace(&self.mode, keys::GAME_CHRONO, nowhere);
    }

    /// The game rebuilt its table, and `world` is not in the same entry any
    /// more. The index the last reading kept is now wrong.
    pub fn the_world_property_moves_slot(&mut self) {
        self.bytes.swap_entries(&self.mode, 0, 1);
    }

    /// The player starts another game. The old `GameMode` stays in memory,
    /// still readable and still plausible, and the manager points at the new
    /// one.
    pub fn the_game_is_replaced(&mut self) {
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

/// Looks for the game, and keeps what the search learned.
///
/// The anchor lives in the fixture, so a second call is the same search
/// looking again. That is what
/// `reader.find::the-same-game-when-looking-again` asks about.
pub fn when_we_look_for_the_game(f: &mut Fixture) -> Option<Game> {
    let (heap, ranges) = (f.heap(), f.ranges());
    block_on(resolve(
        &heap,
        MODULE,
        &mut f.anchor,
        &mut f.binary,
        &ranges,
    ))
}

/// Reads the state of a game already found.
pub fn when_we_read_it(f: &Fixture, game: &mut Game) -> Option<State> {
    game.read(&f.heap())
}

pub fn then_nothing_is_read(state: Option<State>) {
    assert!(
        state.is_none(),
        "the reader produced a state it should have refused: {state:?}"
    );
}

/// A state the reader produced, and the values a test may name in it.
pub struct StateCheck(State);

pub fn then_the_state(state: Option<State>) -> StateCheck {
    StateCheck(state.expect("the reader read no state"))
}

impl StateCheck {
    pub fn level(self, level: i64) -> Self {
        assert_eq!(self.0.level.id, level, "level");
        self
    }

    pub fn previous_level(self, previous: i64) -> Self {
        assert_eq!(self.0.previous, previous, "previous level");
        self
    }

    pub fn dimension(self, dim: i64) -> Self {
        assert_eq!(self.0.dim, dim, "dimension");
        self
    }

    pub fn chrono_ms(self, ms: i64) -> Self {
        assert_eq!(self.0.chrono_ms, ms, "chrono");
        self
    }

    pub fn frame_timer(self, ticks: i64) -> Self {
        assert_eq!(self.0.frame_timer, ticks, "frame timer");
        self
    }

    pub fn duration_ms(self, ms: i64) -> Self {
        assert_eq!(self.0.duration_ms, ms, "duration");
        self
    }
}

pub fn then_the_world_is(game: &Game, name: &str) {
    assert_eq!(game.set, name, "world");
}

pub fn then_nothing_is_found(found: Option<Game>) {
    assert_eq!(
        found.map(|g| g.game_mode),
        None,
        "the reader found a game where there is none to find"
    );
}

/// Asserts that the reader found the game the builder wrote, and hands it
/// back, so that a test may then read it.
pub fn then_the_game_is_found(f: &Fixture, found: Option<Game>) -> Game {
    let game = found.expect("the reader found no game");
    assert_eq!(
        game.game_mode, f.game_mode,
        "the game found is not the GameMode the builder wrote"
    );
    game
}

// -- driving the search ------------------------------------------------------

/// Drives a future to its end, with no executor.
///
/// The reader awaits `next_tick` and nothing else. Outside the runtime there
/// is nothing for it to wait on, so a bare poll loop finishes.
pub fn block_on<F: core::future::Future>(f: F) -> F::Output {
    use core::{
        pin::pin,
        task::{Context, Poll, Waker},
    };
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = f.as_mut().poll(&mut cx) {
            return value;
        }
    }
}

// -- the tests ---------------------------------------------------------------

/** @spec reader.find::a-game-and-its-manager */
#[test]
fn finds_a_game_through_its_manager() {
    let mut heap = given_a_heap().with_a_manager_and_a_game().build();

    let found = when_we_look_for_the_game(&mut heap);

    then_the_game_is_found(&heap, found);
}

/** @spec reader.find::an-orphan-game */
#[test]
fn finds_a_game_that_has_no_manager() {
    let mut heap = given_a_heap().with_a_game().build();

    let found = when_we_look_for_the_game(&mut heap);

    then_the_game_is_found(&heap, found);
}

/** @spec reader.find::rejects-a-game-already-over */
#[test]
fn finds_nothing_when_the_game_is_already_over() {
    let mut heap = given_a_heap()
        .with_a_manager_and_a_game()
        .that_is_over()
        .build();

    let found = when_we_look_for_the_game(&mut heap);

    then_nothing_is_found(found);
}

/** @spec reader.find::the-game-not-one-of-its-views */
#[test]
fn finds_the_game_and_not_one_of_its_views() {
    let mut heap = given_a_heap()
        .with_a_game()
        .and_three_views_of_that_game()
        .build();

    let found = when_we_look_for_the_game(&mut heap);

    then_the_game_is_found(&heap, found);
}

/** @spec reader.find::a-parallel-world */
#[test]
fn finds_a_game_in_a_parallel_world() {
    let mut heap = given_a_heap()
        .with_a_manager_and_a_game()
        .in_world("xml_deepnight")
        .in_dimension(1)
        .build();

    let found = when_we_look_for_the_game(&mut heap);

    let mut game = then_the_game_is_found(&heap, found);
    then_the_world_is(&game, "xml_deepnight");
    then_the_state(when_we_read_it(&heap, &mut game)).dimension(1);
}

/** @spec reader.find::the-same-game-when-looking-again */
#[test]
fn finds_the_same_game_when_it_looks_again() {
    let mut heap = given_a_heap().with_a_manager_and_a_game().build();
    let first = when_we_look_for_the_game(&mut heap);
    then_the_game_is_found(&heap, first);

    let again = when_we_look_for_the_game(&mut heap);

    then_the_game_is_found(&heap, again);
}

/** @spec reader.find::the-new-game-not-the-corpse */
#[test]
fn finds_the_new_game_and_not_the_corpse() {
    let mut heap = given_a_heap().with_a_manager_and_a_game().build();
    let first = when_we_look_for_the_game(&mut heap);
    then_the_game_is_found(&heap, first);
    heap.the_game_is_replaced();

    let found = when_we_look_for_the_game(&mut heap);

    then_the_game_is_found(&heap, found);
}

/** @spec reader.find::nothing-on-another-flash-build */
#[test]
fn finds_nothing_in_a_heap_written_by_another_flash_build() {
    let mut heap = given_a_heap()
        .with_a_manager_and_a_game()
        .written_by_another_flash_build()
        .build();

    let found = when_we_look_for_the_game(&mut heap);

    then_nothing_is_found(found);
}

/** @spec reader.find::a-key-that-is-not-a-string */
#[test]
fn finds_a_game_when_a_key_is_not_a_string() {
    let mut heap = given_a_heap()
        .with_a_manager_and_a_game()
        .with_a_key_that_is_not_a_string_first()
        .build();

    let found = when_we_look_for_the_game(&mut heap);

    then_the_game_is_found(&heap, found);
}

/** @spec reader.read::the-nominal-state */
#[test]
fn reads_the_state_of_the_game_in_progress() {
    let mut heap = given_a_heap()
        .with_a_manager_and_a_game()
        .in_world("xml_adventure")
        .at_level(2)
        .with_frame_timer(23_677)
        .with_game_timer(1_000)
        .with_duration(320.0)
        .build();
    let found = when_we_look_for_the_game(&mut heap);
    let mut game = then_the_game_is_found(&heap, found);

    let state = when_we_read_it(&heap, &mut game);

    then_the_world_is(&game, "xml_adventure");
    then_the_state(state)
        .level(2)
        .previous_level(-1)
        .dimension(0)
        .frame_timer(23_677)
        .chrono_ms(22_677)
        .duration_ms(10_000);
}

/** @spec reader.read::rejects-an-unknown-world */
#[test]
fn refuses_to_read_a_world_it_does_not_know() {
    let mut heap = given_a_heap().with_a_manager_and_a_game().build();
    let found = when_we_look_for_the_game(&mut heap);
    let mut game = then_the_game_is_found(&heap, found);
    heap.the_world_is_no_longer_known();

    let state = when_we_read_it(&heap, &mut game);

    then_nothing_is_read(state);
}

/** @spec reader.read::rejects-a-level-out-of-bounds */
#[test]
fn refuses_to_read_a_level_out_of_bounds() {
    let mut heap = given_a_heap().with_a_manager_and_a_game().build();
    let found = when_we_look_for_the_game(&mut heap);
    let mut game = then_the_game_is_found(&heap, found);
    heap.the_level_becomes(300);

    let state = when_we_read_it(&heap, &mut game);

    then_nothing_is_read(state);
}

/** @spec reader.read::rejects-a-missing-duration */
#[test]
fn refuses_to_read_a_game_whose_duration_is_missing() {
    let mut heap = given_a_heap()
        .with_a_manager_and_a_game()
        .without_a_duration()
        .build();
    let found = when_we_look_for_the_game(&mut heap);
    let mut game = then_the_game_is_found(&heap, found);

    let state = when_we_read_it(&heap, &mut game);

    then_nothing_is_read(state);
}

/** @spec reader.read::rejects-a-missing-chrono */
#[test]
fn refuses_to_read_a_game_whose_chrono_is_missing() {
    let mut heap = given_a_heap().with_a_manager_and_a_game().build();
    let found = when_we_look_for_the_game(&mut heap);
    let mut game = then_the_game_is_found(&heap, found);
    heap.the_chrono_is_lost();

    let state = when_we_read_it(&heap, &mut game);

    then_nothing_is_read(state);
}

/** @spec reader.read::the-halted-clock-when-stopped */
#[test]
fn reads_the_halted_clock_when_the_game_is_stopped() {
    let mut heap = given_a_heap()
        .with_a_manager_and_a_game()
        .with_frame_timer(23_677)
        .stopped_at(12_000)
        .build();
    let found = when_we_look_for_the_game(&mut heap);
    let mut game = then_the_game_is_found(&heap, found);

    let state = when_we_read_it(&heap, &mut game);

    then_the_state(state).chrono_ms(12_000).frame_timer(23_677);
}

/** @spec reader.read::a-property-that-moved-slot */
#[test]
fn reads_a_property_that_moved_slot() {
    let mut heap = given_a_heap()
        .with_a_manager_and_a_game()
        .at_level(2)
        .build();
    let found = when_we_look_for_the_game(&mut heap);
    let mut game = then_the_game_is_found(&heap, found);
    // The first reading keeps the index of the entry.
    then_the_state(when_we_read_it(&heap, &mut game)).level(2);
    heap.the_world_property_moves_slot();

    let state = when_we_read_it(&heap, &mut game);

    then_the_state(state).level(2);
}

/** @spec reader.find::the-mode-the-manager-owns */
#[test]
fn finds_the_mode_the_manager_owns() {
    let mut heap = given_a_heap()
        .with_a_game()
        .and_a_corpse_the_manager_has_left()
        .build();

    let found = when_we_look_for_the_game(&mut heap);

    then_the_game_is_found(&heap, found);
}

/** @spec reader.find::the-first-of-two-candidates */
#[test]
fn finds_the_first_of_two_candidates() {
    let mut heap = given_a_heap().with_a_game().and_a_second_game().build();

    let found = when_we_look_for_the_game(&mut heap);

    then_the_game_is_found(&heap, found);
}
