//! Price resolution over the composable model.
//!
//! One composition, two renderers. [`compose`] reads every source and applies
//! every rule that decides an outcome — staleness, factor bounds, agreement —
//! exactly once. [`render_hard`] turns that into a price or a revert;
//! [`render_soft`] turns the same thing into flags. Neither renderer re-decides
//! anything, which is what keeps `price` and `price_status` from drifting apart.
//!
//! # Guards, and where each one lives
//!
//! * **Per-feed staleness** — every [`FeedSource`] carries its own bound and is
//!   checked against it the moment it is read.
//! * **Composite staleness** — a source made of several feeds is only as fresh
//!   as its stalest component, and that `min` is checked again at the asset-level
//!   ceiling. Both are needed: per-feed alone lets a frozen slow leg ride under
//!   a live fast one for the slow leg's entire window, which is exactly what a
//!   ratio feed going silent during a depeg looks like.
//! * **Factor bounds** — a [`ScaledSource`]'s ratio is checked against its own
//!   range before it reaches the product.
//! * **Agreement** — two sources must land inside the tolerance band.
//! * **Sanity band** — the final price must sit inside the configured bounds.
//! * **Cycle and depth** — enforced on entry, before any config is read, by the
//!   single guarded entry [`resolve_detailed`]. Every caller routes through it.
//!
//! The hard path never falls back. Every failure reverts, because a lending
//! protocol that guesses a price is one that has already lost the money. The
//! soft path never reverts on a *per-asset* problem, because a view that dies
//! on one bad market is a view nobody can use during an incident.
//!
//! # Quote chains are allowed, deliberately
//!
//! A [`ScaledSource`]'s quote may itself be composed, up to
//! [`MAX_RESOLUTION_DEPTH`]. That is the point of the model — SolvBTC needs one
//! hop, an LP share needs two — and the depth cap plus the cycle stack bound it
//! directly. Every hop resolves through this engine, so each intermediate faces
//! its own sanity band, staleness gates, and agreement check before being
//! multiplied into anything. The residual cost is compounding error per hop,
//! which is why the cap is 3 rather than the largest number that works.

use common::errors::OracleError;
use common::math::fp::Wad;
use common::oracle::observation::is_stale;
use common::oracle::policy::require_factor_in_bounds;
use common::types::{
    AssetOracle, FeedSource, PriceFeedRaw, PriceKey, PriceSource, PriceStatus, ProviderRef,
    RedStoneSourceConfig, ReflectorBase, ReflectorSourceConfig, ScaledSource, MAX_RESOLUTION_DEPTH,
};
use soroban_sdk::{panic_with_error, Env};

use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use crate::providers::{multi_feed, reflector};
use crate::registry;
use crate::tolerance::{midpoint_if_in_band, midpoint_price_or_zero, within_tolerance_band};

/// Read discipline. `Hard` lets a provider revert with its own precise error;
/// `Soft` maps every per-asset problem to an absent reading.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Hard,
    Soft,
}

impl Mode {
    fn soft(self) -> bool {
        self == Mode::Soft
    }
}

/// One source, read and dated. `stale` is recorded rather than acted on, so the
/// renderers decide what it means.
struct Reading {
    price_wad: i128,
    timestamp: u64,
    stale: bool,
}

/// Every source a config calls for, in order. `None` is an unreadable source.
struct Composition {
    first: Option<Reading>,
    /// `None` when the config is single-source; `Some(None)` when it is dual and
    /// the second source could not be read.
    second: Option<Option<Reading>>,
}

/// A resolved price plus the interval its sources spanned.
///
/// `low`/`high` are carried even though only `price_wad` is published today.
/// The combination rule is the open question in this design — a source
/// compromised high moves a midpoint by half that error, where collateral wants
/// the low end and debt the high end — and keeping the band computed here means
/// widening `PriceFeedRaw` later is a change to the boundary, not to the engine.
pub(crate) struct Resolved {
    pub price_wad: i128,
    pub timestamp: u64,
    pub low_wad: i128,
    pub high_wad: i128,
}

