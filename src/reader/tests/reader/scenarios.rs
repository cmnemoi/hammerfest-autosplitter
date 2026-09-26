//! The reader's scenarios, written once and run on every Flash player.
//!
//! Three layers:
//!
//! ```text
//! the scenarios         below: what the reader must find, and read
//! the DSL               `given_a_heap`, `when_we_look_for_the_game`, `then_...`
//! the drivers           a heap written in the bytes of one player:
//!                       `pepper_flash_heap.rs`, `ruffle_heap.rs` (desktop
//!                       and browser)
//! ```
//!
//! A scenario never names a player. [`scenario!`] writes it once per driver,
//! so a red names the player that breaks: `finds_a_game_through_its_manager::
//! pepper_flash`. A scenario that belongs to one player says why.
//!
//! The rules are in `docs/specs/memory-reader.md`.

use core::marker::PhantomData;

use hammerfest_reader::hammerfest::{resolve, Anchor, Game, State};
use hammerfest_reader::heap::FlashPlayer;
use hammerfest_reader::search_log::Silent;

use crate::memory_contract::Heap;
use crate::pepper_flash_heap::PepperFlashHeapWriter;
use crate::ruffle_heap::{RuffleDesktopHeap, RuffleWebHeap};

// -- the drivers -------------------------------------------------------------

/// A world, written in the bytes of one Flash player.
pub trait WrittenHeap: Sized {
    /// The player the reader searches this heap as.
    type Player: FlashPlayer;

    /// Writes the world, and sets up the player that will search it.
    fn write(world: &World<Self>) -> (Self, Self::Player);

    /// The bytes as they stand. A mutation changes them, so the heap is
    /// served again for every look and every reading.
    fn memory(&self) -> Heap;

    fn ranges(&self) -> Vec<(u64, u64)>;

    /// The `GameMode` the reader must return, as the reader names it.
    fn game_mode(&self) -> u64;

    /// The memory under the game was recycled: `setName` no longer names a
    /// world the game ships.
    fn the_world_is_no_longer_known(&mut self);

    fn the_level_becomes(&mut self, level: i64);

    /// The `gameChrono` of the game points at memory that cannot be read.
    fn the_chrono_is_lost(&mut self);

    /// The game rebuilt the properties of its `GameMode`, and `world` is not
    /// where the last reading found it any more.
    fn the_world_property_moves(&mut self);

    /// The player starts another game. The old `GameMode` stays in memory,
    /// still readable and still plausible, and the manager points at the new
    /// one.
    fn the_game_is_replaced(&mut self);
}

type HeapOf<W> = <<W as WrittenHeap>::Player as FlashPlayer>::Heap;
pub type GameIn<W> = Game<HeapOf<W>>;

