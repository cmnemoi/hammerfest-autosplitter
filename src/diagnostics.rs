//! Horloge et rafraichissement de la carte memoire. Traces facultatives.

#[cfg(feature = "diagnostics")]
use core::sync::atomic::{AtomicU64, Ordering};

#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn clock_time_get(id: u32, precision: u64, result: *mut u64) -> u16;
}

pub fn now_us() -> u64 {
    let mut ns = 0;
    // WASI CLOCKID_MONOTONIC. Le runtime ecrit un entier u64 dans `ns`.
    let error = unsafe { clock_time_get(1, 1, &mut ns) };
    assert_eq!(error, 0, "horloge WASI indisponible");
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
        if _ok { BYTES.fetch_add(_bytes as u64, Ordering::Relaxed); }
        else { FAILURES.fetch_add(1, Ordering::Relaxed); }
    }
}

pub fn validation_counts() -> [u64; 3] {
    #[cfg(feature = "diagnostics")]
    { [CALLS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed), FAILURES.load(Ordering::Relaxed)] }
    #[cfg(not(feature = "diagnostics"))]
    { [0; 3] }
}

pub fn event(_name: &str) {
    #[cfg(feature = "diagnostics")]
    asr::print_message(&alloc::format!("HF_DIAG event={} t_us={}", _name, now_us()));
}

/// Carte obtenue par un nouvel acces au meme processus, toutes les 100 ms.
/// L'acces principal et les ancres de la partie restent valides.
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
                    now, now_us() - now, self.ranges.as_ref() != Some(&ranges), ranges.len(),
                    ranges.iter().map(|(a,b)| b-a).sum::<u64>()
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
