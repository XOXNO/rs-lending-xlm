//! Oracle configuration rules, expressed as predicates over
//! [`SourceProperties`] rather than matches on provider variants.
//!
//! # Why properties
//!
//! v1 decides its rules by reading enum variants: `RedStone(_) => is_spot =
//! true`, `(RedStone, RedStone) => reject`. Both are proxies. The first is a
//! claim about a company standing in for a claim about a feed; the second is a
//! 3x3 table that answers "could these fail together" with "are they the same
//! variant". Each new provider or composition shape requires revisiting every
//! rule, and each proxy is wrong at the edges — the spot rule rejects a
//! perfectly safe RWA NAV feed, and the provider rule calls two Reflector
//! deployments independent because their addresses differ.
//!
//! Here each rule asks the question it means. Adding a provider or a source
//! shape means teaching `local_properties` about it; no rule below changes.
//!
//! Every function is pure and storage-free: the caller walks the registry to
//! build the [`SourceProperties`], this module decides what they permit.

use soroban_sdk::{panic_with_error, Env, Vec};

use crate::errors::{GenericError, OracleError};
use crate::oracle::observation::{MAX_PRICE_STALE_SECONDS, MIN_PRICE_STALE_SECONDS};
use crate::types::oracle_v2::{
    same_domain_set, IndependencePolicy, ScaledSource, SourceProperties, TrustDomain,
    MAX_RESOLUTION_DEPTH, MAX_SOURCES, MIN_SOURCES,
};

/// One or two independent opinions, no more and no fewer.
///
/// The upper bound is deliberate. `Vec` would admit three, but a median-of-N
/// rule is a different security argument — it *hides* a bad source instead of
/// failing closed on it — and must not arrive by accident through a config that
/// simply lists more sources.
///
/// # Errors
/// * [`OracleError::SourceCountOutOfRange`]
pub fn validate_source_count(env: &Env, count: u32) {
    if !(MIN_SOURCES..=MAX_SOURCES).contains(&count) {
        panic_with_error!(env, OracleError::SourceCountOutOfRange);
    }
}

/// Composition may not nest past [`MAX_RESOLUTION_DEPTH`].
///
/// Checked per source at config time so a read never discovers the problem
/// mid-liquidation. The read path checks again — config-time depth can go stale
/// when a dependency is re-pointed, and a price path must never rely on a
/// promise made by an earlier transaction.
///
/// # Errors
/// * [`OracleError::OracleDepthExceeded`]
pub fn validate_composition_depth(env: &Env, properties: &SourceProperties) {
    if properties.depth > MAX_RESOLUTION_DEPTH {
        panic_with_error!(env, OracleError::OracleDepthExceeded);
    }
}

/// The asset-level staleness ceiling is in range, and no component outlives it.
///
/// The ceiling is not a default anyone inherits — every feed states its own
/// bound — it is the limit the *composite* timestamp is gated against. A
/// composite is only as fresh as its stalest component, so without this the
/// engine would accept a frozen slow leg riding under a live fast one for the
/// slow leg's entire window. That is precisely the shape of a ratio feed going
/// silent while the asset it measures depegs.
///
/// # Errors
/// * [`OracleError::InvalidStalenessConfig`] - ceiling out of protocol bounds,
///   or a component permitted to outlive it.
pub fn validate_staleness_envelope(
    env: &Env,
    asset_max_stale_seconds: u64,
    properties: &SourceProperties,
) {
    if !(MIN_PRICE_STALE_SECONDS..=MAX_PRICE_STALE_SECONDS).contains(&asset_max_stale_seconds) {
        panic_with_error!(env, OracleError::InvalidStalenessConfig);
    }
    if properties.loosest_max_stale_seconds > asset_max_stale_seconds {
        panic_with_error!(env, OracleError::InvalidStalenessConfig);
    }
}

/// At least one source must carry no unsmoothed market leg.
///
/// The threat is market manipulation: moving a traded price for one block is
/// cheap. A time-averaged read raises that cost; a published fundamental is not
/// exposed to it at all. So the question is not "is something smoothed" but "is
/// there an opinion trading cannot cheaply move" — which is what
/// [`SourceProperties::has_unsmoothed_market_leg`] answers, combined with OR so
/// that one bare leg taints its whole source.
///
/// Applies identically at one or two sources, which is the point of dropping the
/// primary/anchor split. At two sources it guarantees a clean opinion to price
/// the dirty one against; at one, that the lone opinion is the clean one.
///
/// An LP share always fails this: its reserve read is market state with no
/// window available at any price. That is deliberate — LP collateral needs a
/// per-share growth limiter before it can ship, and this rule refuses it until
/// that exists rather than leaving the gap to a comment.
///
/// # Errors
/// * [`GenericError::SpotOnlyNotProductionSafe`]
pub fn validate_smoothing(env: &Env, first: &SourceProperties, second: Option<&SourceProperties>) {
    let clean =
        !first.has_unsmoothed_market_leg || second.is_some_and(|s| !s.has_unsmoothed_market_leg);
    if !clean {
        panic_with_error!(env, GenericError::SpotOnlyNotProductionSafe);
    }
}

