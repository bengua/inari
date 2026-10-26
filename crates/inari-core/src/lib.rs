//! Inari core: portable session capsules, provider handover, teleportation.
//!
//! This crate is the Rust implementation of the formats and protocols that the
//! Python reference (`src/inari_core`) defines and tests. Both must produce the
//! same bytes for the same state: a capsule written by one is verified by the other.

pub mod canon;
pub mod capsule;
