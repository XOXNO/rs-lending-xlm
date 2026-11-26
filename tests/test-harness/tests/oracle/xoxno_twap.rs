//! The real XOXNO oracle read through the real price aggregator. A Reflector
//! `Twap` leg on its SEP-40 history reads at any signer cadence, and the
//! multi-feed spot leg (`ProviderRef::Xoxno`) serves the stored aggregate.
use common::oracle::providers::reflector::ReflectorAsset;
use controller::types::{PriceKey, PriceSource, PriceStatus};
use soroban_sdk::{vec, Address, String};
use test_harness::mock_redstone::MockRedStonePriceFeedClient;
use test_harness::oracle::redstone::register_redstone_adapter;
use test_harness::oracle::xoxno::register_xoxno_adapter;
use test_harness::{
    reflector_primary_redstone_anchor_config, usd, usdc_preset, xoxno_single_config, LendingTest,
    DEFAULT_TOLERANCE,
};
use xoxno_oracle::XoxnoOracleClient;

/// The oracle's resolution in the harness, in seconds.
const RESOLUTION: u64 = 300;

/// Scales an 8-decimal oracle price to WAD.
const RAW_TO_WAD: i128 = 10_000_000_000;

struct Fx {
    t: LendingTest,
    usdc: Address,
    redstone: Address,
    xoxno: Address,
    signers: std::vec::Vec<Address>,
    feed_id: String,
}

impl Fx {
    fn oracle(&self) -> XoxnoOracleClient<'_> {
        XoxnoOracleClient::new(&self.t.env, &self.xoxno)
    }

    fn quote(&self) -> PriceStatus {
        let key = PriceKey::Token(self.usdc.clone());
        self.t
            .price_agg_client()
            .quotes(&vec![&self.t.env, key.clone()])
            .get(key)
            .unwrap()
    }

    /// Runs `rounds` rounds `cadence` seconds apart. Every signer submits
    /// `price(round)` with the ledger time as package time. Returns the
    /// `(bucket start, price)` of the last round in each bucket, oldest first.
    fn run(&self, cadence: u64, rounds: u32, price: impl Fn(u32) -> i128) -> Vec<(u64, i128)> {
        let env = &self.t.env;
        let oracle = self.oracle();
        let mut closes: Vec<(u64, i128)> = Vec::new();
        for round in 0..rounds {
            self.t.advance_time_no_refresh(cadence);
            let now = env.ledger().timestamp();
            for signer in self.signers.iter() {
                oracle.submit_price(signer, &self.feed_id, &price(round), &(now * 1_000));
            }
            let bucket = now / RESOLUTION * RESOLUTION;
            match closes.last_mut() {
                Some(last) if last.0 == bucket => last.1 = price(round),
                _ => closes.push((bucket, price(round))),
            }
        }
        closes
    }
}

