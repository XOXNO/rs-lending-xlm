use soroban_sdk::Bytes;

/// Opaque aggregator swap payload.
///
/// The controller forwards bytes to the configured aggregator and enforces
/// token balance deltas around the call.
pub type StrategySwap = Bytes;
