use controller::constants::RAY;
use controller::types::{AccountPositionRaw, ControllerKey, DebtPositionRaw, PositionMode};
use soroban_sdk::{Bytes, Map};
use test_harness::{
    amount_raw, assert_contract_error, errors, hub_asset, usd, usd_cents, HubAssetKey, LendingTest,
    MarketPreset, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, HARNESS_HUB, HARNESS_SPOKE,
};
use test_harness::{eth_preset, usdc_preset, ALICE, BOB, CAROL, LIQUIDATOR};

const SECONDS_PER_YEAR: u64 = 365 * 24 * 60 * 60;

fn borrow_scaled_on_hub(t: &LendingTest, account_id: u64, hub_id: u32, asset_name: &str) -> i128 {
    let asset = t.resolve_asset(asset_name);
    let key = HubAssetKey { hub_id, asset };
    t.env.as_contract(&t.controller_address(), || {
        t.env
            .storage()
            .persistent()
            .get::<_, Map<HubAssetKey, DebtPositionRaw>>(&ControllerKey::BorrowPositions(
                account_id,
            ))
            .and_then(|m| m.get(key))
            .map(|p| p.scaled_amount)
            .unwrap_or(0)
    })
}

fn supply_scaled_on_hub(t: &LendingTest, account_id: u64, hub_id: u32, asset_name: &str) -> i128 {
    let asset = t.resolve_asset(asset_name);
    let key = HubAssetKey { hub_id, asset };
    t.env.as_contract(&t.controller_address(), || {
        t.env
            .storage()
            .persistent()
            .get::<_, Map<HubAssetKey, AccountPositionRaw>>(&ControllerKey::SupplyPositions(
                account_id,
            ))
            .and_then(|m| m.get(key))
            .map(|p| p.scaled_amount)
            .unwrap_or(0)
    })
}

fn usdc_no_seed() -> MarketPreset {
    MarketPreset {
        name: "USDC",
        decimals: 7,
        price_wad: usd(1),
        initial_liquidity: 0.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

#[test]
fn hubs_keep_independent_state_and_indices() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub2 = t.create_hub();
    assert_eq!(
        hub2, 2,
        "the base setup owns hub 1 (HARNESS_HUB); the first test-created hub is 2"
    );

    t.list_market_on_hub(hub2, "USDC", 0.0);

    let a = t.supply_on_hub(HARNESS_HUB, ALICE, "USDC", 1_000.0);
    t.borrow_on_hub(HARNESS_HUB, ALICE, a, "USDC", 500.0);

    let b = t.supply_on_hub(hub2, BOB, "USDC", 1_000.0);
    t.borrow_on_hub(hub2, BOB, b, "USDC", 500.0);

    let s0 = t.pool_state_on_hub(HARNESS_HUB, "USDC");
    let s1 = t.pool_state_on_hub(hub2, "USDC");

    assert!(s0.borrowed > 0 && s1.borrowed > 0);
    assert_eq!(s0.borrow_index, RAY, "hub-1 index starts at RAY");
    assert_eq!(s1.borrow_index, RAY, "hub-2 index starts at RAY");

    t.supply_on_hub(HARNESS_HUB, ALICE, "USDC", 250.0);
    let s1_after_hub1_op = t.pool_state_on_hub(hub2, "USDC");
    assert_eq!(s1_after_hub1_op.supplied, s1.supplied);
    assert_eq!(s1_after_hub1_op.borrowed, s1.borrowed);
    assert_eq!(s1_after_hub1_op.cash, s1.cash);
    assert_eq!(s1_after_hub1_op.supply_index, s1.supply_index);

    t.advance_time(SECONDS_PER_YEAR);
    t.update_indexes_for(&["USDC"]);

    let s0_accrued = t.pool_state_on_hub(HARNESS_HUB, "USDC");
    let s1_idle = t.pool_state_on_hub(hub2, "USDC");
    assert!(
        s0_accrued.borrow_index > RAY,
        "hub-1 borrow index accrued: {}",
        s0_accrued.borrow_index
    );
    assert_eq!(
        s1_idle.borrow_index, RAY,
        "hub-2 borrow index is untouched by hub-1 accrual"
    );

    t.accrue_on_hub(hub2, "USDC");
    let s1_accrued = t.pool_state_on_hub(hub2, "USDC");
    assert!(
        s1_accrued.borrow_index > RAY,
        "hub-2 borrow index accrues independently: {}",
        s1_accrued.borrow_index
    );
}

#[test]
fn bad_debt_is_isolated_to_its_hub() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_market(eth_preset())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub2 = t.create_hub();
    t.list_market_on_hub(hub2, "USDC", 0.0);

    t.supply_on_hub(hub2, CAROL, "USDC", 1_000.0);

    t.supply_on_hub(HARNESS_HUB, BOB, "USDC", 1_000.0);
    let a = t.supply_on_hub(HARNESS_HUB, ALICE, "ETH", 0.002);
    t.borrow_on_hub(HARNESS_HUB, ALICE, a, "USDC", 2.0);

    let si0_before = t.pool_state_on_hub(HARNESS_HUB, "USDC").supply_index;
    let si1_before = t.pool_state_on_hub(hub2, "USDC").supply_index;

    t.set_price("ETH", usd(1));
    t.clean_bad_debt_by_id(a);

    let si0_after = t.pool_state_on_hub(HARNESS_HUB, "USDC").supply_index;
    let si1_after = t.pool_state_on_hub(hub2, "USDC").supply_index;

    assert!(
        si0_after < si0_before,
        "hub-1 USDC supply index is written down by socialized bad debt: {} -> {}",
        si0_before,
        si0_after
    );
    assert_eq!(
        si1_after, si1_before,
        "hub-2 USDC supply index is untouched by hub-1 bad debt"
    );
}

