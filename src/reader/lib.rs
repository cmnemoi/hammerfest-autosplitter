//! The reader: from the bytes of a Flash heap to a [`State`], or to nothing.
//!
//! It reads through [`avm1::Memory`], a trait of its own, and never through
//! the runtime. It prints nothing either: it tells a [`search_log::SearchLog`].
//! So it runs, and is tested, outside the WebAssembly sandbox.
//!
//! The rules are in `docs/specs/memory-reader.md`.

#![no_std]

extern crate alloc;

pub mod avm1;
pub mod hammerfest;
pub mod search_log;

/// Obfuscated property names, taken from `vendor/hf.map.json` by build.rs.
pub mod keys {
    include!(concat!(env!("OUT_DIR"), "/keys.rs"));
}

pub use hammerfest_core::State;
