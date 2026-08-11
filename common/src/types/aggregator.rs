//! Type alias for the encoded swap instruction payload passed to the swap aggregator router.

use soroban_sdk::Bytes;

/// Encoded swap route passed to the aggregator router's `execute_strategy` entry point.
pub type StrategySwap = Bytes;