/// Admits `Twap(3)` on the real XOXNO oracle (resolution 300 s) beside a
/// RedStone anchor. Every leg allows 3 600 s of staleness.
fn admit_twap() -> Fx {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let usdc = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let redstone = register_redstone_adapter(&t, &[("USDC", usd(1))]);
    let (xoxno, signers) = register_xoxno_adapter(&t, &[("USDC", usd(1))], 3, 2);
    XoxnoOracleClient::new(&t.env, &xoxno)
        .add_feed(&feed_id, &ReflectorAsset::Stellar(usdc.clone()));

    let mut cfg = reflector_primary_redstone_anchor_config(
        &t.env,
        &xoxno,
        &usdc,
        &redstone,
        &feed_id,
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    cfg.max_price_stale_seconds = 3_600;
    for index in 0..2 {
        if let PriceSource::Feed(mut feed) = cfg.sources.get_unchecked(index) {
            feed.max_stale_seconds = 3_600;
            feed.decimals = 8;
            cfg.sources.set(index, PriceSource::Feed(feed));
        }
    }
    // Base USD, 8 decimals, resolution 300 s, 3 * 300 <= 3 600.
    t.configure_market_oracle(&usdc, &cfg);
    Fx {
        t,
        usdc,
        redstone,
        xoxno,
        signers,
        feed_id,
    }
}

/// One hour of 60 s rounds with a rising price. The `Twap(3)` leg serves the
/// mean of the closes of the newest three buckets.
#[test]
fn xoxno_twap_leg_reads_the_newest_three_buckets_at_a_60_second_cadence() {
    let fx = admit_twap();
    // 1.000 USD rising by 0.0003 USD a round. Every price is a multiple of
    // three, so the mean of three closes is exact.
    let closes = fx.run(60, 60, |round| 100_000_000 + 30_000 * i128::from(round));
    let newest: std::vec::Vec<(u64, i128)> = closes.iter().rev().take(3).copied().collect();
    let mean_raw = newest.iter().map(|close| close.1).sum::<i128>() / 3;
    assert_eq!(
        mean_raw * 3,
        newest.iter().map(|close| close.1).sum::<i128>()
    );

    // The oracle answers with the same three rounds.
    let rounds = fx
        .oracle()
        .prices(&ReflectorAsset::Stellar(fx.usdc.clone()), &3)
        .expect("history");
    let served: std::vec::Vec<(u64, i128)> = rounds
        .iter()
        .map(|round| (round.timestamp, round.price))
        .collect();
    assert_eq!(served, newest);

    MockRedStonePriceFeedClient::new(&fx.t.env, &fx.redstone)
        .set_price(&fx.feed_id, &(mean_raw * RAW_TO_WAD));
    let status = fx.quote();
    assert_eq!(status.error_code, None);
    assert!(status.valid, "the Twap leg reads at a 60 s cadence");
    assert_eq!(status.primary_wad, mean_raw * RAW_TO_WAD);
    assert_eq!(status.final_wad, mean_raw * RAW_TO_WAD);
}

/// The `Twap(3)` leg reads at a cadence below, equal to and above the
/// resolution.
#[test]
fn xoxno_twap_leg_reads_at_every_signer_cadence() {
    for (cadence, rounds) in [(30u64, 120u32), (60, 60), (150, 24), (300, 12), (400, 9)] {
        let fx = admit_twap();
        fx.run(cadence, rounds, |_| 100_000_000);
        MockRedStonePriceFeedClient::new(&fx.t.env, &fx.redstone).set_price(&fx.feed_id, &usd(1));
        let status = fx.quote();
        assert_eq!(status.error_code, None, "cadence {cadence}");
        assert!(status.valid, "cadence {cadence}");
        assert_eq!(status.primary_wad, usd(1), "cadence {cadence}");
    }
}

/// With fewer than three buckets after the first quorum, the leg fails closed and
/// the anchor alone does not price the key.
#[test]
fn xoxno_twap_leg_fails_closed_until_three_buckets_exist() {
    let fx = admit_twap();
    // The adapter registration wrote the first bucket. Two more rounds inside
    // the same bucket add no round.
    let now = fx.t.env.ledger().timestamp();
    let into_bucket = now % RESOLUTION;
    let step = (RESOLUTION - 1 - into_bucket) / 2;
    fx.run(step, 2, |_| 100_000_000);
    assert_eq!(
        fx.oracle()
            .prices(&ReflectorAsset::Stellar(fx.usdc.clone()), &3)
            .map(|rounds| rounds.len()),
        Some(1)
    );
    MockRedStonePriceFeedClient::new(&fx.t.env, &fx.redstone).set_price(&fx.feed_id, &usd(1));
    let status = fx.quote();
    assert!(!status.valid);
    assert_eq!(status.primary_wad, 0, "the Twap leg has no reading");
    assert_eq!(status.error_code, Some(205), "UnsafePriceNotAllowed");
}

/// The multi-feed spot leg reads the stored aggregate at a 60 s cadence. The
/// aggregate is the lower median of the cluster, whatever the history holds.
#[test]
fn xoxno_spot_leg_serves_the_stored_aggregate_at_a_60_second_cadence() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let usdc = t.resolve_asset("USDC");
    let feed_id = String::from_str(&t.env, "USDC");
    let (xoxno, signers) = register_xoxno_adapter(&t, &[("USDC", usd(1))], 3, 2);
    let oracle = XoxnoOracleClient::new(&t.env, &xoxno);
    let cfg = xoxno_single_config(
        &t.env,
        &xoxno,
        &feed_id,
        usd(1),
        DEFAULT_TOLERANCE.tolerance_bps,
    );
    t.configure_market_oracle(&usdc, &cfg);
    let key = PriceKey::Token(usdc.clone());

    for round in 0..30i128 {
        t.advance_time_no_refresh(60);
        let now = t.env.ledger().timestamp();
        // Signer 0 lags 40 s, signer 1 lags 10 s, signer 2 is current. The
        // lower median is signer 1's price.
        let base = 100_000_000 + 1_000 * round;
        let lags = [(40u64, base + 20), (10, base + 10), (0, base)];
        for (signer, (lag, price)) in signers.iter().zip(lags) {
            oracle.submit_price(signer, &feed_id, &price, &((now - lag) * 1_000));
        }

        let aggregate = oracle.read_price_data_for_feed(&feed_id);
        assert_eq!(aggregate.price.to_u128(), Some((base + 10) as u128));
        assert_eq!(aggregate.package_timestamp, (now - 40) * 1_000);
        assert_eq!(aggregate.write_timestamp, now * 1_000);

        let status = t
            .price_agg_client()
            .quotes(&vec![&t.env, key.clone()])
            .get(key.clone())
            .unwrap();
        assert!(status.valid, "round {round}");
        assert_eq!(status.primary_wad, (base + 10) * RAW_TO_WAD);
        assert_eq!(status.price_timestamp, now - 40);
    }
}
