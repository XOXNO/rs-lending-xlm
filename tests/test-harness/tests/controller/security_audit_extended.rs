//! Extended security harness: novel hypotheses beyond `security_audit.rs`.
//!
//! Each test drives **shipped** controller entrypoints. Status labels match the
//! audit report (proven / refuted / design residual).

use controller::types::{ControllerKey, SpokeAssetArgs};
use soroban_sdk::testutils::Ledger as _;
use test_harness::{
    assert_contract_error, errors, eth_preset, hub_asset, usdc_preset, wbtc_preset, HubAssetKey,
    LendingTest, PositionType, ALICE, BOB, HARNESS_HUB, HARNESS_SPOKE, LIQUIDATOR,
};

fn supply_ltv_and_lt(t: &LendingTest, account_id: u64, asset_name: &str) -> (u32, u32) {
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
        (p.loan_to_value, p.liquidation_threshold)
    })
}

fn set_frozen(t: &LendingTest, asset_name: &str, frozen: bool) {
    let asset = t.resolve_asset(asset_name);
    let config = t.get_asset_config(asset_name);
    t.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset,
        spoke_id: HARNESS_SPOKE,
        can_collateral: config.is_collateralizable,
        can_borrow: config.is_borrowable,
        paused: false,
        frozen,
        ltv: config.loan_to_value,
        threshold: config.liquidation_threshold,
        bonus: config.liquidation_bonus,
        liquidation_fees: config.liquidation_fees,
        supply_cap: 0,
        borrow_cap: 0,
    });
}

fn set_can_collateral(t: &LendingTest, asset_name: &str, can_collateral: bool) {
    let asset = t.resolve_asset(asset_name);
    let config = t.get_asset_config(asset_name);
    t.ctrl_client().edit_asset_in_spoke(&SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset,
        spoke_id: HARNESS_SPOKE,
        can_collateral,
        can_borrow: config.is_borrowable,
        paused: false,
        frozen: false,
        ltv: config.loan_to_value,
        threshold: config.liquidation_threshold,
        bonus: config.liquidation_bonus,
        liquidation_fees: config.liquidation_fees,
        supply_cap: 0,
        borrow_cap: 0,
    });
}

// ---------------------------------------------------------------------------
// H-PAUSE-GLOBAL — global pause circuit breaker matrix
// ---------------------------------------------------------------------------

/// Global pause blocks risk-increasing paths (supply / borrow / flash) while
/// exit and keeper paths (withdraw debt-free, repay, liquidate) stay open.
#[test]
fn poc_global_pause_blocks_risk_increasing_allows_exit_and_liq() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0); // $6k debt against $10k coll
    // Second account: debt-free withdraw under pause.
    t.supply(BOB, "USDC", 1_000.0);

    t.pause();

    assert_contract_error(t.try_supply(ALICE, "USDC", 1.0), errors::CONTRACT_PAUSED);
    assert_contract_error(t.try_borrow(ALICE, "ETH", 0.1), errors::CONTRACT_PAUSED);
    // Pause is `#[when_not_paused]` on the entrypoint (before body validation).
    let receiver = t.controller_address();
    assert_contract_error(
        t.try_flash_loan(ALICE, "USDC", 1.0, &receiver),
        errors::CONTRACT_PAUSED,
    );

    // Debt-free exit still works under global pause.
    let w = t.try_withdraw(BOB, "USDC", 10.0);
    assert!(
        w.is_ok(),
        "H-PAUSE-GLOBAL: debt-free withdraw must remain open while paused; got {w:?}"
    );

    // Permissionless repay still works under global pause.
    let r = t.try_repay(ALICE, "ETH", 0.1);
    assert!(
        r.is_ok(),
        "H-PAUSE-GLOBAL: repay must remain open while paused; got {r:?}"
    );

    // Crash coll so Alice is liquidatable (~$4k coll / $5.8k debt, HF < 1).
    t.set_price("USDC", test_harness::usd_cents(40));
    assert!(
        t.can_be_liquidated(ALICE),
        "precondition: Alice liquidatable after crash"
    );
    let liq = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    assert!(
        liq.is_ok(),
        "H-PAUSE-GLOBAL: liquidate must remain open while globally paused; got {liq:?}"
    );
}

