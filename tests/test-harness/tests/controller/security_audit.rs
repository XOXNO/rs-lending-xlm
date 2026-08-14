use controller::types::ControllerKey;
use soroban_sdk::testutils::Ledger as _;
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, usdc_preset, usdt_stable_preset,
    wbtc_preset, HubAssetKey, LendingTest, PositionType, ALICE, BOB, LIQUIDATOR,
};

fn supply_risk_stamp(t: &LendingTest, account_id: u64, asset_name: &str) -> (u32, u32, u32, u32) {
    let asset = t.resolve_asset(asset_name);
    t.env.as_contract(&t.controller_address(), || {
        let map: soroban_sdk::Map<HubAssetKey, controller::types::AccountPositionRaw> = t
            .env
            .storage()
            .persistent()
            .get(&ControllerKey::SupplyPositions(account_id))
            .expect("supply side map should exist");
        let p = map
            .get(hub_asset(asset))
            .expect("supply position should exist for asset");
        (
            p.loan_to_value,
            p.liquidation_threshold,
            p.liquidation_bonus,
            p.liquidation_fees,
        )
    })
}

fn supply_ltv_and_lt(t: &LendingTest, account_id: u64, asset_name: &str) -> (u32, u32) {
    let (ltv, lt, _, _) = supply_risk_stamp(t, account_id, asset_name);
    (ltv, lt)
}

fn age_oracle_observations(t: &LendingTest) {
    t.env.ledger().with_mut(|ledger| ledger.timestamp += 1_000);
}

#[test]
fn regression_withdraw_restamps_sibling_ltv_after_governance_cut() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 5_000.0);
    t.supply(ALICE, "ETH", 5.0);
    t.borrow(ALICE, "ETH", 3.5);
    t.assert_healthy(ALICE);

    t.edit_asset_config("ETH", |cfg| {
        cfg.loan_to_value = 5_000;
        cfg.liquidation_threshold = 5_500;
    });

    let blocked = t.try_withdraw(ALICE, "USDC", 5_000.0);
    assert_contract_error(blocked, errors::INSUFFICIENT_COLLATERAL);

    t.withdraw(ALICE, "USDC", 100.0);
    let id = t.resolve_account_id(ALICE);
    let (eth_ltv, _) = supply_ltv_and_lt(&t, id, "ETH");
    assert_eq!(
        eth_ltv, 5_000,
        "withdraw must persist sibling restamped LTV"
    );
    t.assert_healthy(ALICE);
}

#[test]
fn regression_borrow_restamps_ltv_after_governance_cut() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.assert_healthy(ALICE);
    let id = t.resolve_account_id(ALICE);
    let (ltv_before, lt_before) = supply_ltv_and_lt(&t, id, "USDC");
    assert_eq!(ltv_before, 7_500);
    assert_eq!(
        lt_before, 8_000,
        "preset LT must stay until a threshold path"
    );

    t.edit_asset_config("USDC", |cfg| {
        cfg.loan_to_value = 5_000;
        cfg.liquidation_threshold = 5_500;
    });

    let blocked = t.try_borrow(ALICE, "ETH", 3.5);
    assert_contract_error(blocked, errors::INSUFFICIENT_COLLATERAL);

    t.borrow(ALICE, "ETH", 2.0);
    let (ltv_after, lt_after) = supply_ltv_and_lt(&t, id, "USDC");
    assert_eq!(ltv_after, 5_000, "borrow must persist restamped LTV");
    assert_eq!(
        lt_after, lt_before,
        "borrow must not restamp liquidation threshold"
    );
    t.assert_healthy(ALICE);
}

#[test]
fn regression_borrow_restamps_ltv_only() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let id = t.resolve_account_id(ALICE);
    let (ltv0, lt0, bonus0, fees0) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!((ltv0, lt0, bonus0, fees0), (7_500, 8_000, 500, 1_200));

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 6_000;
        c.liquidation_threshold = 7_000;
        c.liquidation_bonus = 900;
        c.liquidation_fees = 50;
    });

    t.borrow(ALICE, "ETH", 1.0);

    let (ltv1, lt1, bonus1, fees1) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!(ltv1, 6_000, "borrow must persist the restamped LTV");
    assert_eq!(
        (lt1, bonus1, fees1),
        (lt0, bonus0, fees0),
        "M1: borrow must leave threshold, bonus, and fees at their vintage"
    );
}

