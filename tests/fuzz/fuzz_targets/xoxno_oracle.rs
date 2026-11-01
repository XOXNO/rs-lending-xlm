#![no_main]
//! Differential fuzzing of the XOXNO oracle against a reference model of its
//! signer set, configuration, per-signer submissions, aggregates and history.
//! Every entrypoint's result (including the exact error) and every read is
//! compared against the model after each operation.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use arbitrary::Arbitrary;
use common::constants::BPS;
use common::oracle::observation::{MAX_FUTURE_SKEW_SECONDS, MAX_TWAP_RECORDS};
use common::oracle::providers::reflector::ReflectorAsset;
use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{Address, Env, String, Symbol, Vec};
use xoxno_oracle::{Error, XoxnoOracle, XoxnoOracleClient};

const NOW: u64 = 1_000_000;
const MS: u64 = 1_000;
const SIGNERS: usize = 6;
const FEEDS: usize = 3;
const ASSETS: usize = 3;
const MAX_OPS: usize = 48;
const MAX_HISTORY: usize = MAX_TWAP_RECORDS as usize;
const MAX_SUBMITTED_PRICE: i128 = 1_000_000_000_000_000_000_000_000;
const DEFAULT_MAX_STALE: u64 = 86_400;
const DEFAULT_MAX_SUBMISSION_AGE: u64 = 900;
const MIN_SUBMISSION_AGE: u64 = MAX_FUTURE_SKEW_SECONDS + 1;
const DEFAULT_SPREAD_BPS: u32 = 200;

#[derive(Debug, Arbitrary)]
enum Op {
    Submit {
        signer: u8,
        feed: u8,
        price: PriceIn,
        back_s: i16,
    },
    SubmitBatch {
        signer: u8,
        feeds: [u8; 3],
        len: u8,
        prices: [PriceIn; 3],
        mismatch: bool,
        back_s: i16,
    },
    Advance(u16),
    AddSigner(u8),
    RemoveSigner(u8),
    SetThreshold(u8),
    SetMaxStale(u32),
    SetMaxSubmissionAge(u32),
    SetSkew(u32),
    SetSpread(u16),
    Recompute(u8),
    RegisterFeed(u8),
    AddFeed(u8, u8),
    RemoveFeed(u8),
    PurgeFeed(u8),
    SetResolution(u16),
    PriceAt(u8, u32),
    TransferOwnership(u8),
}

#[derive(Clone, Copy, Debug, Arbitrary)]
struct PriceIn {
    mode: u8,
    jitter_bps: i16,
}

#[derive(Debug, Arbitrary)]
struct In {
    n_signers: u8,
    threshold: u8,
    resolution: u16,
    ops: std::vec::Vec<Op>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Agg {
    price: i128,
    package_ms: u64,
    write_ms: u64,
}

#[derive(Default)]
struct Model {
    signers: std::vec::Vec<usize>,
    threshold: u32,
    max_stale: u64,
    max_sub_age: u64,
    skew_cfg: u64,
    spread_bps: u32,
    resolution: u32,
    known: BTreeSet<usize>,
    asset_feed: BTreeMap<usize, usize>,
    feed_asset: BTreeMap<usize, usize>,
    subs: BTreeMap<(usize, usize), (i128, u64)>,
    agg: BTreeMap<usize, Agg>,
    history: BTreeMap<usize, VecDeque<Agg>>,
}

impl Model {
    fn skew(&self) -> u64 {
        self.skew_cfg.min(self.max_sub_age)
    }

