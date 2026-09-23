//! Hammerfest autosplitter for LiveSplit (Auto Splitting Runtime).
//!
//! This module decides nothing. It does three things, all of them turned
//! toward the runtime:
//!
//! 1. find the Flash plugin process and read the game state inside it;
//! 2. hand that state to [`hammerfest_core::Policy`], which decides;
//! 3. execute the answer, and publish the time and the variables.
//!
//! Everything that is decided lives in the `core` crate, with no memory access
//! and no runtime, under tests. That is the only way to test those rules: the
//! ASR runtime symbols exist only inside the WebAssembly sandbox.
//!
//! The memory measurements are in `docs/reverse-engineering.md`.
//!
//! **What measures does not live here.** The normal build holds the
//! autosplitter and nothing else: no trace, no counter, no timestamp. All of
//! that is in [`diagnostics`], behind the feature of the same name, and the
//! product code only calls into it -- without the feature, those calls have no
//! body. One command checks it: no `HF_` string appears in the normal
//! `.wasm`.

#![no_std]

extern crate alloc;

#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

mod avm1;
mod diagnostics;
mod hammerfest;

/// The runtime symbols, defined so that `cargo test` can link.
#[cfg(test)]
mod asr_stubs;
/// The contract of `avm1::Memory`, and the implementations held to it.
#[cfg(test)]
mod memory_contract;
/// The bytes of a real game, replayed.
#[cfg(test)]
mod replay;
/// The synthetic heap the reader's tests are served.
#[cfg(test)]
mod test_heap;

/// Obfuscated property names, taken from `vendor/hf.map.json` by build.rs.
mod keys {
    include!(concat!(env!("OUT_DIR"), "/keys.rs"));
}

use asr::{future::next_tick, time::Duration, timer, Process};
use hammerfest_core::{Command, Pacing, Policy, State, TimerState};

use hammerfest::Game;

asr::async_main!(stable);
asr::panic_handler!();

/// EternalTwin starts several processes with the same name. Only the one that
/// loaded Pepper Flash matters to us.
const PROCESS_NAMES: &[&str] = &["Eternaltwin.exe", "Eternaltwin", "etwin"];

fn timer_state() -> TimerState {
    match timer::state() {
        timer::TimerState::NotRunning => TimerState::NotRunning,
        timer::TimerState::Running => TimerState::Running,
        // A Hammerfest run has no legal pause: the time counts whatever the
        // player does. A runner who pauses the LiveSplit timer is still in a
        // run, so the core is told the run goes on.
        timer::TimerState::Paused => TimerState::Running,
        timer::TimerState::Ended => TimerState::Ended,
        _ => TimerState::Unknown,
    }
}

async fn main() {
    // EternalTwin processes already examined and set aside: see
    // `attach_plugin`.
    let mut rejected = alloc::vec::Vec::new();
    // What one resolution learns and the next one reuses.
    let mut anchor = hammerfest::Anchor::default();
    // What we keep about the binary outlives the plugin process.
    let mut binary = hammerfest::Binary::default();
    // The policy crosses processes: a plugin that disappears is part of the
    // story of a game.
    let mut policy = Policy::new();

    // One line at load time, which says what build is running. It is the first
    // thing we look for in a log someone sends us.
    // LiveSplit shows no version of an auto splitter anywhere, and the
    // registry has no field for one. This line is the only place a version
    // can be read, so it carries it.
    asr::print_message(&alloc::format!(
        "Hammerfest: autosplitter {} started (budget={}, diagnostics={})",
        env!("CARGO_PKG_VERSION"),
        cfg!(feature = "scan-budget"),
        cfg!(feature = "diagnostics"),
    ));
    diagnostics::event("module_started");

    loop {
        match hammerfest::attach_plugin(PROCESS_NAMES, &mut rejected) {
            Some((process, module, pid)) => {
                asr::print_message("Hammerfest: Flash plugin attached");
                diagnostics::event("plugin_attached");
                // Nothing another process learned is valid here: ASLR moves
                // the module, and the AVM1 heap is built again.
                anchor.reset();
                #[cfg(feature = "known-flash")]
                {
                    let matched = binary.recognize(&process, module);
                    asr::print_message(&alloc::format!(
                        "HF_DIAG event=binary_profile t_us={} matched={matched}",
                        diagnostics::now_us()
                    ));
                }
                run(&process, pid, module, &mut anchor, &mut binary, &mut policy).await;
                asr::print_message("Hammerfest: Flash plugin closed");
            }
            None => {
                apply(policy.tick(timer_state(), None));
                next_tick().await;
            }
        }
    }
}

