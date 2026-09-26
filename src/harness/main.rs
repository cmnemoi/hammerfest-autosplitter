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
//! mise run e2e -- 120 before.wasm after.wasm   # two builds, side by side
//! ```
//!
//! Several modules run in the same loop, one after the other on each tick. So
//! they see the same game at the same moment, and their costs can be
//! compared.
//!
//! It judges three things, and reports the rest:
//!
//! - the module attaches to a player, finds the game and publishes its
//!   level;
//! - one `update()` stays within one tick at the 99th percentile. LiveSplit
//!   gives a tick 8.3 ms, and stops a module that falls five seconds behind;
//! - a start is dated at most 300 ms after level 0, when the game is started
//!   after this check.
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
/// A blink: the eye cannot tell a later start from one on time.
const START_LATENESS: Duration = Duration::from_millis(300);

/// What the module did, as a timer sees it.
#[derive(Default)]
struct Report {
    attached_at: Option<Duration>,
    level_at: Option<Duration>,
    starts: usize,
    splits: usize,
    /// How long after level 0 each start was dated.
    start_lateness: Vec<Duration>,
}

/// A timer that prints what the module asks of it, and keeps a report.
///
/// The runtime owns the timer once the module runs, so the report is shared.
struct WatchingTimer {
    /// The module this timer serves, when several run side by side.
    name: String,
    state: TimerState,
    started: Instant,
    variables: HashMap<String, String>,
    report: Arc<Mutex<Report>>,
}

impl WatchingTimer {
    fn say(&self, what: fmt::Arguments) {
        println!(
            "[{:7.2} s] {}{what}",
            self.started.elapsed().as_secs_f64(),
            self.name
        );
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
        // "Hammerfest: start dated, 267 ms already elapsed"
        if let Some(ms) = line
            .strip_prefix("Hammerfest: start dated, ")
            .and_then(|rest| rest.strip_suffix(" ms already elapsed"))
            .and_then(|ms| ms.parse().ok())
        {
            report.start_lateness.push(Duration::from_millis(ms));
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

/// One module under judgement: its runtime, its report, the time of each of
/// its updates.
struct Judged {
    name: String,
    splitter: livesplit_auto_splitting::AutoSplitter<WatchingTimer>,
    report: Arc<Mutex<Report>>,
    updates: Vec<Duration>,
}

fn load(path: &str, name: String, started: Instant) -> Judged {
    let wasm = fs::read(path)
        .unwrap_or_else(|_| panic!("no module at {path}: run `mise run build` first"));
    let report = Arc::new(Mutex::new(Report::default()));
    let timer = WatchingTimer {
        name: name.clone(),
        state: TimerState::NotRunning,
        started,
        variables: HashMap::new(),
        report: Arc::clone(&report),
    };
    let splitter = Runtime::new(Config::default())
        .and_then(|runtime| runtime.compile(&wasm))
        .and_then(|module| module.instantiate(timer, None, None))
        .expect("the runtime refused the module");
    Judged {
        name,
        splitter,
        report,
        updates: Vec::new(),
    }
}

/// Prints what one module did, and says what failed.
fn judge(judged: &mut Judged) -> Vec<String> {
    let report = judged.report.lock().unwrap();
    let updates = &mut judged.updates;
    updates.sort_unstable();
    let p99 = percentile(updates, 0.99);
    let ms = |duration: Duration| duration.as_secs_f64() * 1000.0;
    let at = |moment: Option<Duration>| {
        moment.map_or_else(
            || "never".to_owned(),
            |d| format!("{:.2} s", d.as_secs_f64()),
        )
    };
    println!();
    if !judged.name.is_empty() {
        println!("{}", judged.name.trim_end_matches(": "));
    }
    println!("updates    {}", updates.len());
    println!(
        "update     p50 {:.3} ms, p99 {:.3} ms, max {:.3} ms",
        ms(percentile(updates, 0.5)),
        ms(p99),
        ms(percentile(updates, 1.0))
    );
    println!("attached   {}", at(report.attached_at));
    println!("level      {}", at(report.level_at));
    println!(
        "timer      {} start(s), {} split(s)",
        report.starts, report.splits
    );
    for lateness in &report.start_lateness {
        println!("start      dated {:.0} ms after level 0", ms(*lateness));
    }

    let mut failures = Vec::new();
    if report.attached_at.is_none() {
        failures.push("the module attached to no player: is a game running?".to_owned());
    }
    if report.level_at.is_none() {
        failures.push(
            "the module published no level: is a game started, past the black screen?".to_owned(),
        );
    }
    // @spec pacing::a-start-dated-within-a-blink
    if let Some(late) = report
        .start_lateness
        .iter()
        .find(|late| **late > START_LATENESS)
    {
        failures.push(format!(
            "a start was dated {:.0} ms after level 0, more than a blink ({:.0} ms). \
             Was the game started after this check?",
            ms(*late),
            ms(START_LATENESS)
        ));
    }
    if p99 > TICK {
        failures.push(format!(
            "one update in a hundred takes more than one tick ({:.3} ms > {:.3} ms)",
            ms(p99),
            ms(TICK)
        ));
    }
    failures
        .into_iter()
        .map(|failure| format!("{}{failure}", judged.name))
        .collect()
}

fn main() -> ExitCode {
    let seconds: u64 = env::args().nth(1).map_or(30, |text| {
        text.parse().expect("the duration of the run, in seconds")
    });
    // Other builds can be judged, such as the one that measures
    // (`mise run build-diagnostics`), or an older one to compare with.
    let mut paths: Vec<String> = env::args().skip(2).collect();
    if paths.is_empty() {
        paths.push(MODULE.to_owned());
    }
    let started = Instant::now();
    let several = paths.len() > 1;
    let mut modules: Vec<Judged> = paths
        .iter()
        .map(|path| {
            let name = if several {
                let file = path.rsplit('/').next().unwrap_or(path);
                format!("{}: ", file.trim_end_matches(".wasm"))
            } else {
                String::new()
            };
            load(path, name, started)
        })
        .collect();

    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        let tick_started = Instant::now();
        for module in &mut modules {
            let before = Instant::now();
            module.splitter.lock().update().expect("the module trapped");
            module.updates.push(before.elapsed());
        }
        std::thread::sleep(TICK.saturating_sub(tick_started.elapsed()));
    }

    let failures: Vec<String> = modules.iter_mut().flat_map(judge).collect();
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
