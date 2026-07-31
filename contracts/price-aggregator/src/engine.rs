use common::errors::OracleError;
use common::math::fp::Wad;
use common::oracle::lp::fair_lp_price_wad;
use common::oracle::observation::is_stale;
use common::oracle::providers::aquarius::{
    aquarius_plane_reserves_call, aquarius_total_shares_call,
};
use common::types::{
    AssetOracle, FeedSource, LpShareSource, PoolKind, PriceFeedRaw, PriceKey, PriceSource,
    PriceStatus, ProviderRef, ScaledSource, MAX_RESOLUTION_DEPTH,
};
use soroban_sdk::{panic_with_error, Env};

use crate::admin;
use crate::observation::OracleObservation;
use crate::providers::{multi_feed, reflector};
use crate::session::Session;
use crate::tolerance::{midpoint_price_or_zero, within_tolerance_band};

struct Reading {
    price_wad: i128,
    timestamp: u64,
    stale: bool,
}

#[derive(Clone, Copy)]
enum LegSlot {
    Primary,
    Secondary,
}

enum Legs {
    One(Reading),
    Two { primary: Reading, anchor: Reading },

    Partial { reading: Reading, slot: LegSlot },
    Empty,
}

pub(crate) struct Outcome {
    pub price_wad: i128,
    pub timestamp: u64,
    pub first_wad: i128,
    pub second_wad: i128,
    pub low_wad: i128,
    pub high_wad: i128,
    pub asset_decimals: u32,
    pub stale: bool,
    pub deviation: bool,

    pub err: Option<OracleError>,
}

impl Outcome {
    fn blank() -> Self {
        Outcome {
            price_wad: 0,
            timestamp: 0,
            first_wad: 0,
            second_wad: 0,
            low_wad: 0,
            high_wad: 0,
            asset_decimals: 0,
            stale: false,
            deviation: false,
            err: None,
        }
    }

    fn with_err(err: OracleError) -> Self {
        Outcome {
            err: Some(err),
            ..Self::blank()
        }
    }

    fn unreadable() -> Self {
        Self::with_err(OracleError::NoLastPrice)
    }

    fn one(r: Reading, asset_decimals: u32) -> Self {
        Outcome {
            price_wad: r.price_wad,
            timestamp: r.timestamp,
            first_wad: r.price_wad,
            second_wad: r.price_wad,
            low_wad: r.price_wad,
            high_wad: r.price_wad,
            asset_decimals,
            stale: r.stale,
            deviation: false,
            err: None,
        }
    }

    fn partial(reading: Reading, slot: LegSlot, asset_decimals: u32) -> Self {
        let (first_wad, second_wad) = match slot {
            LegSlot::Primary => (reading.price_wad, 0),
            LegSlot::Secondary => (0, reading.price_wad),
        };
        Outcome {
            price_wad: 0,
            timestamp: reading.timestamp,
            first_wad,
            second_wad,
            low_wad: 0,
            high_wad: 0,
            asset_decimals,
            stale: reading.stale,

            deviation: true,
            err: None,
        }
    }

    fn failure(&self, oracle: Option<&AssetOracle>) -> Option<OracleError> {
        if let Some(err) = self.err {
            return Some(err);
        }
        let Some(oracle) = oracle else {
            return Some(OracleError::OracleNotConfigured);
        };
        if self.stale {
            return Some(OracleError::PriceFeedStale);
        }
        if self.deviation {
            return Some(OracleError::UnsafePriceNotAllowed);
        }
        if self.price_wad <= 0 {
            return Some(OracleError::InvalidPrice);
        }
        if self.price_wad < oracle.min_sanity_price_wad
            || self.price_wad > oracle.max_sanity_price_wad
        {
            return Some(OracleError::SanityBoundViolated);
        }
        None
    }

    fn config_failure(&self) -> Option<OracleError> {
        match self.err {
            Some(
                err @ (OracleError::OracleCycleDetected
                | OracleError::OracleDepthExceeded
                | OracleError::SourceCountOutOfRange
                | OracleError::UnsupportedPoolKind
                | OracleError::OracleNotConfigured),
            ) => Some(err),
            _ => None,
        }
    }

    fn usable(&self, oracle: Option<&AssetOracle>) -> bool {
        self.failure(oracle).is_none()
    }

