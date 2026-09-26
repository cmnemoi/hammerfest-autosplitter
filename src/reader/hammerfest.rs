//! Resolution of the Hammerfest state inside the Flash plugin process.
//!
//! No static pointer path leads to these values. They are not C variables but
//! properties of ActionScript 2 objects, created at run time by a downloaded
//! SWF. The anchor is therefore an interned string of the SWF, and everything
//! else follows from it.
//!
//! ```text
//! scan "]=[]8" in the heap    `world`, obfuscated name known from hf.map.json
//!   -> String object          the qword in front that points into the module
//!                             is the vtable
//!   -> slots citing the string, scanned over the 8 atom encodings
//!   -> GameMode table         the one that owns this key
//! GameMode.world              -> GameMechanics
//!   .setName                  -> a known Hammerfest world      check
//!   .currentId                -> the level
//! GameMode.gameChrono         -> fl_stop ? haltedTimer : frameTimer-gameTimer
//! ```

use alloc::vec::Vec;

use crate::avm1::{Layout, Memory};
use crate::heap::{Avm1Heap, Object, Slot};
use crate::keys;
use crate::pepper_flash::{find_string, scan_tables, Binary, PepperFlashHeap};
use crate::scan::{move_region_first, Scan};
use crate::search_log::SearchLog;

const MAX_LEVEL: i64 = 256;

/// Where each property sat at the last read, always checked again.
#[derive(Default)]
struct Hints {
    world: Slot,
    chrono: Slot,
    current_id: Slot,
    previous_id: Slot,
    set_name: Slot,
    dim: Slot,
    game_over: Slot,
    frame: Slot,
    game: Slot,
    halted: Slot,
    stop: Slot,
    lock: Slot,
    duration: Slot,
    end_mode: Slot,
}

pub struct Game {
    heap: PepperFlashHeap,
    pub game_mode: u64,
    pub set: &'static str,
    hints: Hints,
}

pub use hammerfest_core::{EndSequence, Level, State, World};

// -- resolution -------------------------------------------------------------

/// What one resolution learns and the next ones reuse.
///
/// Nothing here is the address of a game object -- that would be the trap. The
/// two addresses kept belong to objects that live as long as the SWF: the
/// interned string of a key, and the `GameManager` table. They are checked
/// again before every use, and the `Anchor` is cleared for every new plugin
/// process.
#[derive(Default)]
pub struct Anchor {
    /// The String objects of the anchor keys, interned by the SWF.
    string_world: Option<u64>,
    string_version: Option<u64>,
    /// The property table of the `GameManager`, and the layout that goes with
    /// it.
    ///
    /// This is the anchor the game offers by itself. `GameManager` is born
    /// with the SWF and points at the mode that runs (`transition()` writes
    /// every new mode into it). Finding it **during the menus** means we never
    /// scan again: when the game starts, it is at the end of a pointer.
    manager: Option<u64>,
    layout: Option<Layout>,
    current_hint: Slot,
    /// Has the anchor ever delivered a game?
    ///
    /// While it has not, the fallback scan stays allowed. A wrong anchor -- or
    /// simply one we cannot use -- must not be able to block all detection on
    /// its own. That is what happened when the `ScriptObject -> table` offset
    /// was not derived on that path.
    ///
    /// But the permission cannot last. The scan blocks the loop for one or two
    /// seconds, and while no game has been seen -- so exactly when we wait for
    /// the launch -- it would hide the click. So we trust the anchor at once,
    /// and we question it only after a long silence.
    manager_proven: bool,
    /// Failed attempts since the anchor was set.
    manager_idle: u32,
    /// The region where the last GameMode was found. It is scanned first.
    last_game_mode: Option<u64>,
    /// The regions seen at the last scan, bounds included.
    ///
    /// Anything not in this list is new: a freshly committed region, or a
    /// region that grew. That is where the SWF objects are born -- the scans
    /// that succeed read only sixteen MiB, the ones that fail re-read two
    /// hundred. See `resolve`.
    regions: Vec<(u64, u64)>,
    /// Attempts since the last full scan.
    sweeps: u32,
}

