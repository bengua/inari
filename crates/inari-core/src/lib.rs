//! Inari core: portable session capsules, provider handover, teleportation.
//!
//! This crate is the Rust implementation of the formats and protocols that the
//! Python reference (`src/inari_core`) defines and tests. Both must produce the
//! same bytes for the same state: a capsule written by one is verified by the other.

pub mod agent;
pub mod approvals;
pub mod bus;
pub mod buslink;
pub mod canon;
pub mod capsule;
pub mod demo;
pub mod demo_agent;
pub mod enroll;
pub mod gateway;
pub mod host;
pub mod mesh;
pub mod node;
pub mod passes;
pub mod rpc;
pub mod session;
pub mod signature;
pub mod teleport;
pub mod updates;
pub mod usage;
pub mod wire;
