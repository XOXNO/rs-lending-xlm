//! Off-chain TTL keepalive + index-refresh service for the XOXNO Lending
//! protocol on Soroban.
//!
//! Binary entry point is in `main.rs`. The library is exposed only so tests
//! and one-shot tooling can reuse the modules.

pub mod config;
pub mod discovery;
pub mod keys;
pub mod metrics;
pub mod policy;
pub mod scheduler;
pub mod signer;
pub mod stellar;
