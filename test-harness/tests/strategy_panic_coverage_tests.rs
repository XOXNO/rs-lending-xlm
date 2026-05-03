//! Directed regression tests for every `panic_with_error!` site in
//! `controller/src/strategy.rs` that lacked explicit coverage.
//!
//! Auditing strategy.rs surfaced 8 panic sites that no directed test
//! exercised end-to-end. Each test here pins the exact contract error code
//! so that a regression which substitutes a different error (or, worse,
//! silently succeeds) breaks CI.
//!
//! The tests also cover:
//!   * `multiply` Case 1 (initial_payment == collateral_token).
//!   * `multiply` Case 3 (initial_payment is a third token) happy path.
//!   * `swap_tokens` post-failure allowance state (NEW-01 regression).
extern crate std;

use common::types::AggregatorSwap;
use soroban_sdk::Vec;
use test_harness::{apply_flash_fee, build_aggregator_swap};
use soroban_sdk::token;
use test_harness::mock_aggregator::{BadAggregator, BadMode};
use test_harness::{
    assert_contract_error, errors, eth_preset, usdc_preset, wbtc_preset, LendingTest, ALICE, BOB,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_swap_steps(t: &LendingTest, _token_in: &str, _token_out: &str, min_out: i128) -> AggregatorSwap {
    // Placeholder fixture for compile-clean tests. The new aggregator ABI
    // requires per-path SwapHop entries; tests that actually exercise the
    // swap path must build a real `AggregatorSwap` inline (with `SwapPath`
    // / `SwapHop` matching the strategy's amount_in and tokens). Pre-swap
    // error-path tests pass through this without reaching swap_tokens.
    AggregatorSwap {
        paths: Vec::new(&t.env),
        total_min_out: min_out,
    }
}

fn flatten<T>(
    r: Result<Result<T, soroban_sdk::Error>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<T, soroban_sdk::Error> {
    match r {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e),
        Err(invoke) => Err(invoke.expect("expected contract error, got host-level InvokeError")),
    }
}

// ===========================================================================
// Part 1: missing `panic_with_error!` sites in strategy.rs
// ===========================================================================

// ---------------------------------------------------------------------------
// strategy.rs:91 -- ConvertStepsRequired
//
// When multiply receives an initial_payment whose token is a third token
// (neither collateral nor debt), `convert_steps` MUST be Some. The current
// suite had zero coverage for this panic site.
// ---------------------------------------------------------------------------
#[test]
fn test_multiply_third_token_payment_without_convert_steps_rejects() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    let alice = t.get_or_create_user(ALICE);
    // Mint WBTC to Alice (the third token).
    t.resolve_market("WBTC")
        .token_admin
        .mint(&alice, &10_000_000i128); // 0.1 WBTC

    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let wbtc = t.resolve_asset("WBTC");

    t.fund_router("USDC", 3_000.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 30_000_000_000);

    let ctrl = t.ctrl_client();
    // initial_payment = WBTC (third token), convert_steps = None: must panic.
    let result = ctrl.try_multiply(
        &alice,
        &0u64,
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
        &steps,
        &Some((wbtc, 100_000i128)),
        &None, // <- key: no convert_steps for third-token payment
    );
    assert_contract_error(flatten(result), errors::CONVERT_STEPS_REQUIRED);
}

// ---------------------------------------------------------------------------
// strategy.rs:140 -- AccountModeMismatch
//
// Reusing an existing account with a different mode must be rejected. No
// prior test exercised this path.
// ---------------------------------------------------------------------------
#[test]
fn test_multiply_existing_account_mode_mismatch_rejects() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Create an account explicitly in Multiply mode.
    let account_id = t.create_account_full(ALICE, 0, common::types::PositionMode::Multiply, false);
    t.supply_to(ALICE, account_id, "USDC", 1_000.0);

    t.fund_router("USDC", 3_000.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 30_000_000_000);
    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    // Try to reuse the Multiply account with Long mode: must reject.
    let ctrl = t.ctrl_client();
    let result = ctrl.try_multiply(
        &alice,
        &account_id,
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Long, // mismatch
        &steps,
        &None,
        &None,
    );
    assert_contract_error(flatten(result), errors::ACCOUNT_MODE_MISMATCH);
}

