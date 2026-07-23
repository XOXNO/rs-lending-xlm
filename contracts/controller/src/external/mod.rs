//! Outbound pool, SAC token, price-aggregator, and Blend migration clients.

pub(crate) mod blend;
#[cfg(not(feature = "certora"))]
pub(crate) mod price_aggregator;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/external/price_aggregator.rs"]
pub(crate) mod price_aggregator;

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
