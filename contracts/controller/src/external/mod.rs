//! Cross-contract adapters. Certora substitutes pool, oracle, and NFT clients
//! with harness implementations.

pub(crate) mod blend;
#[cfg(not(feature = "certora"))]
pub(crate) mod pool;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/external/pool.rs"]
pub(crate) mod pool;

#[cfg(not(feature = "certora"))]
pub(crate) mod price_aggregator;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/external/price_aggregator.rs"]
pub(crate) mod price_aggregator;

#[cfg(not(feature = "certora"))]
pub(crate) mod position_nft;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/external/position_nft.rs"]
pub(crate) mod position_nft;