/// A resolved key together with the interval its sources spanned.
pub(crate) struct ResolvedKey {
    pub feed: PriceFeedRaw,
    pub low_wad: i128,
    pub high_wad: i128,
}

// ---------------------------------------------------------------------------
// Hard path
// ---------------------------------------------------------------------------

/// USD price for `key`, fail-closed.
///
/// # Errors
/// * [`OracleError::OracleDepthExceeded`] / [`OracleError::OracleCycleDetected`]
/// * [`OracleError::OracleNotConfigured`] - no config stored for `key`.
/// * [`OracleError::PriceFeedStale`] - a feed, or a composite, past its bound.
/// * [`OracleError::FactorOutOfBounds`] - a scaled ratio outside its range.
/// * [`OracleError::UnsafePriceNotAllowed`] - two sources outside tolerance.
/// * [`OracleError::SanityBoundViolated`] - final price outside the band.
/// * [`OracleError::SourceCountOutOfRange`] - stored config holds no sources.
pub(crate) fn resolve(cache: &mut ResolutionContext, key: &PriceKey, depth: u32) -> PriceFeedRaw {
    if let Some(cached) = cache.cached_key_price(key) {
        return cached;
    }
    resolve_detailed(cache, key, depth).feed
}

/// The single guarded entry into hard resolution.
///
/// Every caller goes through here, so the depth cap and the cycle push apply
/// uniformly. A view that reached the renderer directly would price a
/// self-quoting root one level deeper than it should and would skip the depth
/// cap for the root entirely.
pub(crate) fn resolve_detailed(
    cache: &mut ResolutionContext,
    key: &PriceKey,
    depth: u32,
) -> ResolvedKey {
    let env = cache.env().clone();
    require_depth_within_cap(&env, depth);

    // Pushed before the config is read: a guard installed after resolution
    // cannot see the re-entry it exists to catch.
    cache.push_price_key(key);

    let Some(oracle) = registry::resolve_oracle(&env, key) else {
        panic_with_error!(&env, OracleError::OracleNotConfigured)
    };

    let composition = compose(cache, &oracle, depth, Mode::Hard);
    let resolved = render_hard(&env, &oracle, &composition);
    let feed = PriceFeedRaw {
        price_wad: resolved.price_wad,
        asset_decimals: oracle.asset_decimals,
        timestamp: resolved.timestamp,
    };

    cache.pop_price_key();
    // Memoized only after every guard has passed, so a cached entry is always a
    // fully-checked one. The memo has exactly one writer for that reason.
    cache.store_key_price(key, feed.clone());
    ResolvedKey {
        feed,
        low_wad: resolved.low_wad,
        high_wad: resolved.high_wad,
    }
}

/// Resolves `key` against a config that is not (yet) stored.
///
/// Used by the sanity-band walk, which must prove the live price sits inside a
/// proposed band before committing it. Deliberately does **not** memoize: the
/// price memo has exactly one writer, and a probe result describes a config no
/// reader should see.
///
/// # Errors
/// Same variants as [`resolve`].
pub(crate) fn resolve_probe(
    cache: &mut ResolutionContext,
    key: &PriceKey,
    oracle: &AssetOracle,
) -> Resolved {
    let env = cache.env().clone();
    cache.push_price_key(key);
    let composition = compose(cache, oracle, 0, Mode::Hard);
    let resolved = render_hard(&env, oracle, &composition);
    cache.pop_price_key();
    resolved
}

/// Turns a composition into a price, reverting on anything that would make it
/// unsafe. Decides nothing `compose` has not already established.
fn render_hard(env: &Env, oracle: &AssetOracle, composition: &Composition) -> Resolved {
    let first = require_readable(env, composition.first.as_ref());
    require_fresh(env, first);

    let resolved = match composition.second.as_ref() {
        None => Resolved {
            price_wad: first.price_wad,
            timestamp: first.timestamp,
            low_wad: first.price_wad,
            high_wad: first.price_wad,
        },
        Some(second) => {
            let second = require_readable(env, second.as_ref());
            require_fresh(env, second);
            // `midpoint_if_in_band` is symmetric in its two arguments, so source
            // order cannot change either the price or the accept/reject outcome.
            let price_wad =
                midpoint_if_in_band(env, second.price_wad, first.price_wad, &oracle.tolerance);
            Resolved {
                price_wad,
                // A blend is only as fresh as the weaker source it rests on.
                timestamp: first.timestamp.min(second.timestamp),
                low_wad: first.price_wad.min(second.price_wad),
                high_wad: first.price_wad.max(second.price_wad),
            }
        }
    };

    require_within_sanity_band(env, resolved.price_wad, oracle);
    resolved
}

