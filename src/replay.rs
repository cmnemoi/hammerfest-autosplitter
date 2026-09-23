//! Replaying a capture of a real game.
//!
//! Every other test of the reader is served a heap this project wrote. Those
//! tests prove that the reader follows its own rules. They cannot prove that
//! `MEASURED`, `vendor/hf.map.json` and the layout derivation still match the
//! player and the SWF that ship, because nothing in them comes from Flash.
//!
//! This one does. The bytes were read out of a running game by
//! `scripts/capture_heap.py`, and turned into two dependency-free files by
//! `scripts/replay_fixture.py`:
//!
//! ```text
//!    heap.bin.gz  the bytes of the regions that were kept
//!    index.txt    the plugin range, the state the game was in, the map
//! ```
//!
//! The capture stays out of git when it is whole, so the test skips itself
//! when the files are absent. See `docs/how-to/capture-a-fixture.md`.

extern crate std;

use alloc::{string::String, vec::Vec};
use core::cell::RefCell;
use std::{fs, io::Read, path::PathBuf, println};

use crate::avm1::Memory;

/// A `0x...` address, as `index.txt` writes it.
fn hex(word: &str) -> Option<u64> {
    u64::from_str_radix(word.trim_start_matches("0x"), 16).ok()
}

/// One region of the captured heap.
struct Region {
    base: u64,
    size: u64,
    /// Where its bytes are in `heap.bin`, or `None` when the region was
    /// emptied. An emptied region is still part of the map, at its address and
    /// its size, and it reads as zeros.
    offset: Option<usize>,
}

/// What the game showed when the capture was taken.
///
/// It comes from `metadata.json`, which the capture tool filled by reading the
/// running game. So the oracle of this test is the Python reader, written
/// separately from the Rust one and sharing none of its constants.
///
/// The clocks are not here on purpose. A capture takes half a second, and the
/// game runs during it, so no clock in the file is exact. The fixture says so
/// itself, in its own `note`.
struct Says {
    level: i64,
    set: String,
    dim: i64,
}

pub struct Capture {
    bytes: Vec<u8>,
    regions: Vec<Region>,
    module: (u64, u64),
    says: Says,
    /// Regions that must read as zeros, whatever the file holds.
    ///
    /// Only `smallest_set_of_regions` writes this. It is how that search drops
    /// a region without rewriting eighty-five MiB.
    masked: RefCell<Vec<u64>>,
}

impl Memory for Capture {
    fn read_into(&self, address: u64, buf: &mut [u8]) -> Option<()> {
        let end = address.checked_add(buf.len() as u64)?;
        let region = self
            .regions
            .iter()
            .find(|r| address >= r.base && end <= r.base + r.size)?;
        match region.offset {
            _ if self.masked.borrow().contains(&region.base) => buf.fill(0),
            None => buf.fill(0),
            Some(offset) => {
                let at = offset + (address - region.base) as usize;
                buf.copy_from_slice(self.bytes.get(at..at + buf.len())?);
            }
        }
        Some(())
    }
}

impl Capture {
    /// Loads the capture of that name, or nothing when it is not there.
    pub fn load(name: &str) -> Option<Self> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("replay")
            .join(name);
        let index = fs::read_to_string(dir.join("index.txt")).ok()?;
        let packed = fs::File::open(dir.join("heap.bin.gz")).ok()?;
        let mut bytes = Vec::new();
        flate2::read::GzDecoder::new(packed)
            .read_to_end(&mut bytes)
            .ok()?;

        let mut regions = Vec::new();
        let mut module = (0, 0);
        let mut says = Says {
            level: -1,
            set: String::new(),
            dim: -1,
        };

        for line in index.lines() {
            let mut word = line.split_whitespace();
            match word.next() {
                Some("plugin") => {
                    module = (hex(word.next()?)?, hex(word.next()?)?);
                }
                Some("state") => {
                    for field in word {
                        let (key, value) = field.split_once('=')?;
                        match key {
                            "level" => says.level = value.parse().ok()?,
                            "set" => says.set = String::from(value),
                            "dim" => says.dim = value.parse().ok()?,
                            _ => {}
                        }
                    }
                }
                Some("region") => {
                    let base = hex(word.next()?)?;
                    let size = word.next()?.parse().ok()?;
                    let offset = word.next()?;
                    regions.push(Region {
                        base,
                        size,
                        offset: (offset != "-").then(|| offset.parse().ok()).flatten(),
                    });
                }
                _ => {}
            }
        }

