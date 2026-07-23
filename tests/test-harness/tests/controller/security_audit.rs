//! Security harness PoCs for controller public user paths.
//!
//! These tests drive **shipped** controller entrypoints via the integration
//! harness. Each case pins a hypothesis from the controller audit:
//! - H-RISK-01 / F3: borrow restamps listed LTV before gates (regression)
//! - H-LIQ-16: stale LT stamp blocks liquidation under tighter listing config
//! - H-USER-03: permissionless third-party supply can open new position slots
//! - H-LIQ-12: paused debt blocks liquidation repay
//! - H-USER-02: freezing collateral does not block borrow against its stamp
//! - H-ORC-04: stale oracle freezes liquidation (fail-closed)
//! - H-RISK-03: third-party top-up force-restamps LTV
//! - H-RISK-04: LT cut sticky when post-cut HF would be < 1.05
//! - H-LIQ-DOS: one untransferable collateral leg bricks the whole liquidation

use controller::types::ControllerKey;
use soroban_sdk::testutils::Ledger as _;
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, usdc_preset, usdt_stable_preset,
    wbtc_preset, HubAssetKey, LendingTest, PositionType, ALICE, BOB, LIQUIDATOR,
};

fn supply_risk_stamp(
    t: &LendingTest,
    account_id: u64,
    asset_name: &str,
) -> (u32, u32, u32, u32) {
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

/// Advance wall-clock only so mock temp entries stay live and prices age past
/// the default staleness window (~900s).
fn age_oracle_observations(t: &LendingTest) {
    t.env.ledger().with_mut(|ledger| ledger.timestamp += 1_000);
}

/// H-RISK-01 regression: debt-increasing `borrow` restamps listed supply LTV
/// (and bonus/fees) from live listing config before LTV/HF gates. After an LTV
/// cut, capacity above the **new** listing LTV is rejected without a keeper.
///
/// Mode: **regression after patch** — pre-fix, stamped 75% allowed $7k debt
/// against $10k coll after a 50% cut; post-fix that borrow fails.
#[test]
fn regression_borrow_restamps_ltv_after_governance_cut() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // $10_000 USDC @ listing LTV 75% stamps capacity $7_500 on the position.
    t.supply(ALICE, "USDC", 10_000.0);
    t.assert_healthy(ALICE);
    let id = t.resolve_account_id(ALICE);
    let (ltv_before, lt_before) = supply_ltv_and_lt(&t, id, "USDC");
    assert_eq!(ltv_before, 7_500);
    assert_eq!(lt_before, 8_000, "preset LT must stay until a threshold path");

    // Governance cuts listing LTV to 50% (live capacity $5_000).
    t.edit_asset_config("USDC", |cfg| {
        cfg.loan_to_value = 5_000;
        cfg.liquidation_threshold = 5_500;
    });

    // Borrow $7_000 of ETH (3.5) exceeds live $5_000 capacity → rejected.
    let blocked = t.try_borrow(ALICE, "ETH", 3.5);
    assert_contract_error(blocked, errors::INSUFFICIENT_COLLATERAL);

    // Within new capacity still works; LTV stamp binds, LT stays sticky until
    // a threshold refresh (HF floor may keep 8_000).
    t.borrow(ALICE, "ETH", 2.0);
    let (ltv_after, lt_after) = supply_ltv_and_lt(&t, id, "USDC");
    assert_eq!(ltv_after, 5_000, "borrow must persist restamped LTV");
    assert_eq!(
        lt_after, lt_before,
        "borrow must not restamp liquidation threshold"
    );
    t.assert_healthy(ALICE);
}

/// Borrow restamps liquidation bonus and fees (not only LTV); LT stays put.
#[test]
fn regression_borrow_restamps_bonus_and_fees() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let id = t.resolve_account_id(ALICE);
    let (ltv0, lt0, bonus0, fees0) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!((ltv0, lt0, bonus0, fees0), (7_500, 8_000, 500, 100));

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 7_500;
        c.liquidation_threshold = 8_000;
        c.liquidation_bonus = 300;
        c.liquidation_fees = 50;
    });

    t.borrow(ALICE, "ETH", 1.0);
    let (ltv1, lt1, bonus1, fees1) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!(ltv1, 7_500);
    assert_eq!(lt1, 8_000, "LT must not change on borrow restamp");
    assert_eq!(bonus1, 300, "borrow must restamp liquidation bonus");
    assert_eq!(fees1, 50, "borrow must restamp liquidation fees");
}