// ---------------------------------------------------------------------------
// strategy.rs:284 -- DebtPositionNotFound in swap_debt
// Alice tries to swap an ETH debt she does not owe. The test was missing.
//
// Call order in process_swap_debt:
//   1. handle_create_borrow_strategy(new_debt_token): flash-borrows the new
//      token.
//   2. swap_tokens(new_debt -> existing_debt).
//   3. borrow_positions.get(existing_debt_token): panics if missing (line
//      284).
// The router must be funded correctly and the flash borrow must succeed.
// ---------------------------------------------------------------------------
#[test]
fn test_swap_debt_existing_position_missing_rejects() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    // Alice supplies USDC and borrows WBTC. She does not borrow ETH.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "WBTC", 0.01);

    // Swap direction: new=WBTC, swap WBTC -> ETH. The router must be funded
    // with ETH.
    t.fund_router("ETH", 0.5);
    // 0.001 WBTC (7 decimals = 10_000 raw) flash-borrowed minus 9bps fee.
    let steps =
        build_aggregator_swap(&t, "WBTC", "ETH", apply_flash_fee(10_000), 5_000_000);
    // existing=ETH (Alice does not hold it). new=WBTC (Alice already holds
    // WBTC debt, but swap_debt requires only the existing debt to be
    // present -- not the new one).
    let result = t.try_swap_debt(ALICE, "ETH", 0.001, "WBTC", &steps);
    assert_contract_error(result, errors::DEBT_POSITION_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// strategy.rs:373 -- CollateralPositionNotFound in swap_collateral
// Alice tries to swap WBTC collateral she does not hold.
// ---------------------------------------------------------------------------

#[test]
fn test_swap_collateral_position_missing_rejects() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    // No WBTC supply position.

    let steps = build_swap_steps(&t, "WBTC", "ETH", 5_0000000);
    let result = t.try_swap_collateral(ALICE, "WBTC", 0.01, "ETH", &steps);
    assert_contract_error(result, errors::COLLATERAL_POSITION_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// strategy.rs:527 -- CollateralPositionNotFound in repay_debt_with_collateral
// Alice tries to repay using WBTC collateral she does not hold.
// ---------------------------------------------------------------------------

#[test]
fn test_repay_debt_with_collateral_missing_collateral_rejects() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    // Alice has USDC collateral and ETH debt, but no WBTC collateral.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_0000000);
    let result = t.try_repay_debt_with_collateral(ALICE, "WBTC", 0.01, "ETH", &steps, false);
    assert_contract_error(result, errors::COLLATERAL_POSITION_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// strategy.rs -- DebtPositionNotFound in repay_debt_with_collateral
// (regression)
//
// The audit surfaced two bugs here:
//   1. swap_tokens received `collateral_amount` (the requested amount), not
//      the actual withdrawal delta. Same M-11 pattern that had been fixed
//      for swap_collateral but not ported here.
//   2. debt_tok.transfer ran before borrow_positions.get(debt_token), so a
//      missing debt position host-panicked on the transfer rather than
//      surfacing DebtPositionNotFound (120).
//
// Both fixes landed in strategy.rs: both positions are validated up-front,
// and the function then measures the actual withdrawal delta and feeds it
// into swap_tokens. This test pins the DebtPositionNotFound guard.
// ---------------------------------------------------------------------------
#[test]
fn test_repay_debt_with_collateral_missing_debt_rejects() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    // Alice supplies USDC and borrows ETH (not WBTC).
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    t.fund_router("WBTC", 0.01);
    let steps = build_swap_steps(&t, "USDC", "WBTC", 1_000_000);
    let result = t.try_repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "WBTC", &steps, false);
    assert_contract_error(result, errors::DEBT_POSITION_NOT_FOUND);
}

// ---------------------------------------------------------------------------
// strategy.rs:600 -- CannotCloseWithRemainingDebt
//
// close_position=true must be rejected if the account still has debt.
// Lines 599-601 guard this explicitly; no prior test exercised it.
// ---------------------------------------------------------------------------
#[test]
fn test_repay_debt_with_collateral_close_with_remaining_debt_rejects() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice has a large ETH debt; a small repay will not zero it out.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 5.0);

    // Repay only a tiny fraction; Alice still owes ETH.
    t.fund_router("ETH", 0.01);
    // repay_debt_with_collateral withdraws 20 USDC (raw 200_000_000); no flash fee.
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 200_000_000, 100_000);

    // close_position=true: must reject with CannotCloseWithRemainingDebt.
    let result = t.try_repay_debt_with_collateral(ALICE, "USDC", 20.0, "ETH", &steps, true);
    assert_contract_error(result, errors::CANNOT_CLOSE_WITH_REMAINING_DEBT);
}

