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

use alloc::{vec, vec::Vec};
use asr::future::next_tick;

use crate::avm1::{self, read_u64, Layout, Memory, PROFILES, STR_BUF_CANDIDATES};
use crate::keys;

/// Size of a read block. The heap is about a hundred MiB. At 64 KiB that was
/// a good thousand runtime calls per pass, and each call costs far more than
/// the bytes it brings back.
const CHUNK: usize = 1024 * 1024;
/// Overlap between two blocks, so a pattern across the edge is not missed.
const OVERLAP: usize = 32;
/// Number of blocks read before we yield to the runtime.
#[cfg(not(feature = "scan-budget"))]
const CHUNKS_PER_TICK: usize = 8;
// The same volume ceiling as eight blocks of 1 MiB. The call ceiling limits
// the work when the map holds many small regions.
#[cfg(feature = "scan-budget")]
const BYTES_PER_TICK: u64 = 8 * CHUNK as u64;
#[cfg(feature = "scan-budget")]
const READS_PER_TICK: u64 = 128;

const MAX_LEVEL: i64 = 256;

/// What the scan must count in order to steer itself, and nothing more.
///
/// The budget decides when to yield to the runtime. Everything that only
/// measures -- durations, stages, outcome -- lives in `trace`, which does not
/// exist in the normal build.
#[derive(Default)]
struct Scan {
    /// Reads asked of the runtime, and the bytes they carried.
    calls: u64,
    requested: u64,
    #[cfg(feature = "scan-budget")]
    last_yield_requested: u64,
    #[cfg(feature = "scan-budget")]
    last_yield_calls: u64,
    trace: crate::diagnostics::ScanTrace,
}

impl Scan {
    fn read_block(&mut self, mem: &dyn Memory, base: u64, buf: &mut [u8]) -> bool {
        self.calls += 1;
        self.requested += buf.len() as u64;
        let ok = mem.read_into(base, buf).is_some();
        self.trace.read(buf.len(), ok);
        ok
    }

    fn paused(&mut self) {
        self.trace.paused();
        #[cfg(feature = "scan-budget")]
        {
            self.last_yield_requested = self.requested;
            self.last_yield_calls = self.calls;
        }
    }

    /// Yield on a volume, and not on a number of blocks.
    ///
    /// A map made of many small regions gave one pause per region -- so one
    /// pause for a few KiB read, where eight blocks of 1 MiB allow one pause
    /// for eight MiB.
    fn should_yield(&self, _chunks: usize) -> bool {
        #[cfg(feature = "scan-budget")]
        {
            self.requested - self.last_yield_requested >= BYTES_PER_TICK
                || self.calls - self.last_yield_calls >= READS_PER_TICK
        }
        #[cfg(not(feature = "scan-budget"))]
        {
            _chunks % CHUNKS_PER_TICK == 0
        }
    }

    /// Marks the current stage. Without the `diagnostics` feature, it does
    /// nothing.
    fn stage(&mut self, next: &'static str) {
        self.trace.stage(next, self.requested, self.calls);
    }

    /// The outcome of the attempt, for the trace. Without the feature, it
    /// does nothing.
    fn outcome(&mut self, outcome: &'static str) {
        self.trace.outcome(outcome);
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.trace.stage("done", self.requested, self.calls);
        self.trace.finish(self.requested, self.calls);
    }
}

/// Entry indexes kept from one read to the next, always checked again.
#[derive(Default)]
struct Hints {
    world: u64,
    chrono: u64,
    current_id: u64,
    previous_id: u64,
    set_name: u64,
    dim: u64,
    game_over: u64,
    frame: u64,
    game: u64,
    halted: u64,
    stop: u64,
    lock: u64,
    duration: u64,
    end_mode: u64,
}

pub struct Game {
    pub layout: Layout,
    pub game_mode: u64,
    pub set: &'static str,
    hints: Hints,
}

pub use hammerfest_core::{EndSequence, Level, State, World};

// -- scans ------------------------------------------------------------------

