//! What the reader costs, counted, and held to a baseline.
//!
//! The rules are in `docs/specs/read-cost.md`.

extern crate std;

use core::{
    cell::Cell,
    fmt,
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};
use std::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};

use hammerfest_reader::avm1::Memory;

/// @spec cost::counted-not-timed
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cost {
    /// The calls made to `Memory`.
    pub reads: u64,
    /// The bytes those calls asked for.
    pub bytes: u64,
    /// The ticks given back to the runtime.
    pub yields: u64,
}

impl fmt::Display for Cost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} reads {} bytes {} yields",
            self.reads, self.bytes, self.yields
        )
    }
}

/// A memory that counts what it is asked, and serves it from another.
///
/// @spec cost.count::every-read-and-its-bytes
pub struct Metered<'a> {
    memory: &'a dyn Memory,
    reads: Cell<u64>,
    bytes: Cell<u64>,
}

impl<'a> Metered<'a> {
    pub fn new(memory: &'a dyn Memory) -> Self {
        Self {
            memory,
            reads: Cell::new(0),
            bytes: Cell::new(0),
        }
    }
}

impl Memory for Metered<'_> {
    fn read_into(&self, address: u64, buf: &mut [u8]) -> Option<()> {
        self.reads.set(self.reads.get() + 1);
        self.bytes.set(self.bytes.get() + buf.len() as u64);
        self.memory.read_into(address, buf)
    }
}

/// Drives the reader to its answer, and says what it cost.
///
/// The reader awaits `next_tick` and nothing else, and `next_tick` is pending
/// exactly once. So each pending poll is one tick given back.
///
/// @spec cost.count::every-yield
pub fn measure<F: Future>(metered: &Metered, future: F) -> (F::Output, Cost) {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    let mut yields = 0;
    let answer = loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(answer) => break answer,
            Poll::Pending => yields += 1,
        }
    };
    let cost = Cost {
        reads: metered.reads.get(),
        bytes: metered.bytes.get(),
        yields,
    };
    (answer, cost)
}

/// The cost of each situation, by name.
#[derive(Default)]
pub struct Costs(BTreeMap<String, Cost>);

impl Costs {
    pub fn record(&mut self, situation: &str, cost: Cost) {
        self.0.insert(situation.to_string(), cost);
    }

    /// One line per situation, as `render` writes it. A line that does not
    /// parse is left out, and then fails as a situation with no baseline.
    pub fn parse(text: &str) -> Self {
        let mut costs = Self::default();
        for line in text.lines().filter(|line| !line.starts_with('#')) {
            let words: Vec<&str> = line.split_whitespace().collect();
            if let [situation, reads, "reads", bytes, "bytes", yields, "yields"] = words[..] {
                if let (Ok(reads), Ok(bytes), Ok(yields)) =
                    (reads.parse(), bytes.parse(), yields.parse())
                {
                    costs.record(
                        situation,
                        Cost {
                            reads,
                            bytes,
                            yields,
                        },
                    );
                }
            }
        }
        costs
    }

    pub fn render(&self) -> String {
        self.0
            .iter()
            .map(|(situation, cost)| format!("{situation} {cost}\n"))
            .collect()
    }

    /// Every way these costs and the baseline disagree, one line each.
    ///
    /// @spec cost::any-change-fails
    /// @spec cost::a-missing-measure-fails
    pub fn differences_from(&self, baseline: &Self) -> Vec<String> {
        let situations: std::collections::BTreeSet<&String> =
            self.0.keys().chain(baseline.0.keys()).collect();
        situations
            .into_iter()
            .filter_map(
                |situation| match (baseline.0.get(situation), self.0.get(situation)) {
                    (Some(expected), Some(measured)) if expected == measured => None,
                    (Some(expected), Some(measured)) => Some(format!(
                        "{situation}: expected {expected}, measured {measured}"
                    )),
                    (None, Some(measured)) => {
                        Some(format!("{situation}: no baseline, measured {measured}"))
                    }
                    (Some(expected), None) => Some(format!(
                        "{situation}: expected {expected}, no situation measures it"
                    )),
                    (None, None) => None,
                },
            )
            .collect()
    }
}