#[test]
fn borrow_cannot_cross_hub_cash() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_market(eth_preset())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub2 = t.create_hub();

    t.list_market_on_hub(hub2, "USDC", 100_000.0);

    t.supply_on_hub(HARNESS_HUB, BOB, "USDC", 100.0);

    let a = t.supply_on_hub(HARNESS_HUB, ALICE, "ETH", 10.0);

    t.borrow_on_hub(HARNESS_HUB, ALICE, a, "USDC", 50.0);

    let attempt_raw = amount_raw(1_000.0, 7);
    let hub1_cash = t.pool_state_on_hub(HARNESS_HUB, "USDC").cash;
    let hub2_cash = t.pool_state_on_hub(hub2, "USDC").cash;
    assert!(
        hub1_cash < attempt_raw && hub2_cash >= attempt_raw,
        "hub 1 holds less than the attempt ({}) while hub 2 holds at least it ({})",
        hub1_cash,
        hub2_cash
    );

    let result = t.try_borrow_on_hub(HARNESS_HUB, ALICE, a, "USDC", 1_000.0);
    assert_contract_error(result, errors::INSUFFICIENT_LIQUIDITY);
}

#[test]
fn swap_debt_refinances_debt_across_hubs() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub2 = t.create_hub();

    t.list_market_on_hub(hub2, "USDC", 100_000.0);

    let account_id = t.supply_on_hub(HARNESS_HUB, ALICE, "USDC", 1_000.0);
    t.borrow_on_hub(HARNESS_HUB, ALICE, account_id, "USDC", 300.0);

    assert!(
        borrow_scaled_on_hub(&t, account_id, HARNESS_HUB, "USDC") > 0,
        "precondition: hub-1 USDC debt exists"
    );
    assert_eq!(
        borrow_scaled_on_hub(&t, account_id, hub2, "USDC"),
        0,
        "precondition: no hub-2 USDC debt yet"
    );

    let usdc = t.resolve_asset("USDC");
    let existing_debt = HubAssetKey {
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
    };
    let new_debt = HubAssetKey {
        hub_id: hub2,
        asset: usdc.clone(),
    };
    let caller = t.get_or_create_user(ALICE);

    let steps = Bytes::new(&t.env);
    let new_debt_raw = amount_raw(305.0, 7);
    t.ctrl_client().swap_debt(
        &caller,
        &account_id,
        &existing_debt,
        &new_debt_raw,
        &new_debt,
        &steps,
    );

    assert_eq!(
        borrow_scaled_on_hub(&t, account_id, HARNESS_HUB, "USDC"),
        0,
        "hub-1 USDC debt is fully repaid by the refinance"
    );
    assert!(
        borrow_scaled_on_hub(&t, account_id, hub2, "USDC") > 0,
        "hub-2 USDC debt now carries the refinanced position"
    );

    assert_eq!(
        t.pool_state_on_hub(HARNESS_HUB, "USDC").borrowed,
        0,
        "hub-1 USDC market has no borrows after the refinance"
    );
    assert!(
        t.pool_state_on_hub(hub2, "USDC").borrowed > 0,
        "hub-2 USDC market holds the refinanced borrow"
    );
}

