//! Router adversarial tests, panic-site coverage, oracle boundaries, and supply-cap gates.

use controller::constants::{RAY, WAD};
use soroban_sdk::token;
use soroban_sdk::Address;
use test_harness::mock_aggregator::{BadAggregator, BadMode};
use test_harness::{
    apply_flash_fee, assert_contract_error, build_aggregator_swap, errors, eth_preset, hub_asset,
    tokens, usd, usdc_preset, wbtc_preset, LendingTest, ALICE, BOB,
};

use crate::helpers::build_swap_steps;

const SWAP_REQUESTED_ETH: i128 = 10_000_000;
const SWAP_MIN_OUT_USDC: i128 = 30_000_000_000;

fn install_bad_router(t: &LendingTest, mode: BadMode) -> Address {
    let admin = t.admin.clone();
    let bad = t.env.register(BadAggregator, (admin.clone(), mode));
    t.ctrl_client().set_aggregator(&bad);
    bad
}

fn mint_to(t: &LendingTest, asset_name: &str, target: &Address, raw_amount: i128) {
    let market = t.resolve_market(asset_name);
    market.token_admin.mint(target, &raw_amount);
}

fn assert_overpull_rejected(result: Result<u64, soroban_sdk::Error>) {
    match result {
        Ok(account_id) => panic!("OverPull must be rejected, got Ok(account_id={account_id})"),
        Err(err) => {
            let internal = soroban_sdk::Error::from_contract_error(errors::INTERNAL_ERROR);
            let err_str = format!("{err:?}");
            assert!(
                err == internal || err_str.contains("Error(Contract,"),
                "OverPull must reject via INTERNAL_ERROR or a contract error, got {err:?}"
            );
        }
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

fn set_sanity_bounds(t: &LendingTest, asset_name: &str, min_wad: i128, max_wad: i128) {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller, || {
        let key = controller::types::ControllerKey::AssetOracle(asset.clone());
        let mut oracle: controller::types::MarketOracleConfig =
            t.env.storage().persistent().get(&key).unwrap();
        oracle.min_sanity_price_wad = min_wad;
        oracle.max_sanity_price_wad = max_wad;
        t.env.storage().persistent().set(&key, &oracle);
    });
}

// BadMode::Refund -- router returns token_in to the caller, violating the
// `balance_in_after > balance_in_before` invariant. Must panic with
// InternalError.

#[test]
fn test_swap_tokens_panics_when_router_refunds_token_in() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let bad = install_bad_router(&t, BadMode::Refund);
    // Seed the bad router with USDC output so it can satisfy the mock output
    // transfer before the adversarial token_in refund.
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
        controller::types::PositionMode::Multiply,
        &steps,
    );

    // strategy.rs:474 -- if balance_in_after > balance_in_before, InternalError.
    assert_contract_error(result, errors::INTERNAL_ERROR);
}
// BadMode::OverPull -- router pulls 2x the requested amount via
// `token.transfer(sender, router, 2*amount_in)`. The new ABI has no SEP-41
// allowance to overshoot, so the SAC's `transfer` either succeeds (if
// the controller happens to hold enough) and the controller's
// `verify_router_input_spend` fires `actual_in_spent != amount_in`, or
// fails with the SAC's insufficient-balance error. Either way it's a
// detectable adversary; the controller surfaces InternalError when the
// over-spend lands.

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
        controller::types::PositionMode::Multiply,
        &steps,
    );

    assert_overpull_rejected(result);
}

#[test]
fn test_swap_tokens_refunds_router_underspend() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let bad = install_bad_router(&t, BadMode::UnderPull);
    mint_to(&t, "USDC", &bad, SWAP_MIN_OUT_USDC);

    let alice = t.get_or_create_user(ALICE);
    let eth_before = t.token_balance(ALICE, "ETH");
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(SWAP_REQUESTED_ETH),
        SWAP_MIN_OUT_USDC,
    );

    t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    )
    .expect("underspend should be refunded, not rejected");

    let eth_after = token::Client::new(&t.env, &t.resolve_asset("ETH")).balance(&alice);
    assert!(
        (eth_after as f64 / 10_000_000.0) > eth_before + 0.49,
        "Alice should receive the unspent borrowed ETH"
    );
}

