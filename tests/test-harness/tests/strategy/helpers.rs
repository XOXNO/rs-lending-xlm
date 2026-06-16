use controller::types::StrategySwap;
use test_harness::{mock_swap_payload_xdr, LendingTest};

/// Placeholder swap that should only be used by tests failing before router execution.
pub fn build_swap_steps(
    t: &LendingTest,
    token_in: &str,
    token_out: &str,
    min_out: i128,
) -> StrategySwap {
    mock_swap_payload_xdr(
        &t.env,
        t.resolve_asset(token_in),
        t.resolve_asset(token_out),
        min_out,
    )
}