#[test]
fn liquidation_repays_and_seizes_on_hub_one() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let hub2 = t.create_hub();

    t.list_market_on_hub(hub2, "USDC", 0.0);
    t.list_market_on_hub(hub2, "ETH", 100.0);

    t.supply_on_hub(HARNESS_HUB, BOB, "USDC", 1_000.0);
    let hub1_usdc_before = t.pool_state_on_hub(HARNESS_HUB, "USDC");

    let alice = t.supply_on_hub(hub2, ALICE, "USDC", 10_000.0);
    t.borrow_on_hub(hub2, ALICE, alice, "ETH", 3.0);
    t.assert_healthy(ALICE);

    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    let debt_before = borrow_scaled_on_hub(&t, alice, hub2, "ETH");
    let collateral_before = supply_scaled_on_hub(&t, alice, hub2, "USDC");
    assert!(
        debt_before > 0 && collateral_before > 0,
        "precondition: hub-2 debt and collateral exist"
    );

    t.liquidate_on_hub(hub2, LIQUIDATOR, ALICE, "ETH", 1.0);

    let debt_after = borrow_scaled_on_hub(&t, alice, hub2, "ETH");
    assert!(
        debt_after < debt_before,
        "hub-2 ETH debt must be repaid: {} -> {}",
        debt_before,
        debt_after
    );

    let collateral_after = supply_scaled_on_hub(&t, alice, hub2, "USDC");
    assert!(
        collateral_after < collateral_before,
        "hub-2 USDC collateral must be seized: {} -> {}",
        collateral_before,
        collateral_after
    );
    assert!(
        t.token_balance(LIQUIDATOR, "USDC") > 0.0,
        "liquidator must receive the seized hub-2 USDC collateral"
    );

    let hub1_usdc_after = t.pool_state_on_hub(HARNESS_HUB, "USDC");
    assert_eq!(
        hub1_usdc_after.supplied, hub1_usdc_before.supplied,
        "hub-1 USDC supplied is untouched"
    );
    assert_eq!(
        hub1_usdc_after.borrowed, hub1_usdc_before.borrowed,
        "hub-1 USDC borrowed is untouched"
    );
    assert_eq!(
        hub1_usdc_after.cash, hub1_usdc_before.cash,
        "hub-1 USDC cash is untouched"
    );
    assert_eq!(
        hub1_usdc_after.supply_index, hub1_usdc_before.supply_index,
        "hub-1 USDC supply index is untouched"
    );
    assert_eq!(
        hub1_usdc_after.borrow_index, hub1_usdc_before.borrow_index,
        "hub-1 USDC borrow index is untouched"
    );
}