/// Restamp triggers when only the liquidation bonus diverges (LTV and fees held
/// equal), so the bonus term of the skip guard is load-bearing on its own.
#[test]
fn regression_borrow_restamps_bonus_only() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let id = t.resolve_account_id(ALICE);
    let (ltv0, _, bonus0, fees0) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!((ltv0, bonus0, fees0), (7_500, 500, 100));

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 7_500;
        c.liquidation_bonus = 300;
        c.liquidation_fees = 100;
    });

    t.borrow(ALICE, "ETH", 1.0);
    let (ltv1, _, bonus1, fees1) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!(ltv1, 7_500, "LTV unchanged");
    assert_eq!(fees1, 100, "fees unchanged");
    assert_eq!(bonus1, 300, "bonus-only divergence must restamp");
}

/// Restamp triggers when only the liquidation fees diverge (LTV and bonus held
/// equal), so the fees term of the skip guard is load-bearing on its own.
#[test]
fn regression_borrow_restamps_fees_only() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let id = t.resolve_account_id(ALICE);
    let (ltv0, _, bonus0, fees0) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!((ltv0, bonus0, fees0), (7_500, 500, 100));

    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 7_500;
        c.liquidation_bonus = 500;
        c.liquidation_fees = 50;
    });

    t.borrow(ALICE, "ETH", 1.0);
    let (ltv1, _, bonus1, fees1) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!(ltv1, 7_500, "LTV unchanged");
    assert_eq!(bonus1, 500, "bonus unchanged");
    assert_eq!(fees1, 50, "fees-only divergence must restamp");
}

/// After a listing LTV cut, `get_ltv_collateral_usd` uses live listing LTV
/// without requiring a prior restamping mutator.
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

/// Strategy finalize restamps safe params (`swap_collateral`).
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
    assert_eq!((ltv0, bonus0, fees0), (7_500, 500, 100));

    t.fund_router("ETH", 5.0);
    let steps = build_aggregator_swap(&t, "USDC", "ETH", 10_000_000_000, 5_000_000);
    t.swap_collateral(ALICE, "USDC", 1_000.0, "ETH", &steps);

    let (ltv1, _, bonus1, fees1) = supply_risk_stamp(&t, id, "USDC");
    assert_eq!(ltv1, 5_000, "strategy finalize must restamp LTV");
    assert_eq!(bonus1, 250, "strategy finalize must restamp bonus");
    assert_eq!(fees1, 40, "strategy finalize must restamp fees");
}

/// H-LIQ-16: lowering listing LT does not restamp; HF stays on the old LT and
/// `liquidate` reverts while a restamped account would be liquidatable.
#[test]
fn poc_stale_lt_stamp_blocks_liquidation_after_threshold_cut() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // USDC LT 80%, LTV 75%. Coll $10k, debt $7k → stamped HF = 0.8*10k/7k ≈ 1.14.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.5);
    t.assert_healthy(ALICE);
    assert!(
        !t.can_be_liquidated(ALICE),
        "precondition: account healthy under stamped LT"
    );

    // Cut listing LT to 55% (would yield HF ≈ 0.55*10k/7k ≈ 0.79 if restamped).
    t.edit_asset_config("USDC", |cfg| {
        cfg.loan_to_value = 5_000;
        cfg.liquidation_threshold = 5_500;
    });

    // Without restamp, stamped HF stays ≥ 1 → not liquidatable.
    assert!(
        !t.can_be_liquidated(ALICE),
        "H-LIQ-16: stale high LT must keep HF ≥ 1 after listing cut"
    );
    let rejected = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    assert_contract_error(rejected, errors::HEALTH_FACTOR_TOO_HIGH);

    // Force restamp of LT (has_risks=true requires HF≥1.05 under *new* LT — may
    // fail). Use supply dust restamp path instead: self-supply refreshes LTV
    // always and LT only if post-hypo HF ≥ 1.05. With live LT 55%, hypo HF < 1.05
    // so LT stays sticky — that is H-RISK-04. Prove liquidatability via price
    // drop that breaks stamped HF.
    t.set_price("USDC", test_harness::usd_cents(80));
    // coll $8k, stamped LT 80% → weighted $6.4k, debt $7k → HF < 1
    assert!(
        t.can_be_liquidated(ALICE),
        "price drop under stamped LT must eventually open liquidation"
    );
}

/// H-USER-03 (patched): third parties may top up existing supply legs but cannot
/// open new asset slots on another account (prevents max_supply_positions grief).
///
/// Mode: **regression after patch** — pre-fix, Bob could open ETH/WBTC slots
/// until Alice hit the limit; post-fix new-slot third-party supply reverts
/// `NotAuthorized` while same-asset top-up still works.
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

    // Bob cannot open a new ETH slot on Alice's account.
    let new_slot = t.try_supply_to_account(BOB, ALICE, "ETH", 0.01);
    assert_contract_error(new_slot, errors::NOT_AUTHORIZED);

    // Bob can still gift more USDC into Alice's existing leg.
    let top_up = t.try_supply_to_account(BOB, ALICE, "USDC", 10.0);
    assert!(
        top_up.is_ok(),
        "third-party top-up of an existing supply leg must remain allowed; got {:?}",
        top_up
    );
    t.assert_supply_near(ALICE, "USDC", 1_010.0, 1.0);

    // Owner retains the right to open a second slot themselves.
    t.supply(ALICE, "ETH", 0.01);
    t.assert_position_exists(ALICE, "ETH", PositionType::Supply);
}