// ===========================================================================
// Part 2: initial_payment branches
// ===========================================================================

// ---------------------------------------------------------------------------
// Case 1: initial_payment == collateral_token (happy path).
// strategy.rs:80-82 -- the payment is added directly to collateral_amount.
// No prior directed test exercised this branch end-to-end.
// ---------------------------------------------------------------------------
#[test]
fn test_multiply_with_collateral_token_initial_payment() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let alice = t.get_or_create_user(ALICE);
    let usdc_market = t.resolve_market("USDC");
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    // Mint 500 USDC to Alice so she can pay in with the same token she will
    // use as collateral.
    usdc_market.token_admin.mint(&alice, &500_0000000i128);

    let alice_usdc_before = t.token_balance(ALICE, "USDC");
    t.fund_router("USDC", 3_000.0);
    // 1 ETH flash-borrowed minus 9bps fee. The 500 USDC initial payment is in
    // the COLLATERAL token, not the debt token, so `swap_amount_in` is
    // unaffected by it (it lands directly in `collateral_amount`).
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000),
        30_000_000_000,
    );

    let ctrl = t.ctrl_client();
    let account_id = ctrl.multiply(
        &alice,
        &0u64,
        &0u32,
        &usdc,
        &1_0000000i128, // 1 ETH flash debt
        &eth,
        &common::types::PositionMode::Multiply,
        &steps,
        &Some((usdc.clone(), 500_0000000i128)), // 500 USDC initial payment
        &None,
    );

    // Total collateral must equal initial payment (500) plus the swapped
    // amount (~3000).
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (3_499.0..=3_501.0).contains(&supply),
        "collateral-token initial payment must be added directly to collateral: got {}",
        supply
    );

    // Borrow reflects only the flash-loaned debt; the initial collateral
    // payment does not add to the borrow (distinguishes Case 1 from Case 2).
    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.99..=1.01).contains(&borrow),
        "borrow must be only the flash debt: got {}",
        borrow
    );
    // The 500 USDC initial payment must come out of Alice's wallet; the
    // controller must not synthesize it from elsewhere.
    let alice_usdc_after = t.token_balance(ALICE, "USDC");
    assert!(
        (alice_usdc_before - alice_usdc_after - 500.0).abs() < 1e-6,
        "Alice's USDC wallet should drop by exactly 500, before={}, after={}",
        alice_usdc_before,
        alice_usdc_after
    );
}

// ---------------------------------------------------------------------------
// Case 3: initial_payment is a third token, with convert_steps supplied.
// strategy.rs:87-100 -- the payment is swapped to collateral via
// convert_steps. No prior directed happy-path test.
// ---------------------------------------------------------------------------
#[test]
fn test_multiply_with_third_token_initial_payment_swaps_via_convert_steps() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let wbtc_market = t.resolve_market("WBTC");
    let wbtc = t.resolve_asset("WBTC");

    // Alice pays in with WBTC. Mint some to her.
    wbtc_market.token_admin.mint(&alice, &10_000_000i128); // 0.1 WBTC

    let alice_wbtc_before = t.token_balance(ALICE, "WBTC");
    // Main debt swap (ETH -> USDC) and initial-payment convert (WBTC ->
    // USDC). The mock aggregator funds each side independently, so fund
    // both.
    t.fund_router("USDC", 3_500.0); // 3000 for main + 500 for convert
    // Main: 1 ETH flash-borrow minus 9bps fee.
    let main_steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000),
        30_000_000_000,
    );
    // Convert: 1.0 WBTC at 7 decimals = 10_000_000 raw (the user's mint
    // amount). Initial-payment converts use the actual transferred amount
    // (no flash fee on user-supplied tokens).
    let convert_steps = build_aggregator_swap(&t, "WBTC", "USDC", 10_000_000, 500_0000000);

    let ctrl = t.ctrl_client();
    let account_id = ctrl.multiply(
        &alice,
        &0u64,
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
        &main_steps,
        &Some((wbtc, 10_000_000i128)),
        &Some(convert_steps),
    );

    // Collateral = main swap output (~3000) + convert output (~500) = ~3500.
    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        (3_499.0..=3_501.0).contains(&supply),
        "third-token payment must be converted and added to collateral: got {}",
        supply
    );
    // The WBTC initial payment must come out of Alice's wallet (exact
    // delta depends on harness auto-mint at user creation, so just assert
    // a non-trivial decrement).
    let alice_wbtc_after = t.token_balance(ALICE, "WBTC");
    assert!(
        alice_wbtc_after < alice_wbtc_before,
        "Alice's WBTC wallet must decrease after multiply with WBTC initial payment: before={}, after={}",
        alice_wbtc_before,
        alice_wbtc_after
    );
}

