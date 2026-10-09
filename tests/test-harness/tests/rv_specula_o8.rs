//! Specula oracle MC-2 re-derivation: the per-call error cache in
//! `price-aggregator/src/engine.rs` (`resolve_nested`, the `cached_error` branch)
//! replays `OracleDepthExceeded` at a shallower position in the same call.
//!
//! The chain is `L4 -> L3 -> L2 -> L1 -> USDC`, one level past
//! `MAX_RESOLUTION_DEPTH` (3). Governance refuses it, so `L4` only exists via
//! the testing-only `seed_oracle`. Every assertion below pins the behaviour at
//! HEAD: the replay is visible only on the non-panicking `quotes` batch and only
//! when the over-deep root precedes the key in the batch; `prices` aborts the
//! whole call either way; and the cycle twin of the error is position-independent
//! so its replay is never a false verdict.

use common::types::MAX_RESOLUTION_DEPTH;
use controller::types::{AssetOracle, PriceKey, PriceSource, ScaledSource};
use governance::op::{AdminOperation, ConfigureAssetOracleArgs};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{vec, Address, String, Symbol, Vec};
use test_harness::errors::OracleError;
use test_harness::mock_reflector::{MockReflector, MockReflectorClient};
use test_harness::oracle::redstone::register_redstone_adapter;
use test_harness::{
    assert_contract_error, redstone_single_config, scaled_single_config, usd, LendingTest,
    DEFAULT_MAX_SANITY_PRICE_WAD,
};

const T0: u64 = 100_000;
const DEPTH: u32 = OracleError::OracleDepthExceeded as u32;
const CYCLE: u32 = OracleError::OracleCycleDetected as u32;

fn flat<T, E: Into<soroban_sdk::Error>>(
    result: Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<T, soroban_sdk::Error> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(err.into()),
        Err(Ok(err)) => Err(err),
        Err(Err(_)) => panic!("expected a contract error, got an InvokeError"),
    }
}

/// A mock Reflector that quotes `asset` at `price` (factor leg of a scaled source).
fn factor_reflector(t: &LendingTest, base: &Address, asset: &Address, price: i128) -> Address {
    let addr = t.env.register(MockReflector, ());
    let client = MockReflectorClient::new(&t.env, &addr);
    client.set_base_stellar(base);
    client.set_price(asset, &price);
    client.set_twap_price(asset, &price);
    addr
}

/// A fresh `Ref` key whose oracle is `factor(1.0) x quote`, the mainnet SolvBTC
/// shape (RedStone factor over a `Ref` quote). `Ref` keys carry
/// `asset_decimals = 0` (governance `resolve_oracle`, aggregator
/// `validation::asset_decimals`), so the seeded copy is set the same way. A
/// Reflector factor cannot quote a `Ref` key (`InvalidOracleBase`, 220).
fn scaled_on(t: &LendingTest, quote: &PriceKey, name: &str) -> (PriceKey, AssetOracle) {
    let feed_id = String::from_str(&t.env, "RATIO");
    let adapter = register_redstone_adapter(t, &[("RATIO", usd(1))]);
    let mut cfg = redstone_single_config(&t.env, &adapter, &feed_id, usd(1), 500);
    let PriceSource::Feed(factor) = cfg.sources.get_unchecked(0) else {
        unreachable!("redstone_single_config builds one feed source")
    };
    let mut sources: Vec<PriceSource> = vec![&t.env];
    sources.push_back(PriceSource::Scaled(ScaledSource {
        factor,
        quote: quote.clone(),
        min_factor_wad: 1,
        max_factor_wad: DEFAULT_MAX_SANITY_PRICE_WAD,
    }));
    cfg.sources = sources;
    cfg.asset_decimals = 0;
    (PriceKey::Ref(Symbol::new(&t.env, name)), cfg)
}

fn seed(t: &LendingTest, key: &PriceKey, oracle: &AssetOracle) {
    t.price_agg_client().seed_oracle(key, oracle);
}

fn try_configure(
    t: &LendingTest,
    key: &PriceKey,
    oracle: AssetOracle,
) -> Result<(), soroban_sdk::Error> {
    flat(t.gov_client().try_execute_immediate(
        &t.admin,
        &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
            key: key.clone(),
            oracle,
        }),
    ))
    .map(|_| ())
}

/// `quotes(keys)` error code per key, in batch order.
fn quote_codes(t: &LendingTest, keys: &[&PriceKey]) -> std::vec::Vec<Option<u32>> {
    let mut batch: Vec<PriceKey> = vec![&t.env];
    for key in keys {
        batch.push_back((*key).clone());
    }
    let statuses = t.price_agg_client().quotes(&batch);
    keys.iter()
        .map(|key| statuses.get((*key).clone()).expect("key quoted").error_code)
        .collect()
}

fn try_prices(t: &LendingTest, keys: &[&PriceKey]) -> Result<(), soroban_sdk::Error> {
    let mut batch: Vec<PriceKey> = vec![&t.env];
    for key in keys {
        batch.push_back((*key).clone());
    }
    flat(t.price_agg_client().try_prices(&batch)).map(|_| ())
}

