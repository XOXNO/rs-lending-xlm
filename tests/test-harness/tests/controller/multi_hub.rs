//! Multi-hub isolation keystone.
//!
//! Proves the core invariant of the multi-hub design: markets created on
//! distinct hubs are fully partitioned. The same asset listed on hub 1 and hub 2
//! has independent pool `State` (indices, totals, cash); positions never net
//! across hubs; bad-debt socialization on one hub does not touch the other; and
//! a hub's borrowable cash cannot be drawn by another hub.

use controller::constants::RAY;
use controller::types::{AccountPositionRaw, ControllerKey, DebtPositionRaw};
use soroban_sdk::{Bytes, Map};
use test_harness::{
    amount_raw, hub_asset, usd, usd_cents, HubAssetKey, LendingTest, MarketPreset,
    DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, HARNESS_HUB,
};
use test_harness::{eth_preset, usdc_preset, ALICE, BOB, CAROL, LIQUIDATOR};

const SECONDS_PER_YEAR: u64 = 365 * 24 * 60 * 60;

/// Reads the account's scaled debt on `(hub_id, asset)`; `0` when the borrow
/// position is absent (fully repaid positions are pruned from the map).
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

/// Reads the account's scaled supply on `(hub_id, asset)`; `0` when the supply
/// position is absent (fully seized positions are pruned from the map).
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

/// USDC market with no seeded liquidity, so utilization is driven purely by the
/// test's own supplies and borrows.
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

// 1. The same asset on two hubs keeps independent indices, totals, and cash.
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

    // List USDC on hub 2 as well; hub 1 already holds it from the builder.
    t.list_market_on_hub(hub2, "USDC", 0.0);

    // Hub 1: Alice self-collateralizes USDC and borrows against it.
    let a = t.supply_on_hub(HARNESS_HUB, ALICE, "USDC", 1_000.0);
    t.borrow_on_hub(HARNESS_HUB, ALICE, a, "USDC", 500.0);

    // Hub 2: Bob does the same, on the hub-2 USDC market.
    let b = t.supply_on_hub(hub2, BOB, "USDC", 1_000.0);
    t.borrow_on_hub(hub2, BOB, b, "USDC", 500.0);

    let s0 = t.pool_state_on_hub(HARNESS_HUB, "USDC");
    let s1 = t.pool_state_on_hub(hub2, "USDC");

    // Two distinct, non-empty markets exist for the same token.
    assert!(s0.borrowed > 0 && s1.borrowed > 0);
    assert_eq!(s0.borrow_index, RAY, "hub-1 index starts at RAY");
    assert_eq!(s1.borrow_index, RAY, "hub-2 index starts at RAY");

    // No netting: a hub-1 supply must not move hub-2's State at all.
    t.supply_on_hub(HARNESS_HUB, ALICE, "USDC", 250.0);
    let s1_after_hub1_op = t.pool_state_on_hub(hub2, "USDC");
    assert_eq!(s1_after_hub1_op.supplied, s1.supplied);
    assert_eq!(s1_after_hub1_op.borrowed, s1.borrowed);
    assert_eq!(s1_after_hub1_op.cash, s1.cash);
    assert_eq!(s1_after_hub1_op.supply_index, s1.supply_index);

    // Independent evolution: accrue ONLY hub 1 after a year. Hub 1's index moves;
    // hub 2's index is untouched because nothing accrued it.
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

    // Hub 2 accrues only when its own market is touched.
    t.accrue_on_hub(hub2, "USDC");
    let s1_accrued = t.pool_state_on_hub(hub2, "USDC");
    assert!(
        s1_accrued.borrow_index > RAY,
        "hub-2 borrow index accrues independently: {}",
        s1_accrued.borrow_index
    );
}

