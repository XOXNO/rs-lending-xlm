//! Market lifecycle: creates markets, updates the rate model, and forces interest accrual.

use common::constants::RAY;
use common::errors::GenericError;
use common::ttl::renew_instance;
use common::types::{HubAssetKey, InterestRateModel, MarketParamsRaw, PoolStateRaw};

use crate::cache::Cache;
use crate::ops;
use crate::{events, interest, storage, time};
use soroban_sdk::{assert_with_error, Env, Vec};

/// Creates a new market for `(hub_id, params.asset_id)`.
///
/// Validates params, rejects duplicates, writes zeroed state with indexes at
/// RAY, and emits a market params event.
pub(crate) fn create(env: &Env, hub_id: u32, params: MarketParamsRaw) {
    renew_instance(env);
    params.verify(env);

    let hub_asset = HubAssetKey {
        hub_id,
        asset: params.asset_id.clone(),
    };
    assert_with_error!(
        env,
        !storage::market_exists(env, &hub_asset),
        GenericError::AssetAlreadySupported
    );

    storage::write_params(env, &hub_asset, &params);
    storage::write_state(
        env,
        &hub_asset,
        &PoolStateRaw {
            supplied: 0,
            borrowed: 0,
            revenue: 0,
            borrow_index: RAY,
            supply_index: RAY,
            last_timestamp: time::now_ms(env),
            cash: 0,
        },
    );
    storage::renew_market(env, &hub_asset);

    events::emit_market_params(env, hub_id, hub_asset.asset, params);
}

/// Accrues interest under the old model, commits it, then validates and
/// replaces the interest and flash-loan parameters.
///
/// Interest accrued up to the call uses the old rate model; interest accrued
/// after the call uses the new one.
pub(crate) fn replace_rate_model(env: &Env, hub_asset: HubAssetKey, model: InterestRateModel) {
    ops::renewed_market(env, &hub_asset).commit();

    model.verify(env);
    let params = storage::write_rate_model(env, &hub_asset, &model);
    events::emit_market_params(env, hub_asset.hub_id, hub_asset.asset, params);
}

/// Accrues interest for each market in `hub_assets` and emits one state event
/// per market.
///
/// Writes storage only for markets where time has elapsed; otherwise emits a
/// snapshot of the currently loaded state.
pub(crate) fn accrue(env: &Env, hub_assets: Vec<HubAssetKey>) {
    renew_instance(env);

    for hub_asset in hub_assets.iter() {
        let mut cache = Cache::load(env, &hub_asset);
        let had_elapsed_time = cache.needs_accrual();
        interest::global_sync(env, &mut cache);

        let snapshot = if had_elapsed_time {
            cache.commit()
        } else {
            cache.snapshot()
        };
        events::emit_market_state(env, snapshot);
    }
}