impl Anchor {
    /// Addresses learned in one process mean nothing in the next one: ASLR
    /// moves them, and the AVM1 heap is built again.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Failed attempts allowed before we question an anchor that never gave
/// anything. At about one per second, that leaves a good half minute before
/// the scan starts again.
const MANAGER_IDLE_LIMIT: u32 = 30;

/// Attempts with no new region before we read the whole heap again.
///
/// The safety net for the rule above: an object can be born in memory that is
/// already committed, and no new region would signal it.
const FULL_SWEEP: u32 = 8;

/// Fast path: follows `GameManager.current`, with no scan at all.
///
/// A few reads, against a hundred MiB. That is what lets us look for the game
/// on every tick. It returns None if the anchor is not learned yet, if the
/// GameManager table moved, or if the current mode is not a playable game -- a
/// menu, for example.
pub fn resolve_via_manager(mem: &dyn Memory, anchor: &mut Anchor) -> Option<Game> {
    let heap = PepperFlashHeap::new(anchor.layout?);
    let manager = Object(anchor.manager?);
    if !heap.is_object(mem, manager)? {
        anchor.manager = None;
        return None;
    }
    // No more `current`: this is not the GameManager any more, the table must
    // have moved. Dropping the anchor starts a scan instead of staying blind.
    let Some(current) = heap.property(mem, manager, keys::CURRENT, &mut anchor.current_hint) else {
        anchor.manager = None;
        return None;
    };
    let tbl = heap.object(mem, current.as_object()?)?.0;
    let game = validate(mem, heap, tbl)?;
    anchor.last_game_mode = Some(tbl);
    anchor.manager_proven = true;
    anchor.manager_idle = 0;
    Some(game)
}

/// Looks for the game in progress.
///
/// Three stages, from the cheapest to the most expensive:
///
/// 1. follow `GameManager.current`, if the anchor is known;
/// 2. otherwise, locate the `GameManager` -- it exists as soon as the SWF is
///    loaded, so this search already succeeds in the menus, before any game;
/// 3. as a last resort, look for a `GameMode` directly by its `world` key.
///    This only serves when stage 2 fails.
pub async fn resolve(
    mem: &dyn Memory,
    module: (u64, u64),
    anchor: &mut Anchor,
    binary: &mut Binary,
    ranges: &[(u64, u64)],
    log: &mut dyn SearchLog,
) -> Option<Game> {
    if let Some(game) = resolve_via_manager(mem, anchor) {
        return Some(game);
    }

    let mut cost = Scan::new(log);
    let all = ranges.to_vec();
    if all.is_empty() {
        return None;
    }

    // Scan only what changed.
    //
    // The SWF objects are all born together, in memory that has just been
    // committed. The scans that succeed read only sixteen MiB; the ones that
    // failed re-read two hundred for nothing. And the price of those failures
    // decides everything -- while a useless scan runs, level 0 appears, and
    // the timer starts late.
    //
    // A region that is not in the previous list is new, or it grew. When there
    // is none, nothing can have been born since last time: the scan would
    // learn nothing, so we skip it. Now and then, all the same, a full pass --
    // for the case where an object is born in memory that is already
    // committed, which this rule would never see.
    let mut ranges: Vec<(u64, u64)> = all
        .iter()
        .filter(|r| !anchor.regions.contains(r))
        .copied()
        .collect();
    if ranges.is_empty() {
        anchor.sweeps += 1;
        if !anchor.sweeps.is_multiple_of(FULL_SWEEP) {
            cost.outcome("unchanged_ranges");
            return None;
        }
        ranges = all.clone();
    }
    anchor.regions = all.clone();
    // Tables are searched everywhere, even when the string is only searched
    // in the new memory.
    let mut full = all;
    if let Some(addr) = anchor.last_game_mode {
        move_region_first(&mut ranges, addr);
        move_region_first(&mut full, addr);
    }

    // Set the anchor first. Once the GameManager is known, no scan is needed
    // any more, and the launch of a game is seen in a few reads instead of
    // half a second to several seconds.
    if anchor.manager.is_none() {
        cost.stage("manager");
        if let Some((layout, tbl)) =
            scan_for_manager(mem, module, &ranges, &mut full, anchor, binary, &mut cost).await
        {
            cost.log.manager_found(tbl, layout.profile.name);
            binary.learn(&layout);
            anchor.layout = Some(layout);
            anchor.manager = Some(tbl);
            anchor.current_hint = Slot::default();
            let game = resolve_via_manager(mem, anchor);
            cost.outcome(if game.is_some() {
                "game_via_manager"
            } else {
                "manager_only"
            });
            return game;
        }
    }

    // The anchor is set but `current` points at no game: there simply is
    // none. Nothing to scan, that is already the answer -- and above all, not
    // scanning leaves the loop free to see the game start on the very next
    // tick.
    if anchor.manager.is_some() {
        anchor.manager_idle += 1;
        if anchor.manager_proven || anchor.manager_idle < MANAGER_IDLE_LIMIT {
            return None;
        }
        // Silent for a long time and never proven: the anchor may be the
        // problem itself. We drop it and search again.
        cost.log.silent_anchor_dropped();
        anchor.manager = None;
        anchor.manager_idle = 0;
        return None;
    }

    // Fallback: look for the GameMode itself, anchored on `world` -- a key
    // only a handful of objects carry.
    cost.stage("world_string");
    let (layout, strobj) = find_string(
        mem,
        module,
        &ranges,
        keys::WORLD,
        &mut anchor.string_world,
        binary,
        &mut cost,
    )
    .await?;
    // The interned string and the tables that cite it live in the same AVM1
    // heap. Starting with its region usually saves us from reading the rest,
    // and that is what makes the duration stable from one time to the next.
    move_region_first(&mut full, strobj);

    cost.stage("world_tables");
    let game = scan_tables(
        mem,
        &full,
        layout,
        strobj,
        keys::WORLD,
        &mut cost,
        |l, t| validate(mem, PepperFlashHeap::new(l), t),
    )
    .await?;

    let layout = game.heap.layout();
    cost.log
        .game_mode_found(game.game_mode, game.set, layout.profile.name);
    binary.learn(&layout);
    anchor.last_game_mode = Some(game.game_mode);
    anchor.layout = Some(layout);
    // `Mode.manager` leads to the GameManager. Keeping it does not shorten
    // the start of a game -- the objects die with the game, so the anchor does
    // not survive to the next one -- but it makes any new resolution inside
    // the same game free.
    anchor.manager = layout.child(mem, game.game_mode, keys::MANAGER);
    anchor.current_hint = Slot::default();
    cost.outcome("game_via_world");
    Some(game)
}

/// Locates the `GameManager`, the anchor that makes every later detection
/// immediate.
///
/// It is anchored on `fVersion`, which its constructor sets and which no other
/// class carries. A common key such as `current` -- which every SetManager has
/// -- would be cited dozens of times, and every candidate costs the rebuild of
/// a table.
///
/// The point is the timing: the Flash plugin exists as soon as the application
/// opens, so long before a game starts. This scan has all the time it needs
/// while the player is still on the loading screens, and the game that starts
/// next is seen in a few reads.
async fn scan_for_manager(
    mem: &dyn Memory,
    module: (u64, u64),
    fresh: &[(u64, u64)],
    ranges: &mut [(u64, u64)],
    anchor: &mut Anchor,
    binary: &mut Binary,
    cost: &mut Scan<'_>,
) -> Option<(Layout, u64)> {
    // The string is only searched in what changed -- that is where the SWF
    // has just created it. The tables that cite it, on the other hand, are
    // searched everywhere: once the string is found, we know a game exists,
    // and the next pass stops at the first valid table.
    cost.stage("manager_string");
    let (layout, strobj) = find_string(
        mem,
        module,
        fresh,
        keys::F_VERSION,
        &mut anchor.string_version,
        binary,
        cost,
    )
    .await?;

    // The interned string and the tables that cite it live in the same AVM1
    // heap. Starting with its region usually saves us from reading the rest.
    // That is what made the duration stable on the fallback path, and it was
    // missing here.
    move_region_first(ranges, strobj);

    cost.stage("manager_tables");
    scan_tables(
        mem,
        ranges,
        layout,
        strobj,
        keys::F_VERSION,
        cost,
        |mut l, t| {
            // The cross reference proves the candidate *and* derives the
            // `ScriptObject -> table` offset on the way. Without that offset,
            // nothing below can be read: `GameManager.current` points at a mode
            // whose `manager` field points back at that same GameManager.
            let current = l.get(mem, t, keys::CURRENT)?;
            let mode = l.derive_so_tbl(mem, current, keys::MANAGER)?;
            let back = l.child(mem, mode, keys::MANAGER)?;
            (back == t).then_some((l, t))
        },
    )
    .await
}

/// Is a table that owns `world` really the GameMode?
///
/// `world` alone is not enough: `View` objects carry one too, and they point
/// at the same `GameMechanics`.
///
/// The proof is the `manager` back-pointer, the same one the `GameManager`
/// path uses in the other direction. A mode that names no manager is kept on
/// weaker evidence: it owns a `gameChrono`, and a `View` does not.
///
/// The rules a candidate must pass before it is a game.
///
/// @spec reader::it-is-a-game-mode
/// @spec reader::a-known-world
/// @spec reader::a-level-in-range
fn validate(mem: &dyn Memory, mut heap: PepperFlashHeap, tbl: u64) -> Option<Game> {
    let game_mode = Object(tbl);
    let property =
        |heap: &PepperFlashHeap, object, key| heap.property(mem, object, key, &mut Slot::default());

    let world = property(&heap, game_mode, keys::WORLD)?.as_object()?;
    let world = heap.object_owning(mem, world, keys::SET_NAME)?;

    let set_name = property(&heap, world, keys::SET_NAME)?.as_string()?;
    let set = keys::WORLDS
        .iter()
        .find(|(obf, _)| heap.string_is(mem, set_name, obf))
        .map(|&(_, clear)| clear)?;

    let level = property(&heap, world, keys::CURRENT_ID)?.as_whole_number()?;
    if !(0..MAX_LEVEL).contains(&level) {
        return None;
    }

    // The manager names one mode, the one that runs. A mode it does not name
    // is a game that is over, and the manager has already moved on to the
    // next. That is the positive half of the identification.
    //
    // A mode that names no manager at all is not refused here. An orphan game
    // has no manager to ask, and the `gameChrono` below is then the only
    // evidence left.
    let child = |heap: &PepperFlashHeap, object, key| {
        heap.object(mem, property(heap, object, key)?.as_object()?)
    };
    if let Some(manager) = child(&heap, game_mode, keys::MANAGER) {
        if child(&heap, manager, keys::CURRENT) != Some(game_mode) {
            return None;
        }
    }

    let chrono = child(&heap, game_mode, keys::GAME_CHRONO)?;
    property(&heap, chrono, keys::FRAME_TIMER)?.as_whole_number()?;

    // A GameMode already in game over is a finished GameMode. Holding on to
    // it would read a game that is over instead of waiting for the next one.
    if property(&heap, game_mode, keys::FL_GAME_OVER).and_then(|value| value.as_bool())
        == Some(true)
    {
        return None;
    }

    Some(Game {
        heap,
        game_mode: tbl,
        set,
        hints: Hints::default(),
    })
}

// -- reading ----------------------------------------------------------------

impl Game {
    /// The current state, or None if the resolution is no longer valid.
    ///
    /// Everything is read again from GameMode on every call. Keeping the final
    /// address would be dangerous: the game rebuilds its objects between two
    /// games, and the abandoned slot stays readable, holding a plausible
    /// value.
    ///
    /// The world and the level are checked again here, and not only when the
    /// game is found, because memory is recycled.
    ///
    /// @spec reader::a-true-state-or-none
    /// @spec reader::a-known-world
    /// @spec reader::a-level-in-range
    pub fn read(&mut self, mem: &dyn Memory) -> Option<State> {
        let (chrono_ms, frame_timer) = self.chrono(mem)?;
        let heap = self.heap;
        let hints = &mut self.hints;
        let game_mode = Object(self.game_mode);
        let world = heap.property(mem, game_mode, keys::WORLD, &mut hints.world)?;
        let world = heap.object(mem, world.as_object()?)?;

        // The world must always be a known world. That is what detects that
        // we now read recycled memory.
        //
        // It also says which world, and so which numbering, `currentId`
        // belongs to. Both come from this one object, so they always agree.
        // `GameMode.currentDim` does not: it changes two seconds before
        // `world` does, on the way into a dimension.
        let set_name = heap
            .property(mem, world, keys::SET_NAME, &mut hints.set_name)?
            .as_string()?;
        let set = keys::WORLDS
            .iter()
            .find(|(obf, _)| heap.string_is(mem, set_name, obf))
            .and_then(|&(_, clear)| World::from_set_name(clear))?;

        let level = heap
            .property(mem, world, keys::CURRENT_ID, &mut hints.current_id)?
            .as_whole_number()?;
        if !(0..MAX_LEVEL).contains(&level) {
            return None;
        }
        let previous = heap
            .property(mem, world, keys::PREVIOUS_ID, &mut hints.previous_id)
            .and_then(|value| value.as_whole_number())
            .unwrap_or(-1);

        Some(State {
            level: Level::new(set, level),
            previous,
            chrono_ms,
            frame_timer,
            // `fl_lock` is true during the black screen before level 0. Its
            // fall is the official start of the run.
            locked: heap
                .property(mem, game_mode, keys::FL_LOCK, &mut hints.lock)
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            // Required, like the level and the clock: this is what dates the
            // start of the run. Reading it as zero by default would set the
            // origin at the instant of the resolution, so a timer short by all
            // the delay of the scan -- and silently so. Better to declare the
            // read invalid and scan again.
            duration_ms: hammerfest_core::duration_ms(
                heap.property(mem, game_mode, keys::DURATION, &mut hints.duration)?
                    .as_number()?,
            ),
            dim: heap
                .property(mem, game_mode, keys::CURRENT_DIM, &mut hints.dim)
                .and_then(|value| value.as_whole_number())
                .unwrap_or(0),
            game_over: heap
                .property(mem, game_mode, keys::FL_GAME_OVER, &mut hints.game_over)
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            // The end of the run. `GameMode.endModeTimer` is a Float that the
            // elevator script raises to fourteen seconds of cycles, and no
            // other line of an adventure writes it. What it means is in
            // `EndSequence`; here we only decode the number.
            //
            // Absent or unreadable, the sequence has not started. A run that
            // does not end by itself is a nuisance; a run that ends by
            // accident is a lost run.
            end_sequence: heap
                .property(mem, game_mode, keys::END_MODE_TIMER, &mut hints.end_mode)
                .and_then(|value| value.as_number())
                .map_or(EndSequence::NONE, EndSequence::from_cycles),
        })
    }

