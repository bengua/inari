//! Inari core: portable session capsules, agents driven through their harnesses, a mesh of
//! cores, teleportation, and the web interface they serve. One binary, `inari-core-rs`.
//!
//! The formats were first defined by a Python reference, retired in October 2026: capsules it
//! wrote are test fixtures (`tests/fixtures/python`), and the core is checked to read them
//! hash for hash.

pub mod agent;
pub mod aimemory;
pub mod approvals;
pub mod bus;
pub mod buslink;
pub mod canon;
pub mod capsule;
pub mod demo;
pub mod demo_agent;
pub mod engine;
pub mod enroll;
pub mod gateway;
pub mod host;
pub mod livekit;
pub mod mesh;
pub mod node;
pub mod openfox;
pub mod openfox_library;
pub mod passes;
pub mod personas;
pub mod policy;
pub mod rpc;
pub mod sandbox;
pub mod session;
pub mod signature;
pub mod teleport;
pub mod updates;
pub mod usage;
pub mod wire;
