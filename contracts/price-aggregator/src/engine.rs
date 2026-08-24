//! Core price-resolution engine: reads an oracle's configured sources, blends them
//! into a single price with staleness and deviation flags, and enforces cycle and
//! depth limits while resolving nested (scaled and Aquarius LP) compositions.

use common::errors::OracleError;
use common::math::fp::Wad;
use common::oracle::observation::{is_stale, MAX_LEG_AGE_SPREAD_SECONDS};
use common::types::{
    AssetOracle, FeedNature, FeedSource, PriceFeedRaw, PriceKey, PriceSource, PriceStatus,
    ProviderRef, ScaledSource, MAX_RESOLUTION_DEPTH,
};
use soroban_sdk::{panic_with_error, Env};

use crate::observation::OracleObservation;
use crate::providers::{aquarius, multi_feed, reflector};
use crate::registry;
use crate::session::Session;
use crate::tolerance::{midpoint_price_or_zero, within_tolerance_band};

/// A single source's resolved value: price, observation timestamp, and whether it
/// is considered stale.
struct Reading {
    price_wad: i128,
    timestamp: u64,
    stale: bool,
    nature: FeedNature,
}

/// Identifies which of an oracle's two source legs a `Legs::Partial` reading fills.
#[derive(Clone, Copy)]
enum LegSlot {
    Primary,
    Secondary,
}

/// The set of readings produced by composing an oracle's configured sources: both
/// legs, a single leg (single-source oracle), only one of two configured legs, or
/// none.
enum Legs {
    One(Reading),
    Two { primary: Reading, anchor: Reading },
    Partial { reading: Reading, slot: LegSlot },
    Empty,
}

/// The result of resolving an oracle's price: the blended price and timestamp,
/// each leg's raw value, staleness and deviation flags, and an error when
/// resolution failed outright.
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
    /// Constructs a zeroed `Outcome` with no flags set and no error.
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

    /// Constructs an `Outcome` carrying only the given error.
    fn with_err(err: OracleError) -> Self {
        Outcome {
            err: Some(err),
            ..Self::blank()
        }
    }

    /// Constructs an `Outcome` for the case where none of an oracle's sources
    /// produced a reading.
    fn unreadable() -> Self {
        Self::with_err(OracleError::NoLastPrice)
    }

    /// Constructs an `Outcome` from a single reading, used for single-source
    /// oracles, with both leg values set to that reading's price.
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

    /// Constructs an `Outcome` for a two-source oracle where only one of the two
    /// legs produced a reading. Leaves the blended price at zero and marks the
    /// outcome as a deviation.
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

    /// Returns the error that makes this outcome unusable against `oracle`, if
    /// any. Checks, in order: an error already carried on the outcome, a missing
    /// oracle, staleness, deviation, a non-positive price, and the oracle's
    /// sanity price bounds. Returns `None` when none of these apply.
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

    /// Returns the carried error only if it is a configuration-level failure
    /// (cycle detected, depth exceeded, source count out of range, unsupported
    /// Aquarius pool, or oracle not configured); returns `None` for any other
    /// error or for no error, letting market-condition failures (staleness,
    /// deviation, sanity bounds) pass through as non-fatal.
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

    /// Returns whether this outcome has no applicable failure against `oracle`.
    fn usable(&self, oracle: Option<&AssetOracle>) -> bool {
        self.failure(oracle).is_none()
    }

    /// Converts this outcome's blended price and timestamp into a `PriceFeedRaw`
    /// tagged with `asset_decimals`.
    fn to_feed(&self, asset_decimals: u32) -> PriceFeedRaw {
        PriceFeedRaw {
            price_wad: self.price_wad,
            asset_decimals,
            timestamp: self.timestamp,
        }
    }
}

/// Converts `outcome` into a `PriceFeedRaw`, panicking with the applicable
/// `OracleError` if the outcome is unusable against `oracle` or `oracle` is
/// `None`.
pub(crate) fn force(env: &Env, outcome: &Outcome, oracle: Option<&AssetOracle>) -> PriceFeedRaw {
    if let Some(err) = outcome.failure(oracle) {
        panic_with_error!(env, err);
    }
    let Some(oracle) = oracle else {
        panic_with_error!(env, OracleError::OracleNotConfigured)
    };
    outcome.to_feed(oracle.asset_decimals)
}

/// Converts `outcome` into a `PriceStatus`. Returns an unusable status when the
/// outcome carries an error; otherwise reports the blended and leg prices,
/// timestamp, staleness and deviation flags, and whether the outcome is valid
/// against `oracle`.
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

