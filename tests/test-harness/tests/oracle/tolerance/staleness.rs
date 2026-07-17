use super::{enable_dual_source, set_dual_oracle_dex, setup};
use soroban_sdk::testutils::Ledger as _;
use test_harness::{assert_contract_error, errors, usd, LendingTest, ALICE};

fn age_oracle_observations(t: &LendingTest) {
    // Advance only wall-clock time. Advancing the ledger sequence as well would
    // expire the mock's temporary entries and test missing history (#212)
    // instead of stale observations (#206).
    t.env.ledger().with_mut(|ledger| ledger.timestamp += 1_000);
}


#[test]
fn test_stale_price_allows_supply_without_price_read() {
    let mut t = setup();

    // Supply first while the price is fresh.
    t.supply(ALICE, "USDC", 10_000.0);

    // Advance time beyond the staleness window (900 seconds) without
    // refreshing prices.
    age_oracle_observations(&t);

    // Supply is a risk-decreasing path and V2 emits no per-position oracle
    // price for pure supply analytics. Stale prices are still covered by the
    // strict borrow/withdraw/liquidation tests below.
    t.try_supply(ALICE, "USDC", 1_000.0)
        .expect("supply should not require a fresh oracle price");
}

#[test]
fn test_stale_price_blocks_borrow() {
    let mut t = setup();

    t.supply(ALICE, "USDC", 100_000.0);

    // Advance time beyond the staleness window without refreshing prices.
    age_oracle_observations(&t);

    // Borrow fails: stale price blocked for risk-increasing ops.
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert_contract_error(result, errors::PRICE_FEED_STALE);
}

#[test]
fn test_stale_price_blocks_withdraw_with_borrows() {
    let mut t = setup();

    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 10.0);

    // Advance time beyond the staleness window without refreshing.
    age_oracle_observations(&t);

    // Withdraw fails when borrows exist (risk-increasing).
    let result = t.try_withdraw(ALICE, "USDC", 1_000.0);
    assert_contract_error(result, errors::PRICE_FEED_STALE);
}

#[test]
fn test_missing_twap_history_blocks_strict_borrow() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");
    enable_dual_source(&t, "ETH");

    t.supply(ALICE, "USDC", 100_000.0);
    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &1);

    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert_contract_error(result, errors::REFLECTOR_HISTORY_EMPTY);
}

#[test]
fn test_missing_twap_history_allows_permissive_supply_fallback() {
    let mut t = setup();
    enable_dual_source(&t, "USDC");

    let usdc_asset = t.resolve_asset("USDC");
    t.mock_reflector_client()
        .set_twap_history_mode(&usdc_asset, &1);

    t.supply(ALICE, "USDC", 10_000.0);
    t.assert_supply_near(ALICE, "USDC", 10_000.0, 1.0);
}

#[test]
fn test_primary_anchor_stale_anchor_blocks_strict_borrow() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    let dex_oracle = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    let dex_client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex_oracle);
    let stale_ts = t.env.ledger().timestamp().saturating_sub(1_000);
    dex_client.set_price_at(&usdc_asset, &usd(1), &stale_ts);
    set_dual_oracle_dex(&t, "USDC", dex_oracle);

    t.supply(ALICE, "USDC", 100_000.0);
    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert_contract_error(result, errors::PRICE_FEED_STALE);
}

#[test]
fn test_dual_oracle_future_dex_reverts() {
    let mut t = setup();
    let usdc_asset = t.resolve_asset("USDC");
    let dex_oracle = t
        .env
        .register(test_harness::mock_reflector::MockReflector, ());
    let dex_client = test_harness::mock_reflector::MockReflectorClient::new(&t.env, &dex_oracle);
    let future_ts = t.env.ledger().timestamp() + 120;
    dex_client.set_price_at(&usdc_asset, &usd(1), &future_ts);

    t.supply(ALICE, "USDC", 100_000.0);
    set_dual_oracle_dex(&t, "USDC", dex_oracle);

    let result = t.try_borrow(ALICE, "ETH", 10.0);
    assert_contract_error(result, errors::PRICE_FEED_STALE);
}
