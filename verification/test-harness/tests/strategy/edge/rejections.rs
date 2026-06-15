use super::*;

fn flatten_void(
    r: Result<
        Result<(), soroban_sdk::ConversionError>,
        Result<soroban_sdk::Error, soroban_sdk::InvokeError>,
    >,
) -> Result<(), soroban_sdk::Error> {
    match r {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e.into()),
        Err(invoke) => Err(invoke.expect("expected contract error, got host-level InvokeError")),
    }
}

// The same-asset flow is intentionally supported (self-collateralized
// unwinds): withdrawn collateral repays same-asset debt directly and skips
// the router. This exercises the direct-payment short-circuit.

#[test]
fn test_repay_debt_with_collateral_same_token_nets_positions() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Alice: USDC collateral + USDC debt (self-collateralized position).
    // Needs a second asset to open the position because LTV < 100%.
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 20.0); // extra collateral so USDC debt is borrowable
    t.borrow(ALICE, "USDC", 30_000.0);

    let debt_before = t.borrow_balance(ALICE, "USDC");
    let supply_before = t.supply_balance(ALICE, "USDC");
    assert!(debt_before > 29_000.0 && debt_before < 31_000.0);

    // Net 10k USDC collateral against 10k USDC debt in one call. `steps` is
    // unused in the same-asset path, but the API still requires a value.
    let steps = t.mock_swap_steps("USDC", "USDC", 0);
    t.repay_debt_with_collateral(ALICE, "USDC", 10_000.0, "USDC", &steps, false);

    let debt_after = t.borrow_balance(ALICE, "USDC");
    let supply_after = t.supply_balance(ALICE, "USDC");

    // Debt reduces by ~10k, collateral reduces by ~10k. Allow 1% tolerance
    // for accrued interest and rounding across the withdraw and repay chain.
    let debt_delta = debt_before - debt_after;
    let supply_delta = supply_before - supply_after;
    assert!(
        (debt_delta - 10_000.0).abs() < 100.0,
        "USDC debt should drop ~10k, actually dropped {debt_delta}"
    );
    assert!(
        (supply_delta - 10_000.0).abs() < 100.0,
        "USDC supply should drop ~10k, actually dropped {supply_delta}"
    );
}

#[test]
fn test_repay_debt_with_collateral_same_token_empty_swap_succeeds() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(ALICE, "ETH", 20.0);
    t.borrow(ALICE, "USDC", 30_000.0);

    let debt_before = t.borrow_balance(ALICE, "USDC");
    let supply_before = t.supply_balance(ALICE, "USDC");
    let empty_steps = Bytes::new(&t.env);

    let result =
        t.try_repay_debt_with_collateral(ALICE, "USDC", 10_000.0, "USDC", &empty_steps, false);
    assert!(
        result.is_ok(),
        "same-token repay should skip the router and accept empty swap bytes: {result:?}"
    );

    let debt_after = t.borrow_balance(ALICE, "USDC");
    let supply_after = t.supply_balance(ALICE, "USDC");
    assert!(
        debt_before - debt_after > 9_900.0,
        "USDC debt should drop by about the withdrawn collateral amount"
    );
    assert!(
        supply_before - supply_after > 9_900.0,
        "USDC collateral should be consumed directly without a swap"
    );
}

#[test]
fn test_repay_debt_with_collateral_non_same_token_empty_swap_rejects() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let empty_steps = Bytes::new(&t.env);
    let result =
        t.try_repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "ETH", &empty_steps, false);
    assert_contract_error(result, errors::INVALID_PAYMENTS);
}
// Favorable repay slippage must refund only the per-call excess.

