use common::constants::RAY;
use common::errors::GenericError;
use common::types::{HubAssetKey, InterestRateModel, MarketParamsRaw, PoolStateRaw};

use soroban_sdk::{assert_with_error, Env};

use crate::cache::Cache;
use crate::{events, interest, storage, time};

pub(crate) fn create(env: &Env, hub_id: u32, params: MarketParamsRaw) {
    storage::renew_instance(env);
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

pub(crate) fn replace_rate_model(env: &Env, hub_asset: HubAssetKey, model: InterestRateModel) {
    crate::ops::renewed_market(env, &hub_asset).commit();

    model.verify(env);
    let params = storage::write_rate_model(env, &hub_asset, &model);
    events::emit_market_params(env, hub_asset.hub_id, hub_asset.asset, params);
}

pub(crate) fn accrue(env: &Env, hub_asset: HubAssetKey) {
    storage::renew_instance(env);

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
