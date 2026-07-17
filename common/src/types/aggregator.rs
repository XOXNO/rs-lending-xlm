//! Aggregator boundary type: the opaque [`StrategySwap`] swap payload the
//! controller forwards to the configured aggregator.

use soroban_sdk::Bytes;

/// Opaque swap payload; controller enforces balance deltas around the call.
pub type StrategySwap = Bytes;
