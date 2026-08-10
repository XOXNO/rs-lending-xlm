use common::errors::OracleError;
use common::math::fp::Wad;
use common::oracle::observation::{is_stale, MAX_LEG_AGE_SPREAD_SECONDS};
use common::types::{
    AssetOracle, FeedSource, PriceFeedRaw, PriceKey, PriceSource, PriceStatus, ProviderRef,
    ScaledSource, MAX_RESOLUTION_DEPTH,
};
use soroban_sdk::{panic_with_error, Env};

use crate::observation::OracleObservation;
use crate::providers::{aquarius, redstone, reflector, xoxno};
use crate::registry;
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

    fn one(r: Reading) -> Self {
        Outcome {
            price_wad: r.price_wad,
            timestamp: r.timestamp,
            first_wad: r.price_wad,
            second_wad: r.price_wad,
            stale: r.stale,
            deviation: false,
            err: None,
        }
    }

    fn partial(reading: Reading, slot: LegSlot) -> Self {
        let (first_wad, second_wad) = match slot {
            LegSlot::Primary => (reading.price_wad, 0),
            LegSlot::Secondary => (0, reading.price_wad),
        };
        Outcome {
            price_wad: 0,
            timestamp: reading.timestamp,
            first_wad,
            second_wad,
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
                | OracleError::UnsupportedAquariusPool
                | OracleError::OracleNotConfigured),
            ) => Some(err),
            _ => None,
        }
    }

    fn usable(&self, oracle: Option<&AssetOracle>) -> bool {
        self.failure(oracle).is_none()
    }

    fn to_feed(&self, asset_decimals: u32) -> PriceFeedRaw {
        PriceFeedRaw {
            price_wad: self.price_wad,
            asset_decimals,
            timestamp: self.timestamp,
        }
    }
}

pub(crate) fn force(env: &Env, outcome: &Outcome, oracle: Option<&AssetOracle>) -> PriceFeedRaw {
    if let Some(err) = outcome.failure(oracle) {
        panic_with_error!(env, err);
    }
    let Some(oracle) = oracle else {
        panic_with_error!(env, OracleError::OracleNotConfigured)
    };
    outcome.to_feed(oracle.asset_decimals)
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

pub(crate) fn resolve_nested(
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
    let oracle = oracle.ok_or(OracleError::OracleNotConfigured)?;
    let feed = outcome.to_feed(oracle.asset_decimals);
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
    let oracle = registry::get_oracle(&env, key).ok_or(OracleError::OracleNotConfigured)?;
    session.push_key(key);
    let mut result = Ok(());
    'sources: for source in oracle.sources.iter() {
        match source {
            PriceSource::Feed(_) => {}
            PriceSource::Scaled(scaled) => {
                if let Err(err) = validate_cached_path(session, &scaled.quote, depth + 1) {
                    result = Err(err);
                    break;
                }
            }
            PriceSource::AquariusLp(lp) | PriceSource::AquariusStableLp(lp) => {
                for dependency in [&lp.key_a, &lp.key_b] {
                    if let Err(err) = validate_cached_path(session, dependency, depth + 1) {
                        result = Err(err);
                        break 'sources;
                    }
                }
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
        match registry::get_oracle(&env, key) {
            Some(o) => o,
            None => {
                session.pop_key();
                return (Outcome::with_err(OracleError::OracleNotConfigured), None);
            }
        }
    };

    let outcome = match compose(session, key, &oracle, depth) {
        Ok(legs) => blend(&env, &oracle, legs),
        Err(err) => Outcome::with_err(err),
    };
    session.pop_key();
    (outcome, Some(oracle))
}

fn blend(env: &Env, oracle: &AssetOracle, legs: Legs) -> Outcome {
    match legs {
        Legs::Empty => Outcome::unreadable(),
        Legs::One(r) => Outcome::one(r),
        Legs::Partial { reading, slot } => Outcome::partial(reading, slot),
        Legs::Two { primary, anchor } => {
            // The midpoint weights both legs equally, so a leg far older than
            // its partner would drag the result while each still satisfies its
            // own bound. Bound the spread as well as the absolute ages.
            let age_spread = primary.timestamp.abs_diff(anchor.timestamp);
            let stale = primary.stale || anchor.stale || age_spread > MAX_LEG_AGE_SPREAD_SECONDS;
            let ts = primary.timestamp.min(anchor.timestamp);
            let deviation =
                !within_tolerance_band(env, anchor.price_wad, primary.price_wad, &oracle.tolerance);

            let price_wad = midpoint_price_or_zero(anchor.price_wad, primary.price_wad);
            Outcome {
                price_wad,
                timestamp: ts,
                first_wad: primary.price_wad,
                second_wad: anchor.price_wad,
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

fn compose(
    session: &mut Session,
    key: &PriceKey,
    oracle: &AssetOracle,
    depth: u32,
) -> Result<Legs, OracleError> {
    let count = oracle.sources.len();
    if count == 0 || count > 2 {
        return Err(OracleError::SourceCountOutOfRange);
    }

    let first = read_source(
        session,
        key,
        oracle,
        &oracle.sources.get_unchecked(0),
        depth,
    )?;
    if count == 1 {
        return Ok(match first {
            Some(r) => Legs::One(r),
            None => Legs::Empty,
        });
    }

    let second = read_source(
        session,
        key,
        oracle,
        &oracle.sources.get_unchecked(1),
        depth,
    )?;
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
    key: &PriceKey,
    oracle: &AssetOracle,
    source: &PriceSource,
    depth: u32,
) -> Result<Option<Reading>, OracleError> {
    let Some((observation, component_stale)) =
        evaluate_source(session, key, source, oracle.asset_decimals, depth)?
    else {
        return Ok(None);
    };
    let timestamp = observation.timestamp;
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
    key: &PriceKey,
    source: &PriceSource,
    asset_decimals: u32,
    depth: u32,
) -> Result<Option<(OracleObservation, bool)>, OracleError> {
    match source {
        PriceSource::Feed(feed) => Ok(read_feed(session, feed)),
        PriceSource::Scaled(scaled) => read_scaled(session, scaled, depth),
        PriceSource::AquariusLp(lp) => aquarius::read(session, key, lp, asset_decimals, depth),
        PriceSource::AquariusStableLp(lp) => {
            aquarius::read_stable(session, key, lp, asset_decimals, depth)
        }
    }
}

fn read_feed(session: &mut Session, feed: &FeedSource) -> Option<(OracleObservation, bool)> {
    let observation = match &feed.provider {
        ProviderRef::Reflector(r) => reflector::read_reflector_source(session, r, feed.decimals),
        ProviderRef::RedStone(r) => redstone::read(session, r, feed.decimals),
        ProviderRef::Xoxno(x) => xoxno::read(session, x, feed.decimals),
    }?;

    let stale = is_stale(
        session.now_secs(),
        observation.timestamp,
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
            timestamp: factor.timestamp.min(quote.timestamp),
        },
        factor_stale,
    )))
}

#[cfg(test)]
#[path = "../tests/oracle/engine.rs"]
mod tests;
