//! Protocol-wide constants shared across contracts: fixed-point scales, decimal
//! bounds, price/tolerance/liquidation limits, storage TTL thresholds, and
//! pool interest-index bounds. Declares the [`pool`] and [`shared`] constant
//! modules and re-exports their contents.

pub mod pool;
pub mod shared;

pub use pool::*;
pub use shared::*;

#[cfg(test)]
#[path = "../../tests/constants.rs"]
mod tests;
