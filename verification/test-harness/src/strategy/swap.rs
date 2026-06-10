use common::types::StrategySwap;
use soroban_sdk::{contracttype, xdr::ToXdr, Address, Bytes, Env};

use crate::core::LendingTest;

/// Default flash-loan fee in BPS for strategy presets.
pub const DEFAULT_FLASHLOAN_FEE_BPS: i128 = 9;

/// Returns the net strategy input after the default flash-loan fee.
pub fn apply_flash_fee(requested_raw: i128) -> i128 {
    requested_raw * (10_000 - DEFAULT_FLASHLOAN_FEE_BPS) / 10_000
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MockSwapPayload {
    pub min_out: i128,
    pub token_in: Address,
    pub token_out: Address,
}

pub fn mock_swap_payload_xdr(
    env: &Env,
    token_in: Address,
    token_out: Address,
    min_out: i128,
) -> Bytes {
    MockSwapPayload {
        min_out,
        token_in,
        token_out,
    }
    .to_xdr(env)
}

/// Builds a mock aggregator swap payload.
pub fn build_aggregator_swap(
    t: &LendingTest,
    token_in_name: &str,
    token_out_name: &str,
    _amount_in: i128,
    min_out: i128,
) -> StrategySwap {
    mock_swap_payload_xdr(
        &t.env,
        t.resolve_asset(token_in_name),
        t.resolve_asset(token_out_name),
        min_out,
    )
}
