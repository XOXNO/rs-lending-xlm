//! Multi-hub isolation keystone.
//!
//! Proves the core invariant of the multi-hub design: markets created on
//! distinct hubs are fully partitioned. The same asset listed on hub 0 and hub 1
//! has independent pool `State` (indices, totals, cash); positions never net
//! across hubs; bad-debt socialization on one hub does not touch the other; and
//! a hub's borrowable cash cannot be drawn by another hub.

use controller::constants::RAY;
use controller::types::{AccountPositionRaw, ControllerKey, DebtPositionRaw};
use soroban_sdk::Map;
use test_harness::{
    amount_raw, usd, usd_cents, HubAssetKey, LendingTest, MarketPreset, DEFAULT_ASSET_CONFIG,
    DEFAULT_MARKET_PARAMS,
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
            .get::<_, Map<HubAssetKey, DebtPositionRaw>>(&ControllerKey::BorrowPositions(account_id))
            .and_then(|m| m.get(key))
            .map(|p| p.scaled_amount_ray)
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
            .map(|p| p.scaled_amount_ray)
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

    let hub1 = t.create_hub();
    assert_eq!(hub1, 1, "first created hub id is 1 (hub 0 is the default)");

    // List USDC on hub 1 as well; hub 0 already holds it from the builder.
    t.list_market_on_hub(hub1, "USDC", 0.0);

    // Hub 0: Alice self-collateralizes USDC and borrows against it.
    let a = t.supply_on_hub(0, ALICE, "USDC", 1_000.0);
    t.borrow_on_hub(0, ALICE, a, "USDC", 500.0);

    // Hub 1: Bob does the same, on the hub-1 USDC market.
    let b = t.supply_on_hub(hub1, BOB, "USDC", 1_000.0);
    t.borrow_on_hub(hub1, BOB, b, "USDC", 500.0);

    let s0 = t.pool_state_on_hub(0, "USDC");
    let s1 = t.pool_state_on_hub(hub1, "USDC");

    // Two distinct, non-empty markets exist for the same token.
    assert!(s0.borrowed_ray > 0 && s1.borrowed_ray > 0);
    assert_eq!(s0.borrow_index_ray, RAY, "hub-0 index starts at RAY");
    assert_eq!(s1.borrow_index_ray, RAY, "hub-1 index starts at RAY");

    // No netting: a hub-0 supply must not move hub-1's State at all.
    t.supply_on_hub(0, ALICE, "USDC", 250.0);
    let s1_after_hub0_op = t.pool_state_on_hub(hub1, "USDC");
    assert_eq!(s1_after_hub0_op.supplied_ray, s1.supplied_ray);
    assert_eq!(s1_after_hub0_op.borrowed_ray, s1.borrowed_ray);
    assert_eq!(s1_after_hub0_op.cash, s1.cash);
    assert_eq!(s1_after_hub0_op.supply_index_ray, s1.supply_index_ray);

    // Independent evolution: accrue ONLY hub 0 after a year. Hub 0's index moves;
    // hub 1's index is untouched because nothing accrued it.
    t.advance_time(SECONDS_PER_YEAR);
    t.update_indexes_for(&["USDC"]);

    let s0_accrued = t.pool_state_on_hub(0, "USDC");
    let s1_idle = t.pool_state_on_hub(hub1, "USDC");
    assert!(
        s0_accrued.borrow_index_ray > RAY,
        "hub-0 borrow index accrued: {}",
        s0_accrued.borrow_index_ray
    );
    assert_eq!(
        s1_idle.borrow_index_ray, RAY,
        "hub-1 borrow index is untouched by hub-0 accrual"
    );

    // Hub 1 accrues only when its own market is touched.
    t.accrue_on_hub(hub1, "USDC");
    let s1_accrued = t.pool_state_on_hub(hub1, "USDC");
    assert!(
        s1_accrued.borrow_index_ray > RAY,
        "hub-1 borrow index accrues independently: {}",
        s1_accrued.borrow_index_ray
    );
}

