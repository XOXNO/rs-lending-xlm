use controller::constants::RAY_DECIMALS;
use controller::types::{ControllerKey, SpokeAssetArgs};
use test_harness::{
    assert_contract_error, errors, hub_asset, usd_cents, usdc_preset, usdt_stable_preset,
    LendingTest, PositionType, ALICE, HARNESS_HUB, HARNESS_SPOKE, LIQUIDATOR, STABLECOIN_SPOKE,
};
#[test]
fn test_spoke_category_creation() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    let mut t = t;
    let account_id = t.create_spoke_account(ALICE, 2);
    assert!(account_id > 0, "should create spoke account");
    let attrs = t.get_account_attributes(ALICE);
    assert_eq!(attrs.spoke_id, 2, "account should be in spoke category 1");
}
#[test]
fn test_spoke_enhanced_ltv_and_threshold() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);

    t.borrow(ALICE, "USDT", 9_500.0);
    t.assert_healthy(ALICE);

    let hf = t.health_factor(ALICE);
    assert!(
        (1.0..1.10).contains(&hf),
        "spoke should allow tight but healthy position, HF={}",
        hf
    );
}
#[test]
fn test_spoke_supply_with_category_asset() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 5_000.0);
    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
    t.assert_supply_near(ALICE, "USDC", 5_000.0, 1.0);
    assert!(
        t.token_balance(ALICE, "USDC") < 0.01,
        "wallet should be ~0 after supply"
    );
}
#[test]
fn test_spoke_borrow_with_category_asset() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);

    t.assert_position_exists(ALICE, "USDT", PositionType::Borrow);
    t.assert_borrow_near(ALICE, "USDT", 5_000.0, 1.0);
    let usdt_wallet = t.token_balance(ALICE, "USDT");
    assert!(
        (usdt_wallet - 5_000.0).abs() < 1.0,
        "Alice should receive ~5000 USDT, got {}",
        usdt_wallet
    );
    t.assert_healthy(ALICE);
}
#[test]
fn test_spoke_rejects_non_category_supply() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);

    let result = t.try_supply(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::ASSET_NOT_IN_SPOKE);
}
#[test]
fn test_spoke_rejects_non_category_borrow() {
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);

    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::ASSET_NOT_IN_SPOKE);
}
#[test]
fn test_spoke_edit_asset_params() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.edit_asset_in_spoke("USDC", 2, true, true, 8000, 8500, 300);

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);

    let result = t.try_borrow(ALICE, "USDT", 9_500.0);
    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}
#[test]
fn test_spoke_remove_category_deprecates() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);

    t.remove_spoke_category(2);

    let result = t.try_supply(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::SPOKE_DEPRECATED);
}

