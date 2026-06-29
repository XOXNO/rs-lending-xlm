//! Setup and one-time market bootstrap helpers.

use crate::events::CreateMarketEvent;
use common::errors::GenericError;
use common::types::MarketParamsRaw;
use soroban_sdk::{assert_with_error, Address, Env};

use crate::external::pool::pool_create_market_call;
use crate::{risk::validation, storage};

/// Registers the asset's market on the central pool under `hub_id`. The market
/// record lives on the pool (`pool_create_market_call`, which reverts
/// `AssetAlreadySupported` on a duplicate (hub, asset)); the controller keeps no
/// listing shadow. The asset stays inactive (unpriceable) until
/// `set_market_oracle_config` writes its token-rooted `AssetOracle` entry, and
/// becomes usable on a spoke once `add_asset_to_spoke` lists it there. Consumes
/// the token approval.
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

    let pool_address = storage::get_pool(env);
    // dimensional: params carries Ray rates/utilization, Bps reserve factor, and Token(asset) caps.
    pool_create_market_call(env, &pool_address, hub_id, params);

    storage::renew_controller_instance(env);

    // dimensional: event fields preserve raw Ray rate/utilization and Bps reserve-factor inputs.
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