/// USDC (feed) under a scaled chain `L1..L4`; `L4` is one level past the cap.
/// Returns the fixture, the USDC base address, and `[L1, L2, L3, L4]`.
fn over_deep_chain() -> (LendingTest, Address, [PriceKey; 4]) {
    assert_eq!(
        MAX_RESOLUTION_DEPTH, 3,
        "the chain below is sized for a cap of 3"
    );
    let t = LendingTest::new().standard_two_asset().build();
    t.env.ledger().with_mut(|li| li.timestamp = T0);
    t.refresh_oracle_prices();
    let usdc = t.resolve_asset("USDC");
    let mut quote = PriceKey::Token(usdc.clone());
    let mut keys: std::vec::Vec<PriceKey> = std::vec::Vec::new();
    for level in 1..=4 {
        let (key, cfg) = scaled_on(&t, &quote, &format!("L{level}"));
        if level < 4 {
            // L1..L3 are admitted by governance: USDC sits at depth 3 under L3.
            try_configure(&t, &key, cfg).unwrap_or_else(|e| panic!("L{level} admitted: {e:?}"));
        } else {
            // L4 puts USDC at depth 4. Governance refuses it (229); seed it in.
            assert_contract_error(try_configure(&t, &key, cfg.clone()), DEPTH);
            seed(&t, &key, &cfg);
        }
        keys.push(key.clone());
        quote = key;
    }
    let keys: [PriceKey; 4] = keys.try_into().expect("four levels");
    (t, usdc, keys)
}

/// MC-2 as claimed: `quotes([L4, L2])` reports 229 on `L2` although `L2` alone is
/// valid. The walk from `L4` reaches USDC at depth 4, caches `OracleDepthExceeded`
/// for USDC, and `L2`'s nested read of USDC at depth 2 replays it. Reversing the
/// batch order hides it, which is the signature of a stale per-call cache.
#[test]
fn rv_specula_o8_mc2_quotes_batch_replays_depth_error_at_shallower_position() {
    let (t, _usdc, [l1, l2, _l3, l4]) = over_deep_chain();

    assert_eq!(quote_codes(&t, &[&l2]), [None], "L2 alone prices (depth 2)");
    assert_eq!(quote_codes(&t, &[&l1]), [None], "L1 alone prices (depth 1)");
    assert_eq!(
        quote_codes(&t, &[&l4]),
        [Some(DEPTH)],
        "L4 alone is over-deep"
    );

    // Over-deep root first: the cached USDC error leaks into L2 and L1.
    assert_eq!(
        quote_codes(&t, &[&l4, &l2, &l1]),
        [Some(DEPTH), Some(DEPTH), Some(DEPTH)],
        "MC-2: cached depth error replayed at a shallower position"
    );
    // Valid keys first: each caches a price, and the later over-deep walk only
    // re-validates the cached path at its own depth (validate_cached_path).
    assert_eq!(
        quote_codes(&t, &[&l2, &l1, &l4]),
        [None, None, Some(DEPTH)],
        "batch order decides the verdict"
    );
}

/// The panicking path never diverges: `prices` aborts the whole call on the
/// over-deep root regardless of order, and a batch without it prices normally.
/// The controller only ever calls `prices` for valuation, so it cannot observe
/// the replay; only the `quotes` view (controller `get_all_market_indexes_detailed`
/// and off-chain readers) can.
#[test]
fn rv_specula_o8_mc2_prices_batch_aborts_on_the_root_regardless_of_order() {
    let (t, _usdc, [l1, l2, _l3, l4]) = over_deep_chain();

    assert!(
        try_prices(&t, &[&l2, &l1]).is_ok(),
        "well-formed subtree prices"
    );
    assert_contract_error(try_prices(&t, &[&l4, &l2]), DEPTH);
    assert_contract_error(try_prices(&t, &[&l2, &l4]), DEPTH);
}

/// The cycle twin is not position-dependent: a key in (or depending on) a cycle is
/// unresolvable at every depth, so replaying `OracleCycleDetected` from the cache
/// never contradicts a fresh resolution. `D -> ETH -> USDC -> ETH`: `D` reports
/// 225 alone and in any batch order.
#[test]
fn rv_specula_o8_mc2_cycle_error_replay_is_position_independent() {
    let t = LendingTest::new().standard_two_asset().build();
    t.env.ledger().with_mut(|li| li.timestamp = T0);
    t.refresh_oracle_prices();
    let usdc = t.resolve_asset("USDC");
    let eth = t.resolve_asset("ETH");
    let eth_key = PriceKey::Token(eth.clone());
    let usdc_key = PriceKey::Token(usdc.clone());

    let factor_eth = factor_reflector(&t, &usdc, &eth, usd(2_000));
    let eth_cfg =
        scaled_single_config(&t.env, &factor_eth, &eth, usdc_key.clone(), usd(2_000), 500);
    let factor_usdc = factor_reflector(&t, &eth, &usdc, usd(1) / 2_000);
    let usdc_cfg = scaled_single_config(
        &t.env,
        &factor_usdc,
        &usdc,
        eth_key.clone(),
        usd(1) / 2_000,
        500,
    );
    seed(&t, &usdc_key, &usdc_cfg);
    seed(&t, &eth_key, &eth_cfg);
    let (d, d_cfg) = scaled_on(&t, &eth_key, "D");
    seed(&t, &d, &d_cfg);

    assert_eq!(quote_codes(&t, &[&d]), [Some(CYCLE)]);
    assert_eq!(quote_codes(&t, &[&eth_key, &d]), [Some(CYCLE), Some(CYCLE)]);
    assert_eq!(quote_codes(&t, &[&d, &eth_key]), [Some(CYCLE), Some(CYCLE)]);
    assert_contract_error(try_prices(&t, &[&d]), CYCLE);
}