#[test]
fn test_deprecated_spoke_debt_free_account_can_withdraw_all_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 1_000.0);
    assert_eq!(t.borrow_balance(ALICE, "USDC"), 0.0);

    t.remove_spoke_category(2);

    let result = t.try_withdraw(ALICE, "USDC", 0.0);
    assert!(
        result.is_ok(),
        "debt-free spoke account should be able to exit after category deprecation; got {result:?}"
    );
    assert_eq!(t.supply_balance(ALICE, "USDC"), 0.0);
}
#[test]
fn test_spoke_add_asset_to_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.add_asset_to_spoke("USDT", 2, true, true, 9700, 9800, 200);

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);
    t.assert_healthy(ALICE);
}
#[test]
fn test_spoke_remove_asset_from_category() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.remove_asset_from_spoke("USDT", 2);

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);

    let result = t.try_borrow(ALICE, "USDT", 5_000.0);
    assert_contract_error(result, errors::ASSET_NOT_IN_SPOKE);
}
#[test]
fn test_spoke_liquidation_uses_spoke_bonus() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 9_500.0);

    t.set_price("USDC", usd_cents(90));
    t.assert_liquidatable(ALICE);

    let debt_before = t.borrow_balance(ALICE, "USDT");
    t.liquidate(LIQUIDATOR, ALICE, "USDT", 2_000.0);
    let debt_after = t.borrow_balance(ALICE, "USDT");
    assert!(
        debt_after < debt_before,
        "USDT debt should decrease after liquidation: before={}, after={}",
        debt_before,
        debt_after
    );

    let usdc_received = t.token_balance(LIQUIDATOR, "USDC");
    assert!(usdc_received > 0.0, "liquidator should receive collateral");

    let usdc_value = usdc_received * 0.90;
    let debt_value = 2_000.0;

    if usdc_value > 0.0 {
        let ratio = usdc_value / debt_value;

        assert!(
            ratio > 1.015 && ratio < 1.04,
            "spoke bonus should be ~1.02 (not zero, not 5%): ratio={}",
            ratio
        );
    }
}
#[test]
fn test_spoke_two_assets_same_category() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);

    t.supply(ALICE, "USDC", 5_000.0);
    t.supply(ALICE, "USDT", 5_000.0);

    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
    t.assert_position_exists(ALICE, "USDT", PositionType::Supply);
    t.assert_supply_near(ALICE, "USDC", 5_000.0, 1.0);
    t.assert_supply_near(ALICE, "USDT", 5_000.0, 1.0);

    t.borrow(ALICE, "USDC", 2_000.0);
    t.assert_position_exists(ALICE, "USDC", PositionType::Borrow);
    t.assert_borrow_near(ALICE, "USDC", 2_000.0, 1.0);
    t.assert_healthy(ALICE);
}
#[test]
fn test_spoke_deprecated_category_operations() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.remove_spoke_category(2);

    let remove_result = t.ctrl_client().try_remove_spoke(&2u32);
    let flat_remove: Result<(), soroban_sdk::Error> = match remove_result {
        Ok(Ok(_)) => panic!("expected contract error, got Ok"),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(flat_remove, errors::SPOKE_DEPRECATED);

    let asset_address = t.resolve_asset("USDC");
    let edit_asset_result = t.ctrl_client().try_edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        hub_id: HARNESS_HUB,
        asset: asset_address.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: true,
        frozen: false,
        no_seize: false,
        ltv: 9_000,
        threshold: 9_300,
        bonus: 200,
        supply_cap: 0,
        borrow_cap: 0,
    });
    assert!(
        edit_asset_result.is_ok(),
        "editing a live listing on a deprecated spoke must stay possible: {edit_asset_result:?}"
    );

    let usdt_address = t.resolve_asset("USDT");
    let add_asset_result = t.ctrl_client().try_add_asset_to_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        hub_id: HARNESS_HUB,
        asset: usdt_address,
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: 9_000,
        threshold: 9_300,
        bonus: 200,
        supply_cap: 0,
        borrow_cap: 0,
    });
    let flat_add_asset: Result<(), soroban_sdk::Error> = match add_asset_result {
        Ok(Ok(_)) => panic!("expected contract error, got Ok"),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(flat_add_asset, errors::SPOKE_DEPRECATED);
}

#[test]
fn test_supply_rejects_spoke_mismatch_on_existing_account() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 50.0);

    let result = t.try_supply_with_spoke(ALICE, "USDC", 10.0, 2);
    assert_contract_error(result, errors::SPOKE_MISMATCH);
}

#[test]
fn test_supply_rejects_spoke_mismatch_against_active_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    let _ = t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 50.0);

    let result = t.try_supply_with_spoke(ALICE, "USDC", 10.0, 3);
    assert_contract_error(result, errors::SPOKE_MISMATCH);
}

#[test]
fn test_supply_zero_spoke_rejects_mismatch_against_active_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    let _ = t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 50.0);

    let result = t.try_supply_with_spoke(ALICE, "USDC", 10.0, 0);
    assert_contract_error(result, errors::SPOKE_MISMATCH);
}

/// New-account branch of the spoke-id gate: `account_id == 0` has no stored
/// spoke to match against, so `create_account` must reject an unregistered id
/// instead of minting an account pinned to a spoke that does not exist.
#[test]
fn test_supply_new_account_rejects_unknown_spoke() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    let result = t.try_supply_with_spoke(ALICE, "USDC", 10.0, HARNESS_SPOKE + 98);
    assert_contract_error(result, errors::SPOKE_NOT_FOUND);
}