// 2. Bad-debt socialization on hub 0 writes down only hub 0's supply index.
#[test]
fn bad_debt_is_isolated_to_its_hub() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_market(eth_preset())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub1 = t.create_hub();
    t.list_market_on_hub(hub1, "USDC", 0.0);

    // Hub 1 has its own USDC suppliers who must be left untouched.
    t.supply_on_hub(hub1, CAROL, "USDC", 1_000.0);

    // Hub 0: Bob supplies the USDC that will absorb the loss; Alice posts a tiny
    // ETH collateral and borrows USDC.
    t.supply_on_hub(0, BOB, "USDC", 1_000.0);
    let a = t.supply_on_hub(0, ALICE, "ETH", 0.002); // ~$4 collateral
    t.borrow_on_hub(0, ALICE, a, "USDC", 2.0); // $2 debt, HF healthy

    let si0_before = t.pool_state_on_hub(0, "USDC").supply_index_ray;
    let si1_before = t.pool_state_on_hub(hub1, "USDC").supply_index_ray;

    // Crash ETH so Alice's collateral falls below the $5 bad-debt threshold.
    t.set_price("ETH", usd(1));
    t.clean_bad_debt_by_id(a);

    let si0_after = t.pool_state_on_hub(0, "USDC").supply_index_ray;
    let si1_after = t.pool_state_on_hub(hub1, "USDC").supply_index_ray;

    assert!(
        si0_after < si0_before,
        "hub-0 USDC supply index is written down by socialized bad debt: {} -> {}",
        si0_before,
        si0_after
    );
    assert_eq!(
        si1_after, si1_before,
        "hub-1 USDC supply index is untouched by hub-0 bad debt"
    );
}

// 3. A hub-0 borrow cannot draw on hub-1's cash.
#[test]
fn borrow_cannot_cross_hub_cash() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_market(eth_preset())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub1 = t.create_hub();
    // Hub 1 USDC holds ample cash that the hub-0 borrow must NOT be able to reach.
    t.list_market_on_hub(hub1, "USDC", 100_000.0);

    // Hub 0 USDC has only a thin slice of cash.
    t.supply_on_hub(0, BOB, "USDC", 100.0);

    // Alice posts ample ETH collateral on hub 0 so the health factor is not the
    // binding constraint.
    let a = t.supply_on_hub(0, ALICE, "ETH", 10.0); // $20,000 collateral

    // Control: a borrow within hub-0 cash succeeds, proving HF and collateral are fine.
    t.borrow_on_hub(0, ALICE, a, "USDC", 50.0);

    // The contested borrow size: more than hub-0 holds, less than hub-1 holds.
    let attempt_raw = amount_raw(1_000.0, 7);
    let hub0_cash = t.pool_state_on_hub(0, "USDC").cash;
    let hub1_cash = t.pool_state_on_hub(hub1, "USDC").cash;
    assert!(
        hub0_cash < attempt_raw && hub1_cash >= attempt_raw,
        "hub 0 holds less than the attempt ({}) while hub 1 holds at least it ({})",
        hub0_cash,
        hub1_cash
    );

    // A borrow that exceeds hub-0 cash reverts even though hub 1 holds far more,
    // and even though the collateral easily covers it.
    let result = t.try_borrow_on_hub(0, ALICE, a, "USDC", 1_000.0);
    assert!(
        result.is_err(),
        "hub-0 borrow exceeding hub-0 cash must revert despite hub-1 liquidity"
    );
}

// 4. swap_debt refinances a USDC debt from hub 0 to hub 1 (cross-hub). The
// borrow leg settles on hub 1, the repay leg on hub 0; same underlying token so
// the strategy nets without an aggregator swap.
#[test]
fn swap_debt_refinances_debt_across_hubs() {
    let mut t = LendingTest::new()
        .with_market(usdc_no_seed())
        .with_min_borrow_collateral_disabled()
        .build();

    let hub1 = t.create_hub();
    // Hub 1 USDC must hold cash for the refinancing borrow.
    t.list_market_on_hub(hub1, "USDC", 100_000.0);

    // Hub 0: Alice self-collateralizes USDC and opens a USDC debt.
    let account_id = t.supply_on_hub(0, ALICE, "USDC", 1_000.0);
    t.borrow_on_hub(0, ALICE, account_id, "USDC", 300.0);

    assert!(
        borrow_scaled_on_hub(&t, account_id, 0, "USDC") > 0,
        "precondition: hub-0 USDC debt exists"
    );
    assert_eq!(
        borrow_scaled_on_hub(&t, account_id, hub1, "USDC"),
        0,
        "precondition: no hub-1 USDC debt yet"
    );

    // Refinance: borrow USDC on hub 1, repay the hub-0 USDC debt. A small buffer
    // above the 300 debt absorbs the flash fee; the over-repay is refunded.
    let usdc = t.resolve_asset("USDC");
    let existing_debt = HubAssetKey {
        hub_id: 0,
        asset: usdc.clone(),
    };
    let new_debt = HubAssetKey {
        hub_id: hub1,
        asset: usdc.clone(),
    };
    let caller = t.get_or_create_user(ALICE);
    // Same-token net path never executes the swap; the payload is inert.
    let steps = t.mock_swap_steps("USDC", "USDC", usd(1));
    let new_debt_raw = amount_raw(305.0, 7);
    t.ctrl_client().swap_debt(
        &caller,
        &account_id,
        &existing_debt,
        &new_debt_raw,
        &new_debt,
        &steps,
    );

    // The debt moved hubs: hub-0 USDC debt is cleared, hub-1 USDC debt carries it.
    assert_eq!(
        borrow_scaled_on_hub(&t, account_id, 0, "USDC"),
        0,
        "hub-0 USDC debt is fully repaid by the refinance"
    );
    assert!(
        borrow_scaled_on_hub(&t, account_id, hub1, "USDC") > 0,
        "hub-1 USDC debt now carries the refinanced position"
    );

    // The two markets reflect the move: hub-0 borrowed drains to zero, hub-1
    // borrowed becomes non-zero.
    assert_eq!(
        t.pool_state_on_hub(0, "USDC").borrowed_ray,
        0,
        "hub-0 USDC market has no borrows after the refinance"
    );
    assert!(
        t.pool_state_on_hub(hub1, "USDC").borrowed_ray > 0,
        "hub-1 USDC market holds the refinanced borrow"
    );
}