// ===========================================================================
// Part 3: swap_tokens allowance hygiene under hostile router
// ===========================================================================

// ---------------------------------------------------------------------------
// After an OverPull-triggered transaction rollback, the controller's
// allowance on the router must remain at zero. NEW-01 regression.
//
// Soroban rolls back state on contract panics, so the allowance approved by
// swap_tokens line 451 is undone atomically. Verify it post-op. Distinct
// from fuzz_strategy_flashloan prop 2, which covers the success-path
// zeroing (line 483).
// ---------------------------------------------------------------------------
#[test]
fn test_swap_tokens_allowance_remains_zero_after_overpull_rejection() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let admin = t.admin.clone();
    let bad = t.env.register(BadAggregator, (admin, BadMode::OverPull));
    t.ctrl_client().set_aggregator(&bad);

    // Pre-seed output so the bad router can transfer (before it attempts to
    // over-pull).
    t.resolve_market("USDC")
        .token_admin
        .mint(&bad, &300_000_000_000_i128);

    let steps = build_swap_steps(&t, "ETH", "USDC", 30_000_000_000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert!(result.is_err(), "OverPull must be rejected");

    // After rollback, the controller's ETH allowance on the bad router must
    // be zero. A regression that leaks the pre-approved allowance would
    // expose the controller to future drains.
    let eth = t.resolve_asset("ETH");
    let eth_tok = token::Client::new(&t.env, &eth);
    let allowance = eth_tok.allowance(&t.controller_address(), &bad);
    assert_eq!(
        allowance, 0,
        "post-rollback allowance on rejected swap must be zero, got {}",
        allowance
    );
}

// ---------------------------------------------------------------------------
// Defense-in-depth: allowance still zero after a successful swap via the
// happy-path mock. The controller explicitly calls `approve(..0, 0)` at
// strategy.rs:483. Regression: a commit that removed that zeroing call
// would leave the allowance equal to `amount_in`.
// ---------------------------------------------------------------------------
#[test]
fn test_swap_tokens_allowance_zero_after_successful_multiply() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.fund_router("USDC", 3_000.0);
    // 1 ETH flash-borrow minus 9bps fee.
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000),
        30_000_000_000,
    );
    let _account_id = t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );

    let eth = t.resolve_asset("ETH");
    let eth_tok = token::Client::new(&t.env, &eth);
    let allowance = eth_tok.allowance(&t.controller_address(), &t.aggregator);
    assert_eq!(
        allowance, 0,
        "controller allowance on the router must be zero after a successful swap (strategy.rs:483), got {}",
        allowance
    );
}

// ===========================================================================
// Part 4: Bob-not-owner regression with account created via multiply
// ===========================================================================

// ---------------------------------------------------------------------------
// strategy.rs:136 -- AccountNotInMarket for a Multiply reuse by wrong owner.
//
// swap_debt and swap_collateral already cover this path in edge_tests; the
// multiply reuse path (lines 135-137) was untested.
// ---------------------------------------------------------------------------
#[test]
fn test_multiply_reusing_account_wrong_owner_rejects() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice creates a leveraged position first — needs a real fixture so
    // her swap completes.
    t.fund_router("USDC", 3_000.0);
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000),
        30_000_000_000,
    );
    let alice_account = t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );

    // Bob tries to reuse Alice's account. He fails at the owner check
    // BEFORE swap_tokens runs, so an empty placeholder fixture is fine —
    // its `total_min_out > 0` satisfies the entry-point validation, and
    // the owner check fires next.
    t.fund_router("USDC", 3_000.0);
    let steps2 = build_swap_steps(&t, "ETH", "USDC", 30_000_000_000);
    let bob = t.get_or_create_user(BOB);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let ctrl = t.ctrl_client();
    let result = ctrl.try_multiply(
        &bob,
        &alice_account, // Bob points at Alice's account_id
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
        &steps2,
        &None,
        &None,
    );
    assert_contract_error(flatten(result), errors::ACCOUNT_NOT_IN_MARKET);
}