/// Spoke ids are 1-based, so `0` is the "unset" sentinel and can never name a
/// listing. On an existing account it surfaces as `SpokeMismatch`; on a new one
/// there is nothing to mismatch, so the explicit `spoke_id >= 1` guard answers.
#[test]
fn test_supply_new_account_rejects_zero_spoke() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    let result = t.try_supply_with_spoke(ALICE, "USDC", 10.0, 0);
    assert_contract_error(result, errors::SPOKE_NOT_FOUND);
}

#[test]
fn test_deprecated_spoke_debt_free_account_can_partially_withdraw_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 1_000.0);
    t.remove_spoke_category(2);

    let result = t.try_withdraw(ALICE, "USDC", 100.0);
    assert!(
        result.is_ok(),
        "deprecated spoke must not block debt-free partial exits; got {result:?}"
    );
    assert_eq!(
        t.supply_balance_raw(ALICE, "USDC"),
        900 * 10_000_000,
        "the withdraw must debit exactly 100 USDC from the 1_000 supplied"
    );
}

#[test]
fn test_deprecated_spoke_repay_allowed_but_new_borrow_blocked() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 2_000.0);
    t.remove_spoke_category(2);

    let borrow_more = t.try_borrow(ALICE, "USDT", 1.0);
    assert_contract_error(borrow_more, errors::SPOKE_DEPRECATED);

    let debt_before = t.borrow_balance_raw(ALICE, "USDT");
    let repay = t.try_repay(ALICE, "USDT", 500.0);
    assert!(
        repay.is_ok(),
        "deprecated spoke must not block debt-reducing repay; got {repay:?}"
    );
    assert_eq!(
        debt_before - t.borrow_balance_raw(ALICE, "USDT"),
        500 * 10_000_000,
        "the repay must retire exactly the 500 USDT paid"
    );
}

#[test]
fn test_deprecated_spoke_with_debt_keeps_stored_params_on_withdraw() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);
    let account_id = t.resolve_account_id(ALICE);
    let stamped = |t: &LendingTest| -> (u32, u32) {
        let p = t
            .ctrl_client()
            .get_account_positions(&account_id)
            .0
            .get(hub_asset(t.resolve_asset("USDC")))
            .expect("USDC supply position should exist");
        (p.loan_to_value, p.liquidation_threshold)
    };
    let before = stamped(&t);
    t.remove_spoke_category(2);

    let result = t.try_withdraw(ALICE, "USDC", 4_000.0);
    assert!(
        result.is_ok(),
        "deprecated spoke must keep stored position params on safe withdrawals; got {result:?}"
    );
    assert_eq!(
        stamped(&t),
        before,
        "the deprecated spoke must leave the stamped (ltv, threshold) untouched"
    );
    t.assert_healthy(ALICE);
}

#[test]
fn test_deprecated_spoke_with_debt_withdraw_still_enforces_stored_spoke_ltv() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);
    t.remove_spoke_category(2);

    let result = t.try_withdraw(ALICE, "USDC", 5_000.0);
    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}

#[test]
fn test_deprecated_spoke_category_still_allows_liquidation() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 9_500.0);
    t.remove_spoke_category(2);
    t.set_price("USDC", usd_cents(85));
    t.assert_liquidatable(ALICE);

    let debt_before = t.borrow_balance_raw(ALICE, "USDT");
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "USDT", 500.0);
    assert!(
        result.is_ok(),
        "deprecated spoke must not block liquidation; got {result:?}"
    );
    assert_eq!(
        debt_before - t.borrow_balance_raw(ALICE, "USDT"),
        500 * 10_000_000,
        "the liquidation must retire exactly the 500 USDT repaid"
    );
}

#[test]
fn test_deprecated_spoke_blocks_new_borrow_but_preserves_exit() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 2_000.0);

    t.remove_spoke_category(2);

    assert_contract_error(t.try_borrow(ALICE, "USDT", 100.0), errors::SPOKE_DEPRECATED);

    t.withdraw(ALICE, "USDC", 4_000.0);
}

fn force_delist(t: &LendingTest, asset_name: &str, spoke_id: u32) {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller_address(), || {
        t.env
            .storage()
            .persistent()
            .remove(&ControllerKey::SpokeAsset(spoke_id, hub_asset(asset)));
    });
}

