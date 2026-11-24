//! # chainlab_engine
//!
//! The simulation core of CHAINLAB. It has **no UI dependencies**: the
//! terminal front end only reads [`Sim`] state and pushes [`Command`]s.
//!
//! Given the same chain config, parameter values, seed and command log the
//! engine produces a bit-identical state after any number of ticks. See
//! [`fixed`] for the numeric approach and [`rng`] for randomness.
//!
//! Reading order for learning the mechanics:
//! 1. [`engine::source`]     raw sources (harvest, depletion, regrowth)
//! 2. [`engine::processor`]  recipes (inputs -> outputs over a cycle)
//! 3. [`engine::logistics`]  carriers moving goods along links
//! 4. [`engine::workers`]    crew size and diminishing returns
//! 5. [`engine::stats`]      how throughput and utilisation are measured

pub mod config;
pub mod engine;
pub mod fixed;
pub mod rng;
pub mod scenarios;

pub use engine::{Command, Sim, TICKS_PER_SEC};
