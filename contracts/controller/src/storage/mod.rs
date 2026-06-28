//! Controller storage accessors and Soroban TTL renewal.
//!
//! Account metadata, supply maps, and debt maps use separate persistent keys.
//! Spoke, oracle, instance, and session keys stay behind typed helpers to
//! preserve storage-key stability.

mod account;
mod instance;
mod spoke;
mod ttl;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/storage.rs"]
mod verification_storage;
// Certora-only getters preserve storage signatures while replacing heavy reads
// with verifier-friendly values.

pub(crate) use account::*;
pub(crate) use instance::*;
pub(crate) use spoke::*;
pub(crate) use ttl::*;
#[cfg(feature = "certora")]
pub(crate) use verification_storage::*;
// Verification storage helpers are not compiled into production contracts.
