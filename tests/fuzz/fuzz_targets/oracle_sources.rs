#![no_main]
//! Drives the price aggregator through every provider (Reflector spot and TWAP,
//! RedStone, Xoxno, Chainlink) and every source shape (feed, scaled, Aquarius
//! constant-product and stable LP, dual blends), through both the validating
//! `set_oracle` path and the unvalidated `seed_oracle` path.

use arbitrary::Arbitrary;
use common::constants::{BPS, MAX_REASONABLE_PRICE_WAD, MAX_TOLERANCE, MIN_TOLERANCE, WAD};
use common::math::fp_core::mul_div_half_up;
use common::oracle::observation::{
    MAX_FUTURE_SKEW_SECONDS, MAX_ORACLE_DECIMALS, MAX_PRICE_STALE_SECONDS, MAX_TWAP_RECORDS,
    MIN_ORACLE_RESOLUTION_SECONDS, MIN_PRICE_STALE_SECONDS,
};
use common::oracle::providers::chainlink::{ChainlinkRound, CHAINLINK_DECIMALS};
use common::oracle::providers::redstone::{RedStonePriceData, REDSTONE_DECIMALS};
use common::oracle::providers::reflector::{ReflectorAsset, ReflectorPriceData};
use common::types::{
    AquariusLpSource, AssetOracle, ChainlinkFeedRef, FeedNature, FeedSource, IndependencePolicy,
    MultiFeedRef, OracleAssetRef, OracleReadMode, OracleTolerance, PriceFeedRaw, PriceKey,
    PriceSource, PriceStatus, ProviderRef, ReflectorFeedRef, ScaledSource,
};
use libfuzzer_sys::fuzz_target;
use price_aggregator::{PriceAggregator, PriceAggregatorClient};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{vec, Address, BytesN, Env, String, Symbol, Vec, I256, U256};
use stellar_fuzz::edge_biased_amount;
use stellar_fuzz::oracle_mocks::{
    FuzzAquarius, FuzzAquariusClient, FuzzChainlink, FuzzChainlinkClient, FuzzMultiFeed,
    FuzzMultiFeedClient, FuzzReflector, FuzzReflectorClient, FuzzToken, FuzzTokenClient,
};

const NOW: u64 = 1_000_000;

// Fault selectors; `corrupt % 16` below 8 leaves the feed well-formed.
const C_ZERO: u8 = 8;
const C_BAD_VALUE: u8 = 9;
const C_MISSING: u8 = 10;
const C_PANIC: u8 = 11;
const C_DECIMALS: u8 = 12;
const C_STALE: u8 = 13;
const C_FUTURE: u8 = 14;
const C_STALE_CFG: u8 = 15;

#[derive(Clone, Debug, Arbitrary)]
struct FeedIn {
    kind: u8,
    wad: u64,
    jitter_bps: i16,
    age_s: u16,
    future: bool,
    corrupt: u8,
    dec_raw: u8,
    stale_raw: u32,
    twap_records: u8,
    twap_shape: u8,
    resolution_raw: u16,
    base_sel: u8,
    market: bool,
}

#[derive(Debug, Arbitrary)]
struct LpIn {
    reserve_a: u64,
    reserve_b: u64,
    shares: u64,
    amp: u32,
    dec_a: u8,
    dec_b: u8,
    dec_share: u8,
    corrupt: u8,
    min_value: u8,
}

#[derive(Debug, Arbitrary)]
struct In {
    feeds: [FeedIn; 4],
    lp: LpIn,
    shape: u8,
    anchor: u64,
    tol_extra: u16,
    band_bps: u16,
    band_shift_bps: i16,
    independence: u8,
    factor_band_bps: u16,
    shared: bool,
    ops: [(u8, u16, i16); 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    ReflectorSpot,
    ReflectorTwap,
    RedStone,
    Xoxno,
    Chainlink,
}

impl Kind {
    fn of(sel: u8) -> Self {
        match sel % 5 {
            0 => Kind::ReflectorSpot,
            1 => Kind::ReflectorTwap,
            2 => Kind::RedStone,
            3 => Kind::Xoxno,
            _ => Kind::Chainlink,
        }
    }

