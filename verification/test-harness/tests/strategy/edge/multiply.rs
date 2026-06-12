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
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
        &steps,
        &Some((eth.clone(), 5_000000i128)),
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

// The initial payment token is the one user-supplied call target in the
// multiply flow; it must be a listed market asset. The assert fires before
// the controller invokes the token contract, so no balance is needed.
#[test]
fn test_multiply_rejects_unlisted_initial_payment_token() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let unlisted = t
        .env
        .register_stellar_asset_contract_v2(t.admin())
        .address()
        .clone();

    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);
    let result = t.ctrl_client().try_multiply(
        &alice,
        &0u64,
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
        &steps,
        &Some((unlisted, 1_0000000i128)),
        &None,
    );

    match result {
        Err(Ok(err)) => assert_eq!(
            err,
            soroban_sdk::Error::from_contract_error(errors::ASSET_NOT_SUPPORTED),
            "unlisted initial payment token must fail AssetNotSupported"
        ),
        other => panic!("expected AssetNotSupported, got {:?}", other),
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
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::CONTRACT_PAUSED);
}
// The borrow-cap check runs after pool.create_strategy(). The borrow cap is
// set extremely low ($0.001), so multiply rejects after the borrow exceeds
// the cap.

#[test]
fn test_multiply_borrow_cap_would_exceed() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("ETH", |c| {
            // Set borrow cap extremely low: 1 unit (0.0000001 ETH).
            c.borrow_cap = 1;
        })
        .build();

    // Attempt to multiply with 1 ETH debt, exceeding the borrow cap. Flow:
    // create_strategy -> check borrow cap -> reject with a specific code.
    let steps = build_swap_steps(&t, "ETH", "USDC", 5000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::BORROW_CAP_REACHED);
}
// Reusing an account that already holds the collateral asset must add to the
// existing position, not replace it.

#[test]
fn test_multiply_preserves_existing_collateral_balance() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let account_id = t.create_account_full(ALICE, 0, common::types::PositionMode::Multiply, false);
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
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
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
// E-mode account in the stablecoin category, but debt is ETH (not in
// category). Validation runs before the swap, so the error is clean.

#[test]
fn test_multiply_emode_wrong_category_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        // ETH is NOT in e-mode category 1
        .build();

    // Use the raw controller client so `e_mode_category=1` can be passed
    // explicitly.
    let caller = t.get_or_create_user(ALICE);
    let collateral_addr = t.resolve_asset("USDC");
    let debt_addr = t.resolve_asset("ETH");
    let steps = build_swap_steps(&t, "ETH", "USDC", 1000_0000000);

    let ctrl = t.ctrl_client();
    let result = ctrl.try_multiply(
        &caller,
        &0u64, // account_id = 0 (create new)
        &1u32, // e_mode_category = 1
        &collateral_addr,
        &10_0000000i128,                        // 1 ETH worth of debt
        &debt_addr,                             // ETH -- not in e-mode category 1
        &common::types::PositionMode::Multiply, // mode = 1 (multiply)
        &steps,
        &None, // initial_payment
        &None, // convert_steps
    );

    // ETH is not in e-mode category 1. `token_e_mode_config` surfaces
    // `EModeCategoryNotFound` (300) when the asset is unregistered.
    assert_contract_error(flatten(result), errors::EMODE_CATEGORY_NOT_FOUND);
}
// E-mode account in the stablecoin category, but collateral is ETH (not in
// category).

