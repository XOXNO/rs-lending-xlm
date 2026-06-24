//! Controller storage accessors and Soroban TTL renewal.
//!
//! Account metadata, supply maps, and debt maps use separate persistent keys.
//! Market, e-mode, pool-list, instance, and session keys stay behind typed
//! helpers to preserve storage-key stability.

mod account;
mod emode;
mod instance;
mod market;
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
pub(crate) use ttl::*;
#[cfg(feature = "certora")]
pub(crate) use verification_storage::*;
// Verification storage helpers are not compiled into production contracts.
