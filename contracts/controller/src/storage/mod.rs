//! Persistent storage accessors and TTL management.
//!
//! This module is deliberately split by concern rather than by key type so
//! that each subdomain can be understood, tested, and verified in isolation:
//!
//! - `account` — account metadata + the two per-side position maps
//!   (supply vs borrow). The split is load-bearing for gas and for the
//!   per-operation storage discipline described in ADR 0002.
//! - `debt` — helpers for the global `IsolatedDebt(asset)` counters that
//!   back the isolation debt ceiling feature.
//! - `emode` — e-mode category definitions and per-asset membership within
//!   categories. E-mode state is intentionally *not* stored inside accounts
//!   so that categories can be deprecated without mutating live accounts
//!   (see ADR 0008).
//! - `instance` — singleton instance-storage flags (approved tokens for
//!   listing, the flash-loan-in-progress guard).
//! - `market` — the `MarketConfig` record (status, pool address, asset
//!   risk params, oracle wiring).
//! - `pools` — the registry of deployed pool addresses and the list used by
//!   keepalive / batch index updates.
//! - `ttl` — the three-tier TTL renewal functions (user keys, protocol
//!   shared keys, controller instance). TTL policy is centralized here so
//!   that any change to Soroban rent parameters is made in one place.
//!
//! # Certora note
//!
//! When the `certora` feature is enabled the entire storage surface is
//! replaced by a harness (`verification_storage`) that returns nondet
//! values while preserving the key-type signatures the rest of the crate
//! compiles against. This is required because full persistent storage
//! reasoning is prohibitively expensive for the prover.
//!
//! Business logic should almost never call these functions directly;
//! prefer `ControllerCache` for reads and the higher-level flows for writes.

mod account;
mod debt;
mod emode;
mod instance;
mod market;
mod pools;
mod ttl;
#[cfg(feature = "certora")]
#[path = "../../../../verification/certora/controller/harness/storage.rs"]
mod verification_storage;
// ^ Augmentation (not replacement): adds verification-friendly getters and
//   Compat* types used by specs and harnesses. Prod storage modules remain.
//   This is the preferred "less magic" pattern vs full #[path] swaps.

pub(crate) use account::*;
pub(crate) use debt::*;
pub(crate) use emode::*;
pub(crate) use instance::*;
pub(crate) use market::*;
pub(crate) use pools::*;
pub(crate) use ttl::*;
#[cfg(feature = "certora")]
pub(crate) use verification_storage::*;
// The verification_storage symbols (get_position, asset_config::*, etc.)
// are available to specs only under certora; they delegate some cross-contract
// reads to pool_interface (summarized via the cross_contract harness).
