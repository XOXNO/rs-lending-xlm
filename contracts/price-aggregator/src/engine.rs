//! Price resolution over the composable model.
//!
//! One recursive entry point, [`resolve`], and one dispatch,
//! [`evaluate_source`]. Reading those two answers "how is this asset priced"
//! without cross-referencing a validator.
//!
//! # Guards, and where each one lives
//!
//! * **Per-feed staleness** — every [`FeedSource`] carries its own bound and is
//!   gated against it the moment it is read.
//! * **Composite staleness** — a source made of several feeds is only as fresh
//!   as its stalest component, and that `min` is gated again at the asset-level
//!   ceiling. Both gates are needed: per-feed alone lets a frozen slow leg ride
//!   under a live fast one for the slow leg's entire window, which is exactly
//!   what a ratio feed going silent during a depeg looks like.
//! * **Factor bounds** — a [`ScaledSource`]'s ratio is checked against its own
//!   range before it reaches the product.
//! * **Agreement** — two sources must land inside the tolerance band.
//! * **Sanity band** — the final price must sit inside the configured bounds.
//! * **Cycle and depth** — enforced on entry, before any config is read, by the
//!   single guarded entry [`resolve_detailed`]. Every caller routes through it.
//!
//! Nothing here falls back. Every failure reverts, because a lending protocol
//! that guesses a price is a lending protocol that has already lost the money.
//!
//! # Quote chains are allowed, deliberately
//!
//! The older model enforced "one hop, no quote chains": a Reflector quoted base
//! had to name an asset whose own oracle was USD-rooted, so composition depth
//! was structurally pinned at exactly two. Here a [`ScaledSource`]'s quote may
//! itself be composed, up to `MAX_RESOLUTION_DEPTH`.
//!
//! That is the point of the model, not an oversight — SolvBTC needs one hop and
//! an LP share needs two — but it means the one-hop rule's protection has to
//! come from somewhere else. It does, and from something stronger:
//!
//! * The one-hop rule bounded depth by forbidding the second hop, because
//!   nothing else bounded it. `MAX_RESOLUTION_DEPTH` and the cycle stack bound
//!   it directly.
//! * Every hop resolves through [`resolve`], so **each intermediate price faces
//!   its own sanity band, staleness gates, and agreement check** before it is
//!   multiplied into anything. The old rule validated only the endpoints; a
//!   chain here is checked at every link.
//!
//! The residual cost is compounding: each hop multiplies its own error into the
//! result. That argues for keeping chains shallow in practice, which is what the
//! depth cap enforces — it is set to 3, not to the largest number that works.

use common::errors::OracleError;
use common::math::fp::Wad;
use common::oracle::observation::is_stale;
use common::oracle::policy::require_factor_in_bounds;
use common::types::{
    AssetOracle, FeedSource, PriceFeedRaw, PriceKey, PriceSource, ProviderRef,
    RedStoneSourceConfig, ReflectorBase, ReflectorSourceConfig, ScaledSource, MAX_RESOLUTION_DEPTH,
};
use soroban_sdk::{panic_with_error, Env};

use crate::context::ResolutionContext;
use crate::observation::OracleObservation;
use crate::providers::{multi_feed, reflector};
use crate::registry;
use crate::tolerance::midpoint_if_in_band;

/// A resolved price plus the spread the sources actually disagreed by.
///
/// `low`/`high` are carried even though only `price_wad` is published today.
/// The combination rule is the one open question in this design — a source
/// compromised high moves a midpoint by half that error, where collateral wants
/// the low end and debt the high end — and keeping the band computed here means
/// widening `PriceFeedRaw` later is a change to the boundary, not to the engine.
pub(crate) struct Resolved {
    pub price_wad: i128,
    pub timestamp: u64,
    pub low_wad: i128,
    pub high_wad: i128,
}

/// USD price for `key`, fail-closed.
///
/// # Errors
/// * [`OracleError::OracleDepthExceeded`] / [`OracleError::OracleCycleDetected`]
/// * [`OracleError::OracleNotConfigured`] - no config, migrated or legacy.
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

/// A resolved key together with the interval its sources spanned.
pub(crate) struct ResolvedKey {
    pub feed: PriceFeedRaw,
    pub low_wad: i128,
    pub high_wad: i128,
}

/// The single guarded entry into resolution.
///
/// Every caller goes through here, so the depth cap and the cycle push apply
/// uniformly. A view that reached `resolve_with` directly would price a
/// self-quoting root one level deeper than it should and would skip the depth
/// cap for the root entirely - bounded, but it would make the module's own
/// claim that cycle and depth are "enforced on entry" false.
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

    let resolved = resolve_with(cache, &oracle, depth);
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