/// Every aligned address where `pat` appears.
async fn scan_bytes(
    mem: &dyn Memory,
    ranges: &[(u64, u64)],
    pat: &[u8],
    align: usize,
    limit: usize,
    cost: &mut Scan,
) -> Vec<u64> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; CHUNK];
    let mut chunks = 0usize;

    for &(start, end) in ranges {
        let mut base = start;
        while base < end {
            let n = core::cmp::min(CHUNK as u64, end - base) as usize;
            if cost.read_block(mem, base, &mut buf[..n]) {
                let mut i = 0;
                while i + pat.len() <= n {
                    if &buf[i..i + pat.len()] == pat {
                        out.push(base + i as u64);
                        if out.len() >= limit {
                            return out;
                        }
                    }
                    i += align;
                }
            }
            if n <= OVERLAP {
                break;
            }
            base += (n - OVERLAP) as u64;
            chunks += 1;
            if cost.should_yield(chunks) {
                cost.paused();
                next_tick().await;
            }
        }
    }
    out
}

/// Scans the aligned positions where `pat` appears and calls `on_hit` on each
/// one. Returning `true` stops the scan.
///
/// `on_hit` receives the address **and the bytes that follow**, up to the end
/// of the block already read. That is what lets us sort candidates without
/// crossing the process boundary again. A frequent pattern -- the String
/// vtable, which all the thousands of String objects in the heap carry --
/// would otherwise cost one remote read per object, and such a read costs far
/// more than the bytes it brings back.
async fn scan_bytes_until(
    mem: &dyn Memory,
    ranges: &[(u64, u64)],
    pat: &[u8],
    align: usize,
    cost: &mut Scan,
    mut on_hit: impl FnMut(u64, &[u8]) -> bool,
) {
    let mut buf = vec![0u8; CHUNK];
    let mut chunks = 0usize;

    for &(start, end) in ranges {
        let mut base = start;
        while base < end {
            let n = core::cmp::min(CHUNK as u64, end - base) as usize;
            if cost.read_block(mem, base, &mut buf[..n]) {
                let mut i = 0;
                while i + pat.len() <= n {
                    if &buf[i..i + pat.len()] == pat && on_hit(base + i as u64, &buf[i..n]) {
                        return;
                    }
                    i += align;
                }
            }
            if n <= OVERLAP {
                break;
            }
            base += (n - OVERLAP) as u64;
            chunks += 1;
            if cost.should_yield(chunks) {
                cost.paused();
                next_tick().await;
            }
        }
    }
}

/// Scans the aligned qwords whose value is one of `values`.
///
/// One pass for all candidates, and not one pass per candidate. Looking for
/// what points at eight addresses cost eight re-reads of the heap, that is
/// seven hundred MiB per failed attempt.
async fn scan_u64_any(
    mem: &dyn Memory,
    ranges: &[(u64, u64)],
    values: &[u64],
    cost: &mut Scan,
    mut on_hit: impl FnMut(u64) -> bool,
) {
    let mut buf = vec![0u8; CHUNK];
    let mut chunks = 0usize;

    for &(start, end) in ranges {
        let mut base = start;
        while base < end {
            let n = core::cmp::min(CHUNK as u64, end - base) as usize;
            if cost.read_block(mem, base, &mut buf[..n]) {
                let mut i = 0;
                while i + 8 <= n {
                    let v = u64::from_le_bytes([
                        buf[i],
                        buf[i + 1],
                        buf[i + 2],
                        buf[i + 3],
                        buf[i + 4],
                        buf[i + 5],
                        buf[i + 6],
                        buf[i + 7],
                    ]);
                    if values.contains(&v) && on_hit(base + i as u64) {
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
                next_tick().await;
            }
        }
    }
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
    cost: &mut Scan,
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
                next_tick().await;
            }
        }
    }
}