#[test]
fn test_multiply_emode_wrong_category_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_emode(1, STABLECOIN_EMODE)
        .with_emode_asset(1, "USDC", true, true)
        .with_emode_asset(1, "USDT", true, true)
        .build();

    let caller = t.get_or_create_user(ALICE);
    let collateral_addr = t.resolve_asset("ETH"); // not in e-mode category
    let debt_addr = t.resolve_asset("USDC"); // in e-mode category
                                             // Fund the mock router so the swap itself succeeds; this lets the emode
                                             // check on the deposit leg fire (otherwise the router fails first).
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
        &0u64,            // account_id = 0 (create new)
        &1u32,            // e_mode_category = 1
        &collateral_addr, // ETH: not in e-mode category
        &1000_0000000i128,
        &debt_addr,
        &common::types::PositionMode::Multiply,
        &steps,
        &None, // initial_payment
        &None, // convert_steps
    );

    // ETH is not in the e-mode category, so `token_e_mode_config` rejects
    // with EMODE_CATEGORY_NOT_FOUND (300).
    assert_contract_error(flatten(result), errors::EMODE_CATEGORY_NOT_FOUND);
}
// New isolated collateral via multiply must still enforce the debt asset's
// isolation_borrow_enabled flag.

#[test]
fn test_multiply_isolated_debt_not_enabled() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("USDC", |c| {
            c.is_isolated_asset = true;
            c.isolation_debt_ceiling_usd_wad = usd(1_000_000);
        })
        // ETH has isolation_borrow_enabled = false (default)
        .build();

    t.fund_router("USDC", 3000.0); // Pre-fund the router with output tokens.
    let steps = build_swap_steps(&t, "ETH", "USDC", 3000_0000000);
    let result = t.try_multiply(
        ALICE,
        "USDC",
        1.0,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::NOT_BORROWABLE_ISOLATION);
}
// An existing non-isolated account must not add an isolated collateral leg.

#[test]
fn test_multiply_rejects_isolated_collateral_on_existing_non_isolated_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_market_config("USDC", |c| {
            c.is_isolated_asset = true;
            c.isolation_debt_ceiling_usd_wad = usd(1_000_000);
        })
        .with_market_config("ETH", |c| {
            c.isolation_borrow_enabled = true;
        })
        .build();

    let account_id = t.create_account_full(ALICE, 0, common::types::PositionMode::Multiply, false);
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
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );

    // The account's `is_isolated` flag is false but the requested collateral
    // would force isolation: reject with MIX_ISOLATED_COLLATERAL (303).
    assert_contract_error(flatten(result), errors::MIX_ISOLATED_COLLATERAL);
}
// The debt asset is siloed, but `multiply` creates a fresh account with no
// existing borrows. The siloed-borrow restriction therefore lives in the
// `swap_debt` tests instead.

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
        common::types::PositionMode::Normal,
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

    let account_id = t.create_account_full(ALICE, 0, common::types::PositionMode::Multiply, false);
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
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
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

    let account_id = t.create_account_full(ALICE, 0, common::types::PositionMode::Multiply, false);
    let bob = t.get_or_create_user(BOB);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    t.fund_router("USDC", 3_000.0);
    let steps = build_swap_steps(&t, "ETH", "USDC", 3000_0000000);

    let result = t.ctrl_client().try_multiply(
        &bob,
        &account_id,
        &0u32,
        &usdc,
        &1_0000000i128,
        &eth,
        &common::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );

    // Bob calls multiply targeting Alice's existing account. The ownership
    // check must fail with AccountNotInMarket, not as a generic auth failure.
    assert_contract_error(flatten(result), errors::ACCOUNT_NOT_IN_MARKET);
}
// The post-deposit supply cap check in multiply must reject oversized output.

#[test]
fn test_multiply_rejects_supply_cap_after_deposit() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("USDC", |c| {
            c.supply_cap = 1; // extremely low: 1 unit (0.0000001 USDC).
        })
        .build();

    t.fund_router("USDC", 100.0);
    // 0.05 ETH (raw 500_000) flash-borrowed minus 9bps fee.
    let steps = build_aggregator_swap(&t, "ETH", "USDC", apply_flash_fee(500_000), 100_0000000);

    let result = t.try_multiply(
        ALICE,
        "USDC",
        0.05,
        "ETH",
        common::types::PositionMode::Multiply,
        &steps,
    );
    assert_contract_error(result, errors::SUPPLY_CAP_REACHED);
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
        common::types::PositionMode::Multiply,
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
        &0u32,
        &usdc,
        &1_000_000_000i128,
        &xlm,
        &common::types::PositionMode::Multiply,
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
