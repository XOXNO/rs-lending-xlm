use controller::types::SpokeAssetArgs;
use test_harness::{
    assert_contract_error, errors, eth_preset, f64_to_i128, hub_asset, usd, usd_cents, usdc_preset,
    usdt_stable_preset, LendingTest, PositionType, ALICE, HARNESS_HUB, HARNESS_SPOKE, LIQUIDATOR,
    STABLECOIN_SPOKE,
};
// 1. test_spoke_category_creation

#[test]
fn test_spoke_category_creation() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    // The build created the category. Verify by creating an spoke account;
    // a missing category would fail.
    let mut t = t;
    let account_id = t.create_spoke_account(ALICE, 2);
    assert!(account_id > 0, "should create spoke account");
    let attrs = t.get_account_attributes(ALICE);
    assert_eq!(attrs.spoke_id, 2, "account should be in spoke category 1");
}
// 2. test_spoke_enhanced_ltv_and_threshold

#[test]
fn test_spoke_enhanced_ltv_and_threshold() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    // Spoke LTV = 97%, threshold = 98%.
    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);

    // Borrow 95% = $9500 USDT. Standard 75% LTV blocks this; spoke 97% allows it.
    t.borrow(ALICE, "USDT", 9_500.0);
    t.assert_healthy(ALICE);

    let hf = t.health_factor(ALICE);
    assert!(
        (1.0..1.10).contains(&hf),
        "spoke should allow tight but healthy position, HF={}",
        hf
    );
}
// 3. test_spoke_supply_with_category_asset

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
// 4. test_spoke_borrow_with_category_asset

#[test]
fn test_spoke_borrow_with_category_asset() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

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
// 5. test_spoke_rejects_non_category_supply

#[test]
fn test_spoke_rejects_non_category_supply() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset()) // ETH is not in the spoke category.
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);

    // Supplying ETH must fail because ETH is outside the spoke category.
    let result = t.try_supply(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::ASSET_NOT_IN_SPOKE);
}
// 6. test_spoke_rejects_non_category_borrow

#[test]
fn test_spoke_rejects_non_category_borrow() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);

    // Borrowing ETH must fail because ETH is outside the spoke category.
    let result = t.try_borrow(ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::ASSET_NOT_IN_SPOKE);
}
// 7. test_spoke_deprecated_blocks_new_accounts

#[test]
fn test_spoke_deprecated_blocks_new_accounts() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    // Create the spoke account BEFORE deprecation so the harness's local
    // deprecation assert does not short-circuit. The contract path under
    // test is the one that supplies under a deprecated category, which
    // routes through `active_spoke_category` -> `ensure_spoke_not_deprecated`.
    t.create_spoke_account(ALICE, 2);

    // Deprecate the spoke category.
    t.remove_spoke_category(2);

    // Supplying under the deprecated category must reject with the
    // contract error SpokeDeprecated (301).
    let result = t.try_supply(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::SPOKE_DEPRECATED);
}
// 9. test_spoke_edit_asset_params

#[test]
fn test_spoke_edit_asset_params() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    // Edit the collateral asset to lower its spoke LTV from 97% to 80%.
    t.edit_asset_in_spoke("USDC", 2, true, true, 8000, 8500, 300);

    // Now create the account and borrow at 95%; the new 80% LTV must reject.
    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);

    let result = t.try_borrow(ALICE, "USDT", 9_500.0);
    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}
// 10. test_spoke_remove_category_deprecates

#[test]
fn test_spoke_remove_category_deprecates() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    // Create the spoke account before deprecation; the harness's local
    // deprecation assert blocks creation under a deprecated category.
    t.create_spoke_account(ALICE, 2);

    t.remove_spoke_category(2);

    // Confirm deprecation via the contract path: supply must reject with
    // SpokeDeprecated (301).
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
// 11. test_spoke_add_asset_to_category

#[test]
fn test_spoke_add_asset_to_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        // USDT not yet in the category.
        .build();

    // Add USDT to the category at runtime with the stablecoin spoke params.
    t.add_asset_to_spoke("USDT", 2, true, true, 9700, 9800, 200);

    // USDT is valid collateral and debt in the spoke category.
    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);
    t.assert_healthy(ALICE);
}
// 12. test_spoke_remove_asset_from_category