#[test]
fn liquidation_seizes_hub_one_collateral_without_hub_zero_listing() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let hub2 = t.create_hub();

    t.list_market_on_hub(hub2, "USDC", 0.0);
    t.list_market_on_hub(hub2, "ETH", 100.0);

    let alice = t.supply_on_hub(hub2, ALICE, "USDC", 10_000.0);
    t.borrow_on_hub(hub2, ALICE, alice, "ETH", 3.0);
    t.assert_healthy(ALICE);

    t.ctrl_client()
        .remove_asset_from_spoke(&hub_asset(t.resolve_asset("USDC")), &1u32);

    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    let collateral_before = supply_scaled_on_hub(&t, alice, hub2, "USDC");
    assert!(
        collateral_before > 0,
        "precondition: hub-2 collateral exists"
    );

    t.liquidate_on_hub(hub2, LIQUIDATOR, ALICE, "ETH", 1.0);

    let collateral_after = supply_scaled_on_hub(&t, alice, hub2, "USDC");
    assert!(
        collateral_after < collateral_before,
        "hub-2 collateral must be seized: {} -> {}",
        collateral_before,
        collateral_after
    );
    assert!(
        t.token_balance(LIQUIDATOR, "USDC") > 0.0,
        "liquidator must receive the seized hub-2 collateral"
    );
}

#[test]
fn liquidation_charges_seized_collateral_hub_fee() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("USDC", |c| c.liquidation_fees = 0)
        .build();

    let hub2 = t.create_hub();

    t.list_market_on_hub_with_fees(hub2, "USDC", 0.0, 2_000);
    t.list_market_on_hub(hub2, "ETH", 1_000.0);

    let alice = t.supply_on_hub(hub2, ALICE, "USDC", 10_000.0);
    t.borrow_on_hub(hub2, ALICE, alice, "ETH", 3.0);
    t.assert_healthy(ALICE);

    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    t.liquidate_on_hub(hub2, LIQUIDATOR, ALICE, "ETH", 1.0);

    let hub2_fee = t.claim_revenue_on_hub(hub2, "USDC");
    assert!(
        hub2_fee > 0,
        "hub-2 USDC seizure must accrue the hub-2 fee (20%), not hub-1's 0%: got {}",
        hub2_fee
    );
}

#[test]
fn keeper_and_revenue_serve_hub_one_markets() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_market(eth_preset())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub2 = t.create_hub();
    t.list_market_on_hub(hub2, "USDC", 0.0);
    t.list_market_on_hub(hub2, "ETH", 0.0);

    t.supply_on_hub(hub2, BOB, "USDC", 100_000.0);

    let alice = t.supply_on_hub(hub2, ALICE, "ETH", 10.0);
    t.borrow_on_hub(hub2, ALICE, alice, "USDC", 10_000.0);

    assert_eq!(
        t.pool_state_on_hub(hub2, "USDC").borrow_index,
        RAY,
        "hub-2 USDC index starts at RAY"
    );

    t.advance_time(SECONDS_PER_YEAR);

    t.update_indexes_on_hub(hub2, &["USDC"]);

    assert!(
        t.pool_state_on_hub(hub2, "USDC").borrow_index > RAY,
        "controller update_indexes must accrue the hub-2 USDC index"
    );
    assert_eq!(
        t.pool_state_on_hub(HARNESS_HUB, "USDC").borrow_index,
        RAY,
        "hub-1 USDC index is untouched by a hub-2 keeper update"
    );

    let claimed = t.claim_revenue_on_hub(hub2, "USDC");
    assert!(
        claimed > 0,
        "hub-2 USDC protocol revenue must be claimable through the controller: got {}",
        claimed
    );

    assert_eq!(
        t.claim_revenue_on_hub(HARNESS_HUB, "USDC"),
        0,
        "hub-1 USDC has no revenue to claim"
    );
}