#[cfg(test)]
mod situations {
    use std::{env, fs, path::PathBuf};

    use super::*;
    use crate::replay::Capture;
    use crate::test_heap::{given_a_heap, MODULE};
    use hammerfest_reader::hammerfest::{resolve, Anchor, Game};
    use hammerfest_reader::pepper_flash::Binary;
    use hammerfest_reader::search_log::Silent;

    const BASELINE: &str = "fixtures/read-cost.txt";
    const HEADER: &str = "\
# What each situation costs the reader. See docs/specs/read-cost.md.
# Do not edit by hand. Run `mise run read-cost-update`, and commit the change.
";

    /// One search, with what earlier searches learned, and its cost recorded.
    fn search(
        costs: &mut Costs,
        situation: &str,
        memory: &dyn Memory,
        module: (u64, u64),
        (anchor, binary): (&mut Anchor, &mut Binary),
        ranges: &[(u64, u64)],
    ) -> Option<Game> {
        let metered = Metered::new(memory);
        let (game, cost) = measure(
            &metered,
            resolve(&metered, module, anchor, binary, ranges, &mut Silent),
        );
        costs.record(situation, cost);
        game
    }

    fn a_real_game(costs: &mut Costs) {
        let capture = Capture::load("main-world").expect(
            "the capture fixtures/replay/main-world is missing, so its cost cannot be measured",
        );
        let (mut anchor, mut binary) = (Anchor::default(), Binary::default());
        let ranges = capture.ranges();

        let first = search(
            costs,
            "real-game/first-search",
            &capture,
            capture.module,
            (&mut anchor, &mut binary),
            &ranges,
        );
        let second = search(
            costs,
            "real-game/second-search",
            &capture,
            capture.module,
            (&mut anchor, &mut binary),
            &ranges,
        );
        assert!(
            first.is_some(),
            "the first search found no game in a real heap"
        );
        let mut game = second.expect("the second search found no game in a real heap");

        for situation in ["real-game/first-read", "real-game/next-read"] {
            let metered = Metered::new(&capture);
            let (state, cost) = measure(&metered, async { game.read(&metered) });
            assert!(state.is_some(), "{situation} read no state in a real game");
            costs.record(situation, cost);
        }
    }

    fn a_game_that_is_over(costs: &mut Costs) {
        let mut fixture = given_a_heap().with_a_game().that_is_over().build();
        let (heap, ranges) = (fixture.heap(), fixture.ranges());

        for situation in ["game-over/first-search", "game-over/second-search"] {
            let found = search(
                costs,
                situation,
                &heap,
                MODULE,
                (&mut fixture.anchor, &mut fixture.binary),
                &ranges,
            );
            assert!(found.is_none(), "{situation} found a game that is over");
        }
    }

    fn measured() -> Costs {
        let mut costs = Costs::default();
        a_real_game(&mut costs);
        a_game_that_is_over(&mut costs);
        costs
    }

