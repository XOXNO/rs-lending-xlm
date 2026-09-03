//! GH-21. The same token listed on two hubs can be looped through `multiply`
//! in `Multiply` mode: borrow on one hub, deposit on the other. Every book
//! must stay consistent and the loop must unwind cleanly.

use common::types::HubAssetKey;
use controller::constants::WAD;
use controller::types::PositionMode;
use soroban_sdk::{vec, Bytes, Vec};
use test_harness::{
    usd, LendingTest, MarketPreset, ALICE, BOB, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS,
    HARNESS_HUB, HARNESS_SPOKE,
};

const YEAR_SECS: u64 = 365 * 86_400;

fn usdc() -> MarketPreset {
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
fn borrowing_usdc_on_hub_two_against_usdc_on_hub_one_keeps_both_books_straight() {
    let mut t = LendingTest::new()
        .with_market(usdc())
        .with_min_borrow_collateral_disabled()
        .build();
    let hub2 = t.create_hub();
    t.list_market_on_hub(hub2, "USDC", 100_000.0);
    t.supply_on_hub(HARNESS_HUB, BOB, "USDC", 100_000.0);
    let account = t.create_account_full(ALICE, HARNESS_SPOKE, PositionMode::Multiply);
    t.supply_to(ALICE, account, "USDC", 1_000.0);

    let usdc = t.resolve_asset("USDC");
    let collateral = HubAssetKey {
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
    };
    let debt = HubAssetKey {
        hub_id: hub2,
        asset: usdc.clone(),
    };
    let alice = t.get_or_create_user(ALICE);
    let unit = 10_000_000i128;
    t.ctrl_client().multiply(
        &alice,
        &account,
        &HARNESS_SPOKE,
        &collateral,
        &(500 * unit),
        &debt,
        &PositionMode::Multiply,
        &Bytes::new(&t.env),
        &None,
        &None,
    );

    let (supplies, debts) = t.ctrl_client().get_account_positions(&account);
    assert!(supplies.get(collateral.clone()).unwrap().scaled_amount > 0);
    assert!(debts.get(debt.clone()).unwrap().scaled_amount > 0);
    let hf = t.health_factor_for_raw(ALICE, account);
    // 0.8 * 1_500 / 500 = 2.4, minus the origination fee on the deposit leg.
    assert!(hf > 2 * WAD && hf < 3 * WAD, "hf {hf}");

    let s1 = t.pool_state_on_hub(HARNESS_HUB, "USDC");
    let s2 = t.pool_state_on_hub(hub2, "USDC");
    assert_eq!(
        s2.borrowed,
        debts.get(debt.clone()).unwrap().scaled_amount,
        "hub-2 debt is exactly the loop's debt"
    );
    assert_eq!(s1.borrowed, 0, "hub-1 has no borrowers");

    t.advance_time(YEAR_SECS);
    t.update_indexes_on_hub(hub2, &["USDC"]);
    t.update_indexes_on_hub(HARNESS_HUB, &["USDC"]);
    let s2_after = t.pool_state_on_hub(hub2, "USDC");
    assert!(
        s2_after.borrow_index > s2.borrow_index,
        "interest accrues on the borrowing hub"
    );
    let s1_after = t.pool_state_on_hub(HARNESS_HUB, "USDC");
    assert_eq!(
        s1_after.supply_index, s1.supply_index,
        "no borrowers on hub one, no supplier yield"
    );

    // Unwind: repay hub-2 debt with one unit of overpayment, withdraw hub-1 collateral.
    let debt_units = t.ctrl_client().get_borrow_amount(&account, &debt) + 1;
    t.resolve_market("USDC")
        .token_admin
        .mint(&alice, &debt_units);
    let repay: Vec<(HubAssetKey, i128)> = vec![&t.env, (debt.clone(), debt_units)];
    t.ctrl_client().repay(&alice, &account, &repay);
    let withdraw: Vec<(HubAssetKey, i128)> = vec![&t.env, (collateral.clone(), 0)];
    t.ctrl_client().withdraw(&alice, &account, &withdraw, &None);
    assert!(
        !t.account_exists(account),
        "the loop unwinds to an empty, burned account"
    );
    assert_eq!(t.pool_state_on_hub(hub2, "USDC").borrowed, 0);
}