/// Resolves the price feed for `key`, returning the session-cached value if
/// present. Otherwise computes it, panicking on any resolution failure, and
/// caches the result on the session before returning it.
pub(crate) fn resolve(session: &mut Session, key: &PriceKey, depth: u32) -> PriceFeedRaw {
    if let Some(cached) = session.cached_price(key) {
        return cached;
    }
    let (feed, _) = compute_hard(session, key, depth, None);
    session.store_price(key, feed.clone());
    feed
}

/// Resolves the `PriceStatus` for `key`, returning the session-cached status if
/// present. Otherwise computes it without panicking, caches the result on the
/// session, and returns it.
pub(crate) fn resolve_status(session: &mut Session, key: &PriceKey, depth: u32) -> PriceStatus {
    if let Some(cached) = session.cached_status(key) {
        return cached;
    }
    let (outcome, oracle) = resolve_outcome(session, key, depth, None);
    let status = to_status(&outcome, oracle.as_ref());
    session.store_status(key, status.clone());
    status
}

/// Resolves `oracle` for `key` at depth 0 and panics with the applicable
/// `OracleError` if it produces any unusable outcome, including market-condition
/// failures such as staleness or sanity-bound violations. Used to confirm an
/// oracle (typically one with an Aquarius LP source) is fully priceable before
/// it is stored.
pub(crate) fn probe_priceable(session: &mut Session, key: &PriceKey, oracle: &AssetOracle) {
    let env = session.env().clone();
    let (outcome, resolved) = resolve_outcome(session, key, 0, Some(oracle));
    let _ = force(&env, &outcome, resolved.as_ref().or(Some(oracle)));
}

/// Resolves `oracle` for `key` at depth 0 and panics only if the outcome carries
/// a configuration-level error (cycle, depth, source count, unsupported pool, or
/// missing oracle). Market-condition failures such as staleness or sanity-bound
/// violations do not panic here.
pub(crate) fn probe(session: &mut Session, key: &PriceKey, oracle: &AssetOracle) {
    let env = session.env().clone();
    let (outcome, _) = resolve_outcome(session, key, 0, Some(oracle));
    if let Some(err) = outcome.config_failure() {
        panic_with_error!(&env, err);
    }
}

/// Resolves the price feed for `key`, panicking on failure, caches it on the
/// session, and returns both the feed and the underlying `Outcome`.
pub(crate) fn resolve_detailed(
    session: &mut Session,
    key: &PriceKey,
    depth: u32,
) -> (PriceFeedRaw, Outcome) {
    let (feed, outcome) = compute_hard(session, key, depth, None);
    session.store_price(key, feed.clone());
    (feed, outcome)
}

/// Resolves the outcome for `key` and forces it into a `PriceFeedRaw`, panicking
/// on any failure. Returns both the feed and the outcome it was derived from.
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

/// Resolves the price feed for `key` during composition of a dependent source
/// (for example a scaled source's quote leg). Returns the session-cached price
/// or cached error if present, after validating that the cached path still
/// respects the depth and cycle-detection limits at `depth`. Otherwise computes
/// the outcome fresh, caches either the resulting price or the resulting error
/// on the session, and returns the corresponding `Result`.
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

/// Validates that resolving `key` from a session cache hit would still respect
/// the depth and cycle limits at `depth`, by walking the oracle's configured
/// sources (recursing into a scaled source's quote and an Aquarius LP source's
/// paired dependencies) as if resolving them fresh. Returns the first
/// `OracleError` encountered, if any.
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

/// Resolves the `Outcome` for `key` at `depth`: enforces the depth and cycle
/// limits, loads the oracle configuration (`override_oracle` if given, otherwise
/// from the registry), composes and blends its sources, and tracks `key` on the
/// session's resolution stack for the duration of the call to detect cycles.
/// Returns the outcome together with the oracle configuration used, when one was
/// found.
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