#[test]
fn test_swap_collateral_refunds_router_underspend_to_caller() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let bad = install_bad_router(&t, BadMode::UnderPull);
    mint_to(&t, "ETH", &bad, 50_000_000);

    t.supply(ALICE, "USDC", 1_000.0);
    let alice_usdc_before = t.token_balance_raw(ALICE, "USDC");
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 10_000_000_000, 50_000_000);

    t.try_swap_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps)
        .expect("swap_collateral should refund router underspend");

    let alice_usdc_after = t.token_balance_raw(ALICE, "USDC");
    assert_eq!(
        alice_usdc_after - alice_usdc_before,
        5_000_000_000,
        "half of the withdrawn USDC should be refunded to Alice's wallet"
    );

    let usdc = t.resolve_asset("USDC");
    let usdc_tok = token::Client::new(&t.env, &usdc);
    assert_eq!(
        usdc_tok.balance(&t.controller_address()),
        0,
        "controller must not strand unspent swap_collateral input"
    );
}

#[test]
fn test_repay_debt_with_collateral_refunds_router_underspend_to_caller() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let bad = install_bad_router(&t, BadMode::UnderPull);
    mint_to(&t, "ETH", &bad, 5_000_000);

    t.supply(ALICE, "USDC", 2_000.0);
    t.borrow(ALICE, "ETH", 0.5);
    let account_id = t.resolve_account_id(ALICE);
    let alice_usdc_before = t.token_balance_raw(ALICE, "USDC");
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 20_000_000_000, 5_000_000);

    t.try_repay_debt_with_collateral(ALICE, "USDC", 2_000.0, "ETH", &steps, false)
        .expect("repay_debt_with_collateral should refund router underspend");

    let alice_usdc_after = t.token_balance_raw(ALICE, "USDC");
    assert_eq!(
        alice_usdc_after - alice_usdc_before,
        10_000_000_000,
        "half of the withdrawn USDC should be refunded to Alice's wallet"
    );
    assert_eq!(
        t.borrow_balance_raw(ALICE, "ETH"),
        0,
        "router output should fully repay the ETH debt"
    );
    assert!(
        !t.account_exists(account_id),
        "fully repaid and fully withdrawn account should be removed"
    );
}
// BadMode::OutputShortfall -- router pulls token_in but transfers zero
// token_out. The controller's positive output-delta check rejects the swap
// immediately.

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
        controller::types::PositionMode::Multiply,
        &steps,
    );

    // The positive output-delta check in `strategy::swap_tokens` rejects the
    // shortfall immediately with INTERNAL_ERROR.
    assert_contract_error(result, errors::INTERNAL_ERROR);
}