/// Shared trust must match what the config declares, exactly.
///
/// Two opinions are only worth two if different parties can be wrong
/// independently. v1 approximated that with "reject same provider variant",
/// which is both too strict (two genuinely independent RedStone deployments) and
/// too blunt (it never records *what* is shared, so a reviewer cannot weigh it).
///
/// This does not decide for governance — it forces disclosure. The shared set is
/// computed from the actual dependency graph and must equal the declaration, so
/// a config that is effectively single-source cannot look independent by
/// accident, and the declaration rides into the config event where an indexer
/// can alarm on it.
///
/// Set equality, not subset: a shared domain introduced by a later edit must be
/// re-declared rather than silently absorbed into an old waiver.
///
/// # The kind floor
///
/// Trust domains key on `(kind, contract)`, so one operator running two
/// deployments reads as two domains. Every *shared kind* must therefore be
/// attributable to a declared shared domain; a kind shared across two addresses
/// with no shared domain means one operator is standing behind both opinions
/// while the domain rule calls them independent, and there is no waiver for it.
///
/// # Errors
/// * [`OracleError::IndependenceNotDeclared`]
pub fn validate_independence(
    env: &Env,
    first: &SourceProperties,
    second: &SourceProperties,
    policy: &IndependencePolicy,
) {
    let shared = first.shared_with(env, second);

    match policy {
        IndependencePolicy::RequireDisjoint => {
            if !shared.is_empty() {
                panic_with_error!(env, OracleError::IndependenceNotDeclared);
            }
        }
        IndependencePolicy::AllowShared(declared) => {
            if !same_domain_set(&shared, declared) {
                panic_with_error!(env, OracleError::IndependenceNotDeclared);
            }
        }
    }

    require_shared_kinds_are_explained(env, first, second, &shared);
}

/// Every provider family common to both sources must be explained by a domain
/// they actually share. An unexplained shared kind is one operator behind two
/// addresses — independence the domain rule cannot see through.
fn require_shared_kinds_are_explained(
    env: &Env,
    first: &SourceProperties,
    second: &SourceProperties,
    shared_domains: &Vec<TrustDomain>,
) {
    let second_kinds = second.kinds(env);
    for kind in first.kinds(env).iter() {
        let shared_kind = second_kinds.iter().any(|k| k == kind);
        if !shared_kind {
            continue;
        }
        let explained = shared_domains.iter().any(|d| d.kind == kind);
        if !explained {
            panic_with_error!(env, OracleError::IndependenceNotDeclared);
        }
    }
}

/// A scaled source's factor bounds are a well-formed, positive range.
///
/// # Errors
/// * [`OracleError::InvalidSanityBounds`]
pub fn validate_factor_bounds(env: &Env, scaled: &ScaledSource) {
    if scaled.min_factor_wad <= 0 || scaled.max_factor_wad < scaled.min_factor_wad {
        panic_with_error!(env, OracleError::InvalidSanityBounds);
    }
}

/// Read-time gate on the factor itself.
///
/// The final sanity band only bounds the *product*, and it has to be sized for
/// the quote's volatility — a band wide enough for BTC leaves enormous room for
/// a wrong ratio to hide in. A wrapper ratio is a slow, tightly-known quantity,
/// so bounding it directly is far stronger than anything the output band can
/// express, and it is the difference between a compromised ratio feed being
/// caught and it repricing the asset arbitrarily.
///
/// # Errors
/// * [`OracleError::FactorOutOfBounds`]
pub fn require_factor_in_bounds(env: &Env, factor_wad: i128, scaled: &ScaledSource) {
    if factor_wad < scaled.min_factor_wad || factor_wad > scaled.max_factor_wad {
        panic_with_error!(env, OracleError::FactorOutOfBounds);
    }
}

#[cfg(test)]
#[path = "../../tests/oracle/policy.rs"]
mod tests;
