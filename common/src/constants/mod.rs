//! Protocol-wide constants shared across contracts: fixed-point scales, decimal
//! bounds, price/tolerance/liquidation limits, storage TTL thresholds, and
//! pool interest-index bounds. Split across private `pool` and `shared`
//! submodules whose contents are re-exported flat from this module.

mod pool;
mod shared;

pub use pool::*;
pub use shared::*;

#[cfg(test)]
#[path = "../../tests/constants.rs"]
mod tests;