/// Global pause does not disable post-pool HF gates on debt-bearing withdraw.
#[test]
fn refutation_global_pause_withdraw_still_enforces_hf() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.pause();

    // Full withdraw would leave debt unbacked → must fail HF/LTV gate, not succeed.
    let drained = t.try_withdraw(ALICE, "USDC", 0.0);
    assert_contract_error(drained, errors::INSUFFICIENT_COLLATERAL);
}

// ---------------------------------------------------------------------------
// H-RISK-02 — multi-collateral partial LTV restamp
// ---------------------------------------------------------------------------

/// Touching one supply leg restamps only that leg; untouched legs keep stamped LTV
/// after a governance cut (lazy stamp residual, multi-asset form of H-RISK-01).
#[test]
fn poc_multi_collateral_partial_ltv_restamp_on_one_leg() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 5_000.0);
    t.supply(ALICE, "ETH", 2.0); // ~$4k
    let id = t.resolve_account_id(ALICE);
    let (usdc_ltv0, _) = supply_ltv_and_lt(&t, id, "USDC");
    let (eth_ltv0, _) = supply_ltv_and_lt(&t, id, "ETH");
    assert_eq!(usdc_ltv0, 7_500);
    assert_eq!(eth_ltv0, 7_500);

    // Cut both listing LTVs to 50%.
    t.edit_asset_config("USDC", |c| {
        c.loan_to_value = 5_000;
        c.liquidation_threshold = 5_500;
    });
    t.edit_asset_config("ETH", |c| {
        c.loan_to_value = 5_000;
        c.liquidation_threshold = 5_500;
    });

    // Touch only USDC via third-party top-up.
    t.try_supply_to_account(BOB, ALICE, "USDC", 1.0)
        .expect("top-up existing USDC leg");
    let (usdc_ltv1, _) = supply_ltv_and_lt(&t, id, "USDC");
    let (eth_ltv1, _) = supply_ltv_and_lt(&t, id, "ETH");
    assert_eq!(usdc_ltv1, 5_000, "H-RISK-02: touched USDC must restamp LTV");
    assert_eq!(
        eth_ltv1, 7_500,
        "H-RISK-02: untouched ETH must keep stamped LTV after listing cut"
    );

    // Capacity still uses high ETH stamp: borrow more than live dual-50% capacity.
    // Live: USDC ~$5k@50% + ETH ~$4k@50% = $4.5k. Stamped: USDC $5k@50% + ETH $4k@75% = $5.5k.
    let ok = t.try_borrow(ALICE, "WBTC", 0.08); // ~$4.8k at $60k
    assert!(
        ok.is_ok(),
        "H-RISK-02: borrow between live and stamped multi-coll capacity must pass; got {ok:?}"
    );
}

// ---------------------------------------------------------------------------
// H-USER-18 — delisted collateral flag still backs debt
// ---------------------------------------------------------------------------

/// Setting `can_collateral=false` blocks new supply but existing stamped supply
/// still backs new borrows (same design family as freeze / sticky LTV).
#[test]
fn poc_delisted_collateral_flag_still_backs_new_borrows() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    set_can_collateral(&t, "USDC", false);

    assert_contract_error(t.try_supply(ALICE, "USDC", 1.0), errors::NOT_COLLATERAL);

    let borrowed = t.try_borrow(ALICE, "ETH", 1.0);
    assert!(
        borrowed.is_ok(),
        "H-USER-18: delisting can_collateral must not strip existing stamp from HF/LTV; got {borrowed:?}"
    );
}

// ---------------------------------------------------------------------------
// H-LIQ-21 — frozen collateral remains seizable
// ---------------------------------------------------------------------------