#[test]
fn test_repay_debt_with_collateral_refund_only_uses_repay_excess() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 0.5);

    let eth_market = t.resolve_market("ETH");
    let eth_client = token::Client::new(&t.env, &eth_market.asset);
    eth_market
        .token_admin
        .mint(&t.controller_address(), &50_0000000i128);

    t.fund_router("ETH", 1.0);
    // repay_debt_with_collateral withdraws 1_000 USDC (raw 10_000_000_000); no flash fee.
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 10_000_000_000, 1_0000000);

    let alice_eth_before = t.token_balance(ALICE, "ETH");
    t.repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps, false);
    let alice_eth_after = t.token_balance(ALICE, "ETH");
    let controller_eth_after = eth_client.balance(&t.controller_address());

    assert!(
        (alice_eth_after - alice_eth_before - 0.5).abs() < 0.01,
        "caller should only receive the 0.5 ETH repayment excess, before={}, after={}",
        alice_eth_before,
        alice_eth_after
    );
    assert_eq!(
        controller_eth_after, 50_0000000i128,
        "unrelated controller ETH balance must not be swept during repay refund"
    );
}
// Withdrawing too much collateral for too little debt repayment must fail the
// final HF check.

#[test]
fn test_repay_debt_with_collateral_health_factor_guard() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 30.0);

    t.fund_router("ETH", 1.0);
    // repay_debt_with_collateral withdraws 50_000 USDC (raw 500_000_000_000); no flash fee.
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 500_000_000_000, 1_0000000);
    let result = t.try_repay_debt_with_collateral(ALICE, "USDC", 50_000.0, "ETH", &steps, false);

    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}

#[test]
fn test_repay_debt_with_collateral_rejects_zero_and_negative_collateral_amount() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let account_id = t.create_account(ALICE);
    let caller = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let steps = build_swap_steps(&t, "USDC", "ETH", 1_0000000);
    let ctrl = t.ctrl_client();

    let zero = ctrl.try_repay_debt_with_collateral(
        &caller,
        &account_id,
        &usdc,
        &0i128,
        &eth,
        &steps,
        &false,
    );
    assert_contract_error(flatten_void(zero), errors::AMOUNT_MUST_BE_POSITIVE);

    let negative = ctrl.try_repay_debt_with_collateral(
        &caller,
        &account_id,
        &usdc,
        &-1i128,
        &eth,
        &steps,
        &false,
    );
    assert_contract_error(flatten_void(negative), errors::AMOUNT_MUST_BE_POSITIVE);
}
// A full close must repay the debt, drain remaining collateral, and remove
// the account.

#[test]
fn test_repay_debt_with_collateral_close_position_removes_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let account_id = t.resolve_account_id(ALICE);

    let alice_usdc_before = t.token_balance(ALICE, "USDC");
    t.fund_router("ETH", 1.0);
    // repay_debt_with_collateral withdraws 1_000 USDC (raw 10_000_000_000); no flash fee.
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 10_000_000_000, 1_0000000);
    t.repay_debt_with_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps, true);

    assert!(
        !t.account_exists(account_id),
        "close_position should remove the fully closed account"
    );
    // Close-position semantics: residual collateral must be returned to the
    // caller's wallet, not swept inside the controller.
    let alice_usdc_after = t.token_balance(ALICE, "USDC");
    assert!(
        alice_usdc_after >= alice_usdc_before,
        "close_position must refund residual USDC collateral to Alice, before={}, after={}",
        alice_usdc_before,
        alice_usdc_after
    );
}
// Even without close_position=true, the account must be removed when the
// flow zeroes every remaining position.

#[test]
fn test_repay_debt_with_collateral_removes_empty_account_without_close() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 2_000.0);
    t.borrow(ALICE, "ETH", 0.5);
    let account_id = t.resolve_account_id(ALICE);

    t.fund_router("ETH", 0.5);
    // repay_debt_with_collateral withdraws 2_000 USDC (raw 20_000_000_000); no flash fee.
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 20_000_000_000, 5_000000);
    t.repay_debt_with_collateral(ALICE, "USDC", 2_000.0, "ETH", &steps, false);

    assert!(
        !t.account_exists(account_id),
        "repay-with-collateral should remove the account when both sides reach zero"
    );
}
// Partial swap into a new asset should respect the supply-position limit.

#[test]
fn test_swap_collateral_rejects_new_asset_when_supply_limit_reached() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market(usdt_stable_preset())
        .with_market(dai_preset())
        .with_position_limits(4, 10)
        .build();

    let account_id = t.create_account(ALICE);
    t.supply_to(ALICE, account_id, "USDC", 10_000.0);
    t.supply_to(ALICE, account_id, "ETH", 1.0);
    t.supply_to(ALICE, account_id, "WBTC", 0.1);
    t.supply_to(ALICE, account_id, "USDT", 5_000.0);

    t.fund_router("DAI", 1.0);
    let steps = build_swap_steps(&t, "USDC", "DAI", 1_0000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 100.0, "DAI", &steps);
    assert_contract_error(result, errors::POSITION_LIMIT_EXCEEDED);
}
// E-mode account; new collateral is not in the e-mode category.

