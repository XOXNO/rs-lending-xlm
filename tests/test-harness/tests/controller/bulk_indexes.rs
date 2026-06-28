//! Bulk market-index prefetch invariants.
//!
//! Controller flows seed the tx-local index cache through the pool's
//! `bulk_get_indexes` endpoint (the pool simulates accrual). These tests pin
//! that the bulk path returns exactly the indexes a per-asset `get_sync_data` +
//! `simulate_update_indexes` derives, and that unlisted assets keep their
//! pre-prefetch panic semantics.

use common::rates::simulate_update_indexes;
use controller::constants::MS_PER_SECOND;
use controller::types::MarketIndexRaw;
use soroban_sdk::testutils::Address as _;
use test_harness::{eth_preset, hub_asset, usdc_preset, LendingTest, ALICE, BOB};

#[test]
fn test_detailed_indexes_view_matches_pool_simulation() {
    // Two utilized markets accrue for a day; the controller view (which seeds
    // its cache via one bulk_get_indexes call) must report exactly the
    // indexes a native simulation over the raw pool state produces.
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();

    // Cross-utilize both markets so both accrue supplier rewards.
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "USDC", 1_000.0);
    t.borrow(ALICE, "ETH", 10.0);
    t.borrow(BOB, "USDC", 100.0);

    t.advance_time(86_400);

    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let now_ms = t.env.ledger().timestamp() * MS_PER_SECOND;

    let assets = soroban_sdk::vec![&t.env, usdc.clone(), eth.clone()];
    let views = t.ctrl_client().get_market_indexes_detailed(&assets);
    assert_eq!(views.len(), 2);

    for (i, asset) in [usdc, eth].iter().enumerate() {
        let pool = t.resolve_market_by_asset(asset).pool.clone();
        let sync =
            pool::LiquidityPoolClient::new(&t.env, &pool).get_sync_data(&hub_asset(asset.clone()));
        let expected = MarketIndexRaw::from(&simulate_update_indexes(&t.env, now_ms, &sync));
        let view = views.get_unchecked(i as u32);

        assert!(
            expected.borrow_index_ray > controller::constants::RAY,
            "market must have accrued for the equality to be meaningful"
        );
        assert_eq!(
            view.borrow_index_ray, expected.borrow_index_ray,
            "bulk-seeded borrow index must equal the lazy simulation"
        );
        assert_eq!(
            view.supply_index_ray, expected.supply_index_ray,
            "bulk-seeded supply index must equal the lazy simulation"
        );
    }
}

#[test]
fn test_index_view_with_unlisted_asset_still_fails() {
    // The prefetch skips assets without a market config, so an unlisted asset
    // reaches the lazy per-asset path and fails there exactly as it did
    // before the bulk endpoint existed.
    let t = LendingTest::new().with_market(usdc_preset()).build();

    let unlisted = soroban_sdk::Address::generate(&t.env);
    let assets = soroban_sdk::vec![&t.env, t.resolve_asset("USDC"), unlisted];
    let result = t.ctrl_client().try_get_market_indexes_detailed(&assets);
    assert!(result.is_err(), "unlisted asset must still fail the view");
}