    fn to_feed(&self) -> PriceFeedRaw {
        PriceFeedRaw {
            price_wad: self.price_wad,
            asset_decimals: self.asset_decimals,
            timestamp: self.timestamp,
        }
    }
}

pub(crate) fn force(env: &Env, outcome: &Outcome, oracle: Option<&AssetOracle>) -> PriceFeedRaw {
    if let Some(err) = outcome.failure(oracle) {
        panic_with_error!(env, err);
    }
    outcome.to_feed()
}

pub(crate) fn to_status(outcome: &Outcome, oracle: Option<&AssetOracle>) -> PriceStatus {
    if outcome.err.is_some() {
        return PriceStatus::unusable();
    }
    PriceStatus {
        final_wad: outcome.price_wad,
        primary_wad: outcome.first_wad,
        secondary_wad: outcome.second_wad,
        price_timestamp: outcome.timestamp,
        stale: outcome.stale,
        deviation: outcome.deviation,
        valid: outcome.usable(oracle),
    }
}

pub(crate) fn resolve(session: &mut Session, key: &PriceKey, depth: u32) -> PriceFeedRaw {
    if let Some(cached) = session.cached_price(key) {
        return cached;
    }
    let (feed, _) = compute_hard(session, key, depth, None);
    session.store_price(key, feed.clone());
    feed
}

pub(crate) fn resolve_status(session: &mut Session, key: &PriceKey, depth: u32) -> PriceStatus {
    if let Some(cached) = session.cached_status(key) {
        return cached;
    }
    let (outcome, oracle) = resolve_outcome(session, key, depth, None);
    let status = to_status(&outcome, oracle.as_ref());
    session.store_status(key, status.clone());
    status
}

/// Strict probe: the config must produce a usable price right now, not merely be
/// structurally sound. Used when listing an asset whose price is derived rather
/// than reported - there is no incident to work around when adding something new,
/// and a derived source that cannot resolve at listing is a configuration error.
pub(crate) fn probe_priceable(session: &mut Session, key: &PriceKey, oracle: &AssetOracle) {
    let env = session.env().clone();
    let (outcome, resolved) = resolve_outcome(session, key, 0, Some(oracle));
    let _ = force(&env, &outcome, resolved.as_ref().or(Some(oracle)));
}

pub(crate) fn probe(session: &mut Session, key: &PriceKey, oracle: &AssetOracle) {
    let env = session.env().clone();
    let (outcome, _) = resolve_outcome(session, key, 0, Some(oracle));
    if let Some(err) = outcome.config_failure() {
        panic_with_error!(&env, err);
    }
}

pub(crate) fn resolve_detailed(
    session: &mut Session,
    key: &PriceKey,
    depth: u32,
) -> (PriceFeedRaw, Outcome) {
    let (feed, outcome) = compute_hard(session, key, depth, None);
    session.store_price(key, feed.clone());
    (feed, outcome)
}

fn compute_hard(
    session: &mut Session,
    key: &PriceKey,
    depth: u32,
    override_oracle: Option<&AssetOracle>,
) -> (PriceFeedRaw, Outcome) {
    let env = session.env().clone();
    let (outcome, oracle) = resolve_outcome(session, key, depth, override_oracle);
    let feed = force(&env, &outcome, oracle.as_ref());
    (feed, outcome)
}

fn resolve_nested(
    session: &mut Session,
    key: &PriceKey,
    depth: u32,
) -> Result<PriceFeedRaw, OracleError> {
    if let Some(cached) = session.cached_price(key) {
        validate_cached_path(session, key, depth)?;
        return Ok(cached);
    }
    if let Some(cached) = session.cached_error(key) {
        validate_cached_path(session, key, depth)?;
        return Err(cached);
    }
    let (outcome, oracle) = resolve_outcome(session, key, depth, None);
    if let Some(err) = outcome.failure(oracle.as_ref()) {
        session.store_error(key, err);
        return Err(err);
    }
    let feed = outcome.to_feed();
    session.store_price(key, feed.clone());
    Ok(feed)
}