/// H-LIQ-12 / ADR 0011: pausing a debt asset blocks liquidator repay on that leg.
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

/// H-USER-02: freezing collateral does not block borrowing against its stamp.
#[test]
fn poc_frozen_collateral_still_backs_new_borrows() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);

    // Freeze USDC (blocks new supply; withdraw still allowed). Flags are live.
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
            ltv: config.loan_to_value,
            threshold: config.liquidation_threshold,
            bonus: config.liquidation_bonus,
            liquidation_fees: config.liquidation_fees,
            supply_cap: 0,
            borrow_cap: 0,
        });
    }

    // New supply of frozen USDC is blocked.
    let supply_blocked = t.try_supply(ALICE, "USDC", 1.0);
    assert_contract_error(supply_blocked, errors::SPOKE_ASSET_FROZEN);

    // Borrow against frozen USDC collateral still works (flags check borrow asset).
    let borrowed = t.try_borrow(ALICE, "ETH", 1.0);
    assert!(
        borrowed.is_ok(),
        "H-USER-02: freeze on collateral must not block borrow of another asset; got {:?}",
        borrowed
    );
}

/// Flash-loan guard blocks nested user mutators (sanity for pool trust boundary).
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

/// Permissionless repay: any funded caller may reduce another account's debt
/// (debt-decreasing only; no owner check on the controller path).
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

    // Bob pays Alice's debt through the real `repay` entrypoint.
    let bob = t.get_or_create_user(BOB);
    let market = t.resolve_market("USDT");
    let raw = f64_to_i128(100.0, market.decimals);
    market.token_admin.mint(&bob, &raw);
    let payments = vec![&t.env, (hub_asset(market.asset.clone()), raw)];
    t.ctrl_client().repay(&bob, &alice_id, &payments);

    // Bob repaid exactly 100 USDT; Alice's debt must drop by that amount.
    t.assert_borrow_near(ALICE, "USDT", before - 100.0, 0.01);
}

/// H-ORC-04: liquidate uses the same fail-closed price stack as borrow/HF.
/// Aging oracle observations past `max_price_stale` freezes liquidation of an
/// underwater account until feeds are refreshed (ops residual, not a bypass).
///
/// Mode: **behavior PoC** of shipped controller `liquidate` entrypoint.
#[test]
fn poc_stale_oracle_blocks_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    // Crash coll so the account is underwater under fresh prices.
    t.set_price("USDC", test_harness::usd_cents(50));
    assert!(
        t.can_be_liquidated(ALICE),
        "precondition: liquidatable while prices are fresh"
    );

    age_oracle_observations(&t);

    let result = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert_contract_error(result, errors::PRICE_FEED_STALE);
}

/// H-RISK-04: debt-bearing LT cuts only apply when post-hypo HF ≥ 1.05.
/// After a listing LT cut that would put HF below that floor, a restamp path
/// (third-party top-up / supply refresh) keeps the **old** LT stamp.
///
/// Mode: **behavior PoC** of shipped `refresh_supply_risk_params` via `supply`.
#[test]
fn poc_lt_cut_stays_sticky_when_hf_below_min() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    // $10k coll, ~$6k debt → HF ≈ 0.8*10k/6k ≈ 1.33 under LT 80%.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    let id = t.resolve_account_id(ALICE);
    let (ltv_before, lt_before) = supply_ltv_and_lt(&t, id, "USDC");
    assert_eq!(lt_before, 8_000, "preset LT stamp");
    assert_eq!(ltv_before, 7_500, "preset LTV stamp");

    // Live LT 61% → weighted $6.1k / $6k debt ≈ HF 1.017 < 1.05 buffer.
    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 5_000;
        c.liquidation_threshold = 6_100;
    });

    // Keeper risky restamp rejects entirely at the outer HF gate.
    let keeper = t.try_update_account_threshold(true, &[id]);
    assert_contract_error(keeper, errors::HEALTH_FACTOR_TOO_LOW);

    // Supply refresh path: LTV always updates; LT stays sticky under HF floor.
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

    // With sticky high LT, account is still not liquidatable at original prices.
    assert!(
        !t.can_be_liquidated(ALICE),
        "sticky LT keeps HF ≥ 1 at original prices after listing cut"
    );
}