#[test]
fn test_spoke_remove_asset_from_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    // Remove USDT from the category.
    t.remove_asset_from_spoke("USDT", 2);

    // Borrowing USDT in spoke must fail after category removal.
    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);

    let result = t.try_borrow(ALICE, "USDT", 5_000.0);
    assert_contract_error(result, errors::ASSET_NOT_IN_SPOKE);
}
// 13. test_spoke_liquidation_uses_spoke_bonus

#[test]
fn test_spoke_liquidation_uses_spoke_bonus() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    // Spoke bonus = 2% (200 BPS), far below the standard 5%.
    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 9_500.0);

    // Drop USDC price to force clear liquidation.
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

    // The liquidator must receive collateral with the 2% spoke bonus.
    let usdc_received = t.token_balance(LIQUIDATOR, "USDC");
    assert!(usdc_received > 0.0, "liquidator should receive collateral");

    // The value ratio must hover near 1.02 (2% spoke bonus), not 1.05
    // (standard). USDC trades at $0.90, so usdc_value = usdc_received * 0.90.
    let usdc_value = usdc_received * 0.90;
    let debt_value = 2_000.0; // USDT at $1.

    if usdc_value > 0.0 {
        let ratio = usdc_value / debt_value;
        // Spoke bonus is 2%, so the ratio must sit near 1.02 (between 1.015
        // and 1.04). A one-sided `< 1.06` check would also pass under the
        // standard 5% bonus.
        assert!(
            ratio > 1.015 && ratio < 1.04,
            "spoke bonus should be ~1.02 (not zero, not 5%): ratio={}",
            ratio
        );
    }
}
// 14. test_spoke_two_assets_same_category

#[test]
fn test_spoke_two_assets_same_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);

    // Supply both stablecoins.
    t.supply(ALICE, "USDC", 5_000.0);
    t.supply(ALICE, "USDT", 5_000.0);

    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
    t.assert_position_exists(ALICE, "USDT", PositionType::Supply);
    t.assert_supply_near(ALICE, "USDC", 5_000.0, 1.0);
    t.assert_supply_near(ALICE, "USDT", 5_000.0, 1.0);

    // Borrow USDC against USDT collateral and vice versa.
    t.borrow(ALICE, "USDC", 2_000.0);
    t.assert_position_exists(ALICE, "USDC", PositionType::Borrow);
    t.assert_borrow_near(ALICE, "USDC", 2_000.0, 1.0);
    t.assert_healthy(ALICE);
}
// 16. test_spoke_deprecated_category_operations_rejected

#[test]
fn test_spoke_deprecated_category_operations_rejected() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    // Deprecate the category first.
    t.remove_spoke_category(2);

    // 1. Trying to remove/deprecate the category again must fail.
    let remove_result = t.ctrl_client().try_remove_spoke(&2u32);
    let flat_remove: Result<(), soroban_sdk::Error> = match remove_result {
        Ok(Ok(_)) => panic!("expected contract error, got Ok"),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(flat_remove, errors::SPOKE_DEPRECATED);

    // 2. Trying to edit an asset in the deprecated category must fail.
    let asset_address = t.resolve_asset("USDC");
    let edit_asset_result = t.ctrl_client().try_edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: asset_address.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        ltv: 9_000,
        threshold: 9_300,
        bonus: 200,
        supply_cap: 0,
        borrow_cap: 0,
    });
    let flat_edit_asset: Result<(), soroban_sdk::Error> = match edit_asset_result {
        Ok(Ok(_)) => panic!("expected contract error, got Ok"),
        Ok(Err(err)) => Err(err.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(flat_edit_asset, errors::SPOKE_DEPRECATED);
}

// Regression: passing a non-zero `spoke_id` argument to supply on an
// EXISTING account must panic if it disagrees with the account's stored
// category. Without this guard the argument was silently ignored — the caller
// believes they are operating in one mode while the account is in another.
// Zero remains the "unspecified" sentinel (kept for harness convention) and
// does not trigger the guard.
#[test]
fn test_supply_rejects_spoke_mismatch_on_existing_account() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    // Alice opens a normal (non-spoke) account via her first supply.
    t.supply(ALICE, "USDC", 50.0);

    // Now she calls supply on the same account with spoke = 1. The account
    // is in spoke = 0; the call must reject.
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

    // Alice opens an spoke 1 account.
    let _ = t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 50.0);

    // Re-supply with a DIFFERENT non-zero category must reject.
    let result = t.try_supply_with_spoke(ALICE, "USDC", 10.0, 3);
    assert_contract_error(result, errors::SPOKE_MISMATCH);
}

