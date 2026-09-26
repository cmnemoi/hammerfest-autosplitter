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
                for i in occurrences(&buf[..n], pat, align) {
                    out.push(base + i as u64);
                    if out.len() >= limit {
                        return out;
                    }
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
                for i in occurrences(&buf[..n], pat, align) {
                    if on_hit(base + i as u64, &buf[i..n]) {
                        return;
                    }
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

/// Scans the aligned words of `width` bytes, 4 or 8, whose value is one of
/// `values`.
///
/// One pass for all candidates, and not one pass per candidate. Looking for
/// what points at eight addresses cost eight re-reads of the heap, that is
/// seven hundred MiB per failed attempt.
pub(crate) async fn scan_words_any(
    mem: &dyn Memory,
    ranges: &[(u64, u64)],
    width: usize,
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
                for offset in words_where(&buf[..n], width, |word| values.contains(&word)) {
                    if on_hit(base + offset as u64) {
                        return;
                    }
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

/// The offsets of the aligned words of `width` bytes, 4 or 8, of `block`
/// whose value passes `test`, in order.
///
/// The test is the whole of the work for almost every word, so it runs in a
/// tight loop over words of the right width.
pub(crate) fn words_where<'a>(
    block: &'a [u8],
    width: usize,
    test: impl Fn(u64) -> bool + Copy + 'a,
) -> impl Iterator<Item = usize> + 'a {
    let (fours, _) = block.as_chunks::<4>();
    let (eights, _) = block.as_chunks::<8>();
    let four_bytes = (width == 4).then(|| {
        fours.iter().enumerate().filter_map(move |(index, word)| {
            test(u64::from(u32::from_le_bytes(*word))).then_some(index * 4)
        })
    });
    let eight_bytes = (width == 8).then(|| {
        eights
            .iter()
            .enumerate()
            .filter_map(move |(index, word)| test(u64::from_le_bytes(*word)).then_some(index * 8))
    });
    four_bytes
        .into_iter()
        .flatten()
        .chain(eight_bytes.into_iter().flatten())
}

/// Moves the region that holds `addr` to the front, if it is there.
pub(crate) fn move_region_first(ranges: &mut [(u64, u64)], addr: u64) {
    if let Some(i) = ranges.iter().position(|&(a, b)| a <= addr && addr < b) {
        ranges.swap(0, i);
    }
}

/// Walks the aligned words of `width` bytes, 4 or 8, whose value lies in
/// `values`, and calls `on_word` on each, with the word that follows it when
/// the block holds it. Returning `true` stops the sweep.
///
/// For a search that cannot say in advance which value it looks for, only
/// where it lies: a pointer into a range, and the length beside it. The test
/// on the range is the whole of the work for almost every word, so it runs in
/// a tight loop, and `on_word` is called for the few that pass it.
pub(crate) async fn scan_words_between(
    mem: &dyn Memory,
    ranges: &[(u64, u64)],
    width: usize,
    values: core::ops::RangeInclusive<u64>,
    cost: &mut Scan<'_>,
    mut on_word: impl FnMut(u64, u64, Option<u64>) -> bool,
) {
    let mut buf = vec![0u8; CHUNK];
    let mut chunks = 0usize;
    let (low, high) = (*values.start(), *values.end());

    for &(start, end) in ranges {
        let mut base = start;
        while base < end {
            let n = core::cmp::min(CHUNK as u64, end - base) as usize;
            if cost.read_block(mem, base, &mut buf[..n]) {
                let block = &buf[..n];
                let hits = words_where(block, width, |value| low <= value && value <= high);
                for i in hits {
                    let value = word_at(block, i, width).unwrap_or(0);
                    if on_word(base + i as u64, value, word_at(block, i + width, width)) {
                        return;
                    }
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

/// The aligned offsets of `block` where `pattern` starts, in order.
///
/// Its first unit -- two bytes, or eight -- is compared in a tight loop, and
/// the whole pattern only where that unit matches. A comparison of the whole
/// pattern at every offset was most of the time of a sweep.
fn occurrences<'a>(
    block: &'a [u8],
    pattern: &'a [u8],
    align: usize,
) -> impl Iterator<Item = usize> + 'a {
    let fits = move |offset: &usize| block.get(*offset..*offset + pattern.len()) == Some(pattern);
    let by_two = (align == 2 && pattern.len() >= 2).then(|| {
        let first = u16::from_le_bytes([pattern[0], pattern[1]]);
        block
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
            .filter_map(move |(index, unit)| {
                (u16::from_le_bytes(*unit) == first).then_some(index * 2)
            })
    });
    let by_eight = (align == 8 && pattern.len() >= 8).then(|| {
        let first = u64::from_le_bytes(pattern[..8].try_into().unwrap_or_default());
        block
            .as_chunks::<8>()
            .0
            .iter()
            .enumerate()
            .filter_map(move |(index, unit)| {
                (u64::from_le_bytes(*unit) == first).then_some(index * 8)
            })
    });
    let anything_else =
        (by_two.is_none() && by_eight.is_none()).then(|| (0..block.len()).step_by(align.max(1)));
    by_two
        .into_iter()
        .flatten()
        .chain(by_eight.into_iter().flatten())
        .chain(anything_else.into_iter().flatten())
        .filter(fits)
}

/// A little endian word of `width` bytes, 4 or 8, if it fits in the buffer.
pub(crate) fn word_at(buf: &[u8], offset: usize, width: usize) -> Option<u64> {
    let raw = buf.get(offset..offset.checked_add(width)?)?;
    let mut bytes = [0u8; 8];
    bytes[..width].copy_from_slice(raw);
    Some(u64::from_le_bytes(bytes))
}