#[test]
fn test_removed_spoke_collateral_asset_blocks_new_supply_but_existing_withdraw_works() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);
    force_delist(&t, "USDC", 2);

    let add_more = t.try_supply(ALICE, "USDC", 1.0);
    assert_contract_error(add_more, errors::ASSET_NOT_IN_SPOKE);

    let withdraw = t.try_withdraw(ALICE, "USDC", 4_000.0);
    assert!(
        withdraw.is_ok(),
        "removed collateral asset must still allow safe withdrawal of an existing position; got {withdraw:?}"
    );
    assert_eq!(
        t.supply_balance_raw(ALICE, "USDC"),
        6_000 * 10_000_000,
        "the withdraw must debit exactly 4_000 USDC from the 10_000 supplied"
    );
    t.assert_healthy(ALICE);
}

#[test]
fn test_removed_spoke_debt_asset_blocks_new_borrow_but_existing_repay_works() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 2_000.0);
    force_delist(&t, "USDT", 2);

    let borrow_more = t.try_borrow(ALICE, "USDT", 1.0);
    assert_contract_error(borrow_more, errors::ASSET_NOT_IN_SPOKE);

    let debt_before = t.borrow_balance_raw(ALICE, "USDT");
    let repay = t.try_repay(ALICE, "USDT", 500.0);
    assert!(
        repay.is_ok(),
        "removed debt asset must still allow debt-reducing repay; got {repay:?}"
    );
    assert_eq!(
        debt_before - t.borrow_balance_raw(ALICE, "USDT"),
        500 * 10_000_000,
        "the repay must retire exactly the 500 USDT paid"
    );
}

#[test]
fn test_removed_spoke_collateral_asset_stays_liquidatable() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 9_500.0);
    force_delist(&t, "USDC", 2);
    t.set_price("USDC", usd_cents(85));

    t.assert_liquidatable(ALICE);

    let debt_before = t.borrow_balance_raw(ALICE, "USDT");
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "USDT", 500.0);
    assert!(
        result.is_ok(),
        "delisted collateral must stay seizable; got {result:?}"
    );
    assert_eq!(
        debt_before - t.borrow_balance_raw(ALICE, "USDT"),
        500 * 10_000_000,
        "the liquidation must retire exactly the 500 USDT repaid against delisted collateral"
    );
}

#[test]
fn test_spoke_collateral_flag_update_blocks_new_supply_but_existing_withdraw_works() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 1_000.0);
    t.edit_asset_in_spoke("USDC", 2, false, true, 9700, 9800, 200);

    let add_more = t.try_supply(ALICE, "USDC", 1.0);
    assert_contract_error(add_more, errors::NOT_COLLATERAL);

    let withdraw = t.try_withdraw(ALICE, "USDC", 100.0);
    assert!(
        withdraw.is_ok(),
        "collateral flag removal must not block withdrawing an existing position; got {withdraw:?}"
    );
    assert_eq!(
        t.supply_balance_raw(ALICE, "USDC"),
        900 * 10_000_000,
        "the withdraw must debit exactly 100 USDC from the 1_000 supplied"
    );
}

#[test]
fn test_spoke_borrow_flag_update_blocks_new_borrow_but_existing_repay_works() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 2_000.0);
    t.edit_asset_in_spoke("USDT", 2, true, false, 9700, 9800, 200);

    let borrow_more = t.try_borrow(ALICE, "USDT", 1.0);
    assert_contract_error(borrow_more, errors::ASSET_NOT_BORROWABLE);

    let debt_before = t.borrow_balance_raw(ALICE, "USDT");
    let repay = t.try_repay(ALICE, "USDT", 500.0);
    assert!(
        repay.is_ok(),
        "borrow flag removal must not block repaying an existing debt; got {repay:?}"
    );
    assert_eq!(
        debt_before - t.borrow_balance_raw(ALICE, "USDT"),
        500 * 10_000_000,
        "the repay must retire exactly the 500 USDT paid"
    );
}

