//! The adapter that reads the process, held to the contract of `Memory`.
//!
//! The reader's tests ask the same four questions of their test heap. Here
//! `ProcessMemory` answers them, over an `asr::Process` whose bytes come from
//! stubs of the runtime. A fifth question is asked of the adapter alone: a
//! host that writes into the buffer and then fails.

mod stubs;

#[path = "../../../reader/tests/contract/memory.rs"]
mod contract;

use asr::{Process, ProcessId};
use contract::{cases, contents, BASE};
use hammerfest_process::ProcessMemory;
use hammerfest_reader::avm1::Memory;
use stubs::Host;

fn regions() -> Vec<(u64, Vec<u8>)> {
    vec![(BASE, contents())]
}

/// The bytes stay served for as long as the guard lives, so every test
/// keeps both.
fn served(host: Host) -> (stubs::Installed, Process) {
    let held = stubs::serve(regions(), host);
    let process = Process::attach_by_pid(ProcessId(1)).expect("the stub always attaches");
    (held, process)
}

#[test]
fn reads_the_bytes_at_an_address() {
    let (_held, process) = served(Host::Honest);
    cases::reads_the_bytes_at_an_address(&ProcessMemory(&process));
}

#[test]
fn reads_up_to_the_last_byte_of_a_range() {
    let (_held, process) = served(Host::Honest);
    cases::reads_up_to_the_last_byte_of_a_range(&ProcessMemory(&process));
}

#[test]
fn refuses_a_read_that_runs_past_the_end_of_a_range() {
    let (_held, process) = served(Host::Honest);
    cases::refuses_a_read_that_runs_past_the_end_of_a_range(&ProcessMemory(&process));
}

#[test]
fn refuses_an_address_that_is_in_no_range() {
    let (_held, process) = served(Host::Honest);
    cases::refuses_an_address_that_is_in_no_range(&ProcessMemory(&process));
}

/// A host that writes into the buffer and then fails.
///
/// `sys.rs` allows it: it promises nothing about the buffer when
/// `process_read` returns `false`. The adapter must still answer `None`,
/// so that no caller can mistake those bytes for a value.
#[test]
fn refuses_even_when_the_host_dirtied_the_buffer() {
    let (_held, process) = served(Host::DirtiesThenFails);
    let mut buf = [0u8; 4];

    assert_eq!(ProcessMemory(&process).read_into(BASE, &mut buf), None);
}
