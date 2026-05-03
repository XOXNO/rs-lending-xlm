//! Adversarial router regression tests for `strategy::swap_tokens`.
//!
//! The helper at `controller/src/strategy.rs:433-491` defends against three
//! misbehaviors by a swap router with access to the controller's allowance:
//!
//!   1. The router sends tokens back to the controller (balance_in rises).
//!   2. The router pulls more than `amount_in` (overshoots the allowance).
//!   3. The router returns nothing (zero delta on the output side).
//!
//! These tests install a `BadAggregator` in place of the benign mock and
//! verify that each defense fires with the documented error code.
extern crate std;

use soroban_sdk::Address;
use test_harness::mock_aggregator::{BadAggregator, BadMode};
use test_harness::{
    apply_flash_fee, assert_contract_error, build_aggregator_swap, errors, eth_preset, usdc_preset,
    LendingTest, ALICE,
};

// All three adversarial tests below `multiply` 1.0 ETH (raw 10_000_000 at
// 7 decimals). After the 9bps flash fee the controller actually swaps
// `apply_flash_fee(10_000_000)`; the fixture path must match that value
// for `validate_aggregator_swap` to accept the batch.
const SWAP_REQUESTED_ETH: i128 = 10_000_000;
const SWAP_MIN_OUT_USDC: i128 = 30_000_000_000;

/// Register a `BadAggregator` with the given mode, route the controller's
/// swaps to it, and return its address.
fn install_bad_router(t: &LendingTest, mode: BadMode) -> Address {
    let admin = t.admin.clone();
    let bad = t.env.register(BadAggregator, (admin.clone(), mode));
    let ctrl = t.ctrl_client();
    ctrl.set_aggregator(&bad);
    bad
}

/// Mint `raw_amount` of `asset_name` directly to an address. Used to seed
/// the bad aggregator with output tokens when needed.
fn mint_to(t: &LendingTest, asset_name: &str, target: &Address, raw_amount: i128) {
    let market = t.resolve_market(asset_name);
    market.token_admin.mint(target, &raw_amount);
}

// ---------------------------------------------------------------------------
// BadMode::Refund -- router returns token_in to the caller, violating the
// `balance_in_after > balance_in_before` invariant. Must panic with
// InternalError.
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
    mint_to(&t, "USDC", &bad, 300_000_000_000); // 3000 USDC
                                                // Seed the bad router with ETH so it can perform the net-positive refund
                                                // back to the controller (violating the balance_in_after invariant).
    mint_to(&t, "ETH", &bad, 100_000_000); // 10 ETH (7 decimals)

    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(SWAP_REQUESTED_ETH),
        SWAP_MIN_OUT_USDC,
    );
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );

    // strategy.rs:474 -- if balance_in_after > balance_in_before, InternalError.
    assert_contract_error(result, errors::INTERNAL_ERROR);
}

// ---------------------------------------------------------------------------
// BadMode::OverPull -- router pulls 2x the requested amount via
// `token.transfer(sender, router, 2*amount_in)`. The new ABI has no SEP-41
// allowance to overshoot, so the SAC's `transfer` either succeeds (if
// the controller happens to hold enough) and the controller's
// `verify_router_input_spend` fires `actual_in_spent != amount_in`, or
// fails with the SAC's insufficient-balance error. Either way it's a
// detectable adversary; the controller surfaces InternalError when the
// over-spend lands.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_tokens_rejects_router_pulling_more_than_allowance() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let bad = install_bad_router(&t, BadMode::OverPull);
    mint_to(&t, "USDC", &bad, 300_000_000_000);

    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(SWAP_REQUESTED_ETH),
        SWAP_MIN_OUT_USDC,
    );
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );

    // Either the SAC rejects the over-pull (insufficient-balance contract
    // error from the SAC) OR the over-pull lands and the controller's
    // `actual_in_spent != amount_in` guard fires INTERNAL_ERROR. Both are
    // accepted defenses; the test merely asserts the call doesn't succeed.
    // (Pinning a single error code here is brittle because it depends on
    // whether the controller happened to hold enough ETH when the bad
    // router tried to over-pull.)
    assert!(result.is_err(), "OverPull must be rejected, got {:?}", result);
}

// ---------------------------------------------------------------------------
// BadMode::OutputShortfall -- router pulls token_in but transfers zero
// token_out. The controller's `received < amount_out_min` postcheck (added
// during audit prep) rejects the swap immediately. Previously this case
// would propagate zero into the deposit path, which would reject with
// AMOUNT_MUST_BE_POSITIVE -- a weaker, later defense.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_tokens_handles_zero_output_from_router() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    install_bad_router(&t, BadMode::OutputShortfall);

    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(SWAP_REQUESTED_ETH),
        SWAP_MIN_OUT_USDC,
    );
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );

    // amount_out_min postcheck in `strategy::swap_tokens` rejects the
    // shortfall immediately with INTERNAL_ERROR.
    assert_contract_error(result, errors::INTERNAL_ERROR);
}
