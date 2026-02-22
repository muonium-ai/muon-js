//! Mini Redis clone scaffolding (RESP3 + multi-DB + in-memory store).
#![allow(dead_code)]

pub mod store;
#[cfg(feature = "mini-redis-core")]
pub mod core;
#[cfg(feature = "mini-redis")]
pub mod resp;
#[cfg(feature = "mini-redis")]
pub mod server;
#[cfg(feature = "mini-redis-wasm")]
pub mod wasm;
#[cfg(feature = "mini-redis-core")]
pub mod persist;
