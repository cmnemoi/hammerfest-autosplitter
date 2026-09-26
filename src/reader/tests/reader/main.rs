//! The reader's tests, in one test binary.
//!
//! The reader is served bytes and nothing else, so everything here runs
//! outside the WebAssembly sandbox, with no stub of the runtime.

extern crate alloc;

mod browser;
mod memory_contract;
mod pepper_flash_heap;
mod read_cost;
mod replay;
mod ruffle_heap;
mod scenarios;