// The spoke arg must match the account's stored spoke: `0` is no longer an
// "unspecified" sentinel (there is no spoke 0), so supplying with `0` on an
// spoke account is rejected like any other mismatch.
#[test]
fn test_supply_zero_spoke_rejects_mismatch_against_active_category() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    let _ = t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 50.0);

    // Caller passes 0; the account is in spoke=2. With no `0` sentinel the
    // strict spoke match rejects the call.
    let result = t.try_supply_with_spoke(ALICE, "USDC", 10.0, 0);
    assert_contract_error(result, errors::SPOKE_MISMATCH);
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
    assert!(
        t.supply_balance(ALICE, "USDC") < 901.0,
        "partial withdraw should reduce the existing collateral position"
    );
}

#[test]
fn test_deprecated_spoke_repay_allowed_but_new_borrow_blocked() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 2_000.0);
    t.remove_spoke_category(2);

    let borrow_more = t.try_borrow(ALICE, "USDT", 1.0);
    assert_contract_error(borrow_more, errors::SPOKE_DEPRECATED);

    let debt_before = t.borrow_balance(ALICE, "USDT");
    let repay = t.try_repay(ALICE, "USDT", 500.0);
    assert!(
        repay.is_ok(),
        "deprecated spoke must not block debt-reducing repay; got {repay:?}"
    );
    assert!(
        t.borrow_balance(ALICE, "USDT") < debt_before,
        "repay should reduce the existing debt position"
    );
}

#[test]
fn test_deprecated_spoke_with_debt_keeps_stored_params_on_withdraw() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);
    t.remove_spoke_category(2);

    // This withdrawal would fail under base USDC LTV (6000 * 75% < 5000 debt),
    // but it is safe under the spoke LTV snapshot stored on the position.
    let result = t.try_withdraw(ALICE, "USDC", 4_000.0);
    assert!(
        result.is_ok(),
        "deprecated spoke must keep stored position params on safe withdrawals; got {result:?}"
    );
    t.assert_healthy(ALICE);
}

#[test]
fn test_deprecated_spoke_with_debt_withdraw_still_enforces_stored_spoke_ltv() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);
    t.remove_spoke_category(2);

    // Even with the frozen spoke LTV snapshot, leaving only 5000 USDC cannot
    // support 5000 USDT debt (5000 * 97% < 5000).
    let result = t.try_withdraw(ALICE, "USDC", 5_000.0);
    assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);
}

#[test]
fn test_deprecated_spoke_category_still_allows_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 9_500.0);
    t.remove_spoke_category(2);
    t.set_price("USDC", usd_cents(85));
    t.assert_liquidatable(ALICE);

    let debt_before = t.borrow_balance(ALICE, "USDT");
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "USDT", 500.0);
    assert!(
        result.is_ok(),
        "deprecated spoke must not block liquidation; got {result:?}"
    );
    assert!(
        t.borrow_balance(ALICE, "USDT") < debt_before,
        "liquidation should reduce debt"
    );
}

#[test]
fn test_deprecated_spoke_views_block_new_borrow_but_preserve_exit_preview() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 2_000.0);
    let account_id = t.resolve_account_id(ALICE);
    let usdc = t.resolve_asset("USDC");
    let usdt = t.resolve_asset("USDT");

    t.remove_spoke_category(2);

    assert_eq!(
        t.ctrl_client()
            .max_borrow(&account_id, &hub_asset(usdt.clone())),
        0,
        "deprecated spoke must preview no additional borrow capacity"
    );
    assert!(
        t.ctrl_client()
            .max_withdraw(&account_id, &hub_asset(usdc.clone()))
            >= f64_to_i128(4_000.0, t.resolve_market("USDC").decimals),
        "deprecated spoke must preview exits using the stored position params, not base fallback"
    );
}