#[test]
fn test_edit_asset_in_spoke_rejects_inverted_or_unsafe_bounds() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();
    let usdc = t.resolve_asset("USDC");

    let inverted = t.ctrl_client().try_edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: 8_500,
        threshold: 8_000,
        bonus: 200,
        supply_cap: 0,
        borrow_cap: 0,
    });
    let flat_inverted: Result<(), soroban_sdk::Error> = match inverted {
        Ok(Ok(_)) => panic!("expected contract error, got Ok"),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(flat_inverted, errors::INVALID_LIQ_THRESHOLD);

    let unsafe_bonus = t.ctrl_client().try_edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: 9_400,
        threshold: 9_500,
        bonus: 600,
        supply_cap: 0,
        borrow_cap: 0,
    });
    let flat_unsafe: Result<(), soroban_sdk::Error> = match unsafe_bonus {
        Ok(Ok(_)) => panic!("expected contract error, got Ok"),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(flat_unsafe, errors::INVALID_LIQ_THRESHOLD);

    t.edit_asset_in_spoke("USDC", 2, true, true, 9_000, 9_300, 200);
    let cfg = t
        .ctrl_client()
        .get_spoke_asset(&2u32, &hub_asset(usdc.clone()));
    assert_eq!(cfg.loan_to_value, 9_000);
    assert_eq!(cfg.liquidation_threshold, 9_300);
    assert!(cfg.liquidation_threshold > cfg.loan_to_value);
}

#[test]
fn test_spoke_per_asset_divergent_params() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.add_asset_to_spoke("USDT", 2, true, true, 9_000, 9_300, 300);

    let account_id = t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 5_000.0);
    t.supply(ALICE, "USDT", 5_000.0);

    let usdc = t.resolve_asset("USDC");
    let usdt = t.resolve_asset("USDT");
    let (supplies, _) = t.ctrl_client().get_account_positions(&account_id);

    let usdc_pos = supplies.get(hub_asset(usdc)).expect("USDC position");
    assert_eq!(
        usdc_pos.loan_to_value, 9_700,
        "USDC keeps its 97% spoke LTV"
    );
    assert_eq!(usdc_pos.liquidation_threshold, 9_800);

    let usdt_pos = supplies.get(hub_asset(usdt)).expect("USDT position");
    assert_eq!(
        usdt_pos.loan_to_value, 9_000,
        "USDT carries its own tighter LTV"
    );
    assert_eq!(usdt_pos.liquidation_threshold, 9_300);
}

const UNIT: i128 = 10_000_000;

#[test]
fn test_spoke_supply_cap_enforced() {
    let spoke_cap = 1_000 * UNIT;

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.edit_asset_in_spoke_caps("USDC", 2, true, true, 9_700, 9_800, 200, spoke_cap, 0);

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 500.0);

    let result = t.try_supply(ALICE, "USDC", 600.0);
    assert_contract_error(result, errors::SPOKE_SUPPLY_CAP_REACHED);
}

#[test]
fn test_spoke_borrow_cap_enforced() {
    let spoke_borrow_cap = 500 * UNIT;

    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.edit_asset_in_spoke_caps(
        "USDT",
        2,
        true,
        true,
        9_700,
        9_800,
        200,
        0,
        spoke_borrow_cap,
    );

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 400.0);
    t.borrow(ALICE, "USDT", 50.0);

    let result = t.try_borrow(ALICE, "USDT", 100.0);
    assert_contract_error(result, errors::SPOKE_BORROW_CAP_REACHED);
}

fn spoke_supply_usage(t: &LendingTest, category_id: u32, asset_name: &str) -> i128 {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .persistent()
            .get::<_, controller::types::SpokeUsageRaw>(
                &controller::types::ControllerKey::SpokeUsage(category_id, hub_asset(asset)),
            )
            .map(|u| u.supplied_scaled_ray)
            .unwrap_or(0)
    })
}

fn spoke_borrow_usage(t: &LendingTest, category_id: u32, asset_name: &str) -> i128 {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller, || {
        t.env
            .storage()
            .persistent()
            .get::<_, controller::types::SpokeUsageRaw>(
                &controller::types::ControllerKey::SpokeUsage(category_id, hub_asset(asset)),
            )
            .map(|u| u.borrowed_scaled_ray)
            .unwrap_or(0)
    })
}

