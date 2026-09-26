//! Finding the process of a Flash player, and the ranges where its heap lives:
//! the Pepper Flash plugin of EternalTwin, or Ruffle desktop.
//!
//! All of it asks the runtime, so none of it belongs to the reader.

use alloc::vec::Vec;
use asr::{Process, ProcessId};
use hammerfest_process::ProcessMemory;
use hammerfest_reader::linear_memory::LinearMemory;
use hammerfest_reader::ruffle::RuffleBuild;

/// The plugin, by platform.
pub const PLUGINS: &[&str] = &[
    "pepflashplayer.dll",
    "libpepflashplayer.so",
    "PepperFlashPlayer",
];

// -- memory ranges ----------------------------------------------------------

/// Does the runtime run on macOS?
///
/// The plugin there is an x86-64 binary, and Rosetta 2 translates it. Two
/// rules below hold on every other platform and not on that one.
fn on_macos() -> bool {
    asr::get_os().is_ok_and(|os| os.as_str() == "macos")
}

/// The ranges where the AVM1 heap lives: readable, writable, and -- except on
/// macOS -- with no file behind them.
///
/// The AVM1 heap is ordinary allocated memory on Windows and on Linux, so the
/// file rule costs nothing there and removes the mapped images from the scan.
///
/// Under Rosetta, every page the guest allocates is attributed to
/// `/usr/libexec/rosetta/runtime`. The rule then removes the one thing worth
/// reading: measured on a running game, the six property names of the game
/// and the seventy-three tables that cite them all live in ranges that carry
/// that name, and none of them in an anonymous one.
fn heap_iter(process: &Process) -> impl Iterator<Item = (u64, u64)> + '_ {
    use asr::MemoryRangeFlags as F;
    let mapped_too = on_macos();
    process.memory_ranges().filter_map(move |r| {
        let flags = r.flags().ok()?;
        if !flags.contains(F::READ | F::WRITE) || (!mapped_too && flags.contains(F::PATH)) {
            return None;
        }
        let (addr, size) = r.range().ok()?;
        let a = addr.value();
        (size > 0).then(|| (a, a + size))
    })
}

pub fn heap_ranges(process: &Process) -> Vec<(u64, u64)> {
    let mut ranges: Vec<(u64, u64)> = heap_iter(process).collect();
    if on_macos() {
        // The smallest ranges first. Every scan stops at its first answer, so
        // the order decides how much is read before it stops.
        //
        // Measured on a running game: the game objects sat in ranges of
        // 0.12 to 0.75 MiB, and the ranges of 4 MiB or less hold 115 MiB of
        // the 1119 MiB the plugin maps. Reading them first is the difference
        // between 3.4 s and a fraction of a second.
        //
        // ponytail: an order, not a filter. A game object born in a large
        // range makes a scan slow again, never wrong. If that ever shows up,
        // remember the ranges that held the answer last time instead.
        ranges.sort_unstable_by_key(|&(a, b)| b - a);
    }
    ranges
}

/// Total committed bytes in this heap.
///
/// It tells us whether a new scan has any chance to learn something. The AVM1
/// objects of the SWF are not born one by one: the heap goes from two to
/// eighty MiB in a few seconds, then it settles. While it does not grow, a new
/// scan can find nothing more -- and when it grows in one step, waiting is
/// pure delay.
///
/// Measuring costs about a hundred runtime calls, against eighty MiB copied
/// for a scan.
pub fn heap_size(process: &Process) -> u64 {
    heap_iter(process).map(|(a, b)| b - a).sum()
}

/// The plugin process: the one EternalTwin process where Pepper Flash is
/// loaded. It exists only while a Flash instance lives, so its pid must never
/// be cached.
///
/// It does not always die with the game: four games in a row were observed in
/// one single process. What dies with the game are the AVM1 objects. Hence the
/// systematic re-check, rather than trust in the process.
///
/// EternalTwin starts half a dozen processes under the same name, and finding
/// which one carries the plugin means attaching to it -- there is no other way
/// to list its modules. But the runtime logs every attach and every detach:
/// probing the six on every tick drowns the log under hundreds of lines per
/// second, which makes the debugger unusable as soon as you leave a game
/// without closing the application.
///
/// So `rejected` keeps the pids already set aside, to probe only the
/// newcomers -- and the PPAPI process is always one of them, because it is
/// born with the game. Pids that disappear are removed, so the list does not
/// grow without end.
pub fn attach_plugin(
    names: &[&str],
    rejected: &mut Vec<ProcessId>,
) -> Option<(Process, (u64, u64), ProcessId)> {
    let mut alive = Vec::new();
    let mut found = None;

    for name in names {
        let Some(pids) = Process::list_by_name(name) else {
            continue;
        };
        for pid in pids {
            alive.push(pid);
            if found.is_some() || rejected.contains(&pid) {
                continue;
            }
            let Some(mem) = Process::attach_by_pid(pid) else {
                continue;
            };
            let range = PLUGINS.iter().find_map(|plugin| {
                let (addr, size) = mem.get_module_range(plugin).ok()?;
                Some((addr.value(), addr.value() + size))
            });
            match range {
                Some(range) => found = Some((mem, range, pid)),
                None => rejected.push(pid),
            }
        }
    }

    rejected.retain(|pid| alive.contains(pid));
    found
}

// -- Ruffle ------------------------------------------------------------------

/// Ruffle desktop, by platform. Its executable is its module.
pub const RUFFLE: &[&str] = &["ruffle", "ruffle.exe"];