    /// @spec cost::any-change-fails
    /// @spec cost::a-missing-measure-fails
    #[test]
    fn the_reader_costs_what_the_baseline_says() {
        let measured = measured();
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(BASELINE);
        if env::var_os("READ_COST_UPDATE").is_some() {
            fs::write(&path, format!("{HEADER}{}", measured.render()))
                .expect("the baseline could not be written");
            return;
        }
        let baseline = Costs::parse(&fs::read_to_string(&path).unwrap_or_default());

        let differences = measured.differences_from(&baseline);

        assert!(
            differences.is_empty(),
            "the reader does not cost what {BASELINE} says:\n  {}\n\
             If the change is wanted, run `mise run read-cost-update` and commit the file.",
            differences.join("\n  ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_contract::Heap;

    fn a_cost(reads: u64, bytes: u64, yields: u64) -> Cost {
        Cost {
            reads,
            bytes,
            yields,
        }
    }

    fn a_baseline(lines: &[(&str, Cost)]) -> Costs {
        let mut costs = Costs::default();
        for (situation, cost) in lines {
            costs.record(situation, *cost);
        }
        costs
    }

    /** @spec cost.compare::the-same-cost */
    #[test]
    fn the_same_cost_fails_nothing() {
        let baseline = a_baseline(&[("real-game/read", a_cost(12, 96, 0))]);
        let measured = a_baseline(&[("real-game/read", a_cost(12, 96, 0))]);

        assert_eq!(measured.differences_from(&baseline), Vec::<String>::new());
    }

    /** @spec cost.compare::a-higher-cost */
    #[test]
    fn a_higher_cost_names_the_situation_and_both_costs() {
        let baseline = a_baseline(&[("real-game/read", a_cost(12, 96, 0))]);
        let measured = a_baseline(&[("real-game/read", a_cost(13, 96, 0))]);

        let differences = measured.differences_from(&baseline);

        assert_eq!(
            differences,
            ["real-game/read: expected 12 reads 96 bytes 0 yields, measured 13 reads 96 bytes 0 yields"]
        );
    }

    /** @spec cost.compare::a-lower-cost */
    #[test]
    fn a_lower_cost_fails_too() {
        let baseline = a_baseline(&[("real-game/read", a_cost(12, 96, 0))]);
        let measured = a_baseline(&[("real-game/read", a_cost(12, 95, 0))]);

        assert_eq!(measured.differences_from(&baseline).len(), 1);
    }

    /** @spec cost.compare::a-situation-with-no-baseline */
    #[test]
    fn a_situation_with_no_baseline_fails() {
        let baseline = Costs::default();
        let measured = a_baseline(&[("real-game/read", a_cost(12, 96, 0))]);

        assert_eq!(
            measured.differences_from(&baseline),
            ["real-game/read: no baseline, measured 12 reads 96 bytes 0 yields"]
        );
    }

    /** @spec cost.compare::a-baseline-with-no-situation */
    #[test]
    fn a_baseline_with_no_situation_fails() {
        let baseline = a_baseline(&[("real-game/read", a_cost(12, 96, 0))]);
        let measured = Costs::default();

        assert_eq!(
            measured.differences_from(&baseline),
            ["real-game/read: expected 12 reads 96 bytes 0 yields, no situation measures it"]
        );
    }

    #[test]
    fn a_baseline_reads_back_what_it_wrote() {
        let written = a_baseline(&[
            ("real-game/read", a_cost(12, 96, 0)),
            ("game-over/first-search", a_cost(300, 1 << 20, 4)),
        ]);

        let read = Costs::parse(&written.render());

        assert_eq!(read.differences_from(&written), Vec::<String>::new());
    }

    /** @spec cost.count::every-read-and-its-bytes */
    #[test]
    fn counts_every_read_and_its_bytes_whether_it_succeeds_or_not() {
        let heap = Heap::default().with_range(0x1000, std::vec![0; 8]);
        let metered = Metered::new(&heap);

        let ((), cost) = measure(&metered, async {
            let _ = metered.read_into(0x1000, &mut [0; 8]);
            let _ = metered.read_into(0x9000, &mut [0; 16]);
        });

        assert_eq!(cost, a_cost(2, 24, 0));
    }

    /// Pending once, as the reader is when it lets a tick pass.
    fn a_tick_given_back() -> impl Future<Output = ()> {
        let mut given_back = false;
        core::future::poll_fn(move |_| {
            if core::mem::replace(&mut given_back, true) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
    }

    /** @spec cost.count::every-yield */
    #[test]
    fn counts_every_tick_given_back() {
        let heap = Heap::default();
        let metered = Metered::new(&heap);

        let ((), cost) = measure(&metered, async {
            for _ in 0..3 {
                a_tick_given_back().await;
            }
        });

        assert_eq!(cost, a_cost(0, 0, 3));
    }
}