// 2. Bad-debt socialization on hub 1 writes down only hub 1's supply index.
#[test]
fn bad_debt_is_isolated_to_its_hub() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_market(eth_preset())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub2 = t.create_hub();
    t.list_market_on_hub(hub2, "USDC", 0.0);

    // Hub 2 has its own USDC suppliers who must be left untouched.
    t.supply_on_hub(hub2, CAROL, "USDC", 1_000.0);

    // Hub 1: Bob supplies the USDC that will absorb the loss; Alice posts a tiny
    // ETH collateral and borrows USDC.
    t.supply_on_hub(HARNESS_HUB, BOB, "USDC", 1_000.0);
    let a = t.supply_on_hub(HARNESS_HUB, ALICE, "ETH", 0.002); // ~$4 collateral
    t.borrow_on_hub(HARNESS_HUB, ALICE, a, "USDC", 2.0); // $2 debt, HF healthy

    let si0_before = t.pool_state_on_hub(HARNESS_HUB, "USDC").supply_index;
    let si1_before = t.pool_state_on_hub(hub2, "USDC").supply_index;

    // Crash ETH so Alice's collateral falls below the $5 bad-debt threshold.
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

// 3. A hub-1 borrow cannot draw on hub-2's cash.
#[test]
fn borrow_cannot_cross_hub_cash() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_market(eth_preset())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub2 = t.create_hub();
    // Hub 2 USDC holds ample cash that the hub-1 borrow must NOT be able to reach.
    t.list_market_on_hub(hub2, "USDC", 100_000.0);

    // Hub 1 USDC has only a thin slice of cash.
    t.supply_on_hub(HARNESS_HUB, BOB, "USDC", 100.0);

    // Alice posts ample ETH collateral on hub 1 so the health factor is not the
    // binding constraint.
    let a = t.supply_on_hub(HARNESS_HUB, ALICE, "ETH", 10.0); // $20,000 collateral

    // Control: a borrow within hub-1 cash succeeds, proving HF and collateral are fine.
    t.borrow_on_hub(HARNESS_HUB, ALICE, a, "USDC", 50.0);

    // The contested borrow size: more than hub-1 holds, less than hub-2 holds.
    let attempt_raw = amount_raw(1_000.0, 7);
    let hub1_cash = t.pool_state_on_hub(HARNESS_HUB, "USDC").cash;
    let hub2_cash = t.pool_state_on_hub(hub2, "USDC").cash;
    assert!(
        hub1_cash < attempt_raw && hub2_cash >= attempt_raw,
        "hub 1 holds less than the attempt ({}) while hub 2 holds at least it ({})",
        hub1_cash,
        hub2_cash
    );

    // A borrow that exceeds hub-1 cash reverts even though hub 2 holds far more,
    // and even though the collateral easily covers it.
    let result = t.try_borrow_on_hub(HARNESS_HUB, ALICE, a, "USDC", 1_000.0);
    assert!(
        result.is_err(),
        "hub-1 borrow exceeding hub-1 cash must revert despite hub-2 liquidity"
    );
}

// 4. swap_debt refinances a USDC debt from hub 1 to hub 2 (cross-hub). The
// borrow leg settles on hub 2, the repay leg on hub 1; same underlying token so
// the strategy nets without an aggregator swap.
#[test]
fn swap_debt_refinances_debt_across_hubs() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub2 = t.create_hub();
    // Hub 2 USDC must hold cash for the refinancing borrow.
    t.list_market_on_hub(hub2, "USDC", 100_000.0);

    // Hub 1: Alice self-collateralizes USDC and opens a USDC debt.
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

    // Refinance: borrow USDC on hub 2, repay the hub-1 USDC debt. A small buffer
    // above the 300 debt absorbs the flash fee; the over-repay is refunded.
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
    // Same-token net path never executes the swap and rejects a non-empty route.
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

    // The debt moved hubs: hub-1 USDC debt is cleared, hub-2 USDC debt carries it.
    assert_eq!(
        borrow_scaled_on_hub(&t, account_id, HARNESS_HUB, "USDC"),
        0,
        "hub-1 USDC debt is fully repaid by the refinance"
    );
    assert!(
        borrow_scaled_on_hub(&t, account_id, hub2, "USDC") > 0,
        "hub-2 USDC debt now carries the refinanced position"
    );

    // The two markets reflect the move: hub-1 borrowed drains to zero, hub-2
    // borrowed becomes non-zero.
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

