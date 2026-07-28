//! Price resolution: compose once into an [`Outcome`], then hard [`force`] or
//! soft [`to_status`].
//!
//! Soft and hard share every step of compose, blend, and gates. Only the edge
//! differs: [`force`] panics with the code from [`Outcome::failure`];
//! [`to_status`] maps the same failure into flags (`valid`, `stale`,
//! `deviation`, unusable).
//!
//! Gate order in [`Outcome::failure`]:
//! 1. Structural / nested `err` (config, cycle, depth, factor band, quote)
//! 2. Unreadable market data → `NoLastPrice`
//! 3. Stale (per-feed window and asset ceiling)
//! 4. Dual-source disagreement (`deviation`)
//! 5. Non-positive final USD price
//! 6. Sanity band
//!
//! Provider reads return `Option` for market data. Structural failures that
//! must not look like a soft miss are carried on [`Outcome::err`] and surface
//! through [`force`] with their precise code. Scaled nested quotes re-run the
//! same evaluator and promote nested gate failures as typed `err`.

use common::errors::OracleError;
use common::math::fp::Wad;
use common::oracle::observation::is_stale;
use common::types::{
    AssetOracle, FeedSource, PriceFeedRaw, PriceKey, PriceSource, PriceStatus, ProviderRef,
    ScaledSource, MAX_RESOLUTION_DEPTH,
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

/// Which dual leg produced a reading when the other is missing.
#[derive(Clone, Copy)]
enum LegSlot {
    Primary,
    Secondary,
}

enum Legs {
    One(Reading),
    Two {
        primary: Reading,
        anchor: Reading,
    },
    /// Dual config with exactly one readable leg → always `deviation`.
    Partial {
        reading: Reading,
        slot: LegSlot,
    },
    Empty,
}

/// Fully evaluated price with gate flags. Soft and hard share this shape.
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
    pub unreadable: bool,
    /// Structural or nested-precise failure. Soft → unusable; hard → panic code.
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
            unreadable: false,
            err: None,
        }
    }

    fn with_err(err: OracleError) -> Self {
        Outcome {
            unreadable: true,
            err: Some(err),
            ..Self::blank()
        }
    }

    fn unreadable() -> Self {
        Outcome {
            unreadable: true,
            ..Self::blank()
        }
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
            unreadable: false,
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
            // Dual config without both opinions never agreed.
            deviation: true,
            unreadable: false,
            err: None,
        }
    }

    /// Shared gate order for hard reverts and soft `valid`.
    ///
    /// `oracle` is `None` only when config was missing (`err` already set).
    fn failure(&self, oracle: Option<&AssetOracle>) -> Option<OracleError> {
        if let Some(err) = self.err {
            return Some(err);
        }
        if self.unreadable {
            return Some(OracleError::NoLastPrice);
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

/// Fail-closed: panic with the precise gate or structural error.
pub(crate) fn force(env: &Env, outcome: &Outcome, oracle: Option<&AssetOracle>) -> PriceFeedRaw {
    if let Some(err) = outcome.failure(oracle) {
        panic_with_error!(env, err);
    }
    outcome.to_feed()
}

/// Soft diagnostic flags. `valid` matches hard success via [`Outcome::failure`].
pub(crate) fn to_status(outcome: &Outcome, oracle: Option<&AssetOracle>) -> PriceStatus {
    if outcome.err.is_some() || outcome.unreadable {
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

/// Hard USD price for `key`. Returns the price memo when present.
pub(crate) fn resolve(session: &mut Session, key: &PriceKey, depth: u32) -> PriceFeedRaw {
    if let Some(cached) = session.cached_price(key) {
        return cached;
    }
    let (feed, _) = compute_hard(session, key, depth, None);
    session.store_price(key, feed.clone());
    feed
}

/// Soft status for `key`. Returns the status memo when present.
pub(crate) fn resolve_status(session: &mut Session, key: &PriceKey, depth: u32) -> PriceStatus {
    if let Some(cached) = session.cached_status(key) {
        return cached;
    }
    let (outcome, oracle) = resolve_outcome(session, key, depth, None);
    let status = to_status(&outcome, oracle.as_ref());
    session.store_status(key, status.clone());
    status
}

/// Configure-time hard probe against an unstored (or staged) config.
/// Does not write price or status memos.
pub(crate) fn probe(session: &mut Session, key: &PriceKey, oracle: &AssetOracle) {
    let env = session.env().clone();
    let (outcome, resolved) = resolve_outcome(session, key, 0, Some(oracle));
    let _ = force(&env, &outcome, resolved.as_ref().or(Some(oracle)));
}

/// Hard resolve plus full [`Outcome`] (leg interval). Recomputes even when a
/// price memo exists, then refreshes that memo.
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

/// Mode-independent evaluation. Does not panic on market-data misses.
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

    // Soft and hard both refuse re-entry without panicking mid-eval.
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
            // Overflow → 0 → force maps to InvalidPrice (soft: invalid).
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
                unreadable: false,
                err: None,
            }
        }
    }
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
        // Refused at config validate; hard backstop if storage is corrupted.
        PriceSource::LpShare(_) => Err(OracleError::UnsupportedPoolKind),
    }
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

    // Same evaluator as the top level — only force / to_status diverge.
    let (quote_out, quote_oracle) = resolve_outcome(session, &scaled.quote, depth + 1, None);
    if let Some(err) = quote_out.failure(quote_oracle.as_ref()) {
        return Err(err);
    }

    let Some(price_wad) = Wad::from(factor.price_wad).try_mul(&env, Wad::from(quote_out.price_wad))
    else {
        return Err(OracleError::InvalidPrice);
    };

    Ok(Some((
        OracleObservation {
            price_wad: price_wad.raw(),
            observed_at: factor.timestamp().min(quote_out.timestamp),
            published_at: None,
        },
        factor_stale,
    )))
}

#[cfg(test)]
#[path = "../tests/oracle/engine.rs"]
mod tests;
