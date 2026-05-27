//! Outbound cross-contract wrappers for pools and SAC token transfers.
//!
//! Business logic routes external calls through this module so Certora builds
//! can replace pool and token effects without changing controller call sites.

#[cfg(not(feature = "certora"))]
pub(crate) mod pool;
#[cfg(feature = "certora")]
#[path = "../../../../verification/certora/controller/harness/cross_contract/pool.rs"]
pub(crate) mod pool;

#[cfg(not(feature = "certora"))]
pub(crate) mod sac;
#[cfg(feature = "certora")]
#[path = "../../../../verification/certora/controller/harness/cross_contract/sac.rs"]
pub(crate) mod sac;