#[test]
fn test_removed_spoke_collateral_asset_blocks_new_supply_but_existing_withdraw_works() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 5_000.0);
    t.remove_asset_from_spoke("USDC", 2);

    let add_more = t.try_supply(ALICE, "USDC", 1.0);
    assert_contract_error(add_more, errors::ASSET_NOT_IN_SPOKE);

    // Removing the asset from the category must block new supply, but the
    // existing collateral position keeps its spoke snapshot.
    let withdraw = t.try_withdraw(ALICE, "USDC", 4_000.0);
    assert!(
        withdraw.is_ok(),
        "removed collateral asset must still allow safe withdrawal of an existing position; got {withdraw:?}"
    );
    t.assert_healthy(ALICE);
}

#[test]
fn test_removed_spoke_debt_asset_blocks_new_borrow_but_existing_repay_works() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 2_000.0);
    t.remove_asset_from_spoke("USDT", 2);

    let borrow_more = t.try_borrow(ALICE, "USDT", 1.0);
    assert_contract_error(borrow_more, errors::ASSET_NOT_IN_SPOKE);

    let debt_before = t.borrow_balance(ALICE, "USDT");
    let repay = t.try_repay(ALICE, "USDT", 500.0);
    assert!(
        repay.is_ok(),
        "removed debt asset must still allow debt-reducing repay; got {repay:?}"
    );
    assert!(t.borrow_balance(ALICE, "USDT") < debt_before);
}

// Liquidation must not depend on the live spoke listing: removing a
// collateral asset from the account's spoke keeps the position seizable on
// its snapshotted risk, with the protocol's liquidation fee falling back to
// zero for the delisted asset (mirroring withdraw's frozen-params policy).
#[test]
fn test_removed_spoke_collateral_asset_stays_liquidatable() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 9_500.0);
    t.remove_asset_from_spoke("USDC", 2);
    t.set_price("USDC", usd_cents(85));
    // Snapshotted position risk still marks the account underwater.
    t.assert_liquidatable(ALICE);

    let debt_before = t.borrow_balance(ALICE, "USDT");
    let result = t.try_liquidate(LIQUIDATOR, ALICE, "USDT", 500.0);
    assert!(
        result.is_ok(),
        "delisted collateral must stay seizable; got {result:?}"
    );
    assert!(
        t.borrow_balance(ALICE, "USDT") < debt_before,
        "liquidation must reduce the debt against delisted collateral"
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
}

#[test]
fn test_spoke_borrow_flag_update_blocks_new_borrow_but_existing_repay_works() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 2_000.0);
    t.edit_asset_in_spoke("USDT", 2, true, false, 9700, 9800, 200);

    let borrow_more = t.try_borrow(ALICE, "USDT", 1.0);
    assert_contract_error(borrow_more, errors::ASSET_NOT_BORROWABLE);

    let debt_before = t.borrow_balance(ALICE, "USDT");
    let repay = t.try_repay(ALICE, "USDT", 500.0);
    assert!(
        repay.is_ok(),
        "borrow flag removal must not block repaying an existing debt; got {repay:?}"
    );
    assert!(t.borrow_balance(ALICE, "USDT") < debt_before);
}

// Defense-in-depth (AAVE-D-028): the controller's own edit_asset_in_spoke
// rejects an edit that would invert the LTV<threshold gap or breach the
// seizure ceiling, even on a direct call that bypasses the governance
// contract's validation. A collateral position inherits its ltv and threshold
// from the asset's spoke config (apply_spoke_to_asset_config), so an asset
// that can never hold ltv >= threshold means no member position can either.
#[test]
fn test_edit_asset_in_spoke_rejects_inverted_or_unsafe_bounds() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();
    let usdc = t.resolve_asset("USDC");

    // ltv >= threshold must reject (the borrow-buffer invariant).
    let inverted = t.ctrl_client().try_edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
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

    // Gap preserved (9_500 > 9_400) but threshold*(1+bonus) > 100% must still
    // reject: 9_500 * (10_000 + 600) = 1.007e8 > 1e8.
    let unsafe_bonus = t.ctrl_client().try_edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
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

    // A valid edit still succeeds and the stored asset keeps threshold > ltv.
    t.edit_asset_in_spoke("USDC", 2, true, true, 9_000, 9_300, 200);
    let cfg = t
        .ctrl_client()
        .get_spoke_asset(&2u32, &hub_asset(usdc.clone()));
    assert_eq!(cfg.loan_to_value, 9_000);
    assert_eq!(cfg.liquidation_threshold, 9_300);
    assert!(cfg.liquidation_threshold > cfg.loan_to_value);
}