// ---------------------------------------------------------------------------
// Soft path
// ---------------------------------------------------------------------------

/// Diagnostic status for `key`. Never reverts on a per-asset problem.
///
/// `valid` is true exactly when [`resolve`] would not revert, which is the
/// property that lets a view be trusted as a pre-flight check. It holds because
/// both paths render the same [`Composition`] under the same rules.
pub(crate) fn resolve_status(
    cache: &mut ResolutionContext,
    key: &PriceKey,
    depth: u32,
) -> PriceStatus {
    if let Some(cached) = cache.cached_key_status(key) {
        return cached;
    }
    let status = compute_status(cache, key, depth);
    cache.store_key_status(key, status.clone());
    status
}

fn compute_status(cache: &mut ResolutionContext, key: &PriceKey, depth: u32) -> PriceStatus {
    let env = cache.env().clone();
    // Depth and cycles are structural, not per-asset: an over-deep or looping
    // config is unusable rather than merely unhealthy.
    if depth > MAX_RESOLUTION_DEPTH || cache.is_price_key_resolving(key) {
        return PriceStatus::unusable();
    }
    let Some(oracle) = registry::resolve_oracle(&env, key) else {
        return PriceStatus::unusable();
    };

    cache.push_price_key(key);
    let composition = compose(cache, &oracle, depth, Mode::Soft);
    cache.pop_price_key();

    render_soft(&env, &oracle, &composition)
}

/// Turns a composition into flags. Mirrors [`render_hard`] decision for
/// decision; `is_valid` is the soft form of the hard path's final gates.
fn render_soft(env: &Env, oracle: &AssetOracle, composition: &Composition) -> PriceStatus {
    let Some(first) = composition.first.as_ref() else {
        return PriceStatus::unusable();
    };

    match composition.second.as_ref() {
        None => {
            let stale = first.stale;
            PriceStatus {
                final_wad: first.price_wad,
                primary_wad: first.price_wad,
                secondary_wad: first.price_wad,
                price_timestamp: first.timestamp,
                stale,
                deviation: false,
                valid: is_valid(first.price_wad, stale, false, oracle),
            }
        }
        Some(second) => {
            // A dual config with one readable source has no second opinion, so
            // the pair never agreed - which is what `deviation` records. The
            // view still reports the leg it did read.
            let Some(second) = second.as_ref() else {
                return PriceStatus {
                    final_wad: 0,
                    primary_wad: first.price_wad,
                    secondary_wad: 0,
                    price_timestamp: first.timestamp,
                    stale: first.stale,
                    deviation: true,
                    valid: false,
                };
            };

            // A diagnostic shows the number it rejected, so the midpoint is
            // reported either way and `deviation` is what keeps it out of `valid`.
            let final_wad = midpoint_price_or_zero(env, second.price_wad, first.price_wad);
            let deviation =
                !within_tolerance_band(env, second.price_wad, first.price_wad, &oracle.tolerance);
            let stale = first.stale || second.stale;
            PriceStatus {
                final_wad,
                primary_wad: first.price_wad,
                secondary_wad: second.price_wad,
                price_timestamp: first.timestamp.min(second.timestamp),
                stale,
                deviation,
                valid: is_valid(final_wad, stale, deviation, oracle),
            }
        }
    }
}

/// True when a composed price would also survive the fail-closed path.
fn is_valid(final_wad: i128, stale: bool, deviation: bool, oracle: &AssetOracle) -> bool {
    if stale || deviation || final_wad <= 0 {
        return false;
    }
    final_wad >= oracle.min_sanity_price_wad && final_wad <= oracle.max_sanity_price_wad
}