    fn recompute(&mut self, feed: usize, now: u64, clear_on_miss: bool) {
        let mut kept = std::vec::Vec::new();
        let mut newest = 0u64;
        for s in &self.signers {
            let Some(&(price, ts)) = self.subs.get(&(feed, *s)) else {
                continue;
            };
            if now.saturating_sub(ts / MS) > self.max_sub_age {
                continue;
            }
            newest = newest.max(ts);
            kept.push((price, ts));
        }
        if (kept.len() as u32) < self.threshold {
            return self.quorum_miss(feed, clear_on_miss);
        }
        let newest = newest.min(now.saturating_mul(MS));
        let skew_ms = self.skew().saturating_mul(MS);
        let cluster: std::vec::Vec<(i128, u64)> = kept
            .into_iter()
            .filter(|(_, ts)| newest.saturating_sub(*ts) <= skew_ms)
            .collect();
        if (cluster.len() as u32) < self.threshold {
            return self.quorum_miss(feed, clear_on_miss);
        }
        let lowest = cluster.iter().map(|c| c.0).min().unwrap();
        let highest = cluster.iter().map(|c| c.0).max().unwrap();
        let spread_free = (self.signers.len() as u32)
            .saturating_sub(self.threshold)
            .saturating_mul(2)
            .saturating_add(1);
        let within = match (
            highest.checked_mul(BPS),
            lowest.checked_mul(BPS + i128::from(self.spread_bps)),
        ) {
            (Some(h), Some(l)) => h <= l,
            _ => false,
        };
        if (cluster.len() as u32) < spread_free && !within {
            return self.quorum_miss(feed, clear_on_miss);
        }
        let mut prices: std::vec::Vec<i128> = cluster.iter().map(|c| c.0).collect();
        prices.sort_unstable();
        let agg = Agg {
            price: prices[(prices.len() - 1) / 2],
            package_ms: cluster.iter().map(|c| c.1).min().unwrap(),
            write_ms: now * MS,
        };
        self.agg.insert(feed, agg);
        let history = self.history.entry(feed).or_default();
        let resolution_ms = u64::from(self.resolution) * MS;
        let replace_last = history
            .back()
            .is_some_and(|last| agg.write_ms < last.write_ms.saturating_add(resolution_ms));
        if replace_last {
            *history.back_mut().unwrap() = agg;
        } else {
            if history.len() >= MAX_HISTORY {
                history.pop_front();
            }
            history.push_back(agg);
        }
    }

    fn quorum_miss(&mut self, feed: usize, clear: bool) {
        if clear {
            self.agg.remove(&feed);
        }
    }

    fn clear_feed(&mut self, feed: usize) {
        self.agg.remove(&feed);
        self.history.remove(&feed);
        self.subs.retain(|(f, _), _| *f != feed);
        if let Some(asset) = self.feed_asset.remove(&feed) {
            self.asset_feed.remove(&asset);
        }
        self.known.remove(&feed);
    }

    fn read(&self, feed: usize, now: u64) -> Result<Agg, Error> {
        let agg = self.agg.get(&feed).ok_or(Error::NoDataForFeed)?;
        if now.saturating_sub(agg.write_ms / MS) > self.max_stale {
            return Err(Error::StaleData);
        }
        Ok(*agg)
    }

    fn history_newest_first(
        &self,
        feed: usize,
        now: u64,
        limit: usize,
    ) -> Result<std::vec::Vec<Agg>, Error> {
        self.read(feed, now)?;
        let h = self
            .history
            .get(&feed)
            .filter(|h| !h.is_empty())
            .ok_or(Error::NoDataForFeed)?;
        Ok(h.iter().rev().take(limit).copied().collect())
    }

    fn require_signer(&self, signer: usize) -> Result<(), Error> {
        if self.signers.contains(&signer) {
            Ok(())
        } else {
            Err(Error::NotAuthorizedSigner)
        }
    }

    fn require_known(&self, feed: usize) -> Result<(), Error> {
        if self.known.contains(&feed) {
            Ok(())
        } else {
            Err(Error::FeedNotKnown)
        }
    }

    fn check_time(&self, ts_ms: u64, now: u64) -> Result<(), Error> {
        if ts_ms / MS > now.saturating_add(MAX_FUTURE_SKEW_SECONDS) {
            return Err(Error::FutureTimestamp);
        }
        if now.saturating_sub(ts_ms / MS) > self.max_sub_age {
            return Err(Error::StaleSubmission);
        }
        Ok(())
    }

