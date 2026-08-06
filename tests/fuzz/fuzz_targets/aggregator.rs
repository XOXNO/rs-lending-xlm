#![no_main]

use arbitrary::Arbitrary;
use common::constants::{BPS, WAD};
use common::math::fp_core::mul_div_half_up;
use common::types::{
    AssetOracle, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleTolerance,
    PriceFeedRaw, PriceKey, PriceSource, ProviderRef,
};
use libfuzzer_sys::fuzz_target;
use mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient};
use price_aggregator::{PriceAggregator, PriceAggregatorClient};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{vec, Address, Env, String};

const NOW: u64 = 1_000_000;
const CEILING: u64 = 3_600;
const PRICE_HI: i128 = 4 * WAD;
const MAX_EXTRA_TOL_BPS: i128 = 5_000;

#[derive(Debug, Arbitrary)]
struct In {
    price_a: i128,
    price_b: i128,
    age_a: i16,
    age_b: i16,
    tol_extra_bps: u16,
    min_sanity: i128,
    max_sanity: i128,
    dual: bool,
}

fn in_range(v: i128, hi: i128) -> i128 {
    v.rem_euclid(hi + 1)
}

fn ts_ms(age: i16) -> u64 {
    let span = 3 * CEILING as i64 + 1;
    let age = (age as i64).rem_euclid(span) - CEILING as i64;
    ((NOW as i64 - age) as u64) * 1_000
}

fn tolerance(env: &Env, extra_bps: u16) -> OracleTolerance {
    let upper = BPS + (extra_bps as i128 % (MAX_EXTRA_TOL_BPS + 1));
    let lower = mul_div_half_up(env, BPS, BPS, upper);
    OracleTolerance {
        upper_ratio_bps: upper as u32,
        lower_ratio_bps: lower as u32,
    }
}

fn feed(env: &Env, adapter: &Address, id: &str) -> PriceSource {
    PriceSource::Feed(FeedSource {
        provider: ProviderRef::RedStone(MultiFeedRef {
            contract: adapter.clone(),
            feed_id: String::from_str(env, id),
            nature: FeedNature::Fundamental,
        }),
        decimals: 8,
        max_stale_seconds: CEILING,
    })
}

fn config(env: &Env, adapter: &Address, ids: &[&str], i: &In, min: i128, max: i128) -> AssetOracle {
    let mut sources = soroban_sdk::Vec::new(env);
    for id in ids {
        sources.push_back(feed(env, adapter, id));
    }
    AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: CEILING,
        sources,
        tolerance: tolerance(env, i.tol_extra_bps),
        independence: IndependencePolicy::AllowShared(vec![env, adapter.clone()]),
        min_sanity_price_wad: min,
        max_sanity_price_wad: max,
    }
}

fn try_price(c: &PriceAggregatorClient, k: &PriceKey) -> Option<PriceFeedRaw> {
    c.try_price(k).ok().and_then(Result::ok)
}

fn feed_id(f: &PriceFeedRaw) -> (i128, u32, u64) {
    (f.price_wad, f.asset_decimals, f.timestamp)
}

fuzz_target!(|i: In| {
    let env = Env::default();
    env.ledger().set_timestamp(NOW);

    let owner = Address::generate(&env);
    let agg = PriceAggregatorClient::new(&env, &env.register(PriceAggregator, (owner,)));
    let adapter = env.register(MockRedStonePriceFeed, ());
    let mock = MockRedStonePriceFeedClient::new(&env, &adapter);

    let pa = in_range(i.price_a, PRICE_HI);
    let pb = in_range(i.price_b, PRICE_HI);
    mock.set_price_data(&String::from_str(&env, "A"), &pa, &ts_ms(i.age_a), &ts_ms(i.age_a));
    mock.set_price_data(&String::from_str(&env, "B"), &pb, &ts_ms(i.age_b), &ts_ms(i.age_b));

    let min = in_range(i.min_sanity, PRICE_HI);
    let max = in_range(i.max_sanity, PRICE_HI);

    let ids_fwd: &[&str] = if i.dual { &["A", "B"] } else { &["A"] };
    let key1 = PriceKey::Token(Address::generate(&env));
    agg.seed_oracle(&key1, &config(&env, &adapter, ids_fwd, &i, min, max));

    let q1 = agg.quote(&key1);
    let r1 = try_price(&agg, &key1);
    assert_eq!(q1.valid, r1.is_some(), "quote.valid must match price resolvability");

    if let Some(f1) = &r1 {
        assert_eq!(f1.price_wad, q1.final_wad, "usable price != quote.final_wad");
        assert!(
            min <= f1.price_wad && f1.price_wad <= max,
            "resolved price {} outside sanity band [{}, {}]",
            f1.price_wad,
            min,
            max
        );
        let (lo, hi) = agg.price_spread(&key1);
        assert!(lo <= hi, "price_spread not ordered: ({}, {})", lo, hi);
        assert!(
            lo <= f1.price_wad && f1.price_wad <= hi,
            "price {} outside spread [{}, {}]",
            f1.price_wad,
            lo,
            hi
        );
    }

    if i.dual {
        let key2 = PriceKey::Token(Address::generate(&env));
        agg.seed_oracle(&key2, &config(&env, &adapter, &["B", "A"], &i, min, max));
        let q2 = agg.quote(&key2);
        assert_eq!(q1.valid, q2.valid, "source order changed usability");
        let r2 = try_price(&agg, &key2);
        assert_eq!(
            r1.as_ref().map(feed_id),
            r2.as_ref().map(feed_id),
            "source order changed the price"
        );
        if r1.is_some() {
            assert_eq!(
                agg.price_spread(&key1),
                agg.price_spread(&key2),
                "source order changed the spread"
            );
        }
    }
});