// Part 1: missing `panic_with_error!` sites in strategy.rs
// strategy.rs:91 -- ConvertStepsRequired
//
// When multiply receives an initial_payment whose token is a third token
// (neither collateral nor debt), `convert_steps` must be Some. The current
// suite had zero coverage for this panic site.
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
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &Some((hub_asset(wbtc), 100_000i128)),
        &None, // <- key: no convert_steps for third-token payment
    );
    assert_contract_error(flatten(result), errors::CONVERT_STEPS_REQUIRED);
}
// strategy.rs:140 -- AccountModeMismatch
//
// Reusing an existing account with a different mode must be rejected. No
// prior test exercised this path.
#[test]
fn test_multiply_existing_account_mode_mismatch_rejects() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Create an account explicitly in Multiply mode.
    let account_id = t.create_account_full(ALICE, 0, controller::types::PositionMode::Multiply);
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
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Long, // mismatch
        &steps,
        &None,
        &None,
    );
    assert_contract_error(flatten(result), errors::ACCOUNT_MODE_MISMATCH);
}
// strategy.rs:284 -- DebtPositionNotFound in swap_debt
// Alice tries to swap an ETH debt she does not owe. The test was missing.
//
// Call order in process_swap_debt:
//   1. borrow_for_strategy(new_debt_token): flash-borrows the new
//      token.
//   2. swap_tokens(new_debt -> existing_debt).
//   3. borrow_positions.get(existing_debt_token): panics if missing (line
//      284).
// The router must be funded correctly and the flash borrow must succeed.
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
    let steps = build_aggregator_swap(&t, "WBTC", "ETH", apply_flash_fee(10_000), 5_000_000);
    // existing=ETH (Alice does not hold it). new=WBTC (Alice already holds
    // WBTC debt, but swap_debt requires only the existing debt to be
    // present -- not the new one).
    let result = t.try_swap_debt(ALICE, "ETH", 0.001, "WBTC", &steps);
    assert_contract_error(result, errors::DEBT_POSITION_NOT_FOUND);
}
// strategy.rs:373 -- CollateralPositionNotFound in swap_collateral
// Alice tries to swap WBTC collateral she does not hold.

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
// strategy.rs:527 -- CollateralPositionNotFound in repay_debt_with_collateral
// Alice tries to repay using WBTC collateral she does not hold.

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
// strategy.rs -- DebtPositionNotFound in repay_debt_with_collateral.
//
// The function validates both positions before token movement, then measures
// the actual withdrawal delta and feeds it
// into swap_tokens. This test pins the DebtPositionNotFound guard.
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
// strategy.rs:600 -- CannotCloseWithRemainingDebt
//
// close_position=true must be rejected if the account still has debt.
// Lines 599-601 guard this explicitly; no prior test exercised it.
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
// Part 2: initial_payment branches
// Case 1: initial_payment == collateral_token (happy path).
// strategy.rs:80-82 -- the payment is added directly to collateral_amount.
// No prior directed test exercised this branch end-to-end.
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
        &hub_asset(usdc.clone()),
        &1_0000000i128, // 1 ETH flash debt
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &Some((hub_asset(usdc.clone()), 500_0000000i128)), // 500 USDC initial payment
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
// Case 3: initial_payment is a third token, with convert_steps supplied.
// strategy.rs:87-100 -- the payment is swapped to collateral via
// convert_steps. No prior directed happy-path test.
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
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &main_steps,
        &Some((hub_asset(wbtc), 10_000_000i128)),
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
// Part 3: swap_tokens allowance hygiene under hostile router
// After an OverPull-triggered transaction rollback, the controller's
// allowance on the router must remain at zero.
//
// Soroban rolls back state on contract panics, so router allowance must remain
// zero after an over-pull rejection.
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
    t.resolve_market("ETH")
        .token_admin
        .mint(&bad, &100_000_000_i128);

    let steps = build_swap_steps(&t, "ETH", "USDC", 30_000_000_000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );
    assert_overpull_rejected(result);

    // After rollback, the controller's ETH allowance on the bad router must
    // be zero, preventing residual approval from exposing controller funds.
    let eth = t.resolve_asset("ETH");
    let eth_tok = token::Client::new(&t.env, &eth);
    let allowance = eth_tok.allowance(&t.controller_address(), &bad);
    assert_eq!(
        allowance, 0,
        "post-rollback allowance on rejected swap must be zero, got {}",
        allowance
    );
}
// Defense-in-depth: allowance still zero after a successful swap via the
// happy-path mock. The controller explicitly calls `approve(..0, 0)` at
// strategy.rs:483. Without the zeroing call, allowance would remain equal to
// `amount_in`.
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
        controller::types::PositionMode::Multiply,
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
// Part 4: Bob-not-owner authorization with account created via multiply
// AccountNotInMarket for a multiply reuse by the wrong owner.
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
        controller::types::PositionMode::Multiply,
        &steps,
    );

    // Bob tries to reuse Alice's account. He fails at the owner check
    // BEFORE swap_tokens runs, so an empty placeholder fixture is fine —
    // the non-empty payload satisfies local swap validation, and the owner
    // check fires next.
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
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps2,
        &None,
        &None,
    );
    assert_contract_error(flatten(result), errors::ACCOUNT_NOT_IN_MARKET);
}

// Price exactly equal to the ceiling must be accepted; even 1 WAD over
// must be rejected. Pins the inequality (≤ vs <).
#[test]
fn test_sanity_bound_ceiling_exact_accept_then_one_over_reject() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Set ETH ceiling at exactly $2000 (current price). Reads must
    // succeed.
    set_sanity_bounds(&t, "ETH", usd(100), usd(2_000));
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Now push ETH price to $2000 + 1 WAD-cent → must reject.
    // 1 WAD-cent = WAD / 100 = 10^16
    t.set_price("ETH", usd(2_000) + WAD / 100);
    let result = t.try_borrow(ALICE, "ETH", 0.1);
    assert_contract_error(result, errors::SANITY_BOUND_VIOLATED);
}

// Floor exact-boundary test.
#[test]
fn test_sanity_bound_floor_exact_accept_then_one_under_reject() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Set ETH floor at exactly $2000 (current price). Reads must
    // succeed.
    set_sanity_bounds(&t, "ETH", usd(2_000), usd(10_000));
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Drop ETH below the floor by 1 WAD-cent → must reject.
    t.set_price("ETH", usd(2_000) - WAD / 100);
    let result = t.try_borrow(ALICE, "ETH", 0.1);
    assert_contract_error(result, errors::SANITY_BOUND_VIOLATED);
}
// 2. Strategy paths respect max-utilization cap