#[test]
fn test_removed_spoke_asset_withdraw_decrements_usage() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 1_000.0);
    let usage_before = spoke_supply_usage(&t, 2, "USDC");
    assert!(usage_before > 0, "supply should record spoke usage");

    force_delist(&t, "USDC", 2);
    let withdraw = t.try_withdraw(ALICE, "USDC", 400.0);
    assert!(
        withdraw.is_ok(),
        "withdraw must still work after asset removal"
    );

    let usage_after = spoke_supply_usage(&t, 2, "USDC");
    assert_eq!(
        usage_before - usage_after,
        400 * 10_000_000 * 10i128.pow(RAY_DECIMALS - 7),
        "withdraw must decrement usage by exactly the 400 USDC withdrawn, \
         even when the asset left the category"
    );
}

#[test]
fn test_deprecated_spoke_repay_decrements_usage() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 2_000.0);
    let usage_before = spoke_borrow_usage(&t, 2, "USDT");
    assert!(usage_before > 0);

    t.remove_spoke_category(2);
    let repay = t.try_repay(ALICE, "USDT", 500.0);
    assert!(
        repay.is_ok(),
        "repay must still work in deprecated category"
    );

    let usage_after = spoke_borrow_usage(&t, 2, "USDT");
    let expected_remaining = usage_before * 3 / 4;
    assert!(
        (usage_after - expected_remaining).abs() <= 1,
        "a 500 USDT partial repay should leave 75% of the original usage: before={usage_before}, after={usage_after}"
    );
}

#[test]
fn test_edit_spoke_supply_cap_below_usage_ratchets_down() {
    let spoke_cap = 1_000 * UNIT;
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.edit_asset_in_spoke_caps("USDC", 2, true, true, 9_700, 9_800, 200, spoke_cap, 0);

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 500.0);

    t.edit_asset_in_spoke_caps("USDC", 2, true, true, 9_700, 9_800, 200, 100 * UNIT, 0);

    assert_contract_error(
        t.try_supply(ALICE, "USDC", 1.0),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );

    t.withdraw(ALICE, "USDC", 450.0);
    assert!(
        t.try_supply(ALICE, "USDC", 10.0).is_ok(),
        "supply must resume once usage drains under the ratcheted cap"
    );
}

#[test]
fn test_spoke_supply_cap_bounds_cumulative_supply() {
    let spoke_cap = 1_000 * UNIT;
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.edit_asset_in_spoke_caps("USDC", 2, true, true, 9_700, 9_800, 200, spoke_cap, 0);

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 500.0);

    t.supply_raw(ALICE, "USDC", 500 * UNIT);

    assert_contract_error(
        t.try_supply(ALICE, "USDC", 1.0),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );
}

#[test]
fn test_edit_spoke_borrow_cap_below_usage_ratchets_down() {
    let spoke_cap = 1_000 * UNIT;
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.edit_asset_in_spoke_caps("USDT", 2, true, true, 9_700, 9_800, 200, 0, spoke_cap);

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 500.0);

    t.edit_asset_in_spoke_caps("USDT", 2, true, true, 9_700, 9_800, 200, 0, 100 * UNIT);

    assert_contract_error(
        t.try_borrow(ALICE, "USDT", 1.0),
        errors::SPOKE_BORROW_CAP_REACHED,
    );

    t.repay(ALICE, "USDT", 450.0);
    assert!(
        t.try_borrow(ALICE, "USDT", 10.0).is_ok(),
        "borrow must resume once usage drains under the ratcheted cap"
    );
}

#[test]
fn test_spoke_spoke_cap_above_from_asset_domain_rejected() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    let usdc = t.resolve_asset("USDC");

    let overflowing_cap = 2_000_000_000_000_000_000_000i128;
    let result = match t.ctrl_client().try_edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: 9_700,
        threshold: 9_800,
        bonus: 200,
        supply_cap: overflowing_cap,
        borrow_cap: 0,
    }) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, errors::INVALID_BORROW_PARAMS);
}