fn account_just_below_restamp_floor() -> (LendingTest, u64) {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.7);
    t.set_price("USDC", test_harness::usd_cents(96));

    assert!(
        !t.can_be_liquidated(ALICE),
        "setup: account stays healthy, only below the restamp floor"
    );

    let id = t.resolve_account_id(ALICE);
    (t, id)
}

#[test]
fn regression_supply_skips_bonus_only_raise_below_min_hf() {
    let (mut t, id) = account_just_below_restamp_floor();
    let (_, lt0, bonus0, fees0) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!((lt0, bonus0, fees0), (8_000, 500, 1_200), "preset tuple");

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 6_000;
        c.liquidation_bonus = 1_000;
    });

    t.try_supply_to_account(BOB, ALICE, "USDC", 1.0)
        .expect("third-party top-up of an existing leg stays allowed");

    let (ltv1, lt1, bonus1, fees1) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!(ltv1, 6_000, "LTV rides outside the gate");
    assert_eq!(
        (lt1, bonus1, fees1),
        (lt0, bonus0, fees0),
        "a bonus-only raise must be skipped below the min-HF floor"
    );
}

#[test]
fn regression_supply_skips_fees_only_cut_below_min_hf() {
    let (mut t, id) = account_just_below_restamp_floor();
    let (_, lt0, bonus0, fees0) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!((lt0, bonus0, fees0), (8_000, 500, 1_200), "preset tuple");

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 6_000;
        c.liquidation_fees = 50;
    });

    t.try_supply_to_account(BOB, ALICE, "USDC", 1.0)
        .expect("third-party top-up of an existing leg stays allowed");

    let (ltv1, lt1, bonus1, fees1) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!(ltv1, 6_000, "LTV rides outside the gate");
    assert_eq!(
        (lt1, bonus1, fees1),
        (lt0, bonus0, fees0),
        "a fees-only cut must be skipped below the min-HF floor"
    );
}

#[test]
fn regression_ltv_collateral_view_uses_live_listing_ltv() {
    use controller::constants::WAD;

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let id = t.resolve_account_id(ALICE);
    let before = t.ctrl_client().get_ltv_collateral_usd(&id);
    assert!(
        (before - 7_500 * WAD).abs() < WAD,
        "precondition LTV collateral ~$7500 wad, got {before}"
    );

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 5_000;
        c.liquidation_threshold = 5_500;
    });

    let (stamped_ltv, _) = supply_ltv_and_lt(&t, id, "USDC");
    assert_eq!(stamped_ltv, 7_500, "storage stamp stays until a mutator");
    let after = t.ctrl_client().get_ltv_collateral_usd(&id);
    assert!(
        (after - 5_000 * WAD).abs() < WAD,
        "view LTV collateral must use live 50% listing, got {after}"
    );
}

#[test]
fn regression_strategy_finalize_restamps_safe_params() {
    use test_harness::build_aggregator_swap;

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let id = t.resolve_account_id(ALICE);

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 5_000;
        c.liquidation_threshold = 5_500;
        c.liquidation_bonus = 250;
        c.liquidation_fees = 40;
    });
    let (ltv0, _, bonus0, fees0) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!((ltv0, bonus0, fees0), (7_500, 500, 1_200));

    t.fund_router("ETH", 5.0);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 10_000_000_000, 5_000_000);
    t.swap_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps);

    let (ltv1, _, bonus1, fees1) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!(
        ltv1, 5_000,
        "strategy path must restamp LTV (finalize LtvOnly)"
    );
    assert_eq!(
        bonus1, 250,
        "touched supply/withdraw legs must restamp bonus (FullTuple)"
    );
    assert_eq!(
        fees1, 40,
        "touched supply/withdraw legs must restamp fees (FullTuple)"
    );
}