// 5. A hub-2 account can be liquidated: its debt is repaid and its collateral
// seized, while a hub-1 market is left untouched. Guards the hub>0 liquidation
// plan path that previously keyed the repay/seize lookups to `{0, asset}` and so
// missed the real hub-2 positions, panicking `InternalError`.
#[test]
fn liquidation_repays_and_seizes_on_hub_one() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let hub2 = t.create_hub();
    // List both markets on hub 2. USDC needs no seed (Alice's own supply funds
    // the seizure); ETH is seeded so Alice can draw a borrow against it.
    t.list_market_on_hub(hub2, "USDC", 0.0);
    t.list_market_on_hub(hub2, "ETH", 100.0);

    // Hub-1 isolation control: Bob is a pure USDC supplier on hub 1 whose market
    // must not move when a hub-2 account is liquidated.
    t.supply_on_hub(HARNESS_HUB, BOB, "USDC", 1_000.0);
    let hub1_usdc_before = t.pool_state_on_hub(HARNESS_HUB, "USDC");

    // Hub 2: Alice posts USDC collateral and borrows ETH, mirroring the canonical
    // liquidatable USDC/ETH setup but entirely on hub 2.
    let alice = t.supply_on_hub(hub2, ALICE, "USDC", 10_000.0);
    t.borrow_on_hub(hub2, ALICE, alice, "ETH", 3.0);
    t.assert_healthy(ALICE);

    // Crash USDC so Alice's hub-2 position is liquidatable.
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    let debt_before = borrow_scaled_on_hub(&t, alice, hub2, "ETH");
    let collateral_before = supply_scaled_on_hub(&t, alice, hub2, "USDC");
    assert!(
        debt_before > 0 && collateral_before > 0,
        "precondition: hub-2 debt and collateral exist"
    );

    // The liquidator repays 1 ETH ($2000) of hub-2 debt.
    t.liquidate_on_hub(hub2, LIQUIDATOR, ALICE, "ETH", 1.0);

    // Repay leg hit the hub-2 debt key: scaled debt fell.
    let debt_after = borrow_scaled_on_hub(&t, alice, hub2, "ETH");
    assert!(
        debt_after < debt_before,
        "hub-2 ETH debt must be repaid: {} -> {}",
        debt_before,
        debt_after
    );

    // Seize leg hit the hub-2 supply key: scaled collateral fell and the
    // liquidator actually received the seized USDC.
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

    // Isolation: the hub-1 USDC market is untouched by a hub-2 liquidation.
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

// 6. MEDIUM-1: a hub-2 collateral whose hub-1 base listing is absent can still be
// seized. After delisting USDC from hub 1 (its token-rooted oracle and the hub-2
// market persist), the seizure must resolve the config under the position's hub.
// Before the fix the lookup keys `(hub_id: 0, asset)`, which is now absent, so
// the liquidation DoS-panics `AssetNotSupported`.
#[test]
fn liquidation_seizes_hub_one_collateral_without_hub_zero_listing() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let hub2 = t.create_hub();
    // USDC is the collateral on hub 2; ETH is the debt (seeded for the borrow).
    t.list_market_on_hub(hub2, "USDC", 0.0);
    t.list_market_on_hub(hub2, "ETH", 100.0);

    // Alice posts USDC collateral and borrows ETH on hub 2.
    let alice = t.supply_on_hub(hub2, ALICE, "USDC", 10_000.0);
    t.borrow_on_hub(hub2, ALICE, alice, "ETH", 3.0);
    t.assert_healthy(ALICE);

    // Delist USDC from hub 1's base spoke, leaving only the hub-2 listing. The
    // token-rooted oracle and the hub-2 market are untouched.
    t.ctrl_client()
        .remove_asset_from_spoke(&hub_asset(t.resolve_asset("USDC")), &1u32);

    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    let collateral_before = supply_scaled_on_hub(&t, alice, hub2, "USDC");
    assert!(
        collateral_before > 0,
        "precondition: hub-2 collateral exists"
    );

    // Before the fix the seizure resolves USDC under (hub 1, USDC) -- now absent
    // -- and DoS-panics `AssetNotSupported`. After the fix it reads hub 2.
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