// Per-asset divergence: two assets in the SAME category carry DIFFERENT risk
// params. Each supplied collateral position inherits its own asset's spoke
// LTV/threshold, proving params are no longer category-wide.
#[test]
fn test_spoke_per_asset_divergent_params() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE) // USDC inherits 9700/9800/200.
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    // USDT joins the same category with a tighter, distinct risk profile.
    t.add_asset_to_spoke("USDT", 2, true, true, 9_000, 9_300, 300);

    let account_id = t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 5_000.0);
    t.supply(ALICE, "USDT", 5_000.0);

    let usdc = t.resolve_asset("USDC");
    let usdt = t.resolve_asset("USDT");
    let (supplies, _) = t.ctrl_client().get_account_positions(&account_id);

    // USDC position keeps the stablecoin-category params.
    let usdc_pos = supplies.get(hub_asset(usdc)).expect("USDC position");
    assert_eq!(
        usdc_pos.loan_to_value, 9_700,
        "USDC keeps its 97% spoke LTV"
    );
    assert_eq!(usdc_pos.liquidation_threshold, 9_800);

    // USDT position carries its own, divergent params in the same category.
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

    let usdc = t.resolve_asset("USDC");
    t.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        ltv: 9_700,
        threshold: 9_800,
        bonus: 200,
        supply_cap: spoke_cap,
        borrow_cap: 0,
    });

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 500.0);

    let result = t.try_supply(ALICE, "USDC", 600.0);
    assert_contract_error(result, errors::SPOKE_SUPPLY_CAP_REACHED);
}

#[test]
fn test_spoke_borrow_cap_enforced() {
    let spoke_borrow_cap = 500 * UNIT;

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    let usdt = t.resolve_asset("USDT");
    t.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: usdt.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        ltv: 9_700,
        threshold: 9_800,
        bonus: 200,
        supply_cap: 0,
        borrow_cap: spoke_borrow_cap,
    });

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 400.0);

    let result = t.try_borrow(ALICE, "USDT", 200.0);
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

    t.remove_asset_from_spoke("USDC", 2);
    let withdraw = t.try_withdraw(ALICE, "USDC", 400.0);
    assert!(
        withdraw.is_ok(),
        "withdraw must still work after asset removal"
    );

    let usage_after = spoke_supply_usage(&t, 2, "USDC");
    assert!(
        usage_after < usage_before,
        "withdraw must decrement usage even when asset left the category"
    );
}

#[test]
fn test_deprecated_spoke_repay_decrements_usage() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

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
    assert!(
        usage_after < usage_before,
        "repay must decrement usage even when category is deprecated"
    );
}

#[test]
fn test_edit_spoke_rejects_supply_cap_below_usage() {
    let spoke_cap = 1_000 * UNIT;
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    let usdc = t.resolve_asset("USDC");
    t.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        ltv: 9_700,
        threshold: 9_800,
        bonus: 200,
        supply_cap: spoke_cap,
        borrow_cap: 0,
    });

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 500.0);

    let result = match t.ctrl_client().try_edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        ltv: 9_700,
        threshold: 9_800,
        bonus: 200,
        supply_cap: 100 * UNIT,
        borrow_cap: 0,
    }) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, errors::SPOKE_CAP_BELOW_USAGE);
}