    /// `Chrono.get()` in milliseconds, and the raw `frameTimer`.
    ///
    /// ```mt
    /// function get() {
    ///     if ( fl_stop )  return haltedTimer;
    ///     else            return Math.floor( frameTimer-gameTimer );
    /// }
    /// ```
    fn chrono(&mut self, mem: &dyn Memory) -> Option<(i64, i64)> {
        let heap = self.heap;
        let hints = &mut self.hints;
        let chrono = heap.property(
            mem,
            Object(self.game_mode),
            keys::GAME_CHRONO,
            &mut hints.chrono,
        )?;
        let chrono = heap.object(mem, chrono.as_object()?)?;

        let frame = heap
            .property(mem, chrono, keys::FRAME_TIMER, &mut hints.frame)?
            .as_whole_number()?;

        let stopped = heap
            .property(mem, chrono, keys::FL_STOP, &mut hints.stop)
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if stopped {
            if let Some(halted) = heap
                .property(mem, chrono, keys::HALTED_TIMER, &mut hints.halted)
                .and_then(|value| value.as_whole_number())
            {
                return Some((halted, frame));
            }
        }

        let game = heap
            .property(mem, chrono, keys::GAME_TIMER, &mut hints.game)?
            .as_whole_number()?;
        Some((frame - game, frame))
    }
}