    fn check_monotonic(&self, feed: usize, signer: usize, ts_ms: u64) -> Result<(), Error> {
        match self.subs.get(&(feed, signer)) {
            Some(&(_, prev)) if ts_ms < prev => Err(Error::StaleSubmission),
            _ => Ok(()),
        }
    }
}

fn check_price(price: i128) -> Result<(), Error> {
    if price <= 0 {
        return Err(Error::InvalidPrice);
    }
    if price > MAX_SUBMITTED_PRICE {
        return Err(Error::PriceOutOfRange);
    }
    Ok(())
}

fn price_of(p: PriceIn, feed: usize) -> i128 {
    let base = 100_000_000i128 * 100 * (feed as i128 + 1);
    match p.mode % 16 {
        0 => 0,
        1 => -1,
        2 => MAX_SUBMITTED_PRICE + 1,
        3 => MAX_SUBMITTED_PRICE,
        4 => 1,
        5 => i128::MAX,
        _ => base * (BPS + i128::from(p.jitter_bps) % 600) / BPS,
    }
}

/// Mostly within a few minutes of `now` (either side of the future-skew bound), so
/// quorums form; the top bits select a wide offset that crosses the age limits.
fn package_ms(now: u64, back_s: i16) -> u64 {
    let back = if back_s as u16 & 0x7000 == 0x7000 {
        i64::from(back_s) * 3
    } else {
        i64::from(back_s % 330)
    };
    (now as i64 - back).max(0) as u64 * MS + u64::from(back_s.unsigned_abs()) % MS
}

fn expect_unit<T: std::fmt::Debug>(
    label: &str,
    got: Result<Result<T, soroban_sdk::ConversionError>, Result<Error, soroban_sdk::InvokeError>>,
    want: Result<(), Error>,
) -> bool {
    match (got, want) {
        (Ok(Ok(_)), Ok(())) => true,
        (Err(Ok(e)), Err(w)) => {
            assert_eq!(e, w, "{label}: wrong error");
            false
        }
        (got, want) => panic!("{label}: contract returned {got:?}, model expected {want:?}"),
    }
}

fn agg_of(d: &common::oracle::providers::redstone::RedStonePriceData) -> Agg {
    Agg {
        price: d.price.to_u128().expect("aggregate price exceeds u128") as i128,
        package_ms: d.package_timestamp,
        write_ms: d.write_timestamp,
    }
}

fn check_reads(
    env: &Env,
    c: &XoxnoOracleClient,
    m: &Model,
    feed_ids: &[String],
    assets: &[ReflectorAsset],
) {
    let now = env.ledger().timestamp();
    for (f, feed_id) in feed_ids.iter().enumerate().take(FEEDS) {
        match (c.try_read_price_data_for_feed(feed_id), m.read(f, now)) {
            (Ok(Ok(d)), Ok(w)) => {
                assert_eq!(
                    agg_of(&d),
                    w,
                    "aggregate for feed {f} diverged from the model"
                )
            }
            (Err(Ok(e)), Err(w)) => assert_eq!(e, w, "read error for feed {f}"),
            (got, want) => panic!("feed {f}: contract read {got:?}, model expected {want:?}"),
        }
        let limit = 1 + (f as u32 * 5) % MAX_TWAP_RECORDS;
        match (
            c.try_read_price_history(feed_id, &limit),
            m.history_newest_first(f, now, limit as usize),
        ) {
            (Ok(Ok(h)), Ok(w)) => {
                let got: std::vec::Vec<Agg> = h.iter().map(|d| agg_of(&d)).collect();
                assert_eq!(got, w, "history for feed {f} diverged from the model");
            }
            (Err(Ok(e)), Err(w)) => assert_eq!(e, w, "history error for feed {f}"),
            (got, want) => panic!("history {f}: contract {got:?}, model {want:?}"),
        }
    }
    // Bulk read fails on the first feed that fails, in request order.
    let mut ids = Vec::new(env);
    let mut want_bulk: Result<std::vec::Vec<Agg>, Error> = Ok(std::vec::Vec::new());
    for f in [2usize, 0, 1].iter().take(1 + (now as usize % 3)) {
        ids.push_back(feed_ids[*f].clone());
        if let Ok(acc) = want_bulk.as_mut() {
            match m.read(*f, now) {
                Ok(agg) => acc.push(agg),
                Err(e) => want_bulk = Err(e),
            }
        }
    }
    match (c.try_read_price_data(&ids), want_bulk) {
        (Ok(Ok(v)), Ok(w)) => {
            let got: std::vec::Vec<Agg> = v.iter().map(|d| agg_of(&d)).collect();
            assert_eq!(got, w, "bulk read diverged from the model");
        }
        (Err(Ok(e)), Err(w)) => assert_eq!(e, w, "bulk read error"),
        (got, want) => panic!("bulk read: contract {got:?}, model {want:?}"),
    }
    assert_eq!(c.base(), ReflectorAsset::Other(Symbol::new(env, "USD")));
    assert_eq!(c.decimals(), 8);

    for (a, asset) in assets.iter().enumerate() {
        let want = m
            .asset_feed
            .get(&a)
            .and_then(|f| m.read(*f, now).ok())
            .map(|agg| (agg.price, agg.package_ms / MS));
        let got = c.lastprice(asset).map(|p| (p.price, p.timestamp));
        assert_eq!(got, want, "lastprice for asset {a} diverged from the model");

        let records = 1 + a as u32 * 4;
        let want_hist: Option<std::vec::Vec<(i128, u64)>> = m
            .asset_feed
            .get(&a)
            .and_then(|f| m.history_newest_first(*f, now, records as usize).ok())
            .filter(|h| !h.is_empty())
            .map(|h| h.iter().map(|x| (x.price, x.package_ms / MS)).collect());
        let got_hist = c.prices(asset, &records).map(|v| {
            v.iter()
                .map(|p| (p.price, p.timestamp))
                .collect::<std::vec::Vec<_>>()
        });
        assert_eq!(
            got_hist, want_hist,
            "prices for asset {a} diverged from the model"
        );
    }

    let mut want_feeds: std::vec::Vec<String> =
        m.known.iter().map(|f| feed_ids[*f].clone()).collect();
    let mut got_feeds: std::vec::Vec<String> = c.feeds().iter().collect();
    want_feeds.sort();
    got_feeds.sort();
    assert_eq!(got_feeds, want_feeds, "registered feed set diverged");
    assert_eq!(
        c.assets().len() as usize,
        m.asset_feed.len(),
        "asset registry size diverged"
    );
    assert_eq!(c.max_submission_age_seconds(), m.max_sub_age);
    assert_eq!(c.max_stale_seconds(), m.max_stale);
    assert_eq!(c.max_relative_skew_seconds(), m.skew());
    assert_eq!(c.max_cluster_spread_bps(), m.spread_bps);
    assert_eq!(c.resolution(), m.resolution);
}

fuzz_target!(|i: In| {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);

    let pool: std::vec::Vec<Address> = (0..SIGNERS).map(|_| Address::generate(&env)).collect();
    let feed_ids: std::vec::Vec<String> = ["feed0", "feed1", "feed2"]
        .iter()
        .map(|s| String::from_str(&env, s))
        .collect();
    let assets: std::vec::Vec<ReflectorAsset> = (0..ASSETS)
        .map(|k| {
            if k < 2 {
                ReflectorAsset::Stellar(Address::generate(&env))
            } else {
                ReflectorAsset::Other(Symbol::new(&env, "BTC"))
            }
        })
        .collect();

    let n = 1 + usize::from(i.n_signers) % (SIGNERS - 1);
    let threshold = 1 + u32::from(i.threshold) % n as u32;
    let mut initial = Vec::new(&env);
    for addr in &pool[..n] {
        initial.push_back(addr.clone());
    }
    let resolution = u32::from(i.resolution % 600);
    let admin = Address::generate(&env);
    let addr = env.register(XoxnoOracle, (admin.clone(), initial, threshold, resolution));
    let c = XoxnoOracleClient::new(&env, &addr);

    let mut m = Model {
        signers: (0..n).collect(),
        threshold,
        max_stale: DEFAULT_MAX_STALE,
        max_sub_age: DEFAULT_MAX_SUBMISSION_AGE,
        skew_cfg: DEFAULT_MAX_SUBMISSION_AGE,
        spread_bps: DEFAULT_SPREAD_BPS,
        resolution,
        ..Default::default()
    };
    assert_eq!(c.get_owner(), Some(admin));
    // Two mapped feeds from the start; feed 2 stays unregistered until an op adds it.
    for f in 0..2 {
        c.add_feed(&feed_ids[f], &assets[f]);
        m.asset_feed.insert(f, f);
        m.feed_asset.insert(f, f);
        m.known.insert(f);
    }
    check_reads(&env, &c, &m, &feed_ids, &assets);

    for op in i.ops.iter().take(MAX_OPS) {
        let now = env.ledger().timestamp();
        match *op {
            Op::Submit {
                signer,
                feed,
                price,
                back_s,
            } => {
                let (s, f) = (usize::from(signer) % SIGNERS, usize::from(feed) % FEEDS);
                let p = price_of(price, f);
                let ts = package_ms(now, back_s);
                let want = m
                    .require_signer(s)
                    .and_then(|_| m.require_known(f))
                    .and_then(|_| check_price(p))
                    .and_then(|_| m.check_time(ts, now))
                    .and_then(|_| m.check_monotonic(f, s, ts));
                let got = c.try_submit_price(&pool[s], &feed_ids[f], &p, &ts);
                if expect_unit("submit_price", got, want) {
                    m.subs.insert((f, s), (p, ts));
                    m.recompute(f, now, false);
                }
            }
            Op::SubmitBatch {
                signer,
                feeds,
                len,
                prices,
                mismatch,
                back_s,
            } => {
                let s = usize::from(signer) % SIGNERS;
                let k = usize::from(len % 4);
                let fs: std::vec::Vec<usize> =
                    feeds[..k].iter().map(|f| usize::from(*f) % FEEDS).collect();
                let mut ps: std::vec::Vec<i128> = fs
                    .iter()
                    .zip(prices.iter())
                    .map(|(f, p)| price_of(*p, *f))
                    .collect();
                if mismatch {
                    ps.push(price_of(prices[0], 0));
                }
                let ts = package_ms(now, back_s);
                let mut want = m.require_signer(s);
                if want.is_ok() && fs.len() != ps.len() {
                    want = Err(Error::LengthMismatch);
                }
                if want.is_ok() {
                    want = m.check_time(ts, now);
                }
                if want.is_ok() {
                    want = fs.iter().try_for_each(|f| {
                        m.require_known(*f)?;
                        m.check_monotonic(*f, s, ts)
                    });
                }
                if want.is_ok() {
                    want = ps.iter().try_for_each(|p| check_price(*p));
                }
                let mut feed_vec = Vec::new(&env);
                for f in &fs {
                    feed_vec.push_back(feed_ids[*f].clone());
                }
                let mut price_vec = Vec::new(&env);
                for p in &ps {
                    price_vec.push_back(*p);
                }
                let got = c.try_submit_prices(&pool[s], &feed_vec, &price_vec, &ts);
                if expect_unit("submit_prices", got, want) {
                    for (f, p) in fs.iter().zip(ps.iter()) {
                        m.subs.insert((*f, s), (*p, ts));
                        m.recompute(*f, now, false);
                    }
                }
            }
            Op::Advance(secs) => env.ledger().set_timestamp(now + u64::from(secs) * 2),
            Op::AddSigner(raw) => {
                let s = usize::from(raw) % SIGNERS;
                let want = if m.signers.contains(&s) {
                    Err(Error::SignerAlreadyRegistered)
                } else {
                    Ok(())
                };
                if expect_unit("add_signer", c.try_add_signer(&pool[s]), want) {
                    m.signers.push(s);
                }
            }
            Op::RemoveSigner(raw) => {
                let s = usize::from(raw) % SIGNERS;
                let want = if !m.signers.contains(&s) {
                    Err(Error::SignerNotRegistered)
                } else if (m.signers.len() as u32) - 1 < m.threshold {
                    Err(Error::CannotRemoveBelowThreshold)
                } else {
                    Ok(())
                };
                if expect_unit("remove_signer", c.try_remove_signer(&pool[s]), want) {
                    m.signers.retain(|x| *x != s);
                    let touched: std::vec::Vec<usize> = m
                        .subs
                        .keys()
                        .filter(|(_, signer)| *signer == s)
                        .map(|(f, _)| *f)
                        .collect();
                    for f in touched {
                        m.subs.remove(&(f, s));
                        m.recompute(f, now, true);
                    }
                }
            }
            Op::SetThreshold(raw) => {
                let t = u32::from(raw % 8);
                let want = if t == 0 || t as usize > m.signers.len() {
                    Err(Error::InvalidThreshold)
                } else {
                    Ok(())
                };
                if expect_unit("set_threshold", c.try_set_threshold(&t), want) {
                    m.threshold = t;
                }
            }
            Op::SetMaxStale(raw) => {
                let v = u64::from(raw % 200_000);
                let want = if v < m.max_sub_age {
                    Err(Error::InvalidSubmissionAge)
                } else {
                    Ok(())
                };
                if expect_unit(
                    "set_max_stale_seconds",
                    c.try_set_max_stale_seconds(&v),
                    want,
                ) {
                    m.max_stale = v;
                }
            }
            Op::SetMaxSubmissionAge(raw) => {
                let v = u64::from(raw % 5_000);
                let want = if v < MIN_SUBMISSION_AGE || v > m.max_stale {
                    Err(Error::InvalidSubmissionAge)
                } else {
                    Ok(())
                };
                let got = c.try_set_max_submission_age_seconds(&v);
                if expect_unit("set_max_submission_age_seconds", got, want) {
                    m.max_sub_age = v;
                }
            }
            Op::SetSkew(raw) => {
                let v = u64::from(raw % 5_000);
                let want = if v > m.max_sub_age || v <= MAX_FUTURE_SKEW_SECONDS {
                    Err(Error::InvalidRelativeSkew)
                } else {
                    Ok(())
                };
                let got = c.try_set_max_relative_skew_seconds(&v);
                if expect_unit("set_max_relative_skew_seconds", got, want) {
                    m.skew_cfg = v;
                }
            }
            Op::SetSpread(raw) => {
                let v = u32::from(raw % 12_000);
                let want = if v == 0 || i128::from(v) > BPS {
                    Err(Error::InvalidClusterSpread)
                } else {
                    Ok(())
                };
                let got = c.try_set_max_cluster_spread_bps(&v);
                if expect_unit("set_max_cluster_spread_bps", got, want) {
                    m.spread_bps = v;
                }
            }
            Op::Recompute(mask) => {
                let fs: std::vec::Vec<usize> = (0..FEEDS).filter(|f| mask >> f & 1 == 1).collect();
                let mut ids = Vec::new(&env);
                for f in &fs {
                    ids.push_back(feed_ids[*f].clone());
                }
                let want = fs.iter().try_for_each(|f| m.require_known(*f));
                if expect_unit("recompute_feeds", c.try_recompute_feeds(&ids), want) {
                    for f in fs {
                        m.recompute(f, now, true);
                    }
                }
            }
            Op::RegisterFeed(raw) => {
                let f = usize::from(raw) % FEEDS;
                let want = if m.known.contains(&f) {
                    Err(Error::FeedAlreadyRegistered)
                } else {
                    Ok(())
                };
                if expect_unit("register_feed", c.try_register_feed(&feed_ids[f]), want) {
                    m.known.insert(f);
                }
            }
            Op::AddFeed(fr, ar) => {
                let (f, a) = (usize::from(fr) % FEEDS, usize::from(ar) % ASSETS);
                let want = if m.asset_feed.contains_key(&a) || m.feed_asset.contains_key(&f) {
                    Err(Error::FeedAlreadyMapped)
                } else {
                    Ok(())
                };
                if expect_unit("add_feed", c.try_add_feed(&feed_ids[f], &assets[a]), want) {
                    m.asset_feed.insert(a, f);
                    m.feed_asset.insert(f, a);
                    m.known.insert(f);
                }
            }
            Op::RemoveFeed(ar) => {
                let a = usize::from(ar) % ASSETS;
                let mapped = m.asset_feed.get(&a).copied();
                let want = mapped.map(|_| ()).ok_or(Error::FeedNotMapped);
                if expect_unit("remove_feed", c.try_remove_feed(&assets[a]), want) {
                    m.clear_feed(mapped.unwrap());
                }
            }
            Op::PurgeFeed(raw) => {
                let f = usize::from(raw) % FEEDS;
                if expect_unit(
                    "purge_feed",
                    c.try_purge_feed(&feed_ids[f]),
                    m.require_known(f),
                ) {
                    m.clear_feed(f);
                }
            }
            Op::SetResolution(raw) => {
                let r = u32::from(raw % 900);
                expect_unit("set_resolution", c.try_set_resolution(&r), Ok(()));
                m.resolution = r;
            }
            Op::PriceAt(ar, back) => {
                let a = usize::from(ar) % ASSETS;
                // Half the probes sit at `now`, where several history entries qualify.
                let at = if back & 1 == 0 {
                    now
                } else {
                    now.saturating_sub(u64::from(back % 20_000))
                };
                let want = m
                    .asset_feed
                    .get(&a)
                    .and_then(|f| m.history_newest_first(*f, now, MAX_HISTORY).ok())
                    .and_then(|h| {
                        h.into_iter().filter(|e| e.package_ms / MS <= at).fold(
                            None::<Agg>,
                            |best, e| match best {
                                Some(b) if e.package_ms <= b.package_ms => Some(b),
                                _ => Some(e),
                            },
                        )
                    })
                    .map(|e| (e.price, e.package_ms / MS));
                let got = c.price(&assets[a], &at).map(|p| (p.price, p.timestamp));
                assert_eq!(got, want, "price(asset {a}, {at}) diverged from the model");
            }
            Op::TransferOwnership(raw) => {
                let next = pool[usize::from(raw) % SIGNERS].clone();
                let live_until = env.ledger().sequence() + 100;
                if c.try_transfer_ownership(&next, &live_until).is_ok()
                    && c.try_accept_ownership().is_ok()
                {
                    assert_eq!(c.get_owner(), Some(next));
                }
            }
        }
        check_reads(&env, &c, &m, &feed_ids, &assets);
    }
});