async fn run(
    process: &Process,
    pid: asr::ProcessId,
    module: (u64, u64),
    anchor: &mut hammerfest::Anchor,
    binary: &mut hammerfest::Binary,
    policy: &mut Policy,
) {
    let mut game: Option<Game> = None;
    // When a full scan is allowed. The rules are in `core::pacing`, with their
    // spec and their tests.
    let mut pacing = Pacing::new();
    // The origin is announced once per game. It is the only line that says how
    // late the scan arrived.
    let mut announced = false;
    let mut fresh_map = diagnostics::FreshMap::default();
    #[cfg(feature = "diagnostics")]
    let mut last_read = None;
    #[cfg(feature = "diagnostics")]
    let mut last_loop = diagnostics::now_us();

    while process.is_open() {
        #[cfg(feature = "diagnostics")]
        {
            let now = diagnostics::now_us();
            if now - last_loop > 50_000 {
                asr::print_message(&alloc::format!(
                    "HF_DIAG event=loop_gap t_us={now} elapsed_us={}",
                    now - last_loop
                ));
            }
            last_loop = now;
        }

        if game.is_none() {
            // The fast path follows `GameManager.current`. It is a few reads,
            // so we can try it every tick. The full scan only runs to learn
            // the anchor, or when the anchor has moved.
            game = hammerfest::resolve_via_manager(process, anchor);

            if game.is_none() {
                // A heap that grows in one step is the SWF creating its
                // objects. Scan again at once, without the wait. That is the
                // only window that matters -- the game starts half a second
                // later -- and a fixed wait added up to one second of delay
                // there, at the mercy of the previous attempt.
                let ranges = fresh_map.poll(pid);
                let now = ranges.map_or_else(
                    || hammerfest::heap_size(process),
                    |rs| rs.iter().map(|(a, b)| b - a).sum(),
                );
                if pacing.may_scan(now) {
                    #[cfg(feature = "diagnostics")]
                    asr::print_message(&alloc::format!(
                        "HF_DIAG event=resolve_trigger t_us={} heap={now}",
                        diagnostics::now_us()
                    ));
                    // The ranges are gathered here, and not inside `resolve`.
                    // That is what keeps the reader off the runtime API.
                    let all =
                        ranges.map_or_else(|| hammerfest::heap_ranges(process), |rs| rs.to_vec());
                    game = hammerfest::resolve(process, module, anchor, binary, &all).await;
                    if game.is_none() {
                        pacing.scan_failed();
                        #[cfg(feature = "diagnostics")]
                        asr::print_message(&alloc::format!(
                            "HF_DIAG event=retry_wait t_us={} ticks={}",
                            diagnostics::now_us(),
                            pacing.wait()
                        ));
                    }
                }
            }
            if game.is_some() {
                pacing.game_found();
            }
        }

        let read = game.as_mut().and_then(|g| g.read(process));
        #[cfg(feature = "diagnostics")]
        {
            let signature = read
                .as_ref()
                .map(|s| (game.as_ref().unwrap().game_mode, s.locked));
            if signature != last_read {
                asr::print_message(&alloc::format!(
                    "HF_DIAG event=read t_us={} state={signature:?}",
                    diagnostics::now_us()
                ));
                last_read = signature;
            }
        }
        if let Some(state) = read.as_ref() {
            publish(state, game.as_ref().map_or("", |g| g.set));
        }

        let actions = policy.tick(timer_state(), read);
        if actions.start {
            diagnostics::started(actions.real_time_ms.unwrap_or(-1), pid);
        }
        match actions.real_time_ms {
            Some(ms) if !announced => {
                announced = true;
                asr::print_message(&alloc::format!(
                    "Hammerfest: start dated, {ms} ms already elapsed"
                ));
                #[cfg(feature = "diagnostics")]
                asr::print_message(&alloc::format!(
                    "HF_DIAG event=origin t_us={} elapsed_ms={ms} start={} fresh={}",
                    diagnostics::now_us(),
                    actions.start,
                    true
                ));
            }
            None => announced = false,
            _ => {}
        }

        let drop_resolution = apply(actions);

        if drop_resolution {
            // Losing a game almost always announces the next one. The game
            // builds a new GameManager at every launch, so the anchor dies
            // with the game and we must scan again. Waiting on top of that
            // would be pure delay, so we restart with no wait.
            game = None;
            pacing.game_lost();
        }

        next_tick().await;
    }
}

/// Executes what the policy decided. Returns `true` if the current resolution
/// must be dropped.
///
/// The order is not decided here. It is a value, built in `core` and held by
/// the spec of [timer commands](../docs/specs/timer-commands.md).
fn apply(actions: hammerfest_core::Actions) -> bool {
    for command in actions.commands().iter() {
        send(command);
    }
    actions.drop_resolution
}

/// Sends one command to LiveSplit.
///
/// This is the whole of what the autosplitter asks of a timer. Driving another
/// one means writing another one of these.
///
/// @spec crossing::one-split-per-crossing
/// @spec crossing::warp-skips-the-levels-never-played
fn send(command: Command) {
    match command {
        Command::Reset => timer::reset(),
        Command::Start => {
            asr::print_message("Hammerfest: game started");
            timer::start();
            diagnostics::event("start_called");
        }
        Command::Split => timer::split(),
        // The levels a warp zone carried the player over. A skipped segment
        // records no time, so it takes no gold and stays out of the sum of
        // best segments.
        Command::SkipSplit => timer::skip_split(),
        // An absolute value: the detection delay does not shift the time, and
        // LiveSplit must not add its own advance between two readings.
        Command::PauseGameTime => timer::pause_game_time(),
        Command::SetGameTime(ms) => timer::set_game_time(Duration::milliseconds(ms)),
    }
}

/// What LiveSplit shows next to the timer.
fn publish(state: &State, set: &str) {
    // `GameInterface.setLevel` writes `""+currentId`. The number the game
    // displays is that index, with no offset.
    timer::set_variable_int("Level", state.level.id);
    timer::set_variable("World", set);
    // The clock the game itself reports at the end of a game
    // (`"T="+gameChrono.get()`). It excludes pauses and level transitions, so
    // it cannot serve as real time -- but it is the number the player sees, so
    // we show it next to the timer.
    timer::set_variable_int("Game clock (ms)", state.chrono_ms);
    // What dates the start of the run. On the first display, the timer must
    // hold this same value. That is what separates a normal catch up from a
    // wrong origin.
    timer::set_variable_int("Play time (ms)", state.duration_ms);
    if state.dim != 0 {
        timer::set_variable_int("Dimension", state.dim);
    }
}