#[test]
fn test_max_supply_respects_spoke_cap_headroom() {
    let spoke_cap = 1_000 * UNIT;
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    let usdc = t.resolve_asset("USDC");
    t.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        ltv: 9_700,
        threshold: 9_800,
        bonus: 200,
        supply_cap: spoke_cap,
        borrow_cap: 0,
    });

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 500.0);

    let account_id = t.resolve_account_id(ALICE);
    let headroom = t
        .ctrl_client()
        .max_supply(&account_id, &hub_asset(usdc.clone()));
    assert!(
        headroom > 400 * UNIT && headroom <= 500 * UNIT,
        "spoke headroom should be ~500 USDC, got {headroom}"
    );

    t.supply_raw(ALICE, "USDC", headroom);
    assert_eq!(
        t.ctrl_client()
            .max_supply(&account_id, &hub_asset(usdc.clone())),
        0
    );
}

// Borrow-side twin of `test_edit_spoke_rejects_supply_cap_below_usage`: editing
// the spoke borrow cap below the category's current borrow usage must be
// rejected (the borrow branch of `validate_spoke_caps_against_usage`).
#[test]
fn test_edit_spoke_rejects_borrow_cap_below_usage() {
    let spoke_cap = 1_000 * UNIT;
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    let usdt = t.resolve_asset("USDT");
    t.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: usdt.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        ltv: 9_700,
        threshold: 9_800,
        bonus: 200,
        supply_cap: 0,
        borrow_cap: spoke_cap,
    });

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 500.0); // ~500 USDT of borrow usage

    let result = match t.ctrl_client().try_edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: usdt.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        ltv: 9_700,
        threshold: 9_800,
        bonus: 200,
        supply_cap: 0,
        borrow_cap: 100 * UNIT, // spoke borrow cap below the ~500 current usage
    }) {
        Ok(res) => res.map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, errors::SPOKE_CAP_BELOW_USAGE);
}

// Integration of the from_asset-domain guard on the spoke path: a spoke cap far
// above the `Ray::from_asset` domain would overflow in a cap preview, so
// `require_cap_within_asset_domain` must reject it at config time. Hub caps are
// disabled so the spoke<=hub check is skipped and the domain guard is binding.
#[test]
fn test_spoke_spoke_cap_above_from_asset_domain_rejected() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    let usdc = t.resolve_asset("USDC");
    // At 7 decimals the ceiling is ~i128::MAX / 10^20 (~1.7e18); 2e21 overflows.
    let overflowing_cap = 2_000_000_000_000_000_000_000i128;
    let result = match t.ctrl_client().try_edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
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

// Round-trip: filling the spoke supply cap collapses headroom and blocks new
// supply; withdrawing decrements usage and restores headroom for a re-supply.
#[test]
fn test_spoke_spoke_supply_cap_headroom_restored_after_withdraw() {
    let spoke_cap = 1_000 * UNIT;
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    let usdc = t.resolve_asset("USDC");
    t.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        ltv: 9_700,
        threshold: 9_800,
        bonus: 200,
        supply_cap: spoke_cap,
        borrow_cap: 0,
    });

    t.create_spoke_account(ALICE, 2);
    let account_id = t.resolve_account_id(ALICE);

    // Fill to the spoke cap: headroom collapses and one more unit reverts.
    t.supply(ALICE, "USDC", 1_000.0);
    assert!(
        t.ctrl_client()
            .max_supply(&account_id, &hub_asset(usdc.clone()))
            <= 1,
        "headroom must collapse at the spoke cap"
    );
    assert_contract_error(
        t.try_supply(ALICE, "USDC", 1.0),
        errors::SPOKE_SUPPLY_CAP_REACHED,
    );

    // Withdraw frees usage; headroom is restored and a re-supply executes.
    t.withdraw(ALICE, "USDC", 400.0);
    let restored = t
        .ctrl_client()
        .max_supply(&account_id, &hub_asset(usdc.clone()));
    assert!(
        restored > 390 * UNIT && restored <= 400 * UNIT,
        "headroom should restore to ~400 USDC after withdraw, got {restored}"
    );
    let res = t.try_supply(ALICE, "USDC", 300.0);
    assert!(
        res.is_ok(),
        "re-supply within restored headroom must execute"
    );
}

