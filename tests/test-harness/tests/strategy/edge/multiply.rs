use super::*;

// Multiply edge cases
// An initial payment in the debt token must enlarge the swap input without
// enlarging the stored debt leg.

#[test]
fn test_multiply_with_debt_token_initial_payment() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let eth_market = t.resolve_market("ETH");
    eth_market.token_admin.mint(&alice, &5_000000i128); // 0.5 ETH

    let alice_eth_before = t.token_balance(ALICE, "ETH");
    t.fund_router("USDC", 4_500.0);
    // multiply: borrow 1 ETH (post-fee = 9_991_000) + 0.5 ETH initial debt
    // payment (5_000_000). swap_amount_in = 14_991_000.
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000) + 5_000_000,
        4500_0000000,
    );

    let account_id = t.ctrl_client().multiply(
        &alice,
        &0u64,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &Some((hub_asset(eth.clone()), 5_000000i128)),
        &None,
    );

    let supply = t.supply_balance_for(ALICE, account_id, "USDC");
    let borrow = t.borrow_balance_for(ALICE, account_id, "ETH");

    assert!(
        (4499.0..=4501.0).contains(&supply),
        "USDC supply should include flash debt plus initial debt-token payment, got {}",
        supply
    );
    assert!(
        (0.99..=1.01).contains(&borrow),
        "borrowed ETH should remain the strategy debt amount only, got {}",
        borrow
    );
    // The 0.5 ETH initial payment must come out of Alice's wallet; the
    // controller must not mint or otherwise replace it.
    let alice_eth_after = t.token_balance(ALICE, "ETH");
    assert!(
        (alice_eth_before - alice_eth_after - 0.5).abs() < 1e-6,
        "Alice's ETH wallet should drop by exactly 0.5 ETH, before={}, after={}",
        alice_eth_before,
        alice_eth_after
    );
}

// An UNLISTED initial-payment token is rejected up front (fail-fast price
// check) BEFORE its token contract is invoked — `OracleNotConfigured`.
#[test]
fn test_multiply_rejects_unlisted_third_token_payment_before_transfer() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let sac = t.env.register_stellar_asset_contract_v2(t.admin());
    let unlisted = sac.address().clone();
    token::StellarAssetClient::new(&t.env, &unlisted).mint(&alice, &1_0000000i128);

    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    let result = t.ctrl_client().try_multiply(
        &alice,
        &0u64,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &Some((hub_asset(unlisted), 1_0000000i128)),
        &None,
    );

    match result {
        Err(Ok(err)) => assert_eq!(
            err,
            soroban_sdk::Error::from_contract_error(errors::ORACLE_NOT_CONFIGURED),
            "unlisted payment token must fail OracleNotConfigured before transfer"
        ),
        other => panic!("expected OracleNotConfigured, got {:?}", other),
    }
}

// A LISTED third-token initial payment without convert steps is rejected with
// ConvertStepsRequired after the fail-fast price check passes.
#[test]
fn test_multiply_rejects_third_token_payment_without_convert() {
    use test_harness::xlm_preset;
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(xlm_preset())
        .build();

    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let xlm = t.resolve_asset("XLM");
    t.resolve_market("XLM")
        .token_admin
        .mint(&alice, &10_0000000i128);

    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    let result = t.ctrl_client().try_multiply(
        &alice,
        &0u64,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &Some((hub_asset(xlm), 1_0000000i128)),
        &None,
    );

    match result {
        Err(Ok(err)) => assert_eq!(
            err,
            soroban_sdk::Error::from_contract_error(errors::CONVERT_STEPS_REQUIRED),
            "third-token payment without convert steps must fail ConvertStepsRequired"
        ),
        other => panic!("expected ConvertStepsRequired, got {:?}", other),
    }
}

#[test]
fn test_multiply_rejects_when_paused() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.pause();

    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::CONTRACT_PAUSED);
}
// Reusing an account that already holds the collateral asset must add to the
// existing position, not replace it.

