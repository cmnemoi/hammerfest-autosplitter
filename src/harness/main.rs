//! Runs the module in LiveSplit's own runtime, against the game that runs on
//! this machine, and judges what it sees.
//!
//! The tests hold the reader and the policy. They never cross into the
//! runtime. This does, and nothing else does: the real `.wasm`, the real
//! runtime, a real player. So it needs a game, and runs on demand only:
//!
//! ```sh
//! mise run e2e              # 30 seconds
//! mise run e2e -- 120       # longer, to play a level or two
//! mise run e2e -- 30 target/diagnostics/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm
//! ```
//!
//! It judges two things, and reports the rest:
//!
//! - the module attaches to a player, finds the game and publishes its
//!   level;
//! - one `update()` stays within one tick at the 99th percentile. LiveSplit
//!   gives a tick 8.3 ms, and stops a module that falls five seconds behind.
//!
//! See `docs/how-to/check-before-a-session.md`.

use std::{
    collections::HashMap,
    env, fmt, fs,
    process::ExitCode,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use livesplit_auto_splitting::{Config, LogLevel, Runtime, Timer, TimerState};

const MODULE: &str = "target/wasm32-unknown-unknown/release/hammerfest_autosplitter.wasm";
const TICK: Duration = Duration::from_nanos(1_000_000_000 / 120);

/// What the module did, as a timer sees it.
#[derive(Default)]
struct Report {
    attached_at: Option<Duration>,
    level_at: Option<Duration>,
    starts: usize,
    splits: usize,
}

/// A timer that prints what the module asks of it, and keeps a report.
///
/// The runtime owns the timer once the module runs, so the report is shared.
struct WatchingTimer {
    state: TimerState,
    started: Instant,
    variables: HashMap<String, String>,
    report: Arc<Mutex<Report>>,
}

impl WatchingTimer {
    fn say(&self, what: fmt::Arguments) {
        println!("[{:7.2} s] {what}", self.started.elapsed().as_secs_f64());
    }
}

impl Timer for WatchingTimer {
    fn state(&self) -> TimerState {
        self.state
    }
    fn current_split_index(&self) -> Option<usize> {
        None
    }
    fn segment_splitted(&self, _: usize) -> Option<bool> {
        None
    }
    fn start(&mut self) {
        self.state = TimerState::Running;
        self.report.lock().unwrap().starts += 1;
        self.say(format_args!("timer: start"));
    }
    fn split(&mut self) {
        self.report.lock().unwrap().splits += 1;
        self.say(format_args!("timer: split"));
    }
    fn skip_split(&mut self) {
        self.say(format_args!("timer: skip"));
    }
    fn undo_split(&mut self) {}
    fn reset(&mut self) {
        self.state = TimerState::NotRunning;
        self.say(format_args!("timer: reset"));
    }
    fn set_game_time(&mut self, _: time::Duration) {}
    fn pause_game_time(&mut self) {}
    fn resume_game_time(&mut self) {}
    fn set_variable(&mut self, key: &str, value: &str) {
        // The clocks change every tick: printing them would drown the rest.
        let is_a_clock = key.ends_with("(ms)");
        if !is_a_clock && self.variables.get(key).map(String::as_str) != Some(value) {
            self.say(format_args!("variable: {key} = {value}"));
            self.variables.insert(key.to_owned(), value.to_owned());
        }
        let mut report = self.report.lock().unwrap();
        if key == "Level" && report.level_at.is_none() {
            report.level_at = Some(self.started.elapsed());
        }
    }
    fn log_auto_splitter(&mut self, message: fmt::Arguments) {
        let line = message.to_string();
        let mut report = self.report.lock().unwrap();
        if line.ends_with(" attached") && report.attached_at.is_none() {
            report.attached_at = Some(self.started.elapsed());
        }
        drop(report);
        self.say(format_args!("log: {line}"));
    }
    fn log_runtime(&mut self, message: fmt::Arguments, level: LogLevel) {
        if matches!(level, LogLevel::Warning | LogLevel::Error) {
            self.say(format_args!("runtime: {message}"));
        }
    }
}

/// The duration at a given fraction of the sorted list.
fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
    sorted.get(index).copied().unwrap_or_default()
}

fn main() -> ExitCode {
    let seconds: u64 = env::args().nth(1).map_or(30, |text| {
        text.parse().expect("the duration of the run, in seconds")
    });
    // Another build can be judged, such as the one that measures:
    // `mise run build-diagnostics`, then its `.wasm` as the second argument.
    let module = env::args().nth(2).unwrap_or_else(|| MODULE.to_owned());
    let wasm = fs::read(&module).expect("no module: run `mise run build` first");
    let report = Arc::new(Mutex::new(Report::default()));
    let timer = WatchingTimer {
        state: TimerState::NotRunning,
        started: Instant::now(),
        variables: HashMap::new(),
        report: Arc::clone(&report),
    };
    let splitter = Runtime::new(Config::default())
        .and_then(|runtime| runtime.compile(&wasm))
        .and_then(|module| module.instantiate(timer, None, None))
        .expect("the runtime refused the module");

    let mut updates = Vec::new();
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        let before = Instant::now();
        splitter.lock().update().expect("the module trapped");
        let spent = before.elapsed();
        updates.push(spent);
        std::thread::sleep(splitter.tick_rate().saturating_sub(spent));
    }

    let timer = report.lock().unwrap();
    updates.sort_unstable();
    let p99 = percentile(&updates, 0.99);
    let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
    println!();
    println!("updates    {}", updates.len());
    println!(
        "update     p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms",
        ms(percentile(&updates, 0.5)),
        ms(p99),
        ms(percentile(&updates, 1.0))
    );
    let at = |moment: Option<Duration>| {
        moment.map_or_else(
            || "never".to_owned(),
            |d| format!("{:.2} s", d.as_secs_f64()),
        )
    };
    println!("attached   {}", at(timer.attached_at));
    println!("level      {}", at(timer.level_at));
    println!(
        "timer      {} start(s), {} split(s)",
        timer.starts, timer.splits
    );

    let mut failures = Vec::new();
    if timer.attached_at.is_none() {
        failures.push("the module attached to no player: is a game running?".to_owned());
    }
    if timer.level_at.is_none() {
        failures.push(
            "the module published no level: is a game started, past the black screen?".to_owned(),
        );
    }
    if p99 > TICK {
        failures.push(format!(
            "one update in a hundred takes more than one tick ({:.3} ms > {:.3} ms)",
            ms(p99),
            ms(TICK)
        ));
    }
    println!();
    if failures.is_empty() {
        println!("PASS");
        ExitCode::SUCCESS
    } else {
        for failure in failures {
            println!("FAIL  {failure}");
        }
        ExitCode::FAILURE
    }
}
