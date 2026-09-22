//! The core of the autosplitter, with no memory access and no runtime.
//!
//! Everything that is decided -- when to start, split, reset, or drop a
//! resolution -- is decided here, from values alone. Nothing in this crate
//! reads a process, talks to LiveSplit, or depends on `asr`. That is exactly
//! what makes it testable: the ASR runtime symbols exist only inside the
//! WebAssembly sandbox.
//!
//! The part that reads Pepper Flash memory lives in the parent crate. It has
//! one job left: produce a [`State`] and execute the [`Actions`].

#![no_std]

pub mod atom;
pub mod end_sequence;
pub mod policy;

pub use end_sequence::EndSequence;
pub use policy::{duration_ms, Actions, Policy, State, TimerState};