#[test]
fn test_multiply_preserves_existing_collateral_balance() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let account_id = t.create_account_full(ALICE, 1, controller::types::PositionMode::Multiply);
    t.supply_to(ALICE, account_id, "USDC", 1_000.0);

    t.fund_router("USDC", 3_000.0);
    // 1 ETH (raw 10_000_000) flash-borrowed minus 9bps fee.
    let steps = build_aggregator_swap(
        &t,
        "ETH",
        "USDC",
        apply_flash_fee(10_000_000),
        30_000_000_000,
    );

    let caller = t.get_or_create_user(ALICE);
    let ctrl = t.ctrl_client();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let result = ctrl.try_multiply(
        &caller,
        &account_id,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );
    assert!(matches!(result, Ok(Ok(_))), "multiply should succeed");

    let final_supply = t.supply_balance_for(ALICE, account_id, "USDC");
    assert!(
        final_supply > 3_500.0,
        "existing collateral must be preserved and increased, got {}",
        final_supply
    );

    // The multiply must also open the new ETH borrow leg; without this the
    // test would silently pass if the borrow side regressed to a no-op.
    let final_borrow = t.borrow_balance_for(ALICE, account_id, "ETH");
    assert!(
        (0.99..=1.01).contains(&final_borrow),
        "new ETH borrow leg should be ~1.0 ETH, got {}",
        final_borrow
    );
    let hf = t.health_factor_for(ALICE, account_id);
    assert!(
        hf >= 1.0,
        "post-multiply HF must remain solvent, got {}",
        hf
    );
}

#[test]
fn test_multiply_reuses_spoke_account_with_zero_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    let account_id = t.create_account_full(ALICE, 2, controller::types::PositionMode::Multiply);
    let caller = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let usdt = t.resolve_asset("USDT");

    t.fund_router("USDC", 2_000.0);
    let steps = build_aggregator_swap(
        &t,
        "USDT",
        "USDC",
        apply_flash_fee(10_000_000_000),
        20_000_000_000,
    );

    let result = t.ctrl_client().try_multiply(
        &caller,
        &account_id,
        &1u32,
        &hub_asset(usdc.clone()),
        &1000_0000000i128,
        &hub_asset(usdt.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );
    assert!(
        matches!(result, Ok(Ok(id)) if id == account_id),
        "expected multiply to reuse account {account_id}, got {result:?}"
    );

    let attrs = t.ctrl_client().get_account_attributes(&account_id);
    assert_eq!(
        attrs.spoke_id, 2,
        "zero spoke_id must reuse the account's stored spoke category"
    );
    assert!(
        t.supply_balance_for(ALICE, account_id, "USDC") > 1_999.0,
        "multiply should add USDC collateral to the existing spoke account"
    );
    assert!(
        (999.0..=1001.0).contains(&t.borrow_balance_for(ALICE, account_id, "USDT")),
        "multiply should open the USDT debt leg on the existing spoke account"
    );
}

#[test]
fn test_multiply_missing_owner_auth_rejects_before_validation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let caller = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    let no_auths: [soroban_sdk::xdr::SorobanAuthorizationEntry; 0] = [];

    expect_host_auth_rejection(
        "multiply",
        t.ctrl_client().set_auths(&no_auths).try_multiply(
            &caller,
            &0u64,
            &1u32,
            &hub_asset(usdc.clone()),
            &1_0000000i128,
            &hub_asset(eth.clone()),
            &controller::types::PositionMode::Multiply,
            &steps,
            &None,
            &None,
        ),
    );
}

#[test]
fn test_multiply_existing_account_not_found() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let caller = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    let missing_account_id = 999u64;

    let result = t.ctrl_client().try_multiply(
        &caller,
        &missing_account_id,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );

    assert_contract_error(
        flatten(result),
        errors::GenericError::AccountNotFound as u32,
    );
}
// Spoke account in the stablecoin category, but debt is ETH (not in
// category). Validation runs before the swap, so the error is clean.

#[test]
fn test_multiply_spoke_wrong_category_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        // ETH is NOT in spoke category 1
        .build();

    // Use the raw controller client so `spoke_id=2` can be passed
    // explicitly.
    let caller = t.get_or_create_user(ALICE);
    let collateral_addr = t.resolve_asset("USDC");
    let debt_addr = t.resolve_asset("ETH");
    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);

    let ctrl = t.ctrl_client();
    let result = ctrl.try_multiply(
        &caller,
        &0u64, // account_id = 0 (create new)
        &2u32, // spoke_id = 2
        &hub_asset(collateral_addr.clone()),
        &10_0000000i128,                            // 1 ETH worth of debt
        &hub_asset(debt_addr.clone()),              // ETH -- not in spoke category 2
        &controller::types::PositionMode::Multiply, // mode = 1 (multiply)
        &steps,
        &None, // initial_payment
        &None, // convert_steps
    );

    // ETH is not listed on the account's spoke, so the borrow gate rejects the
    // leg with AssetNotInSpoke (307).
    assert_contract_error(flatten(result), errors::ASSET_NOT_IN_SPOKE);
}
// Spoke account in the stablecoin category, but collateral is ETH (not in
// category).

