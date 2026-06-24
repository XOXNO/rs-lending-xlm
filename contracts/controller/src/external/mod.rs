//! Outbound pool and SAC token wrappers.

/// Blend V2 pool client used by migration flows.
pub(crate) mod blend;

#[cfg(not(feature = "certora"))]
pub(crate) mod pool;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/external/pool.rs"]
pub(crate) mod pool;

#[cfg(not(feature = "certora"))]
pub(crate) mod sac;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/external/sac.rs"]
pub(crate) mod sac;