/// Combines a config's sources and applies the price-level guards.
fn resolve_with(cache: &mut ResolutionContext, oracle: &AssetOracle, depth: u32) -> Resolved {
    let env = cache.env().clone();
    let count = oracle.sources.len();
    if count == 0 || count > 2 {
        panic_with_error!(&env, OracleError::SourceCountOutOfRange);
    }

    let first = observe_source(cache, oracle, &oracle.sources.get_unchecked(0), depth);
    let resolved = if count == 1 {
        Resolved {
            price_wad: first.price_wad,
            timestamp: first.timestamp(),
            low_wad: first.price_wad,
            high_wad: first.price_wad,
        }
    } else {
        let second = observe_source(cache, oracle, &oracle.sources.get_unchecked(1), depth);
        // `midpoint_if_in_band` is symmetric in its two arguments, so source
        // order cannot change either the price or the accept/reject outcome.
        let price_wad =
            midpoint_if_in_band(&env, second.price_wad, first.price_wad, &oracle.tolerance);
        Resolved {
            price_wad,
            // A blend is only as fresh as the weaker source it rests on.
            timestamp: first.timestamp().min(second.timestamp()),
            low_wad: first.price_wad.min(second.price_wad),
            high_wad: first.price_wad.max(second.price_wad),
        }
    };

    require_within_sanity_band(&env, resolved.price_wad, oracle);
    resolved
}

/// Evaluates one source and applies the composite staleness gate.
fn observe_source(
    cache: &mut ResolutionContext,
    oracle: &AssetOracle,
    source: &PriceSource,
    depth: u32,
) -> OracleObservation {
    let observation = evaluate_source(cache, source, depth);
    // The second gate. Each feed was already checked against its own bound; this
    // one holds the *composed* answer to the asset's ceiling, so a source whose
    // slowest component has frozen cannot keep looking fresh on the strength of
    // its live components.
    require_fresh(
        cache,
        observation.timestamp(),
        oracle.max_price_stale_seconds,
    );
    observation
}

/// One source, one price. The only place a source shape is interpreted.
fn evaluate_source(
    cache: &mut ResolutionContext,
    source: &PriceSource,
    depth: u32,
) -> OracleObservation {
    match source {
        PriceSource::Feed(feed) => read_feed(cache, feed),
        PriceSource::Scaled(scaled) => read_scaled(cache, scaled, depth),
        PriceSource::LpShare(_) => {
            // Types and the fair-value primitive exist; the evaluator does not.
            // LP collateral carries donation, first-depositor, fee-on-transfer,
            // and pool-callback re-entrancy surface that needs its own review,
            // and `validate_smoothing` already refuses such a config. This is
            // the read-path backstop for a config that predates that rule.
            panic_with_error!(cache.env(), OracleError::UnsupportedPoolKind)
        }
    }
}

/// Reads one provider feed and gates it at its own staleness bound.
fn read_feed(cache: &mut ResolutionContext, feed: &FeedSource) -> OracleObservation {
    let env = cache.env().clone();
    let observation = match &feed.provider {
        ProviderRef::Reflector(reflector_ref) => {
            let config = ReflectorSourceConfig {
                contract: reflector_ref.contract.clone(),
                asset: reflector_ref.asset.clone(),
                read_mode: reflector_ref.read_mode,
                decimals: feed.decimals,
                // Read-path-irrelevant; bounded at listing time only.
                resolution_seconds: 0,
                // Quoting is `ScaledSource`'s job now, so a feed is always read
                // in whatever unit it natively publishes.
                base: ReflectorBase::Usd,
            };
            reflector::read_reflector_source(cache, &config, false)
        }
        ProviderRef::MultiFeed(multi_feed_ref) => {
            let config = RedStoneSourceConfig {
                contract: multi_feed_ref.contract.clone(),
                feed_id: multi_feed_ref.feed_id.clone(),
                decimals: feed.decimals,
                max_stale_seconds: feed.max_stale_seconds,
            };
            multi_feed::read_multi_feed_source(cache, &config, false)
        }
    };

    let Some(observation) = observation else {
        panic_with_error!(&env, OracleError::NoLastPrice)
    };
    require_fresh(cache, observation.timestamp(), feed.max_stale_seconds);
    observation
}

/// `factor x price(quote)`, in WAD.
fn read_scaled(
    cache: &mut ResolutionContext,
    scaled: &ScaledSource,
    depth: u32,
) -> OracleObservation {
    let env = cache.env().clone();
    let factor = read_feed(cache, &scaled.factor);

    // Bounded before it reaches the product. The final sanity band has to be
    // sized for the quote's volatility, which leaves a wrong ratio room to hide
    // inside it; a wrapper ratio is a slow, tightly-known quantity, so checking
    // it directly is far stronger than anything the output band can express.
    require_factor_in_bounds(&env, factor.price_wad, scaled);

    let quote = resolve(cache, &scaled.quote, depth + 1);
    let price_wad = Wad::from(factor.price_wad)
        .mul(&env, Wad::from(quote.price_wad))
        .raw();

    OracleObservation {
        price_wad,
        // Only as fresh as the weaker leg. Each leg has already faced its own
        // bound; the caller holds this composite to the asset ceiling.
        observed_at: factor.timestamp().min(quote.timestamp),
        published_at: None,
    }
}

fn require_fresh(cache: &ResolutionContext, timestamp: u64, max_stale_seconds: u64) {
    if is_stale(cache.ledger_timestamp_secs(), timestamp, max_stale_seconds) {
        panic_with_error!(cache.env(), OracleError::PriceFeedStale);
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

/// Differential tests against the v1 path. Kept beside the engine because they
/// are the migration gate: every live market is priced through `lift_legacy`
/// until it is individually migrated.
#[cfg(test)]
#[path = "../tests/oracle/lift_parity.rs"]
mod lift_parity_tests;