#[test]
fn test_spoke_spoke_supply_cap_headroom_restored_after_withdraw() {
    let spoke_cap = 1_000 * UNIT;
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.edit_asset_in_spoke_caps("USDC", 2, true, true, 9_700, 9_800, 200, spoke_cap, 0);

    t.create_spoke_account(ALICE, 2);

    t.supply(ALICE, "USDC", 1_000.0);
    assert_contract_error(
        t.try_supply(ALICE, "USDC", 1.0),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );

    t.withdraw(ALICE, "USDC", 400.0);
    let res = t.try_supply(ALICE, "USDC", 300.0);
    assert!(
        res.is_ok(),
        "re-supply within restored headroom must execute"
    );
}

#[test]
fn test_spoke_spoke_borrow_cap_tightens_as_interest_accrues() {
    let spoke_cap = 1_000 * UNIT;
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.edit_asset_in_spoke_caps("USDT", 2, true, true, 9_700, 9_800, 200, 0, spoke_cap);

    t.supply(LIQUIDATOR, "USDT", 5_000.0);

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 1_000.0);

    t.advance_time(60 * 60 * 24 * 365);

    assert_contract_error(
        t.try_borrow(ALICE, "USDT", 1.0),
        errors::SPOKE_BORROW_CAP_REACHED,
    );
}

#[test]
fn test_add_asset_to_spoke_rejects_liquidation_fees_above_bps() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .build();

    let res = t.ctrl_client().try_add_asset_to_spoke(&SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset: t.resolve_asset("USDC"),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: 9600,
        threshold: 9700,
        bonus: 200,
        liquidation_fees: 10_001,
        supply_cap: 0,
        borrow_cap: 0,
    });
    match res {
        Err(Ok(err)) => assert_eq!(
            err,
            soroban_sdk::Error::from_contract_error(errors::INVALID_LIQ_THRESHOLD)
        ),
        other => panic!("expected InvalidLiqThreshold, got {other:?}"),
    }
}

#[test]
fn test_edit_asset_in_spoke_rejects_liquidation_fees_above_bps() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    let res = t.ctrl_client().try_edit_asset_in_spoke(&SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset: t.resolve_asset("USDC"),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: 9600,
        threshold: 9700,
        bonus: 200,
        liquidation_fees: 10_001,
        supply_cap: 0,
        borrow_cap: 0,
    });
    match res {
        Err(Ok(err)) => assert_eq!(
            err,
            soroban_sdk::Error::from_contract_error(errors::INVALID_LIQ_THRESHOLD)
        ),
        other => panic!("expected InvalidLiqThreshold, got {other:?}"),
    }
}

fn set_spoke_asset_flags(
    t: &LendingTest,
    spoke_id: u32,
    asset_name: &str,
    paused: bool,
    frozen: bool,
) {
    let config = t.get_asset_config(asset_name);
    t.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset: t.resolve_asset(asset_name),
        spoke_id,
        can_collateral: config.is_collateralizable,
        can_borrow: config.is_borrowable,
        paused,
        frozen,
        no_seize: config.no_seize,
        ltv: config.loan_to_value,
        threshold: config.liquidation_threshold,
        bonus: config.liquidation_bonus,
        liquidation_fees: config.liquidation_fees,
        supply_cap: config.supply_cap,
        borrow_cap: config.borrow_cap,
    });
}

#[test]
fn test_paused_spoke_asset_blocks_supply_and_withdraw() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 1_000.0);

    set_spoke_asset_flags(&t, HARNESS_SPOKE, "USDC", true, false);

    assert_contract_error(t.try_supply(ALICE, "USDC", 1.0), errors::SPOKE_ASSET_PAUSED);
    assert_contract_error(
        t.try_withdraw(ALICE, "USDC", 1.0),
        errors::SPOKE_ASSET_PAUSED,
    );

    set_spoke_asset_flags(&t, HARNESS_SPOKE, "USDC", false, false);
    let supply_before = t.supply_balance_raw(ALICE, "USDC");
    assert!(
        t.try_supply(ALICE, "USDC", 1.0).is_ok(),
        "clearing paused must restore supply capacity"
    );
    assert_eq!(
        t.supply_balance_raw(ALICE, "USDC") - supply_before,
        10_000_000,
        "and the unpaused supply must actually credit the 1 USDC"
    );
}