/// H-RISK-03 force path: a third-party top-up restamps **LTV** from live listing
/// config on the supply path (independent of debt-path restamp).
///
/// Mode: **behavior PoC** — top-up still binds LTV; debt open also binds LTV.
#[test]
fn poc_third_party_top_up_force_restamps_ltv() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let id = t.resolve_account_id(ALICE);

    // Cut listing LTV to 50% without owner touching collateral.
    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 5_000;
        c.liquidation_threshold = 5_500;
    });

    // Bob force-restamps via dust top-up of the existing USDC leg.
    t.try_supply_to_account(BOB, ALICE, "USDC", 1.0)
        .expect("third-party top-up of existing leg allowed");
    let (ltv, _) = supply_ltv_and_lt(&t, id, "USDC");
    assert_eq!(
        ltv, 5_000,
        "H-RISK-03: third-party top-up force-restamps LTV"
    );

    // New LTV capacity is ~$5_000; $7_000 debt must fail.
    let blocked = t.try_borrow(ALICE, "ETH", 3.5);
    assert_contract_error(blocked, errors::INSUFFICIENT_COLLATERAL);

    // Borrow within new capacity still works.
    t.borrow(ALICE, "ETH", 2.0);
    t.assert_healthy(ALICE);
}

/// H-LIQ-DOS: liquidation seizure is forced pro-rata across EVERY collateral leg
/// with no liquidator subset selection (see `liquidation.rs` "proportional
/// seizure only"). A single collateral asset whose SAC refuses transfer to the
/// liquidator (AUTH_REQUIRED / issuer-deauthorized / clawback-frozen) reverts the
/// whole bulk withdraw, so an unhealthy multi-collateral account becomes
/// un-liquidatable and its shortfall accrues to the protocol as bad debt.
///
/// Mode: **proven mechanism, conditional precondition** — needs a listed
/// collateral asset whose issuer can withhold authorization from an arbitrary
/// liquidator (realistic for the regulated / RWA SACs this protocol lists, or an
/// issuer-controlled/colluding borrower). The controller has no fallback that
/// lets the liquidator seize only the transferable legs.
#[test]
fn poc_untransferable_collateral_leg_bricks_whole_liquidation() {
    use test_harness::freezable_token::FreezableTokenClient;

    // WBTC is a freezable/regulated asset whose issuer can withhold transfer
    // authorization from an arbitrary receiver (AUTH_REQUIRED / frozen SAC).
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_freezable_market(wbtc_preset())
        .with_market(eth_preset())
        .build();

    // Alice: $10k USDC + 0.1 WBTC ($6k) collateral, borrows 5 ETH ($10k).
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "WBTC", 0.1);
    t.borrow(ALICE, "ETH", 5.0);
    t.assert_healthy(ALICE);

    // Crash USDC so the account is deeply underwater. Below the HF-preserving
    // band (cap < 0), partial liquidation at base bonus is permitted, so the plan
    // reaches the seizure transfer instead of reverting `FullCloseRequired`.
    t.set_price("USDC", test_harness::usd_cents(10));
    t.assert_liquidatable(ALICE);

    // The WBTC issuer withholds authorization from the liquidator: any transfer
    // of WBTC to the liquidator now traps at the token boundary.
    let liquidator = t.get_or_create_user(LIQUIDATOR);
    let wbtc = FreezableTokenClient::new(&t.env, &t.resolve_asset("WBTC"));
    wbtc.set_blocked(&Some(liquidator.clone()));

    // Forced pro-rata seizure always includes a WBTC leg, so the entire
    // liquidation reverts — even a minimal repayment cannot route around it.
    // FreezableToken traps with a host assert (not a controller error code).
    let debt_before = t.borrow_balance(ALICE, "ETH");
    let usdc_before = t.supply_balance(ALICE, "USDC");
    let wbtc_before = t.supply_balance(ALICE, "WBTC");
    let bricked = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    // Host/token trap from FreezableToken — no controller error code to pin.
    assert!(
        bricked.is_err(),
        "H-LIQ-DOS: one untransferable collateral leg must brick the whole \
         liquidation; got {:?}",
        bricked
    );
    let bricked_small = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.1);
    // Host/token trap from FreezableToken — no controller error code to pin.
    assert!(
        bricked_small.is_err(),
        "even a minimal repayment still seizes a WBTC slice and reverts; got {:?}",
        bricked_small
    );
    // Brick must be atomic: no debt repaid and no collateral moved.
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

    // Control: the account is otherwise fully liquidatable. Lift the block and
    // the identical liquidation succeeds, proving the untransferable leg was the
    // sole blocker and that seizure is forced across every collateral leg.
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