// Borrow-side gate at the cap (also covered by `max_utilization.rs`).
#[test]
fn test_borrow_at_cap_then_step_over_rejected() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_params("USDC", |p| {
            // Tight cap: 85 %. (Must stay ≥ optimal=80 % per validator.)
            p.max_utilization_ray = controller::constants::RAY * 85 / 100;
        })
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.supply(BOB, "ETH", 10.0);

    // Borrow $850 → utilization = 85 %. Exactly at cap, allowed.
    t.borrow(BOB, "USDC", 850.0);

    // One more dollar — over the cap, rejected.
    let result = t.try_borrow(BOB, "USDC", 1.0);
    assert_contract_error(result, errors::UTILIZATION_ABOVE_MAX);
}

// Multiply flash-borrows through the same utilization-cap gate on the debt asset.
#[test]
fn test_multiply_at_utilization_cap_then_step_over_rejected() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_params("ETH", |p| {
            p.max_utilization_ray = RAY * 85 / 100;
        })
        .build();

    t.supply(BOB, "ETH", 1_000.0);
    t.supply(ALICE, "USDC", 50_000.0);
    t.supply(BOB, "USDC", 400_000.0);
    t.borrow(BOB, "ETH", 850.0);

    t.fund_router("USDC", 3_000.0);
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000),
        30_000_000_000,
    );
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::UTILIZATION_ABOVE_MAX);
}

#[test]
fn test_strategy_swap_collateral_supply_cap_reached() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    // Bob supplies 1M USDC to fill the pool.
    t.supply(BOB, "USDC", 1_000_000.0);

    // Set the USDC hub supply cap to 1,010,000 tokens (7 decimals). Current
    // total = 1,000,000.
    t.ctrl_client()
        .update_pool_caps(&hub_asset(t.resolve_asset("USDC")), &10_100_000_000_000i128, &0i128);

    // Alice supplies some ETH.
    t.supply("alice", "ETH", 10.0);

    // Alice tries to swap 5 ETH collateral for USDC. 5 ETH = $10,000. The
    // mock swap returns 20,000 USDC ($20,000 at $1/USDC). Total USDC =
    // 1,000,000 + 20,000 = 1,020,000. 1,020,000 > 1,010,000 triggers #105.

    // Fund the router with USDC for the swap.
    t.fund_router("USDC", 100_000.0);

    // 5 ETH (7 decimals) → 50_000_000 raw. swap_collateral does not flash-
    // borrow, so amount_in matches the requested withdrawal exactly.
    let steps = build_aggregator_swap(&t, "ETH", "USDC", 50_000_000, tokens(20_000, 7));

    let res = t.try_swap_collateral("alice", "ETH", 5.0, "USDC", &steps);
    assert_contract_error(res, errors::SUPPLY_CAP_REACHED);
}

#[test]
fn test_strategy_multiply_supply_cap_reached() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    // Bob supplies 1M USDC.
    t.supply(BOB, "USDC", 1_000_000.0);

    // Set the USDC hub supply cap to 1,010,000 tokens (7 decimals). Preserves
    // the dust-disabled sentinel from `with_dust_disabled_all_markets()` so
    // Alice's deliberately-tiny 5 USDC seed position survives the new gate.
    t.ctrl_client()
        .update_pool_caps(&hub_asset(t.resolve_asset("USDC")), &10_100_000_000_000i128, &0i128);

    // Alice has some USDC.
    t.supply("alice", "USDC", 5.0); // Minimal initial position

    // Alice tries to multiply her USDC position. Borrow 10 ETH ($20k), swap
    // to USDC. The mock swap returns 30,000 USDC. Total USDC = 1,000,000
    // (Bob) + 5 (Alice) + 30,000 (swap) = 1,030,005. 1,030,005 > 1,010,000
    // triggers #105.

    t.fund_router("USDC", 100_000.0);

    // 10 ETH flash-borrowed → controller receives apply_flash_fee(100_000_000).
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(100_000_000),
        tokens(30_000, 7),
    );

    let res = t.try_multiply(
        "alice",
        "USDC",
        10.0,
        "ETH",
        controller::types::PositionMode::Multiply, // Multiply mode
        &steps,
    );
    assert_contract_error(res, errors::SUPPLY_CAP_REACHED);
}

#[test]
fn test_strategy_multiply_unsupported_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .with_max_utilization_disabled_all_markets()
        .build();

    t.supply("alice", "USDC", 10.0);
    let steps = t.mock_swap_steps("ETH", "USDC", usd(2000));

    // Try multiply with invalid category 999 using the harness helper.
    let res = t.try_multiply_with_category(
        "alice",
        999, // category
        "USDC",
        5.0,
        "ETH",
        controller::types::PositionMode::Multiply, // mode
        &steps,
    );

    assert_contract_error(res, errors::EMODE_CATEGORY_NOT_FOUND);
}
