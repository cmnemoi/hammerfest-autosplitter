//! Finding the Flash plugin process, and the ranges where its heap lives.
//!
//! All of it asks the runtime, so none of it belongs to the reader.

use alloc::vec::Vec;
use asr::{Process, ProcessId};

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
