use common::errors::{CollateralError, SpokeError};
use common::types::{HubAssetKey, SpokeAssetArgs, SpokeAssetConfig};
use common::validation::{
    require_cap_within_asset_domain, validate_liquidation_fees as common_validate_liquidation_fees,
    validate_risk_bounds as common_validate_risk_bounds,
};
use soroban_sdk::{assert_with_error, panic_with_error, Env};

use crate::external::pool::fetch_pool_sync_data;
use crate::{
    events::{RemoveSpokeAssetEvent, UpdateSpokeAssetEvent},
    storage,
};

enum SpokeAssetMutation {
    Add,
    Edit,
}

/// Registers a new asset market in a spoke after validating risk bounds,
/// liquidation fees, and caps against the pool. Panics if the spoke is
/// deprecated or the asset is already registered in it.
pub(crate) fn add_asset_to_spoke(env: &Env, args: &SpokeAssetArgs) {
    upsert_spoke_asset(env, args, SpokeAssetMutation::Add);
}

/// Updates an existing spoke asset's configuration after validating risk
/// bounds, liquidation fees, and caps against the pool. Panics if the spoke
/// does not exist or the asset is not registered in it.
pub(crate) fn edit_asset_in_spoke(env: &Env, args: &SpokeAssetArgs) {
    upsert_spoke_asset(env, args, SpokeAssetMutation::Edit);
}

/// Validates `args`, checks the spoke/asset registration precondition for
/// `mutation`, validates the caps against the pool's asset decimals, then
/// writes the spoke asset entry and publishes an `UpdateSpokeAssetEvent`.
/// Every validation runs before the storage write.
fn upsert_spoke_asset(env: &Env, args: &SpokeAssetArgs, mutation: SpokeAssetMutation) {
    common_validate_risk_bounds(env, args.ltv, args.threshold, args.bonus);
    common_validate_liquidation_fees(env, args.liquidation_fees);
    assert_with_error!(
        env,
        args.supply_cap >= 0 && args.borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );

    let hub_asset = HubAssetKey {
        hub_id: args.hub_id,
        asset: args.asset.clone(),
    };
    match mutation {
        SpokeAssetMutation::Add => {
            let spoke = storage::get_spoke(env, args.spoke_id);
            assert_with_error!(env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
            assert_with_error!(
                env,
                storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_none(),
                SpokeError::AssetAlreadyInSpoke
            );
        }
        SpokeAssetMutation::Edit => {
            storage::get_spoke(env, args.spoke_id);
            assert_with_error!(
                env,
                storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_some(),
                SpokeError::AssetNotInSpoke
            );
        }
    }

    let market = fetch_pool_sync_data(env, &storage::get_pool(env), &hub_asset);
    require_cap_within_asset_domain(env, args.supply_cap, market.params.asset_decimals);
    require_cap_within_asset_domain(env, args.borrow_cap, market.params.asset_decimals);

    let config = SpokeAssetConfig {
        is_collateralizable: args.can_collateral,
        is_borrowable: args.can_borrow,
        paused: args.paused,
        frozen: args.frozen,
        no_seize: args.no_seize,
        loan_to_value: args.ltv,
        liquidation_threshold: args.threshold,
        liquidation_bonus: args.bonus,
        liquidation_fees: args.liquidation_fees,
        supply_cap: args.supply_cap,
        borrow_cap: args.borrow_cap,
    };
    storage::set_spoke_asset(env, args.spoke_id, &hub_asset, &config);

    UpdateSpokeAssetEvent {
        asset: args.asset.clone(),
        config,
        spoke_id: args.spoke_id,
        hub_id: args.hub_id,
    }
    .publish(env);
}

/// Updates the paused, frozen, and no-seize flags on an existing spoke asset
/// and publishes an `UpdateSpokeAssetEvent`. Panics if the asset is not
/// registered in the spoke, or if any flag would be relaxed from true to
/// false.
pub(crate) fn set_spoke_asset_flags(
    env: &Env,
    spoke_id: u32,
    hub_asset: HubAssetKey,
    paused: bool,
    frozen: bool,
    no_seize: bool,
) {
    let mut config = storage::get_spoke_asset(env, spoke_id, &hub_asset)
        .unwrap_or_else(|| panic_with_error!(env, SpokeError::AssetNotInSpoke));
    require_flag_ratchet(env, &config, paused, frozen, no_seize);
    config.paused = paused;
    config.frozen = frozen;
    config.no_seize = no_seize;
    storage::set_spoke_asset(env, spoke_id, &hub_asset, &config);

    UpdateSpokeAssetEvent {
        asset: hub_asset.asset,
        config,
        spoke_id,
        hub_id: hub_asset.hub_id,
    }
    .publish(env);
}

/// Asserts that `paused`, `frozen`, and `no_seize` only move from false to
/// true relative to `config`, panicking if any flag would be relaxed. The
/// guardian may only tighten; clearing a flag stays timelocked through
/// `edit_asset_in_spoke` (ADR-0007).
fn require_flag_ratchet(
    env: &Env,
    config: &SpokeAssetConfig,
    paused: bool,
    frozen: bool,
    no_seize: bool,
) {
    assert_with_error!(
        env,
        (paused || !config.paused) && (frozen || !config.frozen) && (no_seize || !config.no_seize),
        SpokeError::SpokeAssetFlagRelaxation
    );
}

/// Removes an asset from a spoke's registry and publishes a
/// `RemoveSpokeAssetEvent`. Panics if the asset is not registered in the
/// spoke or still has nonzero supplied or borrowed usage.
pub(crate) fn remove_asset_from_spoke(env: &Env, hub_asset: HubAssetKey, spoke_id: u32) {
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, spoke_id, &hub_asset).is_some(),
        SpokeError::AssetNotInSpoke
    );
    let usage = storage::get_spoke_usage(env, spoke_id, &hub_asset).unwrap_or_default();
    assert_with_error!(
        env,
        usage.supplied_scaled_ray == 0 && usage.borrowed_scaled_ray == 0,
        SpokeError::SpokeAssetInUse
    );

    storage::remove_spoke_asset(env, spoke_id, &hub_asset);

    RemoveSpokeAssetEvent {
        asset: hub_asset.asset,
        spoke_id,
        hub_id: hub_asset.hub_id,
    }
    .publish(env);
}

#[cfg(test)]
#[path = "../../tests/config/asset_flags.rs"]
mod tests;
