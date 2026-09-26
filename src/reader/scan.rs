//! Sweeping the heap, and sharing the tick while doing it.
//!
//! What every search needs, whatever the player: read the regions in large
//! blocks, look for a pattern in them, and give the tick back to the runtime
//! often enough that LiveSplit does not freeze.

use alloc::{vec, vec::Vec};

use crate::avm1::Memory;
use crate::search_log::SearchLog;

/// Size of a read block. The heap is about a hundred MiB. At 64 KiB that was
/// a good thousand runtime calls per pass, and each call costs far more than
/// the bytes it brings back.
pub(crate) const CHUNK: usize = 1024 * 1024;
/// Overlap between two blocks, so a pattern across the edge is not missed.
pub(crate) const OVERLAP: usize = 32;
/// Number of blocks read before we yield to the runtime.
#[cfg(not(feature = "scan-budget"))]
const CHUNKS_PER_TICK: usize = 8;
// The same volume ceiling as eight blocks of 1 MiB. The call ceiling limits
// the work when the map holds many small regions.
#[cfg(feature = "scan-budget")]
const BYTES_PER_TICK: u64 = 8 * CHUNK as u64;
#[cfg(feature = "scan-budget")]
const READS_PER_TICK: u64 = 128;

/// What the scan must count in order to steer itself, and nothing more.
///
/// The budget decides when to yield to the runtime. Everything that only
/// measures -- durations, stages, outcome -- goes to `log`, and the host
/// decides what to do with it.
pub struct Scan<'log> {
    /// Reads asked of the runtime, and the bytes they carried.
    calls: u64,
    requested: u64,
    #[cfg(feature = "scan-budget")]
    last_yield_requested: u64,
    #[cfg(feature = "scan-budget")]
    last_yield_calls: u64,
    pub(crate) log: &'log mut dyn SearchLog,
}

impl<'log> Scan<'log> {
    pub(crate) fn new(log: &'log mut dyn SearchLog) -> Self {
        log.search_started();
        Self {
            calls: 0,
            requested: 0,
            #[cfg(feature = "scan-budget")]
            last_yield_requested: 0,
            #[cfg(feature = "scan-budget")]
            last_yield_calls: 0,
            log,
        }
    }

    pub(crate) fn read_block(&mut self, mem: &dyn Memory, base: u64, buf: &mut [u8]) -> bool {
        self.calls += 1;
        self.requested += buf.len() as u64;
        let ok = mem.read_into(base, buf).is_some();
        self.log.block_read(buf.len(), ok);
        ok
    }

    pub(crate) fn paused(&mut self) {
        self.log.tick_given_back();
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
    pub(crate) fn should_yield(&self, _chunks: usize) -> bool {
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

    /// Marks the current stage, for the log.
    pub(crate) fn stage(&mut self, next: &'static str) {
        self.log.stage(next, self.requested, self.calls);
    }

    /// The outcome of the attempt, for the log.
    pub(crate) fn outcome(&mut self, outcome: &'static str) {
        self.log.outcome(outcome);
    }
}

impl Drop for Scan<'_> {
    fn drop(&mut self) {
        self.log.stage("done", self.requested, self.calls);
        self.log.search_finished(self.requested, self.calls);
    }
}

// -- sharing the tick -------------------------------------------------------

/// Lets one tick pass before the scan goes on.
///
/// The runtime polls the module once per tick. A future that is pending once
/// therefore gives exactly one tick back, and LiveSplit does not freeze during
/// a long scan. It is what `asr::future::next_tick` does. The reader keeps its
/// own so that it needs no runtime.
pub(crate) fn give_the_tick_back() -> impl core::future::Future<Output = ()> {
    let mut given_back = false;
    core::future::poll_fn(move |_| {
        if core::mem::replace(&mut given_back, true) {
            core::task::Poll::Ready(())
        } else {
            core::task::Poll::Pending
        }
    })
}

// -- scans ------------------------------------------------------------------

/// Every aligned address where `pat` appears.
pub(crate) async fn scan_bytes(
    mem: &dyn Memory,
    ranges: &[(u64, u64)],
    pat: &[u8],
    align: usize,
    limit: usize,
    cost: &mut Scan<'_>,
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
                give_the_tick_back().await;
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
pub(crate) async fn scan_bytes_until(
    mem: &dyn Memory,
    ranges: &[(u64, u64)],
    pat: &[u8],
    align: usize,
    cost: &mut Scan<'_>,
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
                give_the_tick_back().await;
            }
        }
    }
}

/// Scans the aligned qwords whose value is one of `values`.
///
/// One pass for all candidates, and not one pass per candidate. Looking for
/// what points at eight addresses cost eight re-reads of the heap, that is
/// seven hundred MiB per failed attempt.
pub(crate) async fn scan_u64_any(
    mem: &dyn Memory,
    ranges: &[(u64, u64)],
    values: &[u64],
    cost: &mut Scan<'_>,
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
                give_the_tick_back().await;
            }
        }
    }
}

/// A qword read from a local buffer, if the offset fits inside it.
pub(crate) fn u64_at(buf: &[u8], off: usize) -> Option<u64> {
    let raw = buf.get(off..off + 8)?;
    Some(u64::from_le_bytes(<[u8; 8]>::try_from(raw).ok()?))
}

/// Moves the region that holds `addr` to the front, if it is there.
pub(crate) fn move_region_first(ranges: &mut [(u64, u64)], addr: u64) {
    if let Some(i) = ranges.iter().position(|&(a, b)| a <= addr && addr < b) {
        ranges.swap(0, i);
    }
}

/// Walks every aligned qword, with the qword that follows it when the block
/// holds it, and calls `on_qword` on each. Returning `true` stops the sweep.
///
/// For a search that cannot say in advance which value it looks for: a
/// pointer into a range, and the length beside it.
pub(crate) async fn scan_qwords(
    mem: &dyn Memory,
    ranges: &[(u64, u64)],
    cost: &mut Scan<'_>,
    mut on_qword: impl FnMut(u64, u64, Option<u64>) -> bool,
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
                    let value = u64_at(&buf[..n], i).unwrap_or(0);
                    if on_qword(base + i as u64, value, u64_at(&buf[..n], i + 8)) {
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
                give_the_tick_back().await;
            }
        }
    }
}