/// Writes a scenario once per driver.
///
/// Inside the body, `given_a_heap()` gives a heap of the driver under test.
macro_rules! scenario {
    ($(#[$meta:meta])* fn $name:ident() $body:block) => {
        mod $name {
            use super::*;

            $(#[$meta])*
            #[test]
            fn pepper_flash() {
                // An item and not a `let`: macro hygiene hides a local
                // binding from the body, and not an item.
                fn given_a_heap() -> World<PepperFlashHeapWriter> {
                    World::default()
                }
                $body
            }

            $(#[$meta])*
            #[test]
            fn ruffle() {
                fn given_a_heap() -> World<RuffleDesktopHeap> {
                    World::default()
                }
                $body
            }

            $(#[$meta])*
            #[test]
            fn ruffle_web() {
                fn given_a_heap() -> World<RuffleWebHeap> {
                    World::default()
                }
                $body
            }
        }
    };
}

// -- the DSL -----------------------------------------------------------------

/// What the heap must hold. The bytes are written once, by [`World::build`].
///
/// A description first, and the bytes at the end, because the objects cite
/// each other: the manager points at the game, and the game points back. One
/// single pass would mean patching what was already written.
pub struct World<W> {
    pub manager: bool,
    pub game_over: bool,
    pub views: usize,
    pub second_game: bool,
    pub corpse: bool,
    pub another_build: bool,
    pub foreign_keys: bool,
    pub set_name: &'static str,
    pub dim: i64,
    pub level: i64,
    pub previous: i64,
    pub frame_timer: i64,
    pub game_timer: i64,
    /// Absent when the game does not carry the property at all.
    pub duration: Option<f64>,
    /// `Chrono.haltedTimer`, and `fl_stop` with it.
    pub halted: Option<i64>,
    /// No object at all: the heap of a player in its menus.
    pub empty: bool,
    written_by: PhantomData<W>,
}

impl<W> Default for World<W> {
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
            empty: false,
            written_by: PhantomData,
        }
    }
}

impl<W: WrittenHeap> World<W> {
    /// Nothing the reader looks for: no `GameManager`, no `GameMode`.
    pub fn with_nothing_in_it(mut self) -> Self {
        self.empty = true;
        self
    }

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

    pub fn build(self) -> Fixture<W> {
        let (written, player) = W::write(&self);
        Fixture {
            written,
            player,
            anchor: Anchor::default(),
        }
    }
}

/// The written heap, and the search that keeps what it learned about it.
pub struct Fixture<W: WrittenHeap> {
    pub written: W,
    pub player: W::Player,
    pub anchor: Anchor<HeapOf<W>>,
}

impl<W: WrittenHeap> Fixture<W> {
    pub fn the_world_is_no_longer_known(&mut self) {
        self.written.the_world_is_no_longer_known();
    }

    pub fn the_level_becomes(&mut self, level: i64) {
        self.written.the_level_becomes(level);
    }

    pub fn the_chrono_is_lost(&mut self) {
        self.written.the_chrono_is_lost();
    }

    pub fn the_world_property_moves(&mut self) {
        self.written.the_world_property_moves();
    }

    pub fn the_game_is_replaced(&mut self) {
        self.written.the_game_is_replaced();
    }
}

/// Looks for the game, and keeps what the search learned.
///
/// The anchor lives in the fixture, so a second call is the same search
/// looking again. That is what
/// `reader.find::the-same-game-when-looking-again` asks about.
pub fn when_we_look_for_the_game<W: WrittenHeap>(f: &mut Fixture<W>) -> Option<GameIn<W>> {
    let (memory, ranges) = (f.written.memory(), f.written.ranges());
    block_on(resolve(
        &memory,
        &mut f.player,
        &mut f.anchor,
        &ranges,
        &mut Silent,
    ))
}

/// Reads the state of a game already found.
pub fn when_we_read_it<W: WrittenHeap>(f: &Fixture<W>, game: &mut GameIn<W>) -> Option<State> {
    game.read(&f.written.memory())
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

pub fn then_the_world_is<H>(game: &Game<H>, name: &str) {
    assert_eq!(game.set, name, "world");
}

pub fn then_nothing_is_found<H>(found: Option<Game<H>>) {
    assert_eq!(
        found.map(|g| g.game_mode),
        None,
        "the reader found a game where there is none to find"
    );
}

/// Asserts that the reader found the game the builder wrote, and hands it
/// back, so that a test may then read it.
pub fn then_the_game_is_found<W: WrittenHeap>(
    f: &Fixture<W>,
    found: Option<GameIn<W>>,
) -> GameIn<W> {
    let game = found.expect("the reader found no game");
    assert_eq!(
        game.game_mode,
        f.written.game_mode(),
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

// -- the scenarios, on every player ------------------------------------------

scenario! {
    /** @spec reader.find::nothing-in-the-menus */
    fn finds_nothing_in_an_empty_heap() {
        let mut heap = given_a_heap().with_nothing_in_it().build();

        let found = when_we_look_for_the_game(&mut heap);

        then_nothing_is_found(found);
    }
}

scenario! {
    /** @spec reader.find::a-game-and-its-manager */
    fn finds_a_game_through_its_manager() {
        let mut heap = given_a_heap().with_a_manager_and_a_game().build();

        let found = when_we_look_for_the_game(&mut heap);

        then_the_game_is_found(&heap, found);
    }
}

scenario! {
    /** @spec reader.find::an-orphan-game */
    fn finds_a_game_that_has_no_manager() {
        let mut heap = given_a_heap().with_a_game().build();

        let found = when_we_look_for_the_game(&mut heap);

        then_the_game_is_found(&heap, found);
    }
}

scenario! {
    /** @spec reader.find::rejects-a-game-already-over */
    fn finds_nothing_when_the_game_is_already_over() {
        let mut heap = given_a_heap()
            .with_a_manager_and_a_game()
            .that_is_over()
            .build();

        let found = when_we_look_for_the_game(&mut heap);

        then_nothing_is_found(found);
    }
}

scenario! {
    /** @spec reader.find::the-game-not-one-of-its-views */
    fn finds_the_game_and_not_one_of_its_views() {
        let mut heap = given_a_heap()
            .with_a_game()
            .and_three_views_of_that_game()
            .build();

        let found = when_we_look_for_the_game(&mut heap);

        then_the_game_is_found(&heap, found);
    }
}

scenario! {
    /** @spec reader.find::a-parallel-world */
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
}

scenario! {
    /** @spec reader.find::the-same-game-when-looking-again */
    fn finds_the_same_game_when_it_looks_again() {
        let mut heap = given_a_heap().with_a_manager_and_a_game().build();
        let first = when_we_look_for_the_game(&mut heap);
        then_the_game_is_found(&heap, first);

        let again = when_we_look_for_the_game(&mut heap);

        then_the_game_is_found(&heap, again);
    }
}

scenario! {
    /** @spec reader.find::the-new-game-not-the-corpse */
    fn finds_the_new_game_and_not_the_corpse() {
        let mut heap = given_a_heap().with_a_manager_and_a_game().build();
        let first = when_we_look_for_the_game(&mut heap);
        then_the_game_is_found(&heap, first);
        heap.the_game_is_replaced();

        let found = when_we_look_for_the_game(&mut heap);

        then_the_game_is_found(&heap, found);
    }
}

scenario! {
    /** @spec reader.find::the-mode-the-manager-owns */
    fn finds_the_mode_the_manager_owns() {
        let mut heap = given_a_heap()
            .with_a_game()
            .and_a_corpse_the_manager_has_left()
            .build();

        let found = when_we_look_for_the_game(&mut heap);

        then_the_game_is_found(&heap, found);
    }
}

scenario! {
    /** @spec reader.find::the-first-of-two-candidates */
    fn finds_the_first_of_two_candidates() {
        let mut heap = given_a_heap().with_a_game().and_a_second_game().build();

        let found = when_we_look_for_the_game(&mut heap);

        then_the_game_is_found(&heap, found);
    }
}

scenario! {
    /** @spec reader.read::the-nominal-state */
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
}

scenario! {
    /** @spec reader.read::rejects-an-unknown-world */
    fn refuses_to_read_a_world_it_does_not_know() {
        let mut heap = given_a_heap().with_a_manager_and_a_game().build();
        let found = when_we_look_for_the_game(&mut heap);
        let mut game = then_the_game_is_found(&heap, found);
        heap.the_world_is_no_longer_known();

        let state = when_we_read_it(&heap, &mut game);

        then_nothing_is_read(state);
    }
}

scenario! {
    /** @spec reader.read::rejects-a-level-out-of-bounds */
    fn refuses_to_read_a_level_out_of_bounds() {
        let mut heap = given_a_heap().with_a_manager_and_a_game().build();
        let found = when_we_look_for_the_game(&mut heap);
        let mut game = then_the_game_is_found(&heap, found);
        heap.the_level_becomes(300);

        let state = when_we_read_it(&heap, &mut game);

        then_nothing_is_read(state);
    }
}

scenario! {
    /** @spec reader.read::rejects-a-missing-duration */
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
}

scenario! {
    /** @spec reader.read::rejects-a-missing-chrono */
    fn refuses_to_read_a_game_whose_chrono_is_missing() {
        let mut heap = given_a_heap().with_a_manager_and_a_game().build();
        let found = when_we_look_for_the_game(&mut heap);
        let mut game = then_the_game_is_found(&heap, found);
        heap.the_chrono_is_lost();

        let state = when_we_read_it(&heap, &mut game);

        then_nothing_is_read(state);
    }
}

scenario! {
    /** @spec reader.read::the-halted-clock-when-stopped */
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
}

scenario! {
    /** @spec reader.read::a-property-that-moved-slot */
    /** @spec ruffle.read::entries-that-moved */
    fn reads_a_property_that_moved() {
        let mut heap = given_a_heap()
            .with_a_manager_and_a_game()
            .at_level(2)
            .build();
        let found = when_we_look_for_the_game(&mut heap);
        let mut game = then_the_game_is_found(&heap, found);
        // The first reading keeps where the property sat.
        then_the_state(when_we_read_it(&heap, &mut game)).level(2);
        heap.the_world_property_moves();

        let state = when_we_read_it(&heap, &mut game);

        then_the_state(state).level(2);
    }
}

// -- the scenarios of Pepper Flash alone -------------------------------------

/// A heap in the bytes of Pepper Flash, for the traps only that player sets.
fn given_a_pepper_flash_heap() -> World<PepperFlashHeapWriter> {
    World::default()
}

/// Pepper Flash only: the String vtable of another build of the plugin sits
/// elsewhere in its module. Ruffle has no such seed to go stale.
/** @spec reader.find::nothing-on-another-flash-build */
#[test]
fn finds_nothing_in_a_heap_written_by_another_flash_build() {
    let mut heap = given_a_pepper_flash_heap()
        .with_a_manager_and_a_game()
        .written_by_another_flash_build()
        .build();

    let found = when_we_look_for_the_game(&mut heap);

    then_nothing_is_found(found);
}

/// Pepper Flash only: its tables are found by walking back from a key, and
/// the Linux player puts a key that is not a String object in the way. A
/// Ruffle map is found from its own header.
/** @spec reader.find::a-key-that-is-not-a-string */
#[test]
fn finds_a_game_when_a_key_is_not_a_string() {
    let mut heap = given_a_pepper_flash_heap()
        .with_a_manager_and_a_game()
        .with_a_key_that_is_not_a_string_first()
        .build();

    let found = when_we_look_for_the_game(&mut heap);

    then_the_game_is_found(&heap, found);
}

// -- the scenarios of Ruffle alone -------------------------------------------

/// Writes a scenario of Ruffle alone once per Ruffle driver: on the desktop,
/// and in a browser.
macro_rules! ruffle_scenario {
    ($(#[$meta:meta])* fn $name:ident() $body:block) => {
        mod $name {
            use super::*;

            $(#[$meta])*
            #[test]
            fn ruffle() {
                fn given_a_ruffle_heap() -> World<RuffleDesktopHeap> {
                    World::default()
                }
                $body
            }

            $(#[$meta])*
            #[test]
            fn ruffle_web() {
                fn given_a_ruffle_heap() -> World<RuffleWebHeap> {
                    World::default()
                }
                $body
            }
        }
    };
}

ruffle_scenario! {
    /// Ruffle only: Pepper Flash keeps no hash beside its keys.
    /** @spec ruffle.read::an-entry-whose-hash-lies */
    fn refuses_to_read_an_entry_whose_hash_lies() {
        let mut heap = given_a_ruffle_heap()
            .with_a_manager_and_a_game()
            .at_level(2)
            .build();
        let found = when_we_look_for_the_game(&mut heap);
        let mut game = then_the_game_is_found(&heap, found);
        // The first reading remembers where `currentId` sits.
        then_the_state(when_we_read_it(&heap, &mut game)).level(2);
        heap.written.the_level_entry_holds_another_hash();

        let state = when_we_read_it(&heap, &mut game);

        then_nothing_is_read(state);
    }
}

ruffle_scenario! {
    /// Ruffle only: the type of a Pepper Flash object is proven by the vtable
    /// of its table, which the Pepper Flash scenarios already hold.
    /** @spec ruffle.find::a-map-that-is-not-an-object */
    fn finds_nothing_in_a_map_that_is_not_an_object() {
        let mut heap = given_a_ruffle_heap().with_a_game().build();
        heap.written.the_game_is_not_an_object();

        let found = when_we_look_for_the_game(&mut heap);

        then_nothing_is_found(found);
    }
}
