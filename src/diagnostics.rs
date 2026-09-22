//! What measures, and not what splits.
//!
//! Only two things here belong to the product, so they live in every build:
//! the clock, and [`FreshMap`] -- the fix for the one second cache depends on
//! it. Everything else only measures, and it disappears without the
//! `diagnostics` feature: [`ScanTrace`] becomes an empty type, and [`event`]
//! and [`validation_read`] become functions with no body.
//!
//! The rule that goes with it: the product code prints nothing to measure. It
//! calls into this module, and the feature decides if anything happens.

#[cfg(feature = "diagnostics")]
use core::sync::atomic::{AtomicU64, Ordering};

#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn clock_time_get(id: u32, precision: u64, result: *mut u64) -> u16;
}

pub fn now_us() -> u64 {
    let mut ns = 0;
    // WASI CLOCKID_MONOTONIC. The runtime writes a u64 into `ns`.
    let error = unsafe { clock_time_get(1, 1, &mut ns) };
    assert_eq!(error, 0, "the WASI clock is not available");
    ns / 1000
}

#[cfg(feature = "diagnostics")]
static CALLS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "diagnostics")]
static BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "diagnostics")]
static FAILURES: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn validation_read(_bytes: usize, _ok: bool) {
    #[cfg(feature = "diagnostics")]
    {
        CALLS.fetch_add(1, Ordering::Relaxed);
        if _ok {
            BYTES.fetch_add(_bytes as u64, Ordering::Relaxed);
        } else {
            FAILURES.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Single reads made outside a scan, to validate candidates.
///
/// They do not go through the scan budget, so the budget does not see them.
/// Counting them apart is the only way to know if the time goes into the
/// passes or into the round trips that follow them.
#[cfg(feature = "diagnostics")]
fn validation_counts() -> [u64; 3] {
    [
        CALLS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
        FAILURES.load(Ordering::Relaxed),
    ]
}

/// A start, and its context: first of the module, first of this process, or a
/// later one. This is what `summarize_startup.py` sorts -- a freshly loaded
/// module has no cache, and a new process has different ones.
///
/// The state lives here and not in the loop. Without that, two variables
/// crossed the signature of `run` to feed one single trace.
#[cfg(feature = "diagnostics")]
static STARTS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "diagnostics")]
static LAST_PID: AtomicU64 = AtomicU64::new(u64::MAX);

pub fn started(_elapsed_ms: i64, _pid: asr::ProcessId) {
    #[cfg(feature = "diagnostics")]
    {
        let first_module = STARTS.fetch_add(1, Ordering::Relaxed) == 0;
        let first_process = LAST_PID.swap(_pid.0, Ordering::Relaxed) != _pid.0;
        asr::print_message(&alloc::format!(
            "HF_START elapsed_ms={_elapsed_ms} first_module={first_module} first_process={first_process}"
        ));
    }
}

pub fn event(_name: &str) {
    #[cfg(feature = "diagnostics")]
    asr::print_message(&alloc::format!("HF_DIAG event={} t_us={}", _name, now_us()));
}

/// A memory map obtained by a new access to the same process, every 100 ms.
/// The main access and the anchors of the game stay valid.
#[derive(Default)]
pub struct FreshMap {
    next_us: u64,
    ranges: Option<alloc::vec::Vec<(u64, u64)>>,
}

impl FreshMap {
    pub fn poll(&mut self, pid: asr::ProcessId) -> Option<&[(u64, u64)]> {
        let now = now_us();
        if now >= self.next_us {
            self.next_us = now + 100_000;
            if let Some(process) = asr::Process::attach_by_pid(pid) {
                let ranges = crate::hammerfest::heap_ranges(&process);
                #[cfg(feature = "diagnostics")]
                asr::print_message(&alloc::format!(
                    "HF_DIAG event=map_refresh t_us={} elapsed_us={} changed={} ranges={} bytes={}",
                    now,
                    now_us() - now,
                    self.ranges.as_ref() != Some(&ranges),
                    ranges.len(),
                    ranges.iter().map(|(a, b)| b - a).sum::<u64>()
                ));
                self.ranges = Some(ranges);
            } else {
                self.ranges = None;
                event("map_refresh_failed");
            }
        }
        self.ranges.as_deref()
    }
}

/// What one resolution attempt cost, and where it had got to.
///
/// Without the feature, this type holds nothing and all its methods are
/// empty: the scan keeps its calls, the binary keeps none of them.
#[cfg(feature = "diagnostics")]
pub struct ScanTrace {
    bytes: u64,
    failures: u64,
    yields: u32,
    started: u64,
    stage_started: u64,
    stage: &'static str,
    outcome: &'static str,
    validation: [u64; 3],
}

#[cfg(not(feature = "diagnostics"))]
pub struct ScanTrace;

#[cfg(feature = "diagnostics")]
impl Default for ScanTrace {
    fn default() -> Self {
        let now = now_us();
        Self {
            bytes: 0,
            failures: 0,
            yields: 0,
            started: now,
            stage_started: now,
            stage: "ranges",
            outcome: "not_found",
            validation: validation_counts(),
        }
    }
}

#[cfg(not(feature = "diagnostics"))]
impl Default for ScanTrace {
    fn default() -> Self {
        Self
    }
}

#[cfg(feature = "diagnostics")]
impl ScanTrace {
    pub fn read(&mut self, bytes: usize, ok: bool) {
        if ok {
            self.bytes += bytes as u64;
        } else {
            self.failures += 1;
        }
    }

    pub fn paused(&mut self) {
        self.yields += 1;
    }

    pub fn stage(&mut self, next: &'static str, requested: u64, calls: u64) {
        let now = now_us();
        asr::print_message(&alloc::format!(
            "HF_DIAG event=stage t_us={now} name={} elapsed_us={} scan_bytes_total={} scan_calls_total={calls}",
            self.stage,
            now - self.stage_started,
            requested
        ));
        self.stage = next;
        self.stage_started = now;
    }

    pub fn outcome(&mut self, outcome: &'static str) {
        self.outcome = outcome;
    }

    /// The summary, on drop: the attempt is over, whatever exit path it took
    /// -- and there are six of them.
    pub fn finish(&mut self, requested: u64, calls: u64) {
        let now = now_us();
        let validation = validation_counts();
        asr::print_message(&alloc::format!(
            "HF_SCAN t_us={now} outcome={} elapsed_us={} requested_bytes={requested} read_bytes={} calls={calls} failures={} yields={} validation_calls={} validation_bytes={} validation_failures={}",
            self.outcome,
            now - self.started,
            self.bytes,
            self.failures,
            self.yields,
            validation[0] - self.validation[0],
            validation[1] - self.validation[1],
            validation[2] - self.validation[2]
        ));
    }
}

#[cfg(not(feature = "diagnostics"))]
impl ScanTrace {
    #[inline]
    pub fn read(&mut self, _bytes: usize, _ok: bool) {}
    #[inline]
    pub fn paused(&mut self) {}
    #[inline]
    pub fn stage(&mut self, _next: &'static str, _requested: u64, _calls: u64) {}
    #[inline]
    pub fn outcome(&mut self, _outcome: &'static str) {}
    #[inline]
    pub fn finish(&mut self, _requested: u64, _calls: u64) {}
}