#[test]
fn swap_collateral_migrates_collateral_across_hubs() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub2 = t.create_hub();
    t.list_market_on_hub(hub2, "USDC", 0.0);

    let account_id = t.supply_on_hub(HARNESS_HUB, ALICE, "USDC", 1_000.0);

    assert!(
        supply_scaled_on_hub(&t, account_id, HARNESS_HUB, "USDC") > 0,
        "precondition: hub-1 USDC collateral exists"
    );
    assert_eq!(
        supply_scaled_on_hub(&t, account_id, hub2, "USDC"),
        0,
        "precondition: no hub-2 USDC collateral yet"
    );

    let usdc = t.resolve_asset("USDC");
    let current = HubAssetKey {
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
    };
    let new = HubAssetKey {
        hub_id: hub2,
        asset: usdc.clone(),
    };
    let caller = t.get_or_create_user(ALICE);

    let steps = Bytes::new(&t.env);
    let migrate_amount = amount_raw(1_000.0, 7);
    t.ctrl_client().swap_collateral(
        &caller,
        &account_id,
        &current,
        &migrate_amount,
        &new,
        &steps,
    );

    assert_eq!(
        supply_scaled_on_hub(&t, account_id, HARNESS_HUB, "USDC"),
        0,
        "hub-1 USDC collateral is fully withdrawn by the migration"
    );
    assert!(
        supply_scaled_on_hub(&t, account_id, hub2, "USDC") > 0,
        "hub-2 USDC collateral now carries the migrated position"
    );

    assert_eq!(
        t.pool_state_on_hub(HARNESS_HUB, "USDC").supplied,
        0,
        "hub-1 USDC market has no supply after the migration"
    );
    assert!(
        t.pool_state_on_hub(hub2, "USDC").supplied > 0,
        "hub-2 USDC market holds the migrated supply"
    );
}

#[test]
fn multiply_opens_cross_hub_same_asset_carry_trade() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub2 = t.create_hub();
    t.list_market_on_hub(hub2, "USDC", 0.0);

    t.supply_on_hub(HARNESS_HUB, BOB, "USDC", 10_000.0);

    let usdc = t.resolve_asset("USDC");
    let alice = t.get_or_create_user(ALICE);
    t.resolve_market("USDC")
        .token_admin
        .mint(&alice, &amount_raw(2_000.0, 7));

    let collateral = HubAssetKey {
        hub_id: hub2,
        asset: usdc.clone(),
    };
    let debt = HubAssetKey {
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
    };

    let steps = Bytes::new(&t.env);
    let debt_to_flash_loan = amount_raw(100.0, 7);
    let account_id = t.ctrl_client().multiply(
        &alice,
        &0u64,
        &HARNESS_SPOKE,
        &collateral,
        &debt_to_flash_loan,
        &debt,
        &PositionMode::Multiply,
        &steps,
        &Some((collateral.clone(), amount_raw(2_000.0, 7))),
        &None,
    );

    assert!(
        borrow_scaled_on_hub(&t, account_id, HARNESS_HUB, "USDC") > 0,
        "hub-1 USDC carries the flash-borrowed debt leg"
    );
    assert!(
        supply_scaled_on_hub(&t, account_id, hub2, "USDC") > 0,
        "hub-2 USDC carries the collateral leg"
    );

    let same_hub_debt = HubAssetKey {
        hub_id: hub2,
        asset: usdc,
    };
    let steps_again = Bytes::new(&t.env);
    let result = match t.ctrl_client().try_multiply(
        &alice,
        &0u64,
        &HARNESS_SPOKE,
        &collateral,
        &debt_to_flash_loan,
        &same_hub_debt,
        &PositionMode::Multiply,
        &steps_again,
        &None,
        &None,
    ) {
        Ok(Ok(id)) => Ok(id),
        Ok(Err(e)) => Err(e),
        Err(invoke) => Err(invoke.expect("expected contract error, got host-level InvokeError")),
    };
    assert_contract_error(result, errors::ASSETS_ARE_THE_SAME);
}
