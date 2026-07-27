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

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::errors::{GenericError, OracleError};
use crate::oracle::observation::{
    MAX_ORACLE_DECIMALS, MAX_PRICE_STALE_SECONDS, MIN_ORACLE_DECIMALS, MIN_PRICE_STALE_SECONDS,
};
use crate::types::composable_oracle::{
    FeedSource, IndependencePolicy, PriceKey, PriceSource, ProviderRef, ScaledSource,
    SourceProperties, MAX_RESOLUTION_DEPTH, MAX_SOURCES, MIN_SOURCES,
};
use crate::types::oracle::OracleReadMode;
use crate::validation::validate_twap_records;

/// Decimals of a priced token, matching the pool-params bound. A reference key
/// carries zero instead.
const MIN_ASSET_DECIMALS: u32 = 3;
const MAX_ASSET_DECIMALS: u32 = 18;

/// Fewest TWAP samples that count as smoothing.
///
/// A one-sample "average" is a spot read wearing a different label, and it would
/// satisfy `validate_smoothing` whose entire justification is that moving a
/// time-average costs more than moving one print.
const MIN_SMOOTHING_TWAP_RECORDS: u32 = 2;

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
/// accident.
///
/// Set equality, not subset: a shared domain introduced by a later edit must be
/// re-declared rather than silently absorbed into an old waiver.
///
/// # Sharing is judged by contract address, not by declared kind
///
/// [`TrustDomain`] carries `(kind, contract)`, but `kind` for a multi-feed
/// adapter is **declared by the proposer and unverifiable on-chain**. Judging
/// sharing on the pair would let two feeds on one adapter be labelled
/// `RedStone` and `Xoxno` and pass `RequireDisjoint` — the exact shape v1
/// rejected outright, and it named the dual-ABI XOXNO adapter as the reason.
///
/// So the rule keys on the address, which no proposer controls the meaning of:
/// any contract reachable from both sources must be declared. The declared
/// domains still carry a kind, because that is what a reviewer reads; it just
/// does not decide anything.
///
/// This also removes v1's over-strictness rather than reproducing it. Two
/// deployments of one provider at different addresses share no contract, so
/// they pass — where an earlier version of this rule rejected them with no
/// waiver available, since a *computed* shared set that is empty can never be
/// matched by a non-empty declaration.
///
/// # Errors
/// * [`OracleError::IndependenceNotDeclared`]
pub fn validate_independence(
    env: &Env,
    first: &SourceProperties,
    second: &SourceProperties,
    policy: &IndependencePolicy,
) {
    let shared_contracts = first.shared_contracts_with(env, second);

    match policy {
        IndependencePolicy::RequireDisjoint => {
            if !shared_contracts.is_empty() {
                panic_with_error!(env, OracleError::IndependenceNotDeclared);
            }
        }
        IndependencePolicy::AllowShared(declared) => {
            // An empty waiver is `RequireDisjoint` spelled differently, and a
            // policy variant that can express the same thing two ways defeats
            // any off-chain rule keyed on the variant.
            if declared.is_empty() {
                panic_with_error!(env, OracleError::IndependenceNotDeclared);
            }
            let mut declared_contracts = Vec::new(env);
            for domain in declared.iter() {
                if !declared_contracts.iter().any(|c| c == domain.contract) {
                    declared_contracts.push_back(domain.contract.clone());
                }
            }
            if !same_address_set(&shared_contracts, &declared_contracts) {
                panic_with_error!(env, OracleError::IndependenceNotDeclared);
            }
        }
    }
}

fn same_address_set(left: &Vec<Address>, right: &Vec<Address>) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter().all(|a| right.iter().any(|b| b == a))
}

/// Bounds every provider-level field a source carries.
///
/// The composable model takes `decimals` and the TWAP window as **config data**,
/// where v1 read decimals from the provider contract and bounded the window at
/// listing time. Dropping those checks without replacing them would make the
/// remaining rules decorative:
///
/// * A feed declaring 7 decimals against a provider publishing 14 is normalized
///   by `Wad::from_token` as if it were 7, pricing the asset 10^7 too high. A
///   wide sanity band - which a dual-source config is exempt from tightening -
///   lets that through.
/// * `decimals` past the WAD scale makes the rescale factor overflow and trap as
///   a raw wasm error rather than a typed one.
/// * `Twap(0)` reads as smoothed, satisfies the smoothing rule, and then reverts
///   on every read: a market that validates and is born bricked.
/// * `Twap(1)` is a one-sample average. It satisfies a rule whose whole
///   justification is that moving a time-average is expensive.
///
/// # Errors
/// * [`OracleError::InvalidOracleDecimals`]
/// * [`OracleError::TwapInsufficientObservations`] / [`OracleError::TwapRecordsOutOfRange`]
/// * [`OracleError::UnsupportedPoolKind`] - LP shares are not priceable yet.
pub fn validate_source_shape(env: &Env, source: &PriceSource) {
    match source {
        PriceSource::Feed(feed) => validate_feed_shape(env, feed),
        PriceSource::Scaled(scaled) => {
            validate_feed_shape(env, &scaled.factor);
            validate_factor_bounds(env, scaled);
        }
        PriceSource::LpShare(_) => {
            // Not reachable through the smoothing rule alone: an LP source
            // paired with any clean source satisfies "at least one opinion is
            // clean", so the config would store and then revert on every read.
            // Refused here, where the refusal is unconditional.
            panic_with_error!(env, OracleError::UnsupportedPoolKind)
        }
    }
}

fn validate_feed_shape(env: &Env, feed: &FeedSource) {
    if !(MIN_ORACLE_DECIMALS..=MAX_ORACLE_DECIMALS).contains(&feed.decimals) {
        panic_with_error!(env, OracleError::InvalidOracleDecimals);
    }
    if let ProviderRef::Reflector(reflector) = &feed.provider {
        if let OracleReadMode::Twap(records) = reflector.read_mode {
            validate_twap_records(env, records);
            if records < MIN_SMOOTHING_TWAP_RECORDS {
                panic_with_error!(env, OracleError::TwapInsufficientObservations);
            }
        }
    }
}

/// Decimals of the priced asset itself, which scale every token amount a
/// consumer derives from the price - including liquidation seize amounts and
/// protocol fees. A reference price has no token and no amounts, so it carries
/// zero and nothing else.
///
/// # Errors
/// * [`OracleError::InvalidOracleDecimals`]
pub fn validate_asset_decimals(env: &Env, key: &PriceKey, asset_decimals: u32) {
    let ok = match key {
        PriceKey::Token(_) => (MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS).contains(&asset_decimals),
        PriceKey::Ref(_) => asset_decimals == 0,
    };
    if !ok {
        panic_with_error!(env, OracleError::InvalidOracleDecimals);
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
