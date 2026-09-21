//! Le coeur de l'autosplitter, sans memoire ni runtime.
//!
//! Tout ce qui se decide -- demarrer, splitter, remettre a zero, abandonner une
//! resolution -- se decide ici, a partir de valeurs. Rien dans ce crate ne lit
//! un process, ne parle a LiveSplit, ni ne depend d'`asr`, et c'est
//! precisement ce qui le rend testable : les symboles du runtime ASR n'existent
//! que dans le bac a sable WebAssembly.
//!
//! La partie qui lit la memoire de Pepper Flash vit dans le crate parent et
//! n'a plus qu'un role : produire un [`State`] et executer des [`Actions`].

#![no_std]

pub mod atom;
pub mod policy;

pub use policy::{duration_ms, Actions, Policy, Rules, State, TimerState};
