//! Price registry: keyed storage for [`AssetOracle`], with a read path that
//! still understands configs written under the previous shape.
//!
//! # Why the old shape is still readable
//!
//! `AssetOracleConfig` is a `#[contracttype]` struct, so each stored value is a
//! map keyed by field-name symbols. Replacing `strategy` / `primary` / `anchor`
//! with `sources` changes that key set, and
//! `env.storage().persistent().get::<_, T>()` **traps** on a shape mismatch — its
//! `Option` reports key absence, not a failed decode.
//!
//! That failure mode is not per-market. The controller prices an account's whole
//! portfolio in one `prices` call, so a single undecodable entry reverts the
//! entire health computation. Upgrading the contract ahead of a completed
//! migration would therefore revert every borrow, withdraw, and **liquidation**
//! while prices kept moving — a total halt with an open bad-debt window, caused
//! by deploy ordering alone.
//!
//! So the new config is stored under a **new key variant**. Old entries are
//! never overwritten, never re-decoded under the new type, and stay readable
//! through [`lift_legacy`] until every market has been migrated by an explicit
//! governance write. [`unmigrated`] is what proves that is done.

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};
use common::errors::OracleError;
use common::types::{
    AssetOracle, AssetOracleConfig, FeedNature, FeedSource, IndependencePolicy, MultiFeedRef,
    OracleSourceConfig, OracleStrategy, PriceKey, PriceSource, ProviderKind, ProviderRef,
    ReflectorBase, ReflectorFeedRef, ScaledSource,
};
use soroban_sdk::{contracttype, panic_with_error, Address, Env, Vec};

#[contracttype]
enum AggregatorKey {
    /// Legacy: token-rooted `AssetOracleConfig`. Read-only after migration
    /// begins; never written by the new path.
    AssetOracle(Address),
    /// Current: [`AssetOracle`] under a [`PriceKey`], so reference prices with
    /// no token can be stored alongside real assets.
    Oracle(PriceKey),
}

// ---------------------------------------------------------------------------
// Current shape
// ---------------------------------------------------------------------------

/// Stored [`AssetOracle`] for `key`, renewing its shared-tier TTL on hit.
pub(crate) fn get_oracle(env: &Env, key: &PriceKey) -> Option<AssetOracle> {
    let storage_key = AggregatorKey::Oracle(key.clone());
    let oracle: Option<AssetOracle> = env.storage().persistent().get(&storage_key);
    if oracle.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&storage_key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    }
    oracle
}