#[test]
fn test_multiply_spoke_wrong_category_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    let caller = t.get_or_create_user(ALICE);
    let collateral_addr = t.resolve_asset("ETH"); // not in spoke category
    let debt_addr = t.resolve_asset("USDC"); // in spoke category
                                             // The collateral spoke gate fires at entry, before any funds move; the
                                             // router funding only keeps the fixture realistic.
    t.fund_router("ETH", 5.0);
    // multiply borrows 1000 USDC (raw 10_000_000_000) minus 9bps fee.
    let steps = build_aggregator_swap(
        &t,
        "USDC",
        "ETH",
        apply_flash_fee(10_000_000_000),
        5_0000000,
    );

    let ctrl = t.ctrl_client();
    let result = ctrl.try_multiply(
        &caller,
        &0u64,                               // account_id = 0 (create new)
        &2u32,                               // spoke_id = 2
        &hub_asset(collateral_addr.clone()), // ETH: not in spoke category
        &1000_0000000i128,
        &hub_asset(debt_addr.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &None, // initial_payment
        &None, // convert_steps
    );

    // ETH is not listed on the account's spoke, so the entry gate rejects the
    // collateral leg with AssetNotInSpoke (307).
    assert_contract_error(flatten(result), errors::ASSET_NOT_IN_SPOKE);
}
#[test]
fn test_multiply_rejects_normal_mode() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    // PositionMode::Normal is reserved for non-strategy accounts; multiply
    // requires Multiply, Long, or Short.
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Normal,
        &steps,
    );
    assert_contract_error(result, errors::INVALID_POSITION_MODE);
}
// An existing account at the supply-position limit cannot open a new
// collateral leg through multiply.

#[test]
fn test_multiply_rejects_new_collateral_when_supply_limit_reached() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_position_limits(1, 4)
        .build();

    let account_id = t.create_account_full(ALICE, 1, controller::types::PositionMode::Multiply);
    t.supply_to(ALICE, account_id, "WBTC", 0.1);

    t.fund_router("USDC", 3000.0);
    // 1 ETH (raw 10_000_000) flash-borrowed minus 9bps fee.
    let steps = build_aggregator_swap(&t, "ETH", "USDC", apply_flash_fee(10_000_000), 3000_0000000);

    let caller = t.get_or_create_user(ALICE);
    let ctrl = t.ctrl_client();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    let result = ctrl.try_multiply(
        &caller,
        &account_id,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );

    assert_contract_error(flatten(result), errors::POSITION_LIMIT_EXCEEDED);
}
// Reusing another user's account must fail before the strategy borrow path.

#[test]
fn test_multiply_existing_account_wrong_owner() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let account_id = t.create_account_full(ALICE, 1, controller::types::PositionMode::Multiply);
    let bob = t.get_or_create_user(BOB);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    t.fund_router("USDC", 3_000.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 3000_0000000);

    let result = t.ctrl_client().try_multiply(
        &bob,
        &account_id,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );

    // Bob calls multiply targeting Alice's existing account. The ownership
    // check must fail with NotAuthorized, not as a host-level auth failure.
    assert_contract_error(flatten(result), errors::NOT_AUTHORIZED);
}
// Favorable slippage refunds must not sweep unrelated controller balances.

#[test]
fn test_multiply_respects_borrow_position_limit() {
    use test_harness::xlm_preset;

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(xlm_preset())
        .build();

    // Liquidity for the second strategy's debt leg.
    t.supply(BOB, "XLM", 10_000.0);

    // First multiply: 1 ETH debt -> 3000 USDC collateral (one borrow position).
    t.fund_router("USDC", 3000.0);
    let steps = build_aggregator_swap(&t, "ETH", "USDC", apply_flash_fee(10_000_000), 3000_0000000);
    let account_id = t.multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        controller::types::PositionMode::Multiply,
        &steps,
    );

    // Cap borrow positions at the count the account already holds.
    t.set_position_limits(8, 1);

    // A second multiply into the same account with a different debt asset
    // would open a second borrow position and must hit the limit gate.
    t.fund_router("USDC", 10.0);
    let steps2 = build_aggregator_swap(
        &t,
        "XLM",
        "USDC",
        apply_flash_fee(1_000_000_000),
        10_0000000,
    );
    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let xlm = t.resolve_asset("XLM");
    let result = match t.ctrl_client().try_multiply(
        &alice,
        &account_id,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_000_000_000i128,
        &hub_asset(xlm.clone()),
        &controller::types::PositionMode::Multiply,
        &steps2,
        &None,
        &None,
    ) {
        Ok(Ok(id)) => Ok(id),
        Ok(Err(err)) => Err(err),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, errors::POSITION_LIMIT_EXCEEDED);
}
