//! Adversarial router regression tests for `strategy::swap_tokens`.
//!
//! The helper at `controller/src/strategy.rs:433-491` defends against three
//! misbehaviors by a swap router with access to the controller's allowance:
//!
//!   1. The router sending tokens BACK to the controller (balance_in increase).
//!   2. The router pulling MORE than `amount_in` (overshoot the allowance).
//!   3. The router returning nothing (zero delta on the output side).
//!
//! These tests install a `BadAggregator` in place of the benign mock and
//! verify each defense fires with the documented error code.
extern crate std;

use common::types::{DexDistribution, Protocol, SwapSteps};
use soroban_sdk::{vec, Address};
use test_harness::mock_aggregator::{BadAggregator, BadMode};
use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, LendingTest, ALICE,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_swap_steps(t: &LendingTest, token_in: &str, token_out: &str, min_out: i128) -> SwapSteps {
    let env = &t.env;
    let in_addr = t.resolve_market(token_in).asset.clone();
    let out_addr = t.resolve_market(token_out).asset.clone();
    SwapSteps {
        amount_out_min: min_out,
        distribution: vec![
            env,
            DexDistribution {
                protocol_id: Protocol::Soroswap,
                path: vec![env, in_addr, out_addr],
                parts: 1,
                bytes: None,
            },
        ],
    }
}

/// Register a `BadAggregator` with the specified mode, configure the
/// controller to route swaps to it, and return its address.
fn install_bad_router(t: &LendingTest, mode: BadMode) -> Address {
    let admin = t.admin.clone();
    let bad = t.env.register(BadAggregator, (admin.clone(), mode));
    let ctrl = t.ctrl_client();
    ctrl.set_aggregator(&bad);
    bad
}

/// Mint `raw_amount` of `asset_name` directly to an address (used to seed
/// the bad aggregator with output tokens when needed).
fn mint_to(t: &LendingTest, asset_name: &str, target: &Address, raw_amount: i128) {
    let market = t.resolve_market(asset_name);
    market.token_admin.mint(target, &raw_amount);
}

// ---------------------------------------------------------------------------
// BadMode::Refund — router returns token_in to the caller, violating the
// `balance_in_after > balance_in_before` invariant. Must panic InternalError.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_tokens_panics_when_router_refunds_token_in() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let bad = install_bad_router(&t, BadMode::Refund);
    // Seed the bad router with USDC output so it can satisfy the swap's
    // `amount_out_min` transfer before the adversarial token_in refund.
    mint_to(&t, "USDC", &bad, 3_000_00000000); // 3000 USDC
    // Seed the bad router with ETH so it can perform the net-positive
    // refund back to the controller (violating balance_in_after invariant).
    mint_to(&t, "ETH", &bad, 10_0000000); // 10 ETH (7 decimals)

    let steps = build_swap_steps(&t, "ETH", "USDC", 3_000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );

    // strategy.rs:474 — if balance_in_after > balance_in_before, InternalError.
    assert_contract_error(result, errors::INTERNAL_ERROR);
}

// ---------------------------------------------------------------------------
// BadMode::OverPull — router pulls 2x the approved amount. The controller
// pre-approves exactly `amount_in`, so `transfer_from` for 2x amount must
// fail inside the token contract (host-level). This proves the controller
// does NOT over-approve.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_tokens_rejects_router_pulling_more_than_allowance() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let bad = install_bad_router(&t, BadMode::OverPull);
    mint_to(&t, "USDC", &bad, 3_000_00000000);

    let steps = build_swap_steps(&t, "ETH", "USDC", 3_000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );

    // The transfer_from for 2x amount_in fails inside the token contract.
    // Any concrete contract error is acceptable evidence that the controller
    // did not pre-approve more than requested; we just need !is_ok.
    assert!(
        result.is_err(),
        "bad router should have been blocked by the token allowance, got Ok({:?})",
        result
    );
}

// ---------------------------------------------------------------------------
// BadMode::OutputShortfall — router pulls token_in but transfers zero
// token_out. The balance_in invariant holds, but the output-side delta is
// zero, which the downstream deposit path rejects (amount must be positive).
// ---------------------------------------------------------------------------

#[test]
fn test_swap_tokens_handles_zero_output_from_router() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    install_bad_router(&t, BadMode::OutputShortfall);

    let steps = build_swap_steps(&t, "ETH", "USDC", 3_000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );

    // After the bad swap, the deposit path sees 0 USDC to add.
    // `supply::process_deposit` rejects with AMOUNT_MUST_BE_POSITIVE.
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}