// The spoke cap is fixed in asset units while debt accrues interest, so a
// position borrowed up to the cap drifts past it as the index grows: a later
// borrow must revert on the spoke cap even though scaled usage is unchanged.
#[test]
fn test_spoke_spoke_borrow_cap_tightens_as_interest_accrues() {
    let spoke_cap = 1_000 * UNIT;
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .with_spoke_asset(2, "USDT", true, true)
        .build();

    let usdt = t.resolve_asset("USDT");
    t.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
        liquidation_fees: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
        hub_id: HARNESS_HUB,
        asset: usdt.clone(),
        spoke_id: 2,
        can_collateral: true,
        can_borrow: true,
        paused: false,
        frozen: false,
        ltv: 9_700,
        threshold: 9_800,
        bonus: 200,
        supply_cap: 0,
        borrow_cap: spoke_cap,
    });

    // A non-spoke USDT supplier so utilization is defined and interest accrues.
    t.supply(LIQUIDATOR, "USDT", 5_000.0);

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 1_000.0); // borrow up to the spoke cap

    let account_id = t.resolve_account_id(ALICE);
    assert!(
        t.ctrl_client()
            .max_borrow(&account_id, &hub_asset(usdt.clone()))
            <= 1,
        "headroom must be ~0 right at the cap"
    );

    t.advance_time(60 * 60 * 24 * 365);

    assert_eq!(
        t.ctrl_client()
            .max_borrow(&account_id, &hub_asset(usdt.clone())),
        0,
        "accrued debt must push the spoke position past the fixed spoke cap"
    );
    assert_contract_error(
        t.try_borrow(ALICE, "USDT", 1.0),
        errors::SPOKE_BORROW_CAP_REACHED,
    );
}

/// A per-spoke `oracle_override` reprices an asset for accounts on that spoke
/// without touching the asset's token-rooted base price (Phase 3 wiring): the
/// override config flows through `edit_asset_in_spoke` into storage, and the
/// account valuation path consults it.
#[test]
fn test_spoke_oracle_override_reprices_collateral() {
    let mut t = LendingTest::new().with_market(eth_preset()).build();

    // eth_preset prices ETH at $2000. Supply 1 ETH on the base spoke.
    t.supply(ALICE, "ETH", 1.0);
    let collateral_base = t.total_collateral_raw(ALICE);
    assert!(collateral_base > 0, "supplied collateral must be valued");

    // Point ETH at a per-spoke override priced at $4000 (2x the base).
    t.set_spoke_oracle_override("ETH", HARNESS_SPOKE, usd(4000));

    let collateral_override = t.total_collateral_raw(ALICE);

    // The spoke's view of ETH doubled while the token-rooted base is unchanged,
    // so the account's collateral USD doubles.
    let ratio = collateral_override as f64 / collateral_base as f64;
    assert!(
        (ratio - 2.0).abs() < 0.01,
        "per-spoke override should reprice ETH ~2x: base={collateral_base} override={collateral_override} ratio={ratio}"
    );
}

// Regression: liquidation_fees is a BPS ratio applied to the seized-collateral
// bonus; values above 100% must be rejected at listing time, not stored to
// break liquidation planning later.
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
        ltv: 9600,
        threshold: 9700,
        bonus: 200,
        liquidation_fees: 10_001,
        supply_cap: 0,
        borrow_cap: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
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
        ltv: 9600,
        threshold: 9700,
        bonus: 200,
        liquidation_fees: 10_001,
        supply_cap: 0,
        borrow_cap: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
    });
    match res {
        Err(Ok(err)) => assert_eq!(
            err,
            soroban_sdk::Error::from_contract_error(errors::INVALID_LIQ_THRESHOLD)
        ),
        other => panic!("expected InvalidLiqThreshold, got {other:?}"),
    }
}

// Regression: a per-spoke oracle override whose `asset_decimals` diverge from
// the pool market's decimals would mis-scale every valuation on the spoke by
// powers of ten; the listing must reject the mismatch.
#[test]
fn test_spoke_oracle_override_rejects_mismatched_decimals() {
    let t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    let pool_decimals = t.resolve_market("USDC").decimals;
    let res = t.try_set_spoke_oracle_override_with_decimals("USDC", 2, usd(1), pool_decimals + 1);
    assert_contract_error(res, errors::INVALID_ASSET);

    t.try_set_spoke_oracle_override_with_decimals("USDC", 2, usd(1), pool_decimals)
        .expect("override with matching decimals must be accepted");
}