/// A qword read from a local buffer, if the offset fits inside it.
fn u64_at(buf: &[u8], off: usize) -> Option<u64> {
    let raw = buf.get(off..off + 8)?;
    Some(u64::from_le_bytes(<[u8; 8]>::try_from(raw).ok()?))
}

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
    current_hint: u64,
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
    #[cfg(test)]
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
    fn layout(&self, module: (u64, u64)) -> Option<Layout> {
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
    fn proven(&self) -> bool {
        self.layout.is_some()
    }

    fn learn(&mut self, layout: &Layout) {
        let mut l = *layout;
        l.str_vt -= layout.module.0;
        l.tbl_vt -= layout.module.0;
        self.layout = Some(l);
    }
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
    let layout = anchor.layout?;
    let manager = anchor.manager?;
    if read_u64(mem, manager)? != layout.tbl_vt {
        anchor.manager = None;
        return None;
    }
    // No more `current`: this is not the GameManager any more, the table must
    // have moved. Dropping the anchor starts a scan instead of staying blind.
    let Some(current) = layout.get_cached(mem, manager, keys::CURRENT, &mut anchor.current_hint)
    else {
        anchor.manager = None;
        return None;
    };
    let tbl = layout.table_of(mem, current)?;
    let game = validate(mem, layout, tbl)?;
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
) -> Option<Game> {
    if let Some(game) = resolve_via_manager(mem, anchor) {
        return Some(game);
    }

    let mut cost = Scan::default();
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
            asr::print_message(&alloc::format!(
                "Hammerfest: GameManager 0x{tbl:x}, layout {}",
                layout.profile.name,
            ));
            binary.learn(&layout);
            anchor.layout = Some(layout);
            anchor.manager = Some(tbl);
            anchor.current_hint = 0;
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
        asr::print_message("Hammerfest: silent anchor, searching again");
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
        |l, t| validate(mem, l, t),
    )
    .await?;

    asr::print_message(&alloc::format!(
        "Hammerfest: GameMode 0x{:x}, world {}, layout {}",
        game.game_mode,
        game.set,
        game.layout.profile.name,
    ));
    binary.learn(&game.layout);
    anchor.last_game_mode = Some(game.game_mode);
    anchor.layout = Some(game.layout);
    // `Mode.manager` leads to the GameManager. Keeping it does not shorten
    // the start of a game -- the objects die with the game, so the anchor does
    // not survive to the next one -- but it makes any new resolution inside
    // the same game free.
    anchor.manager = game.layout.child(mem, game.game_mode, keys::MANAGER);
    anchor.current_hint = 0;
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
    cost: &mut Scan,
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

/// Moves the region that holds `addr` to the front, if it is there.
fn move_region_first(ranges: &mut [(u64, u64)], addr: u64) {
    if let Some(i) = ranges.iter().position(|&(a, b)| a <= addr && addr < b) {
        ranges.swap(0, i);
    }
}

