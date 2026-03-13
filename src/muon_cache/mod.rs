//! MuonCache: embedded Redis-compatible cache (RESP3 + multi-DB + in-memory store).
#![allow(dead_code)]

pub mod store;
#[cfg(feature = "muoncache-core")]
pub mod core;
#[cfg(feature = "muoncache")]
pub mod resp;
#[cfg(feature = "muoncache")]
pub mod server;
#[cfg(feature = "muoncache-wasm")]
pub mod wasm;
#[cfg(feature = "muoncache-core")]
pub mod persist;