fn validate_cached_path(
    session: &mut Session,
    key: &PriceKey,
    depth: u32,
) -> Result<(), OracleError> {
    if depth > MAX_RESOLUTION_DEPTH {
        return Err(OracleError::OracleDepthExceeded);
    }
    if session.is_resolving(key) {
        return Err(OracleError::OracleCycleDetected);
    }

    let env = session.env().clone();
    let oracle = admin::get_oracle(&env, key).ok_or(OracleError::OracleNotConfigured)?;
    session.push_key(key);
    let mut result = Ok(());
    for source in oracle.sources.iter() {
        if let PriceSource::Scaled(scaled) = source {
            if let Err(err) = validate_cached_path(session, &scaled.quote, depth + 1) {
                result = Err(err);
                break;
            }
        }
    }
    session.pop_key();
    result
}

fn resolve_outcome(
    session: &mut Session,
    key: &PriceKey,
    depth: u32,
    override_oracle: Option<&AssetOracle>,
) -> (Outcome, Option<AssetOracle>) {
    let env = session.env().clone();

    if depth > MAX_RESOLUTION_DEPTH {
        return (Outcome::with_err(OracleError::OracleDepthExceeded), None);
    }

    if session.is_resolving(key) {
        return (Outcome::with_err(OracleError::OracleCycleDetected), None);
    }
    session.push_key(key);

    let oracle = if let Some(o) = override_oracle {
        o.clone()
    } else {
        match admin::get_oracle(&env, key) {
            Some(o) => o,
            None => {
                session.pop_key();
                return (Outcome::with_err(OracleError::OracleNotConfigured), None);
            }
        }
    };

    let outcome = match compose(session, &oracle, depth) {
        Ok(legs) => blend(&env, &oracle, legs),
        Err(err) => Outcome::with_err(err),
    };
    session.pop_key();
    (outcome, Some(oracle))
}

fn blend(env: &Env, oracle: &AssetOracle, legs: Legs) -> Outcome {
    match legs {
        Legs::Empty => Outcome::unreadable(),
        Legs::One(r) => Outcome::one(r, oracle.asset_decimals),
        Legs::Partial { reading, slot } => Outcome::partial(reading, slot, oracle.asset_decimals),
        Legs::Two { primary, anchor } => {
            let stale = primary.stale || anchor.stale;
            let ts = primary.timestamp.min(anchor.timestamp);
            let deviation =
                !within_tolerance_band(env, anchor.price_wad, primary.price_wad, &oracle.tolerance);

            let price_wad = midpoint_price_or_zero(anchor.price_wad, primary.price_wad);
            Outcome {
                price_wad,
                timestamp: ts,
                first_wad: primary.price_wad,
                second_wad: anchor.price_wad,
                low_wad: primary.price_wad.min(anchor.price_wad),
                high_wad: primary.price_wad.max(anchor.price_wad),
                asset_decimals: oracle.asset_decimals,
                stale,
                deviation,
                err: None,
            }
        }
    }
}

#[cfg(feature = "certora")]
pub(crate) fn blend_empty(env: &Env, oracle: &AssetOracle) -> Outcome {
    blend(env, oracle, Legs::Empty)
}

#[cfg(feature = "certora")]
pub(crate) fn blend_partial(
    env: &Env,
    oracle: &AssetOracle,
    price_wad: i128,
    timestamp: u64,
    stale: bool,
    primary_slot: bool,
) -> Outcome {
    let slot = if primary_slot {
        LegSlot::Primary
    } else {
        LegSlot::Secondary
    };
    blend(
        env,
        oracle,
        Legs::Partial {
            reading: Reading {
                price_wad,
                timestamp,
                stale,
            },
            slot,
        },
    )
}

fn compose(session: &mut Session, oracle: &AssetOracle, depth: u32) -> Result<Legs, OracleError> {
    let count = oracle.sources.len();
    if count == 0 || count > 2 {
        return Err(OracleError::SourceCountOutOfRange);
    }

    let first = read_source(session, oracle, &oracle.sources.get_unchecked(0), depth)?;
    if count == 1 {
        return Ok(match first {
            Some(r) => Legs::One(r),
            None => Legs::Empty,
        });
    }

    let second = read_source(session, oracle, &oracle.sources.get_unchecked(1), depth)?;
    Ok(match (first, second) {
        (Some(primary), Some(anchor)) => Legs::Two { primary, anchor },
        (Some(reading), None) => Legs::Partial {
            reading,
            slot: LegSlot::Primary,
        },
        (None, Some(reading)) => Legs::Partial {
            reading,
            slot: LegSlot::Secondary,
        },
        (None, None) => Legs::Empty,
    })
}