#[test]
fn test_swap_collateral_emode_wrong_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    // Create an e-mode account, supply USDC, borrow USDT.
    t.create_emode_account(ALICE, 1);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);

    // Try to swap USDC collateral to ETH: ETH is not listed in category 1.
    // `validate_e_mode_asset` rejects missing category membership with #300.
    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "ETH", &steps);
    assert_contract_error(result, errors::EMODE_CATEGORY_NOT_FOUND);
}
// Swap collateral with no borrows: the HF check is skipped. With the
// working mock router, this succeeds.

#[test]
fn test_swap_collateral_no_borrows_skip_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Supply only, no borrows.
    t.supply(ALICE, "USDC", 100_000.0);

    // Swap collateral: the HF check is skipped (no borrows). With the
    // working mock router, this succeeds.
    t.fund_router("ETH", 5.0); // Pre-fund the router with output tokens.
                               // swap_collateral withdraws 1_000 USDC (raw 10_000_000_000); no flash fee.
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 10_000_000_000, 5_0000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "ETH", &steps);
    assert!(
        result.is_ok(),
        "swap_collateral with no borrows should succeed"
    );

    // Verify the ETH supply position was created.
    let eth_supply = t.supply_balance(ALICE, "ETH");
    assert!(
        eth_supply > 0.0,
        "should have ETH supply: got {}",
        eth_supply
    );
    // The 1000 USDC of source collateral must be removed from the supply
    // side; otherwise the swap leg silently regressed to "deposit only".
    let usdc_supply_after = t.supply_balance(ALICE, "USDC");
    assert!(
        (98_999.0..=99_001.0).contains(&usdc_supply_after),
        "USDC supply should drop by ~1000 after swap_collateral, got {}",
        usdc_supply_after
    );
}
#[test]
fn test_strategy_empty_swap_payload_multiply() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let empty_steps = Bytes::new(&t.env);

    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &empty_steps,
    );
    // The controller rejects empty opaque swap bytes before routing.
    assert_contract_error(result, errors::INVALID_PAYMENTS);
}

#[test]
fn test_multiply_zero_debt_amount() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        0.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

#[test]
fn test_swap_debt_zero_amount() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_00000000);
    // Note: try_swap_debt passes new_amount through f64_to_i128, so 0.0 -> 0.
    // The validation require_amount_positive must catch this.
    let result = t.try_swap_debt(ALICE, "ETH", 0.0, "WBTC", &steps);
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}

#[test]
fn test_swap_collateral_zero_amount() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);
    let result = t.try_swap_collateral(ALICE, "USDC", 0.0, "ETH", &steps);
    assert_contract_error(result, errors::AMOUNT_MUST_BE_POSITIVE);
}
// Bob tries to swap Alice's debt: must be rejected.

#[test]
fn test_swap_debt_wrong_account_owner() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    // Get Alice's account ID, then try to swap using Bob's address.
    let alice_account_id = t.resolve_account_id(ALICE);
    let bob_addr = t.get_or_create_user(BOB);
    let existing_addr = t.resolve_asset("ETH");
    let new_addr = t.resolve_asset("WBTC");
    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_00000000);

    let ctrl = t.ctrl_client();
    let result = ctrl.try_swap_debt(
        &bob_addr,
        &alice_account_id,
        &existing_addr,
        &10_0000000i128,
        &new_addr,
        &steps,
    );
    // Flatten Result<Result<(), Error>, InvokeError> so the code can assert.
    let flat: Result<(), soroban_sdk::Error> = match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e.into()),
        Err(invoke) => Err(invoke.expect("expected contract error, got host-level InvokeError")),
    };
    assert_contract_error(flat, errors::ACCOUNT_NOT_IN_MARKET);
}
// Strategy flows must authenticate the account owner address, not just compare it.