/// Finds the interned String object of a key, and the String layout with it.
///
/// The cache saves two full scans per attempt: these objects come from the
/// constant pool of the SWF, so they live as long as the plugin. It is checked
/// again by decoding the string, never assumed valid.
async fn find_string(
    mem: &dyn Memory,
    module: (u64, u64),
    ranges: &[(u64, u64)],
    key: &str,
    cache: &mut Option<u64>,
    binary: &Binary,
    cost: &mut Scan,
) -> Option<(Layout, u64)> {
    let units = key.encode_utf16().count() as u64;
    #[cfg(feature = "diagnostics")]
    asr::print_message(&alloc::format!(
        "HF_DIAG event=find_string t_us={} key={key} proven={} cached={}",
        crate::diagnostics::now_us(),
        binary.proven(),
        cache.is_some()
    ));
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
async fn scan_tables<T>(
    mem: &dyn Memory,
    ranges: &[(u64, u64)],
    layout: Layout,
    strobj: u64,
    key: &str,
    cost: &mut Scan,
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
fn validate(mem: &dyn Memory, mut layout: Layout, tbl: u64) -> Option<Game> {
    let world_atom = layout.get(mem, tbl, keys::WORLD)?;
    let wtbl = layout.derive_so_tbl(mem, world_atom, keys::SET_NAME)?;

    let set_atom = layout.get(mem, wtbl, keys::SET_NAME)?;
    let set = keys::WORLDS
        .iter()
        .find(|(obf, _)| layout.string_eq(mem, set_atom & !7, obf))
        .map(|&(_, clear)| clear)?;

    let level = layout.get_int(mem, wtbl, keys::CURRENT_ID)?;
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
    if let Some(manager) = layout.child(mem, tbl, keys::MANAGER) {
        if layout.child(mem, manager, keys::CURRENT) != Some(tbl) {
            return None;
        }
    }

    let chrono = layout.child(mem, tbl, keys::GAME_CHRONO)?;
    layout.get_int(mem, chrono, keys::FRAME_TIMER)?;

    // A GameMode already in game over is a finished GameMode. Holding on to
    // it would read a game that is over instead of waiting for the next one.
    if layout
        .get(mem, tbl, keys::FL_GAME_OVER)
        .and_then(avm1::as_bool)
        == Some(true)
    {
        return None;
    }

    Some(Game {
        layout,
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
        // Read before we borrow `self.layout`: `chrono_ms` needs all of
        // `self`.
        let (chrono_ms, frame_timer) = self.chrono(mem)?;
        let l = &self.layout;
        let world_atom = l.get_cached(mem, self.game_mode, keys::WORLD, &mut self.hints.world)?;
        let world = l.table_of(mem, world_atom)?;

        // The world must always be a known world. That is what detects that
        // we now read recycled memory.
        //
        // It also says which world, and so which numbering, `currentId`
        // belongs to. Both come from this one object, so they always agree.
        // `GameMode.currentDim` does not: it changes two seconds before
        // `world` does, on the way into a dimension.
        let set_atom = l.get_cached(mem, world, keys::SET_NAME, &mut self.hints.set_name)?;
        let set = keys::WORLDS
            .iter()
            .find(|(obf, _)| l.string_eq(mem, set_atom & !7, obf))
            .and_then(|&(_, clear)| World::from_set_name(clear))?;

        let level = avm1::as_int(l.get_cached(
            mem,
            world,
            keys::CURRENT_ID,
            &mut self.hints.current_id,
        )?)?;
        if !(0..MAX_LEVEL).contains(&level) {
            return None;
        }
        let previous = l
            .get_cached(mem, world, keys::PREVIOUS_ID, &mut self.hints.previous_id)
            .and_then(avm1::as_int)
            .unwrap_or(-1);

        Some(State {
            level: Level::new(set, level),
            previous,
            chrono_ms,
            frame_timer,
            // `fl_lock` is true during the black screen before level 0. Its
            // fall is the official start of the run.
            locked: l
                .get_cached(mem, self.game_mode, keys::FL_LOCK, &mut self.hints.lock)
                .and_then(avm1::as_bool)
                .unwrap_or(false),
            // Required, like the level and the clock: this is what dates the
            // start of the run. Reading it as zero by default would set the
            // origin at the instant of the resolution, so a timer short by all
            // the delay of the scan -- and silently so. Better to declare the
            // read invalid and scan again.
            duration_ms: hammerfest_core::duration_ms(avm1::as_number(
                mem,
                l.get_cached(
                    mem,
                    self.game_mode,
                    keys::DURATION,
                    &mut self.hints.duration,
                )?,
            )?),
            dim: l
                .get_cached(mem, self.game_mode, keys::CURRENT_DIM, &mut self.hints.dim)
                .and_then(avm1::as_int)
                .unwrap_or(0),
            game_over: l
                .get_cached(
                    mem,
                    self.game_mode,
                    keys::FL_GAME_OVER,
                    &mut self.hints.game_over,
                )
                .and_then(avm1::as_bool)
                .unwrap_or(false),
            // The end of the run. `GameMode.endModeTimer` is a Float that the
            // elevator script raises to fourteen seconds of cycles, and no
            // other line of an adventure writes it. What it means is in
            // `EndSequence`; here we only decode the number.
            //
            // Absent or unreadable, the sequence has not started. A run that
            // does not end by itself is a nuisance; a run that ends by
            // accident is a lost run.
            end_sequence: l
                .get_cached(
                    mem,
                    self.game_mode,
                    keys::END_MODE_TIMER,
                    &mut self.hints.end_mode,
                )
                .and_then(|atom| avm1::as_number(mem, atom))
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
        let l = &self.layout;
        let chrono = l.child_cached(
            mem,
            self.game_mode,
            keys::GAME_CHRONO,
            &mut self.hints.chrono,
        )?;

        let frame =
            avm1::as_int(l.get_cached(mem, chrono, keys::FRAME_TIMER, &mut self.hints.frame)?)?;

        let stopped = l
            .get_cached(mem, chrono, keys::FL_STOP, &mut self.hints.stop)
            .and_then(avm1::as_bool)
            .unwrap_or(false);
        if stopped {
            if let Some(halted) = l
                .get_cached(mem, chrono, keys::HALTED_TIMER, &mut self.hints.halted)
                .and_then(avm1::as_int)
            {
                return Some((halted, frame));
            }
        }

        let game =
            avm1::as_int(l.get_cached(mem, chrono, keys::GAME_TIMER, &mut self.hints.game)?)?;
        Some((frame - game, frame))
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve, Anchor, Binary};
    // The heap is held to the contract of `Memory` in `memory_contract`, and
    // every other test of this layer is written against the heap builder in
    // `test_heap`.
    use crate::memory_contract::Heap;
    use crate::test_heap::block_on;

    /** @spec reader.find::nothing-in-the-menus */
    #[test]
    fn finds_nothing_in_an_empty_heap() {
        let found = block_on(resolve(
            &Heap::default(),
            (0x1000, 0x1000),
            &mut Anchor::default(),
            &mut Binary::default(),
            &[],
        ));

        assert!(found.is_none());
    }
}