// Regression: `max_supply` must preview zero on a deprecated spoke because the
// mutating supply path rejects with `SpokeDeprecated`.
#[test]
fn test_deprecated_spoke_max_supply_returns_zero() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();

    t.create_spoke_account(ALICE, 2);
    t.supply(ALICE, "USDC", 100.0);
    let account_id = t.resolve_account_id(ALICE);
    let usdc = t.resolve_asset("USDC");

    assert!(
        t.ctrl_client()
            .max_supply(&account_id, &hub_asset(usdc.clone()))
            > 0,
        "active spoke must preview supply headroom"
    );

    t.remove_spoke_category(2);
    assert_eq!(
        t.ctrl_client().max_supply(&account_id, &hub_asset(usdc)),
        0,
        "deprecated spoke must preview zero supply capacity"
    );
}

/// Sets the per-listing paused/frozen incident flags through the real
/// `edit_asset_in_spoke` entrypoint, preserving the listing's current risk
/// params.
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
        ltv: config.loan_to_value,
        threshold: config.liquidation_threshold,
        bonus: config.liquidation_bonus,
        liquidation_fees: config.liquidation_fees,
        supply_cap: 0,
        borrow_cap: 0,
        oracle_override: controller::types::MarketOracleConfigOption::None,
    });
}

// Regression: `max_supply`/`max_borrow` must preview zero for a paused spoke
// asset because `enforce_spoke_asset_flags` rejects the mutating paths.
#[test]
fn test_paused_spoke_asset_zeroes_supply_and_borrow_previews() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 1_000.0);
    let account_id = t.resolve_account_id(ALICE);
    let usdc = t.resolve_asset("USDC");

    assert!(
        t.ctrl_client()
            .max_supply(&account_id, &hub_asset(usdc.clone()))
            > 0
    );
    assert!(
        t.ctrl_client()
            .max_borrow(&account_id, &hub_asset(usdc.clone()))
            > 0
    );

    set_spoke_asset_flags(&t, HARNESS_SPOKE, "USDC", true, false);

    assert_eq!(
        t.ctrl_client()
            .max_supply(&account_id, &hub_asset(usdc.clone())),
        0,
        "paused listing must preview zero supply capacity"
    );
    assert_eq!(
        t.ctrl_client()
            .max_borrow(&account_id, &hub_asset(usdc.clone())),
        0,
        "paused listing must preview zero borrow capacity"
    );

    // The mutating paths agree with the previews: paused blocks everything.
    assert_contract_error(t.try_supply(ALICE, "USDC", 1.0), errors::SPOKE_ASSET_PAUSED);
    assert_contract_error(
        t.try_withdraw(ALICE, "USDC", 1.0),
        errors::SPOKE_ASSET_PAUSED,
    );

    // Clearing the flag through the same edit restores capacity.
    set_spoke_asset_flags(&t, HARNESS_SPOKE, "USDC", false, false);
    assert!(
        t.ctrl_client().max_supply(&account_id, &hub_asset(usdc)) > 0,
        "clearing paused must restore supply capacity"
    );
}

// Regression: frozen blocks risk-increasing flows (supply/borrow) while still
// allowing exits, so both previews must report zero.
#[test]
fn test_frozen_spoke_asset_zeroes_supply_and_borrow_previews() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.supply(ALICE, "USDC", 1_000.0);
    let account_id = t.resolve_account_id(ALICE);
    let usdc = t.resolve_asset("USDC");

    set_spoke_asset_flags(&t, HARNESS_SPOKE, "USDC", false, true);

    assert_eq!(
        t.ctrl_client()
            .max_supply(&account_id, &hub_asset(usdc.clone())),
        0,
        "frozen listing must preview zero supply capacity"
    );
    assert_eq!(
        t.ctrl_client().max_borrow(&account_id, &hub_asset(usdc)),
        0,
        "frozen listing must preview zero borrow capacity"
    );

    // Frozen blocks risk-increasing entries but keeps exits open.
    assert_contract_error(t.try_supply(ALICE, "USDC", 1.0), errors::SPOKE_ASSET_FROZEN);
    assert!(
        t.try_withdraw(ALICE, "USDC", 100.0).is_ok(),
        "frozen listing must still allow withdrawal"
    );
}