#[test]
fn test_strategy_entrypoints_reject_missing_owner_auth() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let account_id = t.resolve_account_id(ALICE);
    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let wbtc = t.resolve_asset("WBTC");
    let steps = build_swap_steps(&t, "WBTC", "ETH", 1_0000000);
    let no_auths: [soroban_sdk::xdr::SorobanAuthorizationEntry; 0] = [];
    let ctrl = t.ctrl_client();

    expect_host_auth_rejection(
        "swap_debt",
        ctrl.set_auths(&no_auths).try_swap_debt(
            &alice,
            &account_id,
            &eth,
            &10_0000000i128,
            &wbtc,
            &steps,
        ),
    );
    expect_host_auth_rejection(
        "swap_collateral",
        ctrl.set_auths(&no_auths).try_swap_collateral(
            &alice,
            &account_id,
            &usdc,
            &1_0000000i128,
            &wbtc,
            &steps,
        ),
    );
    expect_host_auth_rejection(
        "repay_debt_with_collateral",
        ctrl.set_auths(&no_auths).try_repay_debt_with_collateral(
            &alice,
            &account_id,
            &usdc,
            &1_0000000i128,
            &eth,
            &steps,
            &false,
        ),
    );
}

#[test]
fn test_repay_debt_with_collateral_wrong_account_owner() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let alice_account_id = t.resolve_account_id(ALICE);
    let bob = t.get_or_create_user(BOB);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let steps = build_swap_steps(&t, "USDC", "ETH", 1_0000000);

    let result = t.ctrl_client().try_repay_debt_with_collateral(
        &bob,
        &alice_account_id,
        &usdc,
        &1000_0000000i128,
        &eth,
        &steps,
        &false,
    );
    assert_contract_error(flatten_void(result), errors::ACCOUNT_NOT_IN_MARKET);
}

#[test]
fn test_repay_debt_with_collateral_nonexistent_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let caller = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let steps = build_swap_steps(&t, "USDC", "ETH", 1_0000000);
    let missing_account_id = 999u64;

    let result = t.ctrl_client().try_repay_debt_with_collateral(
        &caller,
        &missing_account_id,
        &usdc,
        &1000_0000000i128,
        &eth,
        &steps,
        &false,
    );
    assert_contract_error(
        flatten_void(result),
        errors::GenericError::AccountNotFound as u32,
    );
}
// Bob tries to swap Alice's collateral: must be rejected.

#[test]
fn test_swap_collateral_wrong_account_owner() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let alice_account_id = t.resolve_account_id(ALICE);
    let bob_addr = t.get_or_create_user(BOB);
    let current_addr = t.resolve_asset("USDC");
    let new_addr = t.resolve_asset("ETH");
    let steps = build_swap_steps(&t, "USDC", "ETH", 5_0000000);

    let ctrl = t.ctrl_client();
    let result = ctrl.try_swap_collateral(
        &bob_addr,
        &alice_account_id,
        &current_addr,
        &1000_0000000i128,
        &new_addr,
        &steps,
    );
    let flat: Result<(), soroban_sdk::Error> = match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e.into()),
        Err(invoke) => Err(invoke.expect("expected contract error, got host-level InvokeError")),
    };
    assert_contract_error(flat, errors::ACCOUNT_NOT_IN_MARKET);
}
// Verify that collateral == debt is caught even when the amounts differ.

#[test]
fn test_multiply_same_asset_is_caught() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let steps = build_swap_steps(&t, "ETH", "ETH", 1000_0000000);
    let result = t.try_multiply(
        ALICE,
        "ETH",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::ASSETS_ARE_THE_SAME);
}
// Already tested in strategy_tests.rs; verify the error code here too.

#[test]
fn test_swap_debt_same_token_error_code() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let steps = t.mock_swap_steps("ETH", "ETH", 0);
    let result = t.try_swap_debt(ALICE, "ETH", 1.0, "ETH", &steps);
    assert_contract_error(result, errors::ASSETS_ARE_THE_SAME);
}

#[test]
fn test_swap_collateral_same_token_error_code() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    let steps = t.mock_swap_steps("USDC", "USDC", 0);
    let result = t.try_swap_collateral(ALICE, "USDC", 1000.0, "USDC", &steps);
    assert_contract_error(result, errors::ASSETS_ARE_THE_SAME);
}