/// How many processes of these names run.
pub fn count_running(names: &[&str]) -> usize {
    names
        .iter()
        .filter_map(|name| Process::list_by_name(name))
        .map(|pids| pids.len())
        .sum()
}

/// A Ruffle desktop process, the range of its executable, and its pid.
pub fn attach_ruffle() -> Option<(Process, (u64, u64), ProcessId)> {
    RUFFLE.iter().find_map(|name| {
        Process::list_by_name(name)?.into_iter().find_map(|pid| {
            let process = Process::attach_by_pid(pid)?;
            let (address, size) = process.get_module_range(name).ok()?;
            let module = (address.value(), address.value() + size);
            Some((process, module, pid))
        })
    })
}

/// The ranges where Ruffle keeps its AVM1 heap, the likeliest first.
///
/// Measured on a live game under Linux, every object of the game sat in
/// `[heap]`, the heap of glibc, and none in the hundred anonymous ranges
/// beside it. So `[heap]` comes first. LiveSplit marks it as a range with a
/// path, which is why the rule of Pepper Flash -- no path -- must not apply
/// here: it would drop the one range worth reading. It is found as a module,
/// by its name.
///
/// The anonymous ranges follow, in case another build or another platform
/// puts the game there. Windows names no heap: its private ranges carry no
/// path, and all of them are read.
pub fn ruffle_heap_ranges(process: &Process) -> Vec<(u64, u64)> {
    use asr::MemoryRangeFlags as F;
    let brk_heap = process
        .get_module_range("[heap]")
        .ok()
        .map(|(address, size)| (address.value(), address.value() + size));
    let anonymous = process.memory_ranges().filter_map(|range| {
        let flags = range.flags().ok()?;
        if !flags.contains(F::READ | F::WRITE) || flags.contains(F::PATH) {
            return None;
        }
        let (address, size) = range.range().ok()?;
        (size > 0).then(|| (address.value(), address.value() + size))
    });
    brk_heap.into_iter().chain(anonymous).collect()
}

// -- Ruffle in Firefox ---------------------------------------------------------

/// The processes of Firefox tabs under Linux, as the runtime names them: the
/// kernel keeps fifteen characters of a name.
pub const FIREFOX_TABS: &[&str] = &["Isolated Web Co", "Web Content"];

/// SpiderMonkey puts one page of its own in front of a linear memory.
const LINEAR_MEMORY_HEADER: u64 = 0x1000;
/// A 32-bit linear memory reserves 4 GiB, so that its accesses need no
/// check: the committed part, then a reserve that cannot be read.
const LINEAR_MEMORY_RESERVATION: u64 = 4 << 30;

/// A Firefox tab where Ruffle runs: its process, and its linear memory.
pub struct RuffleTab {
    pub process: Process,
    pub pid: ProcessId,
    /// The base of the linear memory, and how far its reservation reaches.
    pub base: u64,
    pub span: u64,
    pub build: RuffleBuild,
}

/// Every Firefox tab whose linear memory holds a known build of Ruffle.
///
/// A linear memory is found by the shape of its range, then proven by the
/// vtables of a build. See `docs/specs/ruffle-support.md#ruffle-in-a-browser`.
///
/// A tab process maps thousands of ranges, and listing them costs one call of
/// the runtime each: one tick is given back after each process, so that a
/// look spreads over several ticks.
pub async fn ruffle_tabs() -> Vec<RuffleTab> {
    let mut tabs = Vec::new();
    for name in FIREFOX_TABS {
        let Some(pids) = Process::list_by_name(name) else {
            continue;
        };
        for pid in pids {
            asr::future::next_tick().await;
            let Some(process) = Process::attach_by_pid(pid) else {
                continue;
            };
            let found = linear_memories(&process)
                .into_iter()
                .find_map(|(base, span)| {
                    let memory = ProcessMemory(&process);
                    let build =
                        RuffleBuild::recognised_in(&LinearMemory::new(&memory, base, span))?;
                    Some((base, span, build))
                });
            if let Some((base, span, build)) = found {
                tabs.push(RuffleTab {
                    process,
                    pid,
                    base,
                    span,
                    build,
                });
            }
        }
    }
    tabs
}

/// The linear memories of a process: an anonymous range that can be read and
/// written, followed at once by a range that cannot be read, the two of them
/// at least as large as the reservation. Their base, and their span.
fn linear_memories(process: &Process) -> Vec<(u64, u64)> {
    use asr::MemoryRangeFlags as F;
    let ranges: Vec<(u64, u64, F)> = process
        .memory_ranges()
        .filter_map(|range| {
            let (address, size) = range.range().ok()?;
            Some((address.value(), size, range.flags().ok()?))
        })
        .collect();
    ranges
        .windows(2)
        .filter_map(|pair| {
            let [(start, size, flags), (next, reserve, reserve_flags)] = *pair else {
                return None;
            };
            let is_committed = flags.contains(F::READ | F::WRITE) && !flags.contains(F::PATH);
            let is_a_reserve = next == start + size && !reserve_flags.contains(F::READ);
            let base = start + LINEAR_MEMORY_HEADER;
            (is_committed && is_a_reserve && size + reserve >= LINEAR_MEMORY_RESERVATION)
                .then_some((base, start + size + reserve - base))
        })
        .collect()
}

/// Where the committed part of the linear memory at `base` ends now. It grows
/// with the game.
pub fn committed_end(process: &Process, base: u64) -> Option<u64> {
    process.memory_ranges().find_map(|range| {
        let (address, size) = range.range().ok()?;
        let start = address.value();
        (start + LINEAR_MEMORY_HEADER == base).then_some(start + size)
    })
}