pub(crate) fn set_oracle(env: &Env, key: &PriceKey, oracle: &AssetOracle) {
    let storage_key = AggregatorKey::Oracle(key.clone());
    env.storage().persistent().set(&storage_key, oracle);
    env.storage()
        .persistent()
        .extend_ttl(&storage_key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

/// The oracle to price `key` with: the migrated config if one exists, otherwise
/// the legacy config lifted in memory.
///
/// Reading the new key first means a migrated market never pays for the legacy
/// probe, and a migration is atomic from the reader's point of view — the new
/// entry appearing is the cutover for that key, with no window where both or
/// neither answer.
pub(crate) fn resolve_oracle(env: &Env, key: &PriceKey) -> Option<AssetOracle> {
    if let Some(oracle) = get_oracle(env, key) {
        return Some(oracle);
    }
    let PriceKey::Token(asset) = key else {
        // A reference price has no legacy form: it could not be expressed under
        // the old model, so absence here is simply "not configured".
        return None;
    };
    get_legacy(env, asset).map(|legacy| lift_legacy(env, &legacy))
}

/// True once `key` has been written in the current shape.
pub(crate) fn is_migrated(env: &Env, key: &PriceKey) -> bool {
    env.storage()
        .persistent()
        .has(&AggregatorKey::Oracle(key.clone()))
}

/// Keys from `candidates` that still resolve through the legacy reader.
///
/// The guard on retiring [`lift_legacy`]: the fallback may only be removed once
/// this returns empty for every listed asset. Takes an explicit candidate list
/// because persistent storage is not enumerable.
pub(crate) fn unmigrated(env: &Env, candidates: &Vec<PriceKey>) -> Vec<PriceKey> {
    let mut pending = Vec::new(env);
    for key in candidates.iter() {
        if !is_migrated(env, &key) {
            pending.push_back(key);
        }
    }
    pending
}

// ---------------------------------------------------------------------------
// Legacy shape
// ---------------------------------------------------------------------------

pub(crate) fn get_legacy(env: &Env, asset: &Address) -> Option<AssetOracleConfig> {
    let key = AggregatorKey::AssetOracle(asset.clone());
    let config: Option<AssetOracleConfig> = env.storage().persistent().get(&key);
    if config.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    }
    config
}

/// Projects a legacy config onto the current model, in memory only.
///
/// The mapping is total and mechanical:
///
/// * `Single` becomes one source, `PrimaryWithAnchor` becomes two, in
///   primary-then-anchor order. Order carries no meaning in the new model, so
///   nothing is lost by fixing it.
/// * A Reflector source with a `Quoted` base becomes [`PriceSource::Scaled`] —
///   the old model's one composition shape is a special case of the new one.
/// * A source with no staleness bound of its own inherits the asset-level
///   figure, which is exactly what `OracleSourceConfig::max_stale_seconds` did
///   at read time. Lifting preserves the behaviour rather than the field layout.
///
/// # What lifting cannot know
///
/// [`FeedNature`] has no legacy counterpart. Lifted multi-feed sources are
/// marked [`FeedNature::Market`], the stricter of the two, because a lifted
/// config must never look *safer* than it was verified to be.
///
/// That choice is only sound because **lifted configs are priced, never
/// re-validated**. They were checked against v1's rules when they were written;
/// re-running the current rules over a guess at their nature would reject
/// working markets on the strength of a default. Migrating a market means an
/// explicit governance write that states the nature, and only that write is
/// validated.
///
/// Scaled factor bounds get the same treatment: a legacy quoted source was
/// never bounded, so lifting leaves the range fully open rather than inventing
/// one. A migrated config must set real bounds; a lifted one is no worse than it
/// was.
pub(crate) fn lift_legacy(env: &Env, legacy: &AssetOracleConfig) -> AssetOracle {
    let mut sources = Vec::new(env);
    sources.push_back(lift_source(
        env,
        &legacy.primary,
        legacy.max_price_stale_seconds,
    ));
    if legacy.strategy == OracleStrategy::PrimaryWithAnchor {
        // v1 fails closed on a dual strategy with no anchor leg rather than
        // pricing off the primary alone. Lifting it to a single source would
        // quietly turn a halted market into a live one with no agreement band -
        // and with a sanity band that was never held to the single-source cap,
        // because it was written as dual.
        let Some(anchor) = legacy.anchor.as_ref() else {
            panic_with_error!(env, OracleError::NoLastPrice)
        };
        sources.push_back(lift_source(env, anchor, legacy.max_price_stale_seconds));
    }

    AssetOracle {
        asset_decimals: legacy.asset_decimals,
        max_price_stale_seconds: lifted_stale_ceiling(env, legacy, &sources),
        sources,
        tolerance: legacy.tolerance.clone(),
        // A lifted config makes no independence claim: it was written under
        // rules that never computed one. `RequireDisjoint` is the inert choice
        // here precisely because lifted configs skip validation.
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: legacy.min_sanity_price_wad,
        max_sanity_price_wad: legacy.max_sanity_price_wad,
    }
}

/// The asset ceiling a lifted config must carry so the composite gate is inert.
///
/// v1 has no asset-level gate: a multi-feed leg is checked against its own
/// `max_stale_seconds` and nothing else, and nothing ever compared that to
/// `max_price_stale_seconds`. Real configs exist where a leg's window is the
/// looser of the two.
///
/// Applying the composite gate at the legacy asset figure would therefore reject
/// readings v1 accepts - fail-closed, but for a lending protocol a price path
/// that reverts blocks liquidations while the position keeps deteriorating.
/// Raising the ceiling to cover every leg makes the second gate a no-op for
/// lifted configs, which is exactly the v1 behaviour being preserved. A migrated
/// config states a real ceiling and `validate_staleness_envelope` enforces it.
fn lifted_stale_ceiling(env: &Env, legacy: &AssetOracleConfig, sources: &Vec<PriceSource>) -> u64 {
    let mut ceiling = legacy.max_price_stale_seconds;
    for source in sources.iter() {
        let bound = match &source {
            PriceSource::Feed(feed) => feed.max_stale_seconds,
            PriceSource::Scaled(scaled) => scaled.factor.max_stale_seconds,
            // Unreachable: no legacy shape lifts to an LP share.
            PriceSource::LpShare(_) => 0,
        };
        if bound > ceiling {
            ceiling = bound;
        }
    }
    let _ = env;
    ceiling
}

fn lift_source(env: &Env, source: &OracleSourceConfig, asset_max_stale: u64) -> PriceSource {
    match source {
        OracleSourceConfig::Reflector(reflector) => {
            let feed = FeedSource {
                provider: ProviderRef::Reflector(ReflectorFeedRef {
                    contract: reflector.contract.clone(),
                    asset: reflector.asset.clone(),
                    read_mode: reflector.read_mode,
                }),
                decimals: reflector.decimals,
                // Reflector sources carried no bound of their own and fell back
                // to the asset default; make that explicit rather than implicit.
                max_stale_seconds: asset_max_stale,
            };
            match &reflector.base {
                ReflectorBase::Usd => PriceSource::Feed(feed),
                ReflectorBase::Quoted(quote) => PriceSource::Scaled(ScaledSource {
                    factor: feed,
                    quote: PriceKey::Token(quote.clone()),
                    min_factor_wad: 1,
                    max_factor_wad: i128::MAX,
                }),
            }
        }
        OracleSourceConfig::RedStone(config) => {
            PriceSource::Feed(lift_multi_feed(env, config, ProviderKind::RedStone))
        }
        OracleSourceConfig::Xoxno(config) => {
            PriceSource::Feed(lift_multi_feed(env, config, ProviderKind::Xoxno))
        }
    }
}

fn lift_multi_feed(
    _env: &Env,
    config: &common::types::RedStoneSourceConfig,
    kind: ProviderKind,
) -> FeedSource {
    FeedSource {
        provider: ProviderRef::MultiFeed(MultiFeedRef {
            contract: config.contract.clone(),
            feed_id: config.feed_id.clone(),
            kind,
            nature: FeedNature::Market,
        }),
        decimals: config.decimals,
        max_stale_seconds: config.max_stale_seconds,
    }
}

#[cfg(test)]
#[path = "../tests/oracle/registry.rs"]
mod tests;