#[test]
fn poc_stale_lt_stamp_blocks_liquidation_after_threshold_cut() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.5);
    t.assert_healthy(ALICE);
    assert!(
        !t.can_be_liquidated(ALICE),
        "precondition: account healthy under stamped LT"
    );

    t.edit_asset_config("USDC", |cfg| {
        cfg.loan_to_value = 5_000;
        cfg.liquidation_threshold = 5_500;
    });

    assert!(
        !t.can_be_liquidated(ALICE),
        "H-LIQ-16: stale high LT must keep HF ≥ 1 after listing cut"
    );
    let rejected = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    assert_contract_error(rejected, errors::HEALTH_FACTOR_TOO_HIGH);

    t.set_price("USDC", test_harness::usd_cents(80));

    assert!(
        t.can_be_liquidated(ALICE),
        "price drop under stamped LT must eventually open liquidation"
    );
}

#[test]
fn regression_third_party_cannot_open_new_supply_slots() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_position_limits(2, 4)
        .build();

    t.supply(ALICE, "USDC", 1_000.0);
    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);

    let new_slot = t.try_supply_to_account(BOB, ALICE, "ETH", 0.01);
    assert_contract_error(new_slot, errors::NOT_AUTHORIZED);

    let top_up = t.try_supply_to_account(BOB, ALICE, "USDC", 10.0);
    assert!(
        top_up.is_ok(),
        "third-party top-up of an existing supply leg must remain allowed; got {:?}",
        top_up
    );
    t.assert_supply_near(ALICE, "USDC", 1_010.0, 1.0);

    t.supply(ALICE, "ETH", 0.01);
    t.assert_position_exists(ALICE, "ETH", PositionType::Supply);
}

#[test]
fn poc_paused_debt_blocks_liquidation_repay() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", test_harness::usd_cents(50));
    assert!(t.can_be_liquidated(ALICE));

    t.set_spoke_asset_paused("ETH", true);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::SPOKE_ASSET_PAUSED);
}

/// Pausing a collateral listing must NOT block liquidation of the accounts holding it.
///
/// This test previously pinned the opposite. Seizure is pro-rata across an account's whole
/// collateral set, so gating it on `paused` turned a per-listing halt into a protocol-wide
/// liquidation halt for every account touching that asset. Seizure now has its own flag,
/// `no_seize`; see ADR-0008.
#[test]
fn poc_paused_collateral_does_not_block_liquidation_seizure() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", test_harness::usd_cents(50));
    assert!(t.can_be_liquidated(ALICE));

    t.set_spoke_asset_paused("USDC", true);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert!(
        result.is_ok(),
        "a paused collateral must still be seizable; got {result:?}"
    );
    assert!(
        t.token_balance(LIQUIDATOR, "USDC") > 0.0,
        "liquidator must receive the seized USDC"
    );

    // The halt that does apply to the seizure leg is `no_seize`, and only that one.
    t.set_spoke_asset_flags("USDC", true, false, true);
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0),
        errors::SPOKE_ASSET_SEIZURE_HALTED,
    );
}

#[test]
fn poc_frozen_collateral_still_backs_new_borrows() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);

    {
        use controller::types::SpokeAssetArgs;
        use test_harness::{HARNESS_HUB, HARNESS_SPOKE};
        let asset = t.resolve_asset("USDC");
        let config = t.get_asset_config("USDC");
        t.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
            hub_id: HARNESS_HUB,
            asset,
            spoke_id: HARNESS_SPOKE,
            can_collateral: config.is_collateralizable,
            can_borrow: config.is_borrowable,
            paused: false,
            frozen: true,
            no_seize: false,
            ltv: config.loan_to_value,
            threshold: config.liquidation_threshold,
            bonus: config.liquidation_bonus,
            liquidation_fees: config.liquidation_fees,
            supply_cap: 0,
            borrow_cap: 0,
        });
    }

    let supply_blocked = t.try_supply(ALICE, "USDC", 1.0);
    assert_contract_error(supply_blocked, errors::SPOKE_ASSET_FROZEN);

    let borrowed = t.try_borrow(ALICE, "ETH", 1.0);
    assert!(
        borrowed.is_ok(),
        "H-USER-02: freeze on collateral must not block borrow of another asset; got {:?}",
        borrowed
    );

    t.assert_borrow_near(ALICE, "ETH", 1.0, 0.01);
    t.assert_healthy(ALICE);
}