// ---------------------------------------------------------------------------
// Composition, shared by both renderers
// ---------------------------------------------------------------------------

/// Reads every source the config calls for and records its freshness.
///
/// # Errors
/// * [`OracleError::SourceCountOutOfRange`] - a stored config with no sources,
///   or more than the model admits. **Hard mode only**: the soft renderers
///   exist so a view never reverts, and the controller prices a whole portfolio
///   in one call, so one structurally-corrupt entry taking down every other
///   asset's status would be a worse failure than reporting it unusable. In
///   soft mode this returns an empty composition, which renders `valid: false`.
///
/// Such a config is unreachable through `set_asset_oracle`, which enforces
/// arity; it could only arrive through a storage-level fault or a test seed.
fn compose(
    cache: &mut ResolutionContext,
    oracle: &AssetOracle,
    depth: u32,
    mode: Mode,
) -> Composition {
    let env = cache.env().clone();
    let count = oracle.sources.len();
    if count == 0 || count > 2 {
        if mode.soft() {
            return Composition {
                first: None,
                second: None,
            };
        }
        panic_with_error!(&env, OracleError::SourceCountOutOfRange);
    }

    let first = read_source(cache, oracle, &oracle.sources.get_unchecked(0), depth, mode);
    let second = if count == 2 {
        Some(read_source(
            cache,
            oracle,
            &oracle.sources.get_unchecked(1),
            depth,
            mode,
        ))
    } else {
        None
    };
    Composition { first, second }
}

/// Evaluates one source and dates it against the asset-level ceiling.
fn read_source(
    cache: &mut ResolutionContext,
    oracle: &AssetOracle,
    source: &PriceSource,
    depth: u32,
    mode: Mode,
) -> Option<Reading> {
    let (observation, component_stale) = evaluate_source(cache, source, depth, mode)?;
    let timestamp = observation.timestamp();
    // The second gate. Each feed was already checked against its own bound; this
    // one holds the *composed* answer to the asset's ceiling, so a source whose
    // slowest component has frozen cannot keep looking fresh on the strength of
    // its live components.
    let stale = component_stale
        || is_stale(
            cache.ledger_timestamp_secs(),
            timestamp,
            oracle.max_price_stale_seconds,
        );
    Some(Reading {
        price_wad: observation.price_wad,
        timestamp,
        stale,
    })
}

/// One source, one price, plus whether any component was past its own bound.
///
/// Staleness is reported rather than acted on so the renderers decide: the hard
/// path reverts, the soft path flags. Making a stale component simply vanish
/// would collapse "past its window" into "unreadable" and cost the diagnostic
/// its most useful distinction.
///
/// The only place a source shape is interpreted.
fn evaluate_source(
    cache: &mut ResolutionContext,
    source: &PriceSource,
    depth: u32,
    mode: Mode,
) -> Option<(OracleObservation, bool)> {
    match source {
        PriceSource::Feed(feed) => read_feed(cache, feed, mode),
        PriceSource::Scaled(scaled) => read_scaled(cache, scaled, depth, mode),
        PriceSource::LpShare(_) => {
            // `validate_source_shape` refuses this variant outright, so a stored
            // config carrying one predates that rule. Structural rather than
            // per-asset, so it reverts under either mode.
            panic_with_error!(cache.env(), OracleError::UnsupportedPoolKind)
        }
    }
}