#[test]
fn test_frozen_spoke_asset_blocks_entries_but_allows_exit() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 1_000.0);

    set_spoke_asset_flags(&t, HARNESS_SPOKE, "USDC", false, true);

    assert_contract_error(t.try_supply(ALICE, "USDC", 1.0), errors::SPOKE_ASSET_FROZEN);
    assert!(
        t.try_withdraw(ALICE, "USDC", 100.0).is_ok(),
        "frozen listing must still allow withdrawal"
    );
    assert_eq!(
        t.supply_balance_raw(ALICE, "USDC"),
        900 * 10_000_000,
        "the frozen-listing withdraw must debit exactly 100 USDC"
    );
}

#[test]
fn test_remove_asset_with_live_supply_usage_reverts_until_drained() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDT", 1_000.0);

    let usdt = t.resolve_asset("USDT");
    let result = t
        .ctrl_client()
        .try_remove_asset_from_spoke(&hub_asset(usdt), &2u32);
    let flat: Result<(), soroban_sdk::Error> = match result {
        Ok(Ok(_)) => panic!("expected contract error, got Ok"),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(flat, errors::SPOKE_ASSET_IN_USE);

    t.withdraw_all(ALICE, "USDT");
    t.remove_asset_from_spoke("USDT", 2);
}

#[test]
fn test_remove_asset_with_live_borrow_usage_reverts() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 2_000.0);

    let usdt = t.resolve_asset("USDT");
    let result = t
        .ctrl_client()
        .try_remove_asset_from_spoke(&hub_asset(usdt), &2u32);
    let flat: Result<(), soroban_sdk::Error> = match result {
        Ok(Ok(_)) => panic!("expected contract error, got Ok"),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(flat, errors::SPOKE_ASSET_IN_USE);
}

#[test]
fn test_update_account_threshold_skips_force_delisted_asset() {
    let mut t = LendingTest::new().stablecoin_spoke_two_asset().build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 5_000.0);
    t.supply(ALICE, "USDT", 5_000.0);
    let account_id = t.resolve_account_id(ALICE);

    let usdt = t.resolve_asset("USDT");
    t.env.as_contract(&t.controller_address(), || {
        t.env
            .storage()
            .persistent()
            .remove(&ControllerKey::SpokeAsset(2, hub_asset(usdt)));
    });

    let result = t.try_update_account_threshold(false, &[account_id]);
    assert!(
        result.is_ok(),
        "threshold sync must skip delisted assets, not revert the batch: {result:?}"
    );
}

#[test]
fn test_update_account_threshold_syncs_deprecated_spoke_listing() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 1_000.0);
    let account_id = t.resolve_account_id(ALICE);

    let stamped_ltv = |t: &LendingTest| -> u32 {
        t.ctrl_client()
            .get_account_positions(&account_id)
            .0
            .get(hub_asset(t.resolve_asset("USDC")))
            .expect("USDC supply position should exist")
            .loan_to_value
    };
    assert_eq!(
        stamped_ltv(&t),
        9_700,
        "precondition: the position carries the spoke's own LTV"
    );

    t.remove_spoke_category(2);

    let usdc = t.resolve_asset("USDC");
    let listing = t
        .ctrl_client()
        .get_spoke_asset(&2u32, &hub_asset(usdc.clone()));
    t.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset: usdc,
        spoke_id: 2,
        can_collateral: listing.is_collateralizable,
        can_borrow: listing.is_borrowable,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: 5_000,
        threshold: listing.liquidation_threshold,
        bonus: listing.liquidation_bonus,
        liquidation_fees: listing.liquidation_fees,
        supply_cap: 0,
        borrow_cap: 0,
    });

    let result = t.try_update_account_threshold(false, &[account_id]);
    assert!(
        result.is_ok(),
        "threshold sync must work on deprecated spokes: {result:?}"
    );

    assert_eq!(
        stamped_ltv(&t),
        5_000,
        "the keeper sync must write the new listing LTV onto a deprecated spoke's positions"
    );
}