        Some(Self {
            bytes,
            regions,
            module,
            says,
            masked: RefCell::new(Vec::new()),
        })
    }

    fn ranges(&self) -> Vec<(u64, u64)> {
        self.regions
            .iter()
            .map(|r| (r.base, r.base + r.size))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hammerfest_core::{Level, Policy, State, TimerState, World};

    use crate::hammerfest::{resolve, Anchor, Binary};
    use crate::test_heap::block_on;

    /// @spec reader::the-right-layout
    #[test]
    fn reads_a_real_game_out_of_a_capture() {
        let Some(capture) = Capture::load("main-world") else {
            println!("no capture in fixtures/replay/main-world, test skipped");
            return;
        };

        let found = block_on(resolve(
            &capture,
            capture.module,
            &mut Anchor::default(),
            &mut Binary::default(),
            &capture.ranges(),
        ));

        let mut game = found.expect("the reader found no game in a real heap");
        let state = game.read(&capture).expect("the reader read no state");

        assert_eq!(game.set, capture.says.set, "world");
        assert_eq!(
            Some(state.level.world),
            World::from_set_name(&capture.says.set),
            "world of the level"
        );
        assert_eq!(state.level.id, capture.says.level, "level");
        assert_eq!(state.dim, capture.says.dim, "dimension");
    }

    /// From the bytes a Flash player wrote, to the command LiveSplit gets.
    ///
    /// The two halves of the program never meet in any other test. The reader
    /// is proven on a heap we wrote, and the policy on states we wrote. Here
    /// the state comes out of a real game, and the level that follows it is
    /// the only thing this test invents.
    ///
    /// @spec crossing::one-split-per-crossing
    #[test]
    fn a_real_game_that_crosses_a_level_splits() {
        let Some(capture) = Capture::load("main-world") else {
            println!("no capture in fixtures/replay/main-world, test skipped");
            return;
        };
        let mut game = block_on(resolve(
            &capture,
            capture.module,
            &mut Anchor::default(),
            &mut Binary::default(),
            &capture.ranges(),
        ))
        .expect("the reader found no game in a real heap");
        let state = game.read(&capture).expect("the reader read no state");

        // The policy acts on a confirmed level, so every state is read twice.
        let mut policy = Policy::new();
        policy.tick(TimerState::Running, Some(state));
        policy.tick(TimerState::Running, Some(state));

        // The next level, as the game would write it one second later.
        let next = State {
            level: Level::new(state.level.world, state.level.id + 1),
            previous: state.level.id,
            chrono_ms: state.chrono_ms + 1_000,
            frame_timer: state.frame_timer + 32,
            ..state
        };
        policy.tick(TimerState::Running, Some(next));
        let actions = policy.tick(TimerState::Running, Some(next));

        assert!(actions.split, "a crossing of one level must split");
        assert_eq!(actions.skips, 0, "it must skip nothing");
    }

    /// Does the reader still find the game when those regions read as zeros?
    fn still_finds_the_game(capture: &Capture, masked: &[u64]) -> bool {
        capture.masked.replace(masked.to_vec());
        let found = block_on(resolve(
            capture,
            capture.module,
            &mut Anchor::default(),
            &mut Binary::default(),
            &capture.ranges(),
        ));
        let Some(mut game) = found else {
            return false;
        };
        let Some(state) = game.read(capture) else {
            return false;
        };
        game.set == capture.says.set && state.level.id == capture.says.level
    }

    /// The smallest set of regions that still holds the game.
    ///
    /// A capture is eighty-five MiB, and git keeps a file for ever. Only the
    /// regions that carry the object graph are worth that, and no rule tells
    /// which ones they are: the search reads the whole heap, so "the regions
    /// it read" is every region.
    ///
    /// So we ask instead. Empty the biggest region, look again, and keep it
    /// emptied while the game is still found. What is left is needed.
    ///
    /// It is not a test. Run it when a new capture has to be trimmed:
    ///
    /// ```sh
    /// cargo test -p hammerfest-autosplitter smallest -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "a tool, not a test: it trims a new capture"]
    fn smallest_set_of_regions() {
        let Some(capture) = Capture::load("main-world") else {
            println!("no capture in fixtures/replay/main-world");
            return;
        };
        assert!(
            still_finds_the_game(&capture, &[]),
            "the game is not there to begin with"
        );

        let mut order: Vec<(u64, u64)> = capture.regions.iter().map(|r| (r.size, r.base)).collect();
        order.sort_unstable();
        order.reverse();

        let mut masked: Vec<u64> = Vec::new();
        for (_, base) in order {
            masked.push(base);
            if !still_finds_the_game(&capture, &masked) {
                masked.pop();
            }
        }

        let kept: Vec<&Region> = capture
            .regions
            .iter()
            .filter(|r| !masked.contains(&r.base))
            .collect();
        let bytes: u64 = kept.iter().map(|r| r.size).sum();
        println!(
            "{} regions of {} kept, {:.1} MiB",
            kept.len(),
            capture.regions.len(),
            bytes as f64 / (1 << 20) as f64
        );
        let mut line = String::from("mise run replay-fixture -- main-world --keep");
        for region in kept {
            line.push_str(&std::format!(" {:#x}", region.base));
        }
        println!("{line}");
    }
}