// 5. A hub-1 account can be liquidated: its debt is repaid and its collateral
// seized, while a hub-0 market is left untouched. Guards the hub>0 liquidation
// plan path that previously keyed the repay/seize lookups to `{0, asset}` and so
// missed the real hub-1 positions, panicking `InternalError`.
#[test]
fn liquidation_repays_and_seizes_on_hub_one() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    let hub1 = t.create_hub();
    // List both markets on hub 1. USDC needs no seed (Alice's own supply funds
    // the seizure); ETH is seeded so Alice can draw a borrow against it.
    t.list_market_on_hub(hub1, "USDC", 0.0);
    t.list_market_on_hub(hub1, "ETH", 100.0);

    // Hub-0 isolation control: Bob is a pure USDC supplier on hub 0 whose market
    // must not move when a hub-1 account is liquidated.
    t.supply_on_hub(0, BOB, "USDC", 1_000.0);
    let hub0_usdc_before = t.pool_state_on_hub(0, "USDC");

    // Hub 1: Alice posts USDC collateral and borrows ETH, mirroring the canonical
    // liquidatable USDC/ETH setup but entirely on hub 1.
    let alice = t.supply_on_hub(hub1, ALICE, "USDC", 10_000.0);
    t.borrow_on_hub(hub1, ALICE, alice, "ETH", 3.0);
    t.assert_healthy(ALICE);

    // Crash USDC so Alice's hub-1 position is liquidatable.
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);

    let debt_before = borrow_scaled_on_hub(&t, alice, hub1, "ETH");
    let collateral_before = supply_scaled_on_hub(&t, alice, hub1, "USDC");
    assert!(
        debt_before > 0 && collateral_before > 0,
        "precondition: hub-1 debt and collateral exist"
    );

    // The liquidator repays 1 ETH ($2000) of hub-1 debt.
    t.liquidate_on_hub(hub1, LIQUIDATOR, ALICE, "ETH", 1.0);

    // Repay leg hit the hub-1 debt key: scaled debt fell.
    let debt_after = borrow_scaled_on_hub(&t, alice, hub1, "ETH");
    assert!(
        debt_after < debt_before,
        "hub-1 ETH debt must be repaid: {} -> {}",
        debt_before,
        debt_after
    );

    // Seize leg hit the hub-1 supply key: scaled collateral fell and the
    // liquidator actually received the seized USDC.
    let collateral_after = supply_scaled_on_hub(&t, alice, hub1, "USDC");
    assert!(
        collateral_after < collateral_before,
        "hub-1 USDC collateral must be seized: {} -> {}",
        collateral_before,
        collateral_after
    );
    assert!(
        t.token_balance(LIQUIDATOR, "USDC") > 0.0,
        "liquidator must receive the seized hub-1 USDC collateral"
    );

    // Isolation: the hub-0 USDC market is untouched by a hub-1 liquidation.
    let hub0_usdc_after = t.pool_state_on_hub(0, "USDC");
    assert_eq!(
        hub0_usdc_after.supplied_ray, hub0_usdc_before.supplied_ray,
        "hub-0 USDC supplied is untouched"
    );
    assert_eq!(
        hub0_usdc_after.borrowed_ray, hub0_usdc_before.borrowed_ray,
        "hub-0 USDC borrowed is untouched"
    );
    assert_eq!(
        hub0_usdc_after.cash, hub0_usdc_before.cash,
        "hub-0 USDC cash is untouched"
    );
    assert_eq!(
        hub0_usdc_after.supply_index_ray, hub0_usdc_before.supply_index_ray,
        "hub-0 USDC supply index is untouched"
    );
    assert_eq!(
        hub0_usdc_after.borrow_index_ray, hub0_usdc_before.borrow_index_ray,
        "hub-0 USDC borrow index is untouched"
    );
}