/// Reads one provider feed and measures it against its own staleness bound.
fn read_feed(
    cache: &mut ResolutionContext,
    feed: &FeedSource,
    mode: Mode,
) -> Option<(OracleObservation, bool)> {
    let env = cache.env().clone();
    let observation = match &feed.provider {
        ProviderRef::Reflector(reflector_ref) => {
            let config = ReflectorSourceConfig {
                contract: reflector_ref.contract.clone(),
                asset: reflector_ref.asset.clone(),
                read_mode: reflector_ref.read_mode,
                decimals: feed.decimals,
                // Read-path-irrelevant; bounded at config time only.
                resolution_seconds: 0,
                // Quoting is `ScaledSource`'s job now, so a feed is always read
                // in whatever unit it natively publishes.
                base: ReflectorBase::Usd,
            };
            reflector::read_reflector_source(cache, &config, mode.soft())
        }
        ProviderRef::MultiFeed(multi_feed_ref) => {
            let config = RedStoneSourceConfig {
                contract: multi_feed_ref.contract.clone(),
                feed_id: multi_feed_ref.feed_id.clone(),
                decimals: feed.decimals,
                max_stale_seconds: feed.max_stale_seconds,
            };
            multi_feed::read_multi_feed_source(cache, &config, mode.soft())
        }
    };

    let observation = match observation {
        Some(observation) => observation,
        None if mode.soft() => return None,
        None => panic_with_error!(&env, OracleError::NoLastPrice),
    };

    let stale = is_stale(
        cache.ledger_timestamp_secs(),
        observation.timestamp(),
        feed.max_stale_seconds,
    );
    Some((observation, stale))
}

/// `factor x price(quote)`, in WAD.
fn read_scaled(
    cache: &mut ResolutionContext,
    scaled: &ScaledSource,
    depth: u32,
    mode: Mode,
) -> Option<(OracleObservation, bool)> {
    let env = cache.env().clone();
    let (factor, factor_stale) = read_feed(cache, &scaled.factor, mode)?;

    // Bounded before it reaches the product. The final sanity band has to be
    // sized for the quote's volatility, which leaves a wrong ratio room to hide
    // inside it; a wrapper ratio is a slow, tightly-known quantity, so checking
    // it directly is far stronger than anything the output band can express.
    if mode.soft() {
        if factor.price_wad < scaled.min_factor_wad || factor.price_wad > scaled.max_factor_wad {
            return None;
        }
    } else {
        require_factor_in_bounds(&env, factor.price_wad, scaled);
    }

    let (quote_price_wad, quote_timestamp) = match mode {
        Mode::Hard => {
            let quote = resolve(cache, &scaled.quote, depth + 1);
            (quote.price_wad, quote.timestamp)
        }
        Mode::Soft => {
            // The quote must be fully usable to back a reprice: a soft read that
            // accepted a stale or out-of-band quote would report a status the
            // hard path would reject, breaking the parity `valid` promises.
            let status = resolve_status(cache, &scaled.quote, depth + 1);
            if !status.valid {
                return None;
            }
            (status.final_wad, status.price_timestamp)
        }
    };

    let price_wad = Wad::from(factor.price_wad)
        .mul(&env, Wad::from(quote_price_wad))
        .raw();

    Some((
        OracleObservation {
            price_wad,
            // Only as fresh as the weaker leg. Each leg has already faced its
            // own bound; the caller holds this composite to the asset ceiling.
            observed_at: factor.timestamp().min(quote_timestamp),
            published_at: None,
        },
        factor_stale,
    ))
}

// ---------------------------------------------------------------------------
// Hard-path gates
// ---------------------------------------------------------------------------

fn require_readable<'a>(env: &Env, reading: Option<&'a Reading>) -> &'a Reading {
    // Unreachable in practice: a hard read reverts inside the provider with its
    // own error rather than returning absent. This is the backstop for a reader
    // that returns instead.
    reading.unwrap_or_else(|| panic_with_error!(env, OracleError::NoLastPrice))
}

fn require_fresh(env: &Env, reading: &Reading) {
    if reading.stale {
        panic_with_error!(env, OracleError::PriceFeedStale);
    }
}

fn require_within_sanity_band(env: &Env, price_wad: i128, oracle: &AssetOracle) {
    if price_wad <= 0 {
        panic_with_error!(env, OracleError::InvalidPrice);
    }
    if price_wad < oracle.min_sanity_price_wad || price_wad > oracle.max_sanity_price_wad {
        panic_with_error!(env, OracleError::SanityBoundViolated);
    }
}

fn require_depth_within_cap(env: &Env, depth: u32) {
    if depth > MAX_RESOLUTION_DEPTH {
        panic_with_error!(env, OracleError::OracleDepthExceeded);
    }
}

#[cfg(test)]
#[path = "../tests/oracle/engine.rs"]
mod tests;