#[test]
fn poc_liquidation_seizes_frozen_collateral() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", test_harness::usd_cents(40));
    assert!(t.can_be_liquidated(ALICE));

    set_frozen(&t, "USDC", true);
    assert_contract_error(t.try_supply(ALICE, "USDC", 1.0), errors::SPOKE_ASSET_FROZEN);

    let liq = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    assert!(
        liq.is_ok(),
        "H-LIQ-21: frozen collateral must remain seizable; got {liq:?}"
    );
    assert!(
        t.token_balance(LIQUIDATOR, "USDC") > 0.0,
        "liquidator must receive seized frozen USDC"
    );
}

// ---------------------------------------------------------------------------
// H-REPAY-FLAGS — freeze vs pause on debt repay
// ---------------------------------------------------------------------------

#[test]
fn poc_frozen_debt_still_repayable_paused_debt_blocked() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);

    set_frozen(&t, "ETH", true);
    let frozen_repay = t.try_repay(ALICE, "ETH", 0.1);
    assert!(
        frozen_repay.is_ok(),
        "freeze must not block debt repay; got {frozen_repay:?}"
    );

    t.set_spoke_asset_paused("ETH", true);
    assert_contract_error(t.try_repay(ALICE, "ETH", 0.1), errors::SPOKE_ASSET_PAUSED);
}

// ---------------------------------------------------------------------------
// H-FLASH-CLEAN — flash guard covers clean_bad_debt
// ---------------------------------------------------------------------------

#[test]
fn refutation_flash_guard_blocks_clean_bad_debt() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let id = t.resolve_account_id(ALICE);

    t.set_flash_loan_ongoing(true);
    assert_contract_error(
        t.try_clean_bad_debt_by_id(id),
        errors::FLASH_LOAN_ONGOING,
    );
    t.set_flash_loan_ongoing(false);
}

// ---------------------------------------------------------------------------
// H-LIQ-SELF — self-liquidation blocked
// ---------------------------------------------------------------------------

#[test]
fn refutation_owner_cannot_self_liquidate() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", test_harness::usd_cents(40));
    assert!(t.can_be_liquidated(ALICE));

    assert_contract_error(
        t.try_liquidate(ALICE, ALICE, "ETH", 0.5),
        errors::SELF_LIQUIDATION_NOT_ALLOWED,
    );
}

// ---------------------------------------------------------------------------
// H-LIQ-EMPTY — empty / healthy liquidate reverts
// ---------------------------------------------------------------------------

#[test]
fn refutation_liquidate_healthy_and_empty_payments() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    assert!(!t.can_be_liquidated(ALICE));

    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.1),
        errors::HEALTH_FACTOR_TOO_HIGH,
    );
}

// ---------------------------------------------------------------------------
// H-BAD-DEBT-GATE — clean_bad_debt refuses healthy residual
// ---------------------------------------------------------------------------

#[test]
fn refutation_clean_bad_debt_rejects_non_residual() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 1.0);
    let id = t.resolve_account_id(ALICE);

    assert_contract_error(t.try_clean_bad_debt_by_id(id), errors::CANNOT_CLEAN_BAD_DEBT);
}

// ---------------------------------------------------------------------------
// H-ORC-CTRL-HARD — controller write path uses hard prices (fail-closed stale)
// ---------------------------------------------------------------------------

/// Borrow (write path) reverts on stale oracle the same way liquidate does —
/// proves controller does not soft-accept via `prices_status` on mutators.
#[test]
fn poc_stale_oracle_blocks_borrow_write_path() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    // Age every feed past the default ~900s window.
    t.env.ledger().with_mut(|ledger| ledger.timestamp += 1_000);

    assert_contract_error(t.try_borrow(ALICE, "ETH", 0.1), errors::PRICE_FEED_STALE);
}

// ---------------------------------------------------------------------------
// H-WITHDRAW-PAUSED-ASSET — spoke pause blocks user withdraw (not freeze)
// ---------------------------------------------------------------------------

#[test]
fn poc_spoke_pause_blocks_withdraw_freeze_allows() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 1_000.0);

    set_frozen(&t, "USDC", true);
    let frozen_w = t.try_withdraw(ALICE, "USDC", 10.0);
    assert!(
        frozen_w.is_ok(),
        "freeze must allow withdraw; got {frozen_w:?}"
    );

    t.set_spoke_asset_paused("USDC", true);
    assert_contract_error(t.try_withdraw(ALICE, "USDC", 10.0), errors::SPOKE_ASSET_PAUSED);
}

