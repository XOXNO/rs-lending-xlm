//! Controller storage accessors and Soroban TTL renewal.
//!
//! Account metadata, supply maps, and debt maps are separate persistent keys
//! so supply-only and repay-only flows avoid touching unrelated position state.
//! Market, e-mode, pool-list, and instance/session keys are kept behind typed
//! helpers to preserve storage-key stability.
//!
//! Mutating business logic should read through `Cache` unless it is updating a
//! final position map, TTL, or guard flag.

mod account;
mod emode;
mod instance;
mod market;
mod pools;
mod ttl;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/storage.rs"]
mod verification_storage;
// Certora-only getters preserve storage signatures while replacing heavy reads
// with verifier-friendly values.

pub(crate) use account::*;
pub(crate) use emode::*;
pub(crate) use instance::*;
pub(crate) use market::*;
pub(crate) use pools::*;
pub(crate) use ttl::*;
#[cfg(feature = "certora")]
pub(crate) use verification_storage::*;
// Verification storage helpers are not compiled into production contracts.
