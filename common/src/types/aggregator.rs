use soroban_sdk::Bytes;

/// Opaque aggregator swap payload.
///
/// The controller does not decode or validate this payload. It only forwards
/// the bytes to the configured aggregator, while enforcing its own concrete
/// token balance deltas around the call.
pub type StrategySwap = Bytes;

pub type SwapSteps = StrategySwap;