/// Converts composed `Legs` into an `Outcome`. For two readings, marks the
/// outcome stale when either leg is individually stale, or when both legs
/// are market-nature feeds whose timestamps differ by more than the maximum
/// allowed leg-age spread. Takes the earlier of the two timestamps, flags a
/// deviation when the legs fall outside the oracle's tolerance band, and
/// sets the blended price to the midpoint of the two legs (zero if the
/// midpoint computation fails).
fn blend(env: &Env, oracle: &AssetOracle, legs: Legs) -> Outcome {
    match legs {
        Legs::Empty => Outcome::unreadable(),
        Legs::One(r) => Outcome::one(r),
        Legs::Partial { reading, slot } => Outcome::partial(reading, slot),
        Legs::Two { primary, anchor } => {
            // The midpoint weights both legs equally, so a market leg far older
            // than its market partner would drag the result while each still
            // satisfies its own bound. A fundamental leg is exempt: it prices a
            // slow-moving quantity and its own bound is the intended budget, so
            // holding it to a market leg's cadence would only fail closed.
            let spread_bounded =
                primary.nature == FeedNature::Market && anchor.nature == FeedNature::Market;
            let age_spread = primary.timestamp.abs_diff(anchor.timestamp);
            let stale = primary.stale
                || anchor.stale
                || (spread_bounded && age_spread > MAX_LEG_AGE_SPREAD_SECONDS);
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

// Gated to exactly the configurations where its only caller exists: the
// spec module compiles `oracle_rules` only when the build is unfocused or
// targets that rule set, so under any other focused build these have no
// caller. Matching the caller's cfg is what keeps that from warning --
// `#[allow(dead_code)]` would hide it instead of describing it.
/// Certora harness entry point: blends an empty leg set for `oracle`, exercising
/// the same path as an oracle whose sources produced no readings.
#[cfg(all(
    feature = "certora",
    any(not(feature = "certora-focused"), feature = "certora-oracle-rules")
))]
pub(crate) fn blend_empty(env: &Env, oracle: &AssetOracle) -> Outcome {
    blend(env, oracle, Legs::Empty)
}

/// Certora harness entry point: blends a single partial reading into a leg slot
/// for `oracle`, exercising the same path as a two-source oracle where only one
/// leg produced a reading.
#[cfg(all(
    feature = "certora",
    any(not(feature = "certora-focused"), feature = "certora-oracle-rules")
))]
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
                nature: FeedNature::Market,
            },
            slot,
        },
    )
}

/// Reads each of `oracle`'s configured sources (one or two) and assembles the
/// resulting `Legs` variant based on which sources produced a reading. Returns
/// `SourceCountOutOfRange` if the oracle has zero or more than two sources.
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

/// Evaluates `source` into a `Reading`, if it produces one. Combines the
/// source's own component-level staleness with staleness computed against
/// `oracle.max_price_stale_seconds`.
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
        nature: source_nature(source),
    }))
}

/// Returns the feed nature of `source`: the provider's nature for a plain
/// feed, the factor feed's provider nature for a scaled source (the quote
/// leg's nature is not considered), and always `Market` for an Aquarius LP
/// or Aquarius stable-LP source.
fn source_nature(source: &PriceSource) -> FeedNature {
    match source {
        PriceSource::Feed(feed) => feed.provider.nature(),
        PriceSource::Scaled(scaled) => scaled.factor.provider.nature(),
        PriceSource::AquariusLp(_) | PriceSource::AquariusStableLp(_) => FeedNature::Market,
    }
}

/// Dispatches `source` to its evaluation routine (feed, scaled, Aquarius LP, or
/// Aquarius stable LP), returning the resulting observation and its
/// component-level staleness flag, if any.
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
        PriceSource::AquariusLp(lp) => {
            aquarius::read(session, key, lp, asset_decimals, depth, false)
        }
        PriceSource::AquariusStableLp(lp) => {
            aquarius::read(session, key, lp, asset_decimals, depth, true)
        }
    }
}

/// Reads `feed` from its provider (Reflector, RedStone, or Xoxno), returning
/// `None` if the provider has no observation. Computes staleness against
/// `feed.max_stale_seconds`.
fn read_feed(session: &mut Session, feed: &FeedSource) -> Option<(OracleObservation, bool)> {
    let observation = match &feed.provider {
        ProviderRef::Reflector(r) => reflector::read_reflector_source(session, r, feed.decimals),
        ProviderRef::RedStone(r) => multi_feed::read_multi_feed_source(session, r, feed.decimals),
        ProviderRef::Xoxno(x) => multi_feed::read_multi_feed_source(session, x, feed.decimals),
    }?;

    let stale = is_stale(
        session.now_secs(),
        observation.timestamp,
        feed.max_stale_seconds,
    );
    Some((observation, stale))
}

/// Reads a scaled source: reads the factor feed (returning `None` if unread),
/// checks it falls within `scaled.min_factor_wad`/`max_factor_wad`, resolves the
/// nested `quote` price at `depth + 1`, and multiplies factor and quote into a
/// price using the earlier of their two timestamps. Returns
/// `FactorOutOfBounds` if the factor is out of range and `InvalidPrice` if the
/// multiplication fails.
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
