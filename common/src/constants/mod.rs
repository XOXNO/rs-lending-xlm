//! Protocol-wide constants shared across contracts. They cover fixed-point scales,
//! decimal bounds, price limits, tolerance limits, liquidation limits, storage TTL
//! thresholds, and pool interest-index bounds. The constants live in private `pool`
//! and `shared` submodules. This module re-exports their contents flat.

mod pool;
mod shared;

pub use pool::*;
pub use shared::*;

#[cfg(test)]
#[path = "../../tests/constants.rs"]
mod tests;