// 7. MEDIUM-1: the seizure protocol fee is resolved from the collateral's own
// hub, not hub 1. Hub-1 USDC charges 0% and hub-2 USDC charges 20%, so the
// hub-2 seizure accrues a claimable fee only when the right hub config is read.
#[test]
fn liquidation_charges_seized_collateral_hub_fee() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market_config("USDC", |c| c.liquidation_fees = 0)
        .build();

    let hub2 = t.create_hub();
    // Hub-2 USDC carries a 20% liquidation fee; hub-1 USDC carries 0%.
    t.list_market_on_hub_with_fees(hub2, "USDC", 0.0, 2_000);
    t.list_market_on_hub(hub2, "ETH", 1_000.0);

    let alice = t.supply_on_hub(hub2, ALICE, "USDC", 10_000.0);
    t.borrow_on_hub(hub2, ALICE, alice, "ETH", 3.0);
    t.assert_healthy(ALICE);

    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    // No time advances, so the only protocol revenue the hub-2 USDC market can
    // accrue is the liquidation fee on the seized collateral.
    t.liquidate_on_hub(hub2, LIQUIDATOR, ALICE, "ETH", 1.0);

    let hub2_fee = t.claim_revenue_on_hub(hub2, "USDC");
    assert!(
        hub2_fee > 0,
        "hub-2 USDC seizure must accrue the hub-2 fee (20%), not hub-1's 0%: got {}",
        hub2_fee
    );
}

// 8. MEDIUM-2: the keeper index-update and revenue-claim verbs serve hub>0
// markets. Both forced the hub-1 coordinate before the fix, so a hub-2 market's
// index could never be accrued through the controller and its protocol revenue
// was unclaimable.
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

    // A real USDC supplier on hub 2 funds the borrow and keeps the market solvent.
    t.supply_on_hub(hub2, BOB, "USDC", 100_000.0);

    // Drive hub-2 USDC utilization: Alice posts ETH and borrows USDC on hub 2.
    let alice = t.supply_on_hub(hub2, ALICE, "ETH", 10.0); // $20,000 collateral
    t.borrow_on_hub(hub2, ALICE, alice, "USDC", 10_000.0);

    assert_eq!(
        t.pool_state_on_hub(hub2, "USDC").borrow_index,
        RAY,
        "hub-2 USDC index starts at RAY"
    );

    t.advance_time(SECONDS_PER_YEAR);

    // The controller keeper verb accrues the hub-2 index (hub-aware).
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

    // The controller revenue verb claims the hub-2 reserve revenue (hub-aware).
    let claimed = t.claim_revenue_on_hub(hub2, "USDC");
    assert!(
        claimed > 0,
        "hub-2 USDC protocol revenue must be claimable through the controller: got {}",
        claimed
    );
    // Isolation: the hub-1 USDC market accrued nothing to claim.
    assert_eq!(
        t.claim_revenue_on_hub(HARNESS_HUB, "USDC"),
        0,
        "hub-1 USDC has no revenue to claim"
    );
}

// 9. swap_collateral migrates a USDC collateral position from hub 1 to hub 2
// (cross-hub). Only an identical `(hub, asset)` leg is rejected — the same
// underlying token on a different hub nets without an aggregator swap,
// mirroring test 4's swap_debt refinance.
#[test]
fn swap_collateral_migrates_collateral_across_hubs() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub2 = t.create_hub();
    t.list_market_on_hub(hub2, "USDC", 0.0);

    // Hub 1: Alice supplies USDC collateral, no debt involved.
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

    // Migrate: withdraw the hub-1 USDC collateral, deposit it on hub 2.
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
    // Same-token net path never executes the swap and rejects a non-empty route.
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

    // The collateral moved hubs: hub-1 USDC collateral is cleared, hub-2 carries it.
    assert_eq!(
        supply_scaled_on_hub(&t, account_id, HARNESS_HUB, "USDC"),
        0,
        "hub-1 USDC collateral is fully withdrawn by the migration"
    );
    assert!(
        supply_scaled_on_hub(&t, account_id, hub2, "USDC") > 0,
        "hub-2 USDC collateral now carries the migrated position"
    );

    // The two markets reflect the move: hub-1 supplied drains to zero, hub-2
    // supplied becomes non-zero.
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