    fn slot(self) -> usize {
        match self {
            Kind::ReflectorSpot | Kind::ReflectorTwap => 0,
            Kind::RedStone => 1,
            Kind::Xoxno => 2,
            Kind::Chainlink => 3,
        }
    }
}

/// Mock contracts registered so far, plus one reusable instance per provider slot.
struct Registry {
    shared: bool,
    reuse: [Option<Address>; 4],
    all: std::vec::Vec<Address>,
}

impl Registry {
    fn contract(
        &mut self,
        env: &Env,
        slot: usize,
        register: impl FnOnce(&Env) -> Address,
    ) -> Address {
        if self.shared {
            if let Some(existing) = &self.reuse[slot] {
                return existing.clone();
            }
        }
        let addr = register(env);
        self.reuse[slot] = Some(addr.clone());
        self.all.push(addr.clone());
        addr
    }
}

fn pow10(e: u32) -> i128 {
    10i128.pow(e)
}

fn scale_wad(wad: i128, decimals: u32) -> i128 {
    if decimals <= 18 {
        wad / pow10(18 - decimals)
    } else {
        wad.saturating_mul(pow10(decimals - 18))
    }
}

fn raw_price(wad: i128, decimals: u32, corrupt: u8) -> i128 {
    match corrupt {
        C_ZERO => 0,
        C_BAD_VALUE => -scale_wad(wad, decimals).max(1),
        _ => scale_wad(wad, decimals).max(1),
    }
}

fn jitter(value: i128, bps: i16) -> i128 {
    let delta = i128::from(bps) % 3_000;
    (value.saturating_mul(BPS + delta) / BPS).max(1)
}

fn scaled_price(factor_wad: i128, quote_wad: i128) -> i128 {
    factor_wad
        .checked_mul(quote_wad)
        .map_or(MAX_REASONABLE_PRICE_WAD, |v| v / WAD)
        .max(1)
}

fn reflector_base(env: &Env, sel: u8, quote: Option<&Address>) -> ReflectorAsset {
    let usd = ReflectorAsset::Other(Symbol::new(env, "USD"));
    match (sel % 4, quote) {
        (3, Some(_)) => usd,
        (3, None) => ReflectorAsset::Stellar(Address::generate(env)),
        (2, None) => ReflectorAsset::Other(Symbol::new(env, "EUR")),
        (_, Some(q)) => ReflectorAsset::Stellar(q.clone()),
        (_, None) => usd,
    }
}

fn twap_history(
    env: &Env,
    f: &FeedIn,
    raw: i128,
    ts: u64,
    resolution: u32,
    records: u32,
) -> Option<Vec<ReflectorPriceData>> {
    let res = u64::from(resolution);
    let n = records.max(1);
    let shape = f.twap_shape % 8;
    let (count, step) = match shape {
        0 | 1 => (n, res),
        2 => (n + 1, res),
        3 => (n.saturating_sub(1).max(1), res * 2),
        4 => (n.max(2), res.saturating_sub(1)),
        5 => (n + 2, res),
        6 => return Some(Vec::new(env)),
        _ => return None,
    };
    let mut out = Vec::new(env);
    for k in 0..count {
        let timestamp = if shape == 1 && k == 0 {
            NOW + MAX_FUTURE_SKEW_SECONDS + 1
        } else {
            ts.saturating_sub(u64::from(k) * step)
        };
        let wiggle = i128::from(k) * i128::from(f.jitter_bps) / 1_000;
        out.push_back(ReflectorPriceData {
            price: raw.saturating_add(wiggle),
            timestamp,
        });
    }
    Some(out)
}

/// Builds one feed source backed by a mock provider loaded with the fuzzed reading.
/// `quote` is set for the factor leg of a `Scaled` source.
fn build_feed(
    env: &Env,
    reg: &mut Registry,
    idx: u8,
    f: &FeedIn,
    wad: i128,
    quote: Option<&Address>,
) -> FeedSource {
    let kind = Kind::of(f.kind);
    let corrupt = f.corrupt % 16;
    let mut stale = if corrupt == C_STALE_CFG {
        [0, MIN_PRICE_STALE_SECONDS - 1, MAX_PRICE_STALE_SECONDS + 1][f.stale_raw as usize % 3]
    } else {
        MIN_PRICE_STALE_SECONDS
            + u64::from(f.stale_raw) % (MAX_PRICE_STALE_SECONDS - MIN_PRICE_STALE_SECONDS + 1)
    };
    let resolution = if f.resolution_raw % 16 == 15 {
        u32::from(f.resolution_raw) % MIN_ORACLE_RESOLUTION_SECONDS
    } else {
        MIN_ORACLE_RESOLUTION_SECONDS + u32::from(f.resolution_raw) % 900
    };
    let records = if f.twap_records % 32 == 31 {
        [0, MAX_TWAP_RECORDS + 1][usize::from(f.twap_records >> 7)]
    } else {
        1 + u32::from(f.twap_records) % MAX_TWAP_RECORDS
    };
    if matches!(kind, Kind::ReflectorSpot | Kind::ReflectorTwap) && corrupt != C_STALE_CFG {
        let span = if kind == Kind::ReflectorTwap {
            u64::from(records.saturating_sub(1)) * u64::from(resolution)
        } else {
            0
        };
        stale = stale
            .max(span)
            .max(u64::from(resolution))
            .min(MAX_PRICE_STALE_SECONDS);
    }
    let age = if corrupt == C_STALE {
        stale + 1 + u64::from(f.age_s)
    } else {
        u64::from(f.age_s) % (stale + 1)
    };
    let ts = if corrupt == C_FUTURE {
        NOW + MAX_FUTURE_SKEW_SECONDS + 1 + u64::from(f.age_s) % 600
    } else if f.future {
        NOW + u64::from(f.age_s) % (MAX_FUTURE_SKEW_SECONDS + 1)
    } else {
        NOW - age.min(NOW)
    };
    let panics = corrupt == C_PANIC;
    let missing = corrupt == C_MISSING;

    let (provider, decimals) = match kind {
        Kind::ReflectorSpot | Kind::ReflectorTwap => {
            let decimals = 1 + u32::from(f.dec_raw) % MAX_ORACLE_DECIMALS;
            let raw = raw_price(wad, decimals, corrupt);
            let base = reflector_base(env, f.base_sel, quote);
            let spot = (!missing).then_some(ReflectorPriceData {
                price: raw,
                timestamp: ts,
            });
            let history = if kind == Kind::ReflectorTwap && !missing {
                twap_history(env, f, raw, ts, resolution, records)
            } else {
                None
            };
            let contract = reg.contract(env, kind.slot(), |e| e.register(FuzzReflector, ()));
            FuzzReflectorClient::new(env, &contract).setup(
                &base,
                &decimals,
                &resolution,
                &spot,
                &history,
                &panics,
            );
            let asset = match f.base_sel >> 4 {
                14 => OracleAssetRef::Symbol(Symbol::new(env, "XLM")),
                15 => OracleAssetRef::String(String::from_str(env, "XLM")),
                _ => OracleAssetRef::Stellar(Address::generate(env)),
            };
            let read_mode = if kind == Kind::ReflectorTwap {
                OracleReadMode::Twap(records)
            } else {
                OracleReadMode::Spot
            };
            (
                ProviderRef::Reflector(ReflectorFeedRef {
                    contract,
                    asset,
                    read_mode,
                }),
                decimals,
            )
        }
        Kind::RedStone | Kind::Xoxno => {
            let decimals = if kind == Kind::RedStone {
                REDSTONE_DECIMALS
            } else {
                1 + u32::from(f.dec_raw) % MAX_ORACLE_DECIMALS
            };
            let raw = raw_price(wad, decimals, corrupt);
            let price = if corrupt == C_BAD_VALUE && f.age_s & 1 == 1 {
                U256::from_parts(env, 1, 0, 0, 0)
            } else {
                U256::from_u128(env, raw.max(0) as u128)
            };
            let package_ms = ts.saturating_mul(1_000);
            let skew_ms = u64::from(f.resolution_raw) * 1_000;
            let write_ms = match f.twap_shape % 3 {
                0 => package_ms,
                1 => package_ms.saturating_add(skew_ms),
                _ => package_ms.saturating_sub(skew_ms),
            };
            let max_age = if corrupt == C_STALE_CFG {
                stale.saturating_add(1)
            } else {
                stale.saturating_sub(u64::from(f.resolution_raw) % 60)
            };
            let contract = reg.contract(env, kind.slot(), |e| e.register(FuzzMultiFeed, ()));
            let client = FuzzMultiFeedClient::new(env, &contract);
            client.setup(&decimals, &max_age, &(f.base_sel & 0x40 != 0), &panics);
            let feed_id = String::from_str(env, ["F0", "F1", "F2", "F3", "F4"][usize::from(idx)]);
            let data = (!missing).then(|| RedStonePriceData {
                price,
                package_timestamp: package_ms,
                write_timestamp: write_ms,
            });
            client.set_feed(&feed_id, &data);
            let r = MultiFeedRef {
                contract,
                feed_id,
                nature: if f.market {
                    FeedNature::Market
                } else {
                    FeedNature::Fundamental
                },
            };
            let provider = if kind == Kind::RedStone {
                ProviderRef::RedStone(r)
            } else {
                ProviderRef::Xoxno(r)
            };
            (provider, decimals)
        }
        Kind::Chainlink => {
            let raw = raw_price(wad, CHAINLINK_DECIMALS, corrupt);
            let answer = if corrupt == C_BAD_VALUE && f.age_s & 1 == 1 {
                I256::from_parts(env, 1, 0, 0, 0)
            } else {
                I256::from_i128(env, raw)
            };
            let round = (!missing).then(|| ChainlinkRound {
                round_id: u64::from(f.resolution_raw % 4),
                answer,
                timestamp: ts,
            });
            let contract = reg.contract(env, kind.slot(), |e| e.register(FuzzChainlink, ()));
            FuzzChainlinkClient::new(env, &contract).setup(&round, &CHAINLINK_DECIMALS, &panics);
            (
                ProviderRef::Chainlink(ChainlinkFeedRef {
                    contract,
                    data_id: BytesN::from_array(env, &[idx; 32]),
                }),
                CHAINLINK_DECIMALS,
            )
        }
    };
    FeedSource {
        provider,
        decimals: if corrupt == C_DECIMALS {
            decimals % MAX_ORACLE_DECIMALS + 1
        } else {
            decimals
        },
        max_stale_seconds: stale,
    }
}

struct LpBuilt {
    key: PriceKey,
    source: AquariusLpSource,
    share_decimals: u32,
    estimate_wad: i128,
}

fn fuzz_token(env: &Env, decimals: u32) -> Address {
    let addr = env.register(FuzzToken, ());
    FuzzTokenClient::new(env, &addr).setup(&decimals);
    addr
}

fn lp_decimals(raw: u8) -> u32 {
    u32::from(raw) % 19
}

#[allow(clippy::too_many_arguments)]
fn build_lp(
    env: &Env,
    reg: &mut Registry,
    lp: &LpIn,
    stable: bool,
    legs: (&PriceKey, &PriceKey),
    leg_wads: (i128, i128),
    tokens: (&Address, &Address),
) -> LpBuilt {
    let corrupt = lp.corrupt % 16;
    let dec_a = lp_decimals(lp.dec_a);
    let dec_b = lp_decimals(lp.dec_b);
    let dec_share = u32::from(lp.dec_share) % 19;
    let token_a = tokens.0.clone();
    let token_b = tokens.1.clone();
    let share = fuzz_token(env, dec_share);
    let reserve_a = edge_biased_amount(lp.reserve_a, 1, pow10(dec_a + 12)) as u128;
    let reserve_b = edge_biased_amount(lp.reserve_b, 1, pow10(dec_b + 12)) as u128;
    let shares = edge_biased_amount(lp.shares, 1, pow10(dec_share + 12)) as u128;
    let amp = 1 + u128::from(lp.amp % 10_000);

    let mut reserves = vec![env, reserve_a, reserve_b];
    let mut tokens = vec![env, token_a.clone(), token_b.clone()];
    let mut kind = if stable { "stable" } else { "constant_product" };
    let mut share_id = share.clone();
    let mut shares_out = shares;
    let mut amp_out = amp;
    let mut src_dec_a = dec_a;
    match corrupt {
        7 => kind = if stable { "constant_product" } else { "stable" },
        8 => tokens = vec![env, token_b.clone(), token_a.clone()],
        9 => share_id = Address::generate(env),
        10 => {
            reserves = if lp.amp & 1 == 0 {
                vec![env, reserve_a]
            } else {
                vec![env, reserve_a, reserve_b, reserve_b]
            }
        }
        11 => reserves = vec![env, u128::MAX, reserve_b],
        12 => {
            if lp.amp & 1 == 0 {
                shares_out = 0;
            } else {
                reserves = vec![env, 0, reserve_b];
            }
        }
        13 => amp_out = 0,
        14 => src_dec_a = (dec_a + 1) % 19,
        _ => {}
    }
    let pool = env.register(FuzzAquarius, ());
    reg.all.push(pool.clone());
    FuzzAquariusClient::new(env, &pool).setup(
        &shares_out,
        &reserves,
        &tokens,
        &Symbol::new(env, kind),
        &share_id,
        &amp_out,
        &(corrupt == 15),
    );

    let value = reserve_a as f64 / 10f64.powi(dec_a as i32) * leg_wads.0 as f64
        + reserve_b as f64 / 10f64.powi(dec_b as i32) * leg_wads.1 as f64;
    let estimate = value / (shares as f64 / 10f64.powi(dec_share as i32));
    let estimate_wad = if estimate.is_finite() && estimate >= 1.0 {
        (estimate.min(MAX_REASONABLE_PRICE_WAD as f64)) as i128
    } else {
        1
    };
    let min_pool_value_wad = match lp.min_value % 4 {
        0 => MAX_REASONABLE_PRICE_WAD.saturating_mul(1_000_000),
        1 => WAD,
        2 => 1,
        _ => 0,
    };
    LpBuilt {
        key: PriceKey::Token(share),
        source: AquariusLpSource {
            pool,
            token_a,
            token_b,
            key_a: legs.0.clone(),
            key_b: legs.1.clone(),
            reserve_a_decimals: src_dec_a,
            reserve_b_decimals: dec_b,
            min_pool_value_wad,
        },
        share_decimals: dec_share,
        estimate_wad,
    }
}

fn tolerance(env: &Env, extra: u16) -> OracleTolerance {
    let upper = if extra % 50 == 49 {
        BPS + i128::from(extra) % i128::from(MIN_TOLERANCE)
    } else {
        BPS + i128::from(MIN_TOLERANCE)
            + i128::from(extra) % i128::from(MAX_TOLERANCE - MIN_TOLERANCE + 1)
    };
    OracleTolerance {
        upper_ratio_bps: upper as u32,
        lower_ratio_bps: mul_div_half_up(env, BPS, BPS, upper) as u32,
    }
}

fn band(center: i128, half_bps: i128, shift_bps: i128) -> (i128, i128) {
    let c = (center.saturating_mul(BPS + shift_bps) / BPS).max(1);
    let lo = (c.saturating_mul(BPS - half_bps) / BPS).max(1);
    let hi = (c.saturating_mul(BPS + half_bps) / BPS)
        .min(MAX_REASONABLE_PRICE_WAD)
        .max(lo + 1);
    (lo, hi)
}

fn base_oracle(env: &Env, key: &PriceKey, feed: FeedSource, wad: i128) -> AssetOracle {
    let (min, max) = band(wad, 500, 0);
    AssetOracle {
        // A Ref key carries no token, so validation requires zero decimals.
        asset_decimals: if matches!(key, PriceKey::Ref(_)) {
            0
        } else {
            7
        },
        max_price_stale_seconds: feed
            .max_stale_seconds
            .clamp(MIN_PRICE_STALE_SECONDS, MAX_PRICE_STALE_SECONDS),
        sources: vec![env, PriceSource::Feed(feed)],
        tolerance: tolerance(env, 0),
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: min,
        max_sanity_price_wad: max,
    }
}

/// Registers through the validating path when it accepts the config, otherwise seeds it.
fn install(agg: &PriceAggregatorClient, key: &PriceKey, oracle: &AssetOracle) -> bool {
    let admitted = matches!(agg.try_set_oracle(key, oracle), Ok(Ok(())));
    if admitted {
        assert_eq!(
            agg.oracle(key).as_ref(),
            Some(oracle),
            "set_oracle stored a different config"
        );
    } else {
        agg.seed_oracle(key, oracle);
    }
    admitted
}

fn quote(env: &Env, agg: &PriceAggregatorClient, key: &PriceKey) -> Option<PriceStatus> {
    match agg.try_quotes(&vec![env, key.clone()]) {
        Ok(Ok(m)) => Some(m.get(key.clone()).expect("quotes omitted a requested key")),
        Ok(Err(err)) => panic!("quotes output conversion failed: {err:?}"),
        Err(_) => None,
    }
}

fn price(env: &Env, agg: &PriceAggregatorClient, key: &PriceKey) -> Option<PriceFeedRaw> {
    agg.try_prices(&vec![env, key.clone()])
        .ok()
        .and_then(Result::ok)
        .and_then(|m| m.get(key.clone()))
}

fn assert_status_eq(a: &PriceStatus, b: &PriceStatus) {
    assert_eq!(
        (
            a.final_wad,
            a.primary_wad,
            a.secondary_wad,
            a.price_timestamp
        ),
        (
            b.final_wad,
            b.primary_wad,
            b.secondary_wad,
            b.price_timestamp
        ),
        "batched status differs from single-key status"
    );
    assert_eq!(
        (a.stale, a.deviation, a.valid, a.error_code),
        (b.stale, b.deviation, b.valid, b.error_code),
        "batched status flags differ from single-key status"
    );
}

fn check(env: &Env, agg: &PriceAggregatorClient, key: &PriceKey, batch: &[PriceKey]) {
    let q = quote(env, agg, key);
    let p = price(env, agg, key);
    match &q {
        None => assert!(p.is_none(), "quotes failed while prices resolved"),
        Some(q) => {
            assert_eq!(
                q.valid,
                p.is_some(),
                "quote.valid disagrees with prices: {q:?}"
            );
            assert_eq!(
                q.valid,
                q.error_code.is_none(),
                "valid flag and error code disagree: {q:?}"
            );
        }
    }
    if let (Some(q), Some(f)) = (&q, &p) {
        let cfg = agg
            .oracle(key)
            .expect("a key with no oracle resolved a price");
        let now = env.ledger().timestamp();
        assert!(f.price_wad > 0, "non-positive usable price");
        assert_eq!(f.price_wad, q.final_wad, "usable price != quote.final_wad");
        assert_eq!(f.asset_decimals, cfg.asset_decimals);
        assert!(
            cfg.min_sanity_price_wad <= f.price_wad && f.price_wad <= cfg.max_sanity_price_wad,
            "price {} outside sanity band [{}, {}]",
            f.price_wad,
            cfg.min_sanity_price_wad,
            cfg.max_sanity_price_wad
        );
        assert!(
            !q.stale && !q.deviation,
            "valid quote flagged stale or deviating: {q:?}"
        );
        assert!(
            f.timestamp <= now + MAX_FUTURE_SKEW_SECONDS,
            "usable price is future-dated: ts={} now={}",
            f.timestamp,
            now
        );
        assert!(
            now.saturating_sub(f.timestamp) <= cfg.max_price_stale_seconds,
            "usable price older than max_price_stale_seconds: ts={} now={} max={}",
            f.timestamp,
            now,
            cfg.max_price_stale_seconds
        );
        let (lo, hi) = agg.price_spread(key);
        assert!(lo <= hi, "price_spread not ordered: ({lo}, {hi})");
        assert!(
            lo <= f.price_wad && f.price_wad <= hi,
            "price {} outside spread [{lo}, {hi}]",
            f.price_wad
        );
    }
    // One session serves a batch; a key's status must not depend on what else is read with it.
    let mut keys = Vec::new(env);
    for k in batch {
        keys.push_back(k.clone());
    }
    if let (Some(single), Ok(Ok(m))) = (&q, agg.try_quotes(&keys)) {
        if let Some(batched) = m.get(key.clone()) {
            assert_status_eq(single, &batched);
        }
    }
}

fuzz_target!(|i: In| {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);

    let owner = Address::generate(&env);
    let agg = PriceAggregatorClient::new(&env, &env.register(PriceAggregator, (owner,)));
    let mut reg = Registry {
        shared: i.shared,
        reuse: Default::default(),
        all: std::vec::Vec::new(),
    };

    let anchor = if i.anchor.is_multiple_of(4) {
        edge_biased_amount(i.anchor >> 2, 1, MAX_REASONABLE_PRICE_WAD)
    } else {
        edge_biased_amount(i.anchor >> 2, WAD / 10_000, 100_000 * WAD)
    };
    let mut wads = [0i128; 4];
    for (k, f) in i.feeds.iter().enumerate() {
        wads[k] = if f.wad % 5 == 0 {
            edge_biased_amount(f.wad, 1, MAX_REASONABLE_PRICE_WAD)
        } else {
            jitter(anchor, f.jitter_bps)
        };
    }
    // The factor leg of a Scaled source is a ratio near 1, not a price.
    if !i.feeds[2].wad.is_multiple_of(5) {
        wads[2] = jitter(WAD, i.feeds[2].jitter_bps);
    }

    // LP validation requires each leg key to price its own pool token, so the legs are tokens.
    let k0_addr = fuzz_token(&env, lp_decimals(i.lp.dec_a));
    let k1_addr = fuzz_token(&env, lp_decimals(i.lp.dec_b));
    let k0 = PriceKey::Token(k0_addr.clone());
    let k1 = if i.lp.min_value & 0x10 != 0 {
        PriceKey::Ref(Symbol::new(&env, "LEG1"))
    } else {
        PriceKey::Token(k1_addr.clone())
    };
    let f0 = build_feed(&env, &mut reg, 0, &i.feeds[0], wads[0], None);
    let f1 = build_feed(&env, &mut reg, 1, &i.feeds[1], wads[1], None);
    install(&agg, &k0, &base_oracle(&env, &k0, f0.clone(), wads[0]));
    install(&agg, &k1, &base_oracle(&env, &k1, f1.clone(), wads[1]));

    let shape = i.shape % 8;
    let scaled = |reg: &mut Registry, quote_addr: &Address, quote_key: &PriceKey| {
        let factor = build_feed(&env, reg, 2, &i.feeds[2], wads[2], Some(quote_addr));
        let fb = i128::from(i.factor_band_bps % 5_000);
        let (min_factor_wad, max_factor_wad) = if i.factor_band_bps % 41 == 40 {
            (wads[2].saturating_mul(2), wads[2])
        } else {
            (
                wads[2].saturating_mul(BPS - fb) / BPS,
                wads[2].saturating_mul(BPS + fb) / BPS + 1,
            )
        };
        PriceSource::Scaled(ScaledSource {
            factor,
            quote: quote_key.clone(),
            min_factor_wad,
            max_factor_wad,
        })
    };

    let mut key = PriceKey::Token(Address::generate(&env));
    let mut asset_decimals = if i.tol_extra.is_multiple_of(7) {
        u32::from(i.tol_extra % 19)
    } else {
        7
    };
    let (sources, center, half_cap) = match shape {
        0 => (vec![&env, PriceSource::Feed(f0.clone())], wads[0], 1_000),
        1 => (
            vec![
                &env,
                PriceSource::Feed(f0.clone()),
                PriceSource::Feed(f1.clone()),
            ],
            wads[0],
            3_000,
        ),
        2 => (
            vec![&env, scaled(&mut reg, &k0_addr, &k0)],
            scaled_price(wads[2], wads[0]),
            1_000,
        ),
        3 => {
            let center = scaled_price(wads[2], wads[0]);
            let wad = jitter(center, i.feeds[3].jitter_bps / 8);
            let second = build_feed(&env, &mut reg, 3, &i.feeds[3], wad, None);
            (
                vec![
                    &env,
                    scaled(&mut reg, &k0_addr, &k0),
                    PriceSource::Feed(second),
                ],
                center,
                3_000,
            )
        }
        4 | 5 | 7 => {
            let stable = shape == 5;
            let lp = build_lp(
                &env,
                &mut reg,
                &i.lp,
                stable,
                (&k0, &k1),
                (wads[0], wads[1]),
                (&k0_addr, &k1_addr),
            );
            key = lp.key.clone();
            asset_decimals = lp.share_decimals;
            let source = if stable {
                PriceSource::AquariusStableLp(lp.source.clone())
            } else {
                PriceSource::AquariusLp(lp.source.clone())
            };
            let mut sources = vec![&env, source];
            if shape == 7 {
                let wad = jitter(lp.estimate_wad, i.feeds[3].jitter_bps / 8);
                let second = build_feed(&env, &mut reg, 3, &i.feeds[3], wad, None);
                sources.push_back(PriceSource::Feed(second));
            }
            (sources, lp.estimate_wad, 8_000)
        }
        _ => {
            let center = scaled_price(wads[2], wads[1]);
            let wad = jitter(center, i.feeds[3].jitter_bps / 8);
            let first = build_feed(&env, &mut reg, 3, &i.feeds[3], wad, None);
            (
                vec![
                    &env,
                    PriceSource::Feed(first),
                    scaled(&mut reg, &k1_addr, &k1),
                ],
                center,
                3_000,
            )
        }
    };

    let half = 50 + i128::from(i.band_bps) % (half_cap - 49);
    let (mut min, mut max) = band(center, half, i128::from(i.band_shift_bps) % 500);
    if i.band_bps % 37 == 36 {
        core::mem::swap(&mut min, &mut max);
    }
    let mut max_stale = MIN_PRICE_STALE_SECONDS;
    for s in sources.iter() {
        let stale = match s {
            PriceSource::Feed(f) => f.max_stale_seconds,
            PriceSource::Scaled(s) => s.factor.max_stale_seconds,
            _ => MAX_PRICE_STALE_SECONDS,
        };
        max_stale = max_stale.max(stale);
    }
    if i.tol_extra % 13 == 12 {
        max_stale = u64::from(i.tol_extra);
    }
    let mut shared = Vec::new(&env);
    for addr in &reg.all {
        shared.push_back(addr.clone());
    }
    let independence = match i.independence % 3 {
        0 => IndependencePolicy::RequireDisjoint,
        1 => IndependencePolicy::AllowShared(shared),
        _ => IndependencePolicy::AllowShared(Vec::new(&env)),
    };
    let cfg = AssetOracle {
        asset_decimals,
        max_price_stale_seconds: max_stale.min(MAX_PRICE_STALE_SECONDS),
        sources,
        tolerance: tolerance(&env, i.tol_extra),
        independence,
        min_sanity_price_wad: min,
        max_sanity_price_wad: max,
    };

    let admitted = install(&agg, &key, &cfg);
    if admitted && cfg.has_aquarius_lp_source() {
        let q = quote(&env, &agg, &key).expect("an admitted LP oracle must quote");
        assert!(
            q.valid,
            "set_oracle admitted an LP oracle that does not price: {q:?}"
        );
    }

    let batch = [k0.clone(), k1.clone(), key.clone()];
    check(&env, &agg, &key, &batch);
    check(&env, &agg, &k0, &batch);
    check(&env, &agg, &k1, &batch);

    for (op, a, b) in i.ops.iter() {
        match op % 6 {
            0 => {
                let (lo, hi) = band(center, 50 + i128::from(*a) % 3_000, i128::from(*b) % 500);
                if matches!(agg.try_set_sanity_band(&key, &lo, &hi), Ok(Ok(()))) {
                    let stored = agg.oracle(&key).unwrap();
                    assert_eq!(
                        (stored.min_sanity_price_wad, stored.max_sanity_price_wad),
                        (lo, hi)
                    );
                }
            }
            1 => {
                let t = tolerance(&env, *a);
                if matches!(agg.try_set_tolerance(&key, &t), Ok(Ok(()))) {
                    assert_eq!(agg.oracle(&key).unwrap().tolerance, t);
                }
            }
            2 => {
                let now = env.ledger().timestamp();
                env.ledger()
                    .set_timestamp(now + u64::from(*a) * (1 + (*b as u64 & 7)));
            }
            3 => {
                // Replacing a leg re-runs dependent revalidation for every composed key.
                let mut f = i.feeds[0].clone();
                f.jitter_bps = *b;
                f.corrupt = (*a % 16) as u8;
                let wad = jitter(wads[0], *b);
                let feed = build_feed(&env, &mut reg, 4, &f, wad, None);
                let _ = agg.try_set_oracle(&k0, &base_oracle(&env, &k0, feed, wad));
            }
            4 => agg.remove_oracle(&k1),
            _ => {
                let Some(mut replacement) = agg.oracle(&key) else {
                    continue;
                };
                let changed = *a & 1 == 1;
                if changed {
                    replacement.asset_decimals = (replacement.asset_decimals + 1) % 19;
                }
                let result = agg.try_set_oracle(&key, &replacement);
                if changed {
                    assert!(
                        !matches!(result, Ok(Ok(()))),
                        "set_oracle accepted an asset_decimals change"
                    );
                }
            }
        }
        check(&env, &agg, &key, &batch);
    }
});
