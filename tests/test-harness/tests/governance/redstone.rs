//! RedStone source probing on the `configure_market_oracle` forwarder.

use governance::op::{AdminOperation, ConfigureAssetOracleArgs};
use soroban_sdk::String;
use test_harness::oracle::redstone::register_redstone_adapter;
use test_harness::{
    assert_contract_error, errors, usd, usdc_preset, LendingTest, DEFAULT_TOLERANCE,
};

fn try_configure_usdc(
    t: &LendingTest,
    cfg: &controller::types::AssetOracle,
) -> Result<(), soroban_sdk::Error> {
    let asset = t.resolve_market("USDC").asset.clone();
    let admin = t.admin();
    t.gov_client()
        .try_execute_immediate(
            &admin,
            &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
                key: controller::types::PriceKey::Token(asset),
                oracle: cfg.clone(),
            }),
        )
        .map(|inner| inner.map(|_| ()).map_err(|e| e.into()))
        .unwrap_or_else(|e| Err(e.expect("expected contract error")))
}

// A RedStone anchor stale window below the 60-second floor rejects
// InvalidStalenessConfig (#218).
#[test]
#[should_panic(expected = "Error(Contract, #218)")]
fn test_redstone_source_stale_window_rejects_invalid_config() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let asset = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);
    let admin = t.admin();

    let cfg = test_harness::reflector_primary_redstone_anchor_config_with_anchor_stale(
        &t.env,
        &t.mock_reflector,
        &asset,
        &redstone,
        &feed_id,
        30,
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
            key: controller::types::PriceKey::Token(asset),
            oracle: cfg,
        }),
    );
}

#[test]
fn test_redstone_stale_package_timestamp_rejects_config() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);
    let client = test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, &redstone);
    let now_ms = t.env.ledger().timestamp() * 1000;
    let stale_package = now_ms.saturating_sub(901_000);
    client.set_price_data(&feed_id, &usd(1), &stale_package, &now_ms);

    let cfg = test_harness::redstone_single_config(
        &t.env,
        &redstone,
        &feed_id,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    assert_contract_error(try_configure_usdc(&t, &cfg), errors::PRICE_FEED_STALE);
}

#[test]
fn test_redstone_stale_write_timestamp_rejects_config() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);
    let client = test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, &redstone);
    let now_ms = t.env.ledger().timestamp() * 1000;
    let stale_write = now_ms.saturating_sub(901_000);
    client.set_price_data(&feed_id, &usd(1), &now_ms, &stale_write);

    let cfg = test_harness::redstone_single_config(
        &t.env,
        &redstone,
        &feed_id,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    assert_contract_error(try_configure_usdc(&t, &cfg), errors::PRICE_FEED_STALE);
}

// A future timestamp is unreadable, not stale: `from_multi_feed` drops the
// payload, so the configure-time probe reports `NoLastPrice` (#210).
#[test]
fn test_redstone_future_timestamps_reject_config() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);
    let client = test_harness::mock_redstone::MockRedStonePriceFeedClient::new(&t.env, &redstone);
    let future_ms = (t.env.ledger().timestamp() + 120) * 1000;
    client.set_price_data(&feed_id, &usd(1), &future_ms, &future_ms);

    let cfg = test_harness::redstone_single_config(
        &t.env,
        &redstone,
        &feed_id,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    assert_contract_error(try_configure_usdc(&t, &cfg), errors::NO_LAST_PRICE);
}

// A feed id the adapter does not serve is caught by the configure-time
// containment probe: the read returns nothing and the config is refused rather
// than stored to revert on the first borrow.
//
// Reported as `NoLastPrice` (#210), not v1's `InvalidTicker` (#3). The adapter
// ABI answers "no data" the same way for an unknown feed id and a known one
// that has never published, so naming it `InvalidTicker` asserted a distinction
// neither version can actually make.
#[test]
fn test_redstone_missing_feed_id_rejects_config() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let configured_feed_id = String::from_str(&t.env, "ETH");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);

    let cfg = test_harness::redstone_single_config(
        &t.env,
        &redstone,
        &configured_feed_id,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    assert_contract_error(try_configure_usdc(&t, &cfg), errors::NO_LAST_PRICE);
}
