//! Setup and one-time market bootstrap helpers.

use crate::events::CreateMarketEvent;
use common::errors::GenericError;
use common::types::MarketParamsRaw;
use soroban_sdk::{assert_with_error, Address, Env};

use crate::external::pool::pool_create_market_call;
use crate::{risk::validation, storage};

/// Registers a hub asset market on the central pool.
pub fn create_liquidity_pool(
    env: &Env,
    hub_id: u32,
    asset: &Address,
    params: &MarketParamsRaw,
) -> Address {
    validation::require_hub_active(env, hub_id);

    assert_with_error!(
        env,
        storage::is_token_approved(env, asset),
        GenericError::TokenNotApproved
    );
    assert_with_error!(env, params.asset_id == *asset, GenericError::WrongToken);

    let pool_address = storage::get_pool(env);
    pool_create_market_call(env, &pool_address, hub_id, params);

    storage::renew_controller_instance(env);

    CreateMarketEvent {
        hub_id,
        base_asset: asset.clone(),
        max_borrow_rate: params.max_borrow_rate,
        base_borrow_rate: params.base_borrow_rate,
        slope1: params.slope1,
        slope2: params.slope2,
        slope3: params.slope3,
        mid_utilization: params.mid_utilization,
        optimal_utilization: params.optimal_utilization,
        max_utilization: params.max_utilization,
        reserve_factor: params.reserve_factor,
        market_address: pool_address.clone(),
    }
    .publish(env);

    storage::set_token_approved(env, asset, false);

    pool_address
}