#[test]
fn poc_flash_loan_ongoing_blocks_risk_increasing_and_exit_paths() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    t.set_flash_loan_ongoing(true);

    assert_contract_error(t.try_borrow(ALICE, "ETH", 0.1), errors::FLASH_LOAN_ONGOING);
    assert_contract_error(t.try_supply(ALICE, "USDC", 1.0), errors::FLASH_LOAN_ONGOING);
    assert_contract_error(
        t.try_withdraw(ALICE, "USDC", 1.0),
        errors::FLASH_LOAN_ONGOING,
    );
    assert_contract_error(t.try_repay(ALICE, "ETH", 0.1), errors::FLASH_LOAN_ONGOING);
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.1),
        errors::FLASH_LOAN_ONGOING,
    );

    t.set_flash_loan_ongoing(false);
}

#[test]
fn poc_permissionless_repay_any_caller() {
    use soroban_sdk::vec;
    use test_harness::{f64_to_i128, hub_asset};

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(usdt_stable_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "USDT", 1_000.0);
    let before = t.borrow_balance(ALICE, "USDT");
    let alice_id = t.resolve_account_id(ALICE);

    let bob = t.get_or_create_user(BOB);
    let market = t.resolve_market("USDT");
    let raw = f64_to_i128(100.0, market.decimals);
    market.token_admin.mint(&bob, &raw);
    let payments = vec![&t.env, (hub_asset(market.asset.clone()), raw)];
    t.ctrl_client().repay(&bob, &alice_id, &payments);

    t.assert_borrow_near(ALICE, "USDT", before - 100.0, 0.01);
}

#[test]
fn poc_stale_oracle_blocks_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);

    t.set_price("USDC", test_harness::usd_cents(50));
    assert!(
        t.can_be_liquidated(ALICE),
        "precondition: liquidatable while prices are fresh"
    );

    age_oracle_observations(&t);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::PRICE_FEED_STALE);
}

#[test]
fn poc_lt_cut_stays_sticky_when_hf_below_min() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    let id = t.resolve_account_id(ALICE);
    let (ltv_before, lt_before) = supply_ltv_and_lt(&t, id, "USDC");
    assert_eq!(lt_before, 8_000, "preset LT stamp");
    assert_eq!(ltv_before, 7_500, "preset LTV stamp");

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 5_000;
        c.liquidation_threshold = 6_100;
    });

    t.update_account_threshold(true, &[id]);
    assert_eq!(
        supply_ltv_and_lt(&t, id, "USDC").1,
        8_000,
        "H-RISK-04: the keeper path holds the LT stamp under the HF floor too"
    );

    t.try_supply_to_account(BOB, ALICE, "USDC", 1.0)
        .expect("top-up must remain allowed");
    let (ltv_after, lt_after) = supply_ltv_and_lt(&t, id, "USDC");
    assert_eq!(
        ltv_after, 5_000,
        "H-RISK-03/04: LTV always restamps on supply refresh"
    );
    assert_eq!(
        lt_after, 8_000,
        "H-RISK-04: LT stamp must stay sticky when post-cut HF < 1.05"
    );

    assert!(
        !t.can_be_liquidated(ALICE),
        "sticky LT keeps HF ≥ 1 at original prices after listing cut"
    );
}

#[test]
fn poc_third_party_top_up_force_restamps_ltv() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let id = t.resolve_account_id(ALICE);

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 5_000;
        c.liquidation_threshold = 5_500;
    });

    t.try_supply_to_account(BOB, ALICE, "USDC", 1.0)
        .expect("third-party top-up of existing leg allowed");
    let (ltv, _) = supply_ltv_and_lt(&t, id, "USDC");
    assert_eq!(
        ltv, 5_000,
        "H-RISK-03: third-party top-up force-restamps LTV"
    );

    let blocked = t.try_borrow(ALICE, "ETH", 3.5);
    assert_contract_error(blocked, errors::INSUFFICIENT_COLLATERAL);

    t.borrow(ALICE, "ETH", 2.0);
    t.assert_healthy(ALICE);
}

