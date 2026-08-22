use super::*;

#[test]
fn test_multiply_with_debt_token_initial_payment() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let eth_market = t.resolve_market("ETH");
    eth_market.token_admin.mint(&alice, &5_000000i128);

    let alice_eth_before = t.token_balance(ALICE, "ETH");
    t.fund_router("USDC", 4_500.0);

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

    let alice_eth_after = t.token_balance(ALICE, "ETH");
    assert!(
        (alice_eth_before - alice_eth_after - 0.5).abs() < 1e-6,
        "Alice's ETH wallet should drop by exactly 0.5 ETH, before={}, after={}",
        alice_eth_before,
        alice_eth_after
    );
}

#[test]
fn test_multiply_rejects_unlisted_third_token_payment_before_transfer() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let sac = t.env.register_stellar_asset_contract_v2(t.admin());
    let unlisted = sac.address().clone();
    token::StellarAssetClient::new(&t.env, &unlisted).mint(&alice, &1_0000000i128);
    let balance_before = token::TokenClient::new(&t.env, &unlisted).balance(&alice);

    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 1000_0000000);
    let result = t.ctrl_client().try_multiply(
        &alice,
        &0u64,
        &1u32,
        &hub_asset(usdc.clone()),
        &1_0000000i128,
        &hub_asset(eth.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &Some((hub_asset(unlisted.clone()), 1_0000000i128)),
        &None,
    );

    match result {
        // The payment asset is priced with the rest of the strategy legs, so an
        // unlisted token is rejected at the price prefetch, before any transfer.
        Err(Ok(err)) => assert_eq!(
            err,
            soroban_sdk::Error::from_contract_error(errors::ORACLE_NOT_CONFIGURED),
            "unlisted payment token must be rejected before transfer"
        ),
        other => panic!("expected OracleNotConfigured, got {:?}", other),
    }

    assert_eq!(
        token::TokenClient::new(&t.env, &unlisted).balance(&alice),
        balance_before,
        "a rejected multiply must not move the payment token"
    );
}

#[test]
fn test_multiply_rejects_third_token_payment_without_convert() {
    use test_harness::xlm_preset;
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market(xlm_preset())
        .build();

    let alice = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let xlm = t.resolve_asset("XLM");
    t.resolve_market("XLM")
        .token_admin
        .mint(&alice, &10_0000000i128);

    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 1000_0000000);
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
    let mut t = LendingTest::new().standard_two_asset().build();

    t.pause();

    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 1000_0000000);
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

#[test]
fn test_multiply_preserves_existing_collateral_balance() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let account_id = t.create_account_full(ALICE, 1, controller::types::PositionMode::Multiply);
    t.supply_to(ALICE, account_id, "USDC", 1_000.0);

    t.fund_router("USDC", 3_000.0);

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
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

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
        &2u32,
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
    assert_eq!(attrs.spoke_id, 2, "reused account must keep spoke 2");
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
    let mut t = LendingTest::new().standard_two_asset().build();

    let caller = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 1000_0000000);
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
    let mut t = LendingTest::new().standard_two_asset().build();

    let caller = t.get_or_create_user(ALICE);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 1000_0000000);
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
        map_try_ok_value(result),
        errors::GenericError::AccountNotFound as u32,
    );
}

#[test]
fn test_multiply_spoke_wrong_category_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_market(eth_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    let caller = t.get_or_create_user(ALICE);
    let collateral_addr = t.resolve_asset("USDC");
    let debt_addr = t.resolve_asset("ETH");
    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 1000_0000000);

    let ctrl = t.ctrl_client();
    let result = ctrl.try_multiply(
        &caller,
        &0u64,
        &2u32,
        &hub_asset(collateral_addr.clone()),
        &10_0000000i128,
        &hub_asset(debt_addr.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );

    assert_contract_error(map_try_ok_value(result), errors::ASSET_NOT_IN_SPOKE);
}

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
    let collateral_addr = t.resolve_asset("ETH");
    let debt_addr = t.resolve_asset("USDC");

    t.fund_router("ETH", 5.0);

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
        &0u64,
        &2u32,
        &hub_asset(collateral_addr.clone()),
        &1000_0000000i128,
        &hub_asset(debt_addr.clone()),
        &controller::types::PositionMode::Multiply,
        &steps,
        &None,
        &None,
    );

    assert_contract_error(map_try_ok_value(result), errors::ASSET_NOT_IN_SPOKE);
}
#[test]
fn test_multiply_rejects_normal_mode() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 1000_0000000);

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

#[test]
fn test_multiply_rejects_new_collateral_when_supply_limit_reached() {
    let mut t = LendingTest::new()
        .three_asset_usdc_eth_wbtc()
        .with_position_limits(1, 4)
        .build();

    let account_id = t.create_account_full(ALICE, 1, controller::types::PositionMode::Multiply);
    t.supply_to(ALICE, account_id, "WBTC", 0.1);

    t.fund_router("USDC", 3000.0);

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

    assert_contract_error(map_try_ok_value(result), errors::POSITION_LIMIT_EXCEEDED);
}

#[test]
fn test_multiply_existing_account_wrong_owner() {
    let mut t = LendingTest::new().standard_two_asset().build();

    let account_id = t.create_account_full(ALICE, 1, controller::types::PositionMode::Multiply);
    let bob = t.get_or_create_user(BOB);
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");

    t.fund_router("USDC", 3_000.0);
    let steps = build_aggregator_swap(&t, "ETH", "USDC", 0, 3000_0000000);

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

    assert_contract_error(map_try_ok_value(result), errors::NOT_AUTHORIZED);
}

#[test]
fn test_multiply_respects_borrow_position_limit() {
    use test_harness::xlm_preset;

    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_market(xlm_preset())
        .build();

    t.supply(BOB, "XLM", 10_000.0);

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

    t.set_position_limits(5, 1);

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
    let result = map_try_ok_value(t.ctrl_client().try_multiply(
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
    ));
    assert_contract_error(result, errors::POSITION_LIMIT_EXCEEDED);
}