// ---------------------------------------------------------------------------
// H-SUPPLY-SLOT — position already exists check (revalidation of patch)
// ---------------------------------------------------------------------------

#[test]
fn revalidation_third_party_can_top_up_only_existing_leg() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 100.0);
    assert_contract_error(
        t.try_supply_to_account(BOB, ALICE, "ETH", 0.01),
        errors::NOT_AUTHORIZED,
    );
    t.try_supply_to_account(BOB, ALICE, "USDC", 1.0)
        .expect("existing leg top-up");
    t.assert_position_exists(ALICE, "USDC", PositionType::Supply);
}

// ---------------------------------------------------------------------------
// H-DEBT-THIRD-PARTY-BORROW — only owner/delegate may borrow
// ---------------------------------------------------------------------------

/// H-ORC-INBAND: dual-source in-band midpoint is used on the controller write
/// path (not primary alone, not anchor alone).
///
/// `set_oracle_primary_anchor` wires primary=TWAP and anchor=Spot. We set
/// primary (TWAP) **lower** than spot so the three valuations diverge:
/// - primary $1.00 → LTV capacity $7_500
/// - midpoint $1.015 → capacity ≈ $7_612.5
/// - anchor $1.03 → capacity $7_725
///
/// Discriminators:
/// 1. Borrow in (primary_cap, midpoint_cap] must **pass** (fails if primary-only).
/// 2. Borrow in (midpoint_cap, anchor_cap) must **fail** (passes if anchor-only).
#[test]
fn poc_dual_in_band_midpoint_used_on_borrow_path() {
    use test_harness::{usd, usd_cents};

    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_dust_disabled_all_markets()
        .build();

    t.set_oracle_primary_anchor("USDC");
    t.set_oracle_primary_anchor("ETH");
    // set_price writes spot+TWAP; set_safe_price overwrites TWAP only.
    // primary(TWAP)=$1.00, anchor(spot)=$1.03 → midpoint ≈ $1.015.
    t.set_price("USDC", usd_cents(103));
    t.set_safe_price("USDC", usd(1), true, true);
    t.set_price("ETH", usd(2000));
    t.set_safe_price("ETH", usd(2000), true, true);

    // Alice: $7_510 debt (3.755 ETH) sits above primary-only $7_500 and under
    // midpoint ≈ $7_612.5.
    t.supply(ALICE, "USDC", 10_000.0);
    let mid_ok = t.try_borrow(ALICE, "ETH", 3.755);
    assert!(
        mid_ok.is_ok(),
        "H-ORC-INBAND: borrow in (primary_cap, midpoint_cap] must pass under midpoint; got {mid_ok:?}"
    );

    // Bob: $7_650 debt (3.825 ETH) sits above midpoint and under pure-anchor $7_725.
    t.supply(BOB, "USDC", 10_000.0);
    let above_mid = t.try_borrow(BOB, "ETH", 3.825);
    assert_contract_error(above_mid, errors::INSUFFICIENT_COLLATERAL);
}

#[test]
fn refutation_third_party_cannot_borrow_on_victim() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    t.supply(ALICE, "USDC", 10_000.0);
    let alice_id = t.resolve_account_id(ALICE);
    let bob = t.get_or_create_user(BOB);
    let market = t.resolve_market("ETH");
    let raw = test_harness::f64_to_i128(0.1, market.decimals);
    let payments = soroban_sdk::vec![&t.env, (hub_asset(market.asset.clone()), raw)];
    let result = match t
        .ctrl_client()
        .try_borrow(&bob, &alice_id, &payments, &None)
    {
        Ok(res) => res.map(|_| ()).map_err(|e| e.into()),
        Err(e) => Err(e.expect("expected contract error, got InvokeError")),
    };
    assert_contract_error(result, errors::NOT_AUTHORIZED);
}