#[test]
fn poc_untransferable_collateral_leg_bricks_whole_liquidation() {
    use test_harness::freezable_token::FreezableTokenClient;

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_freezable_market(wbtc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "WBTC", 0.1);
    t.borrow(ALICE, "ETH", 5.0);
    t.assert_healthy(ALICE);

    t.set_price("USDC", test_harness::usd_cents(10));
    t.assert_liquidatable(ALICE);

    let liquidator = t.get_or_create_user(LIQUIDATOR);
    let wbtc = FreezableTokenClient::new(&t.env, &t.resolve_asset("WBTC"));
    wbtc.set_blocked(&Some(liquidator.clone()));

    let debt_before = t.borrow_balance(ALICE, "ETH");
    let usdc_before = t.supply_balance(ALICE, "USDC");
    let wbtc_before = t.supply_balance(ALICE, "WBTC");
    let bricked = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    assert!(
        bricked.is_err(),
        "H-LIQ-DOS: one untransferable collateral leg must brick the whole \
         liquidation; got {:?}",
        bricked
    );
    let bricked_small = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.1);

    assert!(
        bricked_small.is_err(),
        "even a minimal repayment still seizes a WBTC slice and reverts; got {:?}",
        bricked_small
    );

    assert_eq!(
        t.borrow_balance(ALICE, "ETH"),
        debt_before,
        "H-LIQ-DOS brick must leave ETH debt unchanged"
    );
    assert_eq!(
        t.supply_balance(ALICE, "USDC"),
        usdc_before,
        "H-LIQ-DOS brick must leave USDC collateral unchanged"
    );
    assert_eq!(
        t.supply_balance(ALICE, "WBTC"),
        wbtc_before,
        "H-LIQ-DOS brick must leave WBTC collateral unchanged"
    );

    wbtc.set_blocked(&None);
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert!(
        t.token_balance(LIQUIDATOR, "WBTC") > 0.0,
        "after unblock the liquidator seizes the WBTC leg"
    );
    assert!(
        t.token_balance(LIQUIDATOR, "USDC") > 0.0,
        "and the USDC leg too — seizure is forced across every collateral asset"
    );
}

#[test]
fn regression_third_party_supply_cannot_force_adverse_tuple_below_min_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    let id = t.resolve_account_id(ALICE);
    let (_, lt_before, bonus_before, fees_before) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!(
        (lt_before, bonus_before, fees_before),
        (8_000, 500, 1_200),
        "preset tuple"
    );

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 5_000;
        c.liquidation_threshold = 6_100;
        c.liquidation_bonus = 1_000;
        c.liquidation_fees = 50;
    });

    t.try_supply_to_account(BOB, ALICE, "USDC", 1.0)
        .expect("third-party top-up of an existing leg stays allowed");

    let (ltv_after, lt_after, bonus_after, fees_after) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!(ltv_after, 5_000, "LTV rides outside the gate");
    assert_eq!(
        (lt_after, bonus_after, fees_after),
        (lt_before, bonus_before, fees_before),
        "M1: threshold, bonus, and fees hold their vintage together under the HF floor"
    );
}

#[test]
fn regression_supply_propagates_bonus_raise_to_healthy_account() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let id = t.resolve_account_id(ALICE);
    let (_, lt_before, bonus_before, _) = supply_risk_stamp(&t, id, "USDC");

    t.edit_asset_config("USDC", |c| c.liquidation_bonus = bonus_before + 500);

    t.try_supply_to_account(BOB, ALICE, "USDC", 1.0)
        .expect("third-party top-up of an existing leg stays allowed");

    let (_, lt_after, bonus_after, _) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!(
        bonus_after,
        bonus_before + 500,
        "a healthy account takes the raised bonus"
    );
    assert_eq!(
        lt_after, lt_before,
        "an unchanged threshold restamps to itself"
    );
}