fn read_source(
    session: &mut Session,
    oracle: &AssetOracle,
    source: &PriceSource,
    depth: u32,
) -> Result<Option<Reading>, OracleError> {
    let Some((observation, component_stale)) = evaluate_source(session, source, depth)? else {
        return Ok(None);
    };
    let timestamp = observation.timestamp();
    let stale = component_stale
        || is_stale(
            session.now_secs(),
            timestamp,
            oracle.max_price_stale_seconds,
        );
    Ok(Some(Reading {
        price_wad: observation.price_wad,
        timestamp,
        stale,
    }))
}

fn evaluate_source(
    session: &mut Session,
    source: &PriceSource,
    depth: u32,
) -> Result<Option<(OracleObservation, bool)>, OracleError> {
    match source {
        PriceSource::Feed(feed) => Ok(read_feed(session, feed)),
        PriceSource::Scaled(scaled) => read_scaled(session, scaled, depth),
        PriceSource::LpShare(lp) => read_lp_share(session, lp, depth),
    }
}

/// Prices a constant-product LP share from pool reserves and the two underlying
/// oracle prices, using the manipulation-resistant fair-value formula. The
/// reserves come from the pool's read-only plane mirror; the underlyings are
/// resolved through the normal nested path (depth/cycle/sanity guarded).
fn read_lp_share(
    session: &mut Session,
    lp: &LpShareSource,
    depth: u32,
) -> Result<Option<(OracleObservation, bool)>, OracleError> {
    match lp.kind {
        PoolKind::ConstantProduct => {}
    }
    let env = session.env().clone();

    let price_a = resolve_nested(session, &lp.key_a, depth + 1)?;
    let price_b = resolve_nested(session, &lp.key_b, depth + 1)?;

    let (reserve_a, reserve_b) = aquarius_plane_reserves_call(&env, &lp.plane, &lp.pool)
        .ok_or(OracleError::NoLastPrice)?;
    let total_shares =
        aquarius_total_shares_call(&env, &lp.pool).ok_or(OracleError::NoLastPrice)?;

    let price_wad = fair_lp_price_wad(
        &env,
        reserve_a,
        lp.reserve_a_decimals,
        price_a.price_wad,
        reserve_b,
        lp.reserve_b_decimals,
        price_b.price_wad,
        total_shares,
        lp.share_decimals,
    )?;

    // The share is only as fresh as its underlyings (already staleness-checked
    // by resolve_nested); carry the older leg's timestamp.
    Ok(Some((
        OracleObservation {
            price_wad,
            observed_at: price_a.timestamp.min(price_b.timestamp),
            published_at: None,
        },
        false,
    )))
}

fn read_feed(session: &mut Session, feed: &FeedSource) -> Option<(OracleObservation, bool)> {
    let observation = match &feed.provider {
        ProviderRef::Reflector(r) => reflector::read_reflector_source(session, r, feed.decimals),
        ProviderRef::MultiFeed(m) => multi_feed::read_multi_feed_source(session, m, feed.decimals),
    }?;

    let stale = is_stale(
        session.now_secs(),
        observation.timestamp(),
        feed.max_stale_seconds,
    );
    Some((observation, stale))
}

fn read_scaled(
    session: &mut Session,
    scaled: &ScaledSource,
    depth: u32,
) -> Result<Option<(OracleObservation, bool)>, OracleError> {
    let env = session.env().clone();
    let Some((factor, factor_stale)) = read_feed(session, &scaled.factor) else {
        return Ok(None);
    };

    if factor.price_wad < scaled.min_factor_wad || factor.price_wad > scaled.max_factor_wad {
        return Err(OracleError::FactorOutOfBounds);
    }

    let quote = resolve_nested(session, &scaled.quote, depth + 1)?;

    let Some(price_wad) = Wad::from(factor.price_wad).try_mul(&env, Wad::from(quote.price_wad))
    else {
        return Err(OracleError::InvalidPrice);
    };

    Ok(Some((
        OracleObservation {
            price_wad: price_wad.raw(),
            observed_at: factor.timestamp().min(quote.timestamp),
            published_at: None,
        },
        factor_stale,
    )))
}

#[cfg(test)]
#[path = "../tests/oracle/engine.rs"]
mod tests;
