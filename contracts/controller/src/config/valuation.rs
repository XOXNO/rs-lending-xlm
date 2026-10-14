//! Per-account valuation exclusions: the guardian's lever against a borrower
//! that shields its account with a supply leg whose feed is in an outage
//! (threat model DoS.1).

use common::errors::{CollateralError, OracleError, SpokeError};
use common::types::{HubAssetKey, PriceStatus};
use soroban_sdk::{assert_with_error, vec, Env};

use crate::events::AccountValuationEvent;
use crate::external::price_aggregator::fetch_prices_status;
use crate::storage;

/// Strict-read failures that admit an exclusion: feed outages a borrower can
/// wait for or an outsider can cause, and no immediate governance role can.
/// `SanityBoundViolated` is left out because the oracle role narrows a sanity
/// band immediately. Configuration failures are left out because only delayed
/// governance operations cause them.
const OUTAGE_FAILURES: [OracleError; 7] = [
    OracleError::UnsafePriceNotAllowed,
    OracleError::PriceFeedStale,
    OracleError::NoLastPrice,
    OracleError::ReflectorHistoryEmpty,
    OracleError::InvalidPrice,
    OracleError::TwapInsufficientObservations,
    OracleError::InsufficientAquariusLiquidity,
];

/// Returns the aggregator's non-reverting status for `hub_asset`'s token.
/// `quotes` report `valid` exactly when the strict `prices` read succeeds.
fn feed_status(env: &Env, hub_asset: &HubAssetKey) -> PriceStatus {
    fetch_prices_status(env, &vec![env, hub_asset.asset.clone()])
        .get(hub_asset.asset.clone())
        .unwrap_or_else(PriceStatus::unusable)
}

/// Reports whether `code` names an outage failure on the allow-list.
fn is_outage_failure(code: u32) -> bool {
    OUTAGE_FAILURES
        .iter()
        .any(|failure| *failure as u32 == code)
}

/// Excludes `account_id`'s supply leg of `hub_asset` from that account's
/// valuation, stamped with the current ledger sequence. Admitted only while the
/// account holds the leg and the asset's strict price read fails with an
/// outage on the allow-list. The leg then counts as zero collateral and is
/// never priced or seized; only `restore_account_asset_valuation` clears it.
pub(crate) fn exclude_account_asset_valuation(env: &Env, account_id: u64, hub_asset: HubAssetKey) {
    assert_with_error!(
        env,
        storage::get_supply_positions(env, account_id).contains_key(hub_asset.clone()),
        CollateralError::CollateralPositionNotFound
    );
    let mut exclusions = storage::get_account_valuation_exclusions(env, account_id);
    assert_with_error!(
        env,
        !exclusions.contains_key(hub_asset.clone()),
        SpokeError::SpokeAssetExclusionUnchanged
    );
    let status = feed_status(env, &hub_asset);
    assert_with_error!(env, !status.valid, SpokeError::SpokeAssetFeedUsable);
    assert_with_error!(
        env,
        status.error_code.is_some_and(is_outage_failure),
        SpokeError::SpokeAssetFeedFailureIneligible
    );

    let excluded_at = env.ledger().sequence();
    exclusions.set(hub_asset.clone(), excluded_at);
    storage::set_account_valuation_exclusions(env, account_id, &exclusions);
    publish(env, account_id, hub_asset, true, excluded_at);
}

/// Returns `account_id`'s supply leg of `hub_asset` to its valuation. Requires
/// the live exclusion to carry `excluded_at` and the asset's strict price read
/// to succeed, so a dead feed keeps its exclusion.
pub(crate) fn restore_account_asset_valuation(
    env: &Env,
    account_id: u64,
    hub_asset: HubAssetKey,
    excluded_at: u32,
) {
    let mut exclusions = storage::get_account_valuation_exclusions(env, account_id);
    assert_with_error!(
        env,
        exclusions.get(hub_asset.clone()) == Some(excluded_at),
        SpokeError::SpokeAssetExclusionUnchanged
    );
    assert_with_error!(
        env,
        feed_status(env, &hub_asset).valid,
        SpokeError::SpokeAssetFeedUnusable
    );

    exclusions.remove(hub_asset.clone());
    storage::set_account_valuation_exclusions(env, account_id, &exclusions);
    publish(env, account_id, hub_asset, false, excluded_at);
}

/// Emits the exclusion change for one account leg.
fn publish(env: &Env, account_id: u64, hub_asset: HubAssetKey, excluded: bool, excluded_at: u32) {
    AccountValuationEvent {
        account_id,
        asset: hub_asset.asset,
        hub_id: hub_asset.hub_id,
        excluded,
        excluded_at,
    }
    .publish(env);
}

#[cfg(test)]
#[path = "../../tests/config/valuation.rs"]
mod tests;
