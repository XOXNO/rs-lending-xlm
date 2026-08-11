//! External contract clients used by the controller: the Blend Pool, the
//! spoke lending pool, the price aggregator oracle, and Stellar Asset
//! Contract tokens. Each submodule selects between the production
//! implementation and a Certora harness stub based on the `certora` feature.

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
