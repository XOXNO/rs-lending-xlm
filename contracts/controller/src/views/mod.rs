//! Read-only views and liquidation estimation.
//! Views use `Cache::new_view`, so disabled-market stale oracles do not block
//! front ends or indexers. Reads can still renew shared-tier TTLs.

use crate::constants::{MAX_VIEW_INPUTS, WAD};
use common::errors::GenericError;
use controller_interface::types::{
    AccountAttributes, AccountPositionRaw, AssetExtendedConfigView, DebtPositionRaw, HubAssetKey,
    LiquidationEstimate, MarketIndexRaw, MarketIndexView, PaymentTuple, SpokeAssetConfig,
    SpokeConfig,
};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Env, Map, Vec};

#[cfg(not(feature = "certora"))]
mod aggregates;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/views/aggregates.rs"]
mod aggregates;
mod limits;
// Certora swaps the pure position-iteration aggregates for summary re-exports
// defined in shared/summaries/mod.rs.

pub use aggregates::{ltv_collateral_in_usd, total_borrow_in_usd, total_collateral_in_usd};

use crate::cache::Cache;
use crate::helpers::utils::hub0;
use crate::oracle::{price_components, token_price};
use crate::positions::{liquidation::execute_liquidation, HubPayment};
use crate::{helpers, storage, validation, Controller, ControllerArgs, ControllerClient};

fn require_view_inputs_bound<T>(env: &Env, values: &Vec<T>) {
    assert_with_error!(
        env,
        values.len() <= MAX_VIEW_INPUTS,
        GenericError::InvalidPayments
    );
}

#[contractimpl]
impl Controller {
    pub fn is_liquidatable(env: Env, account_id: u64) -> bool {
        can_be_liquidated(&env, account_id)
    }

    pub fn get_health_factor(env: Env, account_id: u64) -> i128 {
        health_factor(&env, account_id)
    }

    pub fn get_total_collateral_usd(env: Env, account_id: u64) -> i128 {
        total_collateral_in_usd(&env, account_id)
    }

    pub fn get_total_borrow_usd(env: Env, account_id: u64) -> i128 {
        total_borrow_in_usd(&env, account_id)
    }

    pub fn get_collateral_amount(env: Env, account_id: u64, asset: Address) -> i128 {
        collateral_amount_for_token(&env, account_id, &asset)
    }

    pub fn get_borrow_amount(env: Env, account_id: u64, asset: Address) -> i128 {
        borrow_amount_for_token(&env, account_id, &asset)
    }

    pub fn get_account_positions(
        env: Env,
        account_id: u64,
    ) -> (
        Map<HubAssetKey, AccountPositionRaw>,
        Map<HubAssetKey, DebtPositionRaw>,
    ) {
        get_account_positions(&env, account_id)
    }

    pub fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes {
        get_account_attributes(&env, account_id)
    }

    /// Whether `account_id` still has on-chain account metadata.
    pub fn account_exists(env: Env, account_id: u64) -> bool {
        account_exists(&env, account_id)
    }

    pub fn get_spoke_asset(env: Env, spoke_id: u32, asset: Address) -> SpokeAssetConfig {
        let hub_asset = HubAssetKey { hub_id: 0, asset };
        storage::get_spoke_asset(&env, spoke_id, &hub_asset)
            .unwrap_or_else(|| panic_with_error!(&env, GenericError::AssetNotSupported))
    }

    pub fn get_spoke(env: Env, spoke_id: u32) -> SpokeConfig {
        storage::get_spoke(&env, spoke_id)
    }

    /// Central liquidity pool for all markets; reads instance storage only.
    pub fn get_pool_address(env: Env) -> Address {
        get_pool_address(&env)
    }

    pub fn get_markets_detailed(env: Env, assets: Vec<Address>) -> Vec<AssetExtendedConfigView> {
        get_all_markets_detailed(&env, &assets)
    }

    pub fn get_market_indexes_detailed(env: Env, assets: Vec<Address>) -> Vec<MarketIndexView> {
        get_all_market_indexes_detailed(&env, &assets)
    }

    pub fn get_liquidation_estimate(
        env: Env,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
    ) -> LiquidationEstimate {
        liquidation_estimations_detailed(&env, account_id, &debt_payments)
    }

    pub fn get_liquidation_collateral(env: Env, account_id: u64) -> i128 {
        liquidation_collateral_available(&env, account_id)
    }

    pub fn get_ltv_collateral_usd(env: Env, account_id: u64) -> i128 {
        ltv_collateral_in_usd(&env, account_id)
    }

    /// Largest executable `withdraw` amount.
    pub fn max_withdraw(env: Env, account_id: u64, asset: Address) -> i128 {
        limits::max_withdraw(&env, account_id, &asset)
    }

    /// Supply-cap headroom for `account_id`; `i128::MAX` uncapped, `0` paused or inactive.
    pub fn max_supply(env: Env, account_id: u64, asset: Address) -> i128 {
        limits::max_supply(&env, account_id, &asset)
    }

    /// Largest executable `borrow` amount of `asset`; `0` while
    /// paused, on an inactive/non-borrowable market, or when the asset is
    /// structurally not borrowable for the account.
    pub fn max_borrow(env: Env, account_id: u64, asset: Address) -> i128 {
        limits::max_borrow(&env, account_id, &asset)
    }

    /// Accrued indexes; reads no oracle.
    pub fn get_market_index(env: Env, asset: Address) -> MarketIndexRaw {
        let mut cache = Cache::new_view(&env);
        // dimensional: MarketIndexRaw exposes Ray<Index(asset, supply|borrow)> raw values.
        MarketIndexRaw::from(&cache.cached_market_index(&hub0(&asset)))
    }
}

pub fn health_factor(env: &Env, account_id: u64) -> i128 {
    let mut cache = Cache::new_view(env);
    match storage::try_get_account(env, account_id) {
        // dimensional: return is HealthFactor raw WAD; i128::MAX is no-debt/no-account sentinel.
        Some(account) => helpers::calculate_account_risk_totals(
            env,
            &mut cache,
            &account.supply_positions,
            &account.borrow_positions,
        )
        .health_factor
        .raw(),
        None => i128::MAX,
    }
}

pub fn can_be_liquidated(env: &Env, account_id: u64) -> bool {
    // dimensional: raw WAD HealthFactor is compared to WAD-scaled 1.0.
    health_factor(env, account_id) < WAD
}

pub fn collateral_amount_for_token(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };
    let position = match storage::try_get_supply_position(env, account_id, &hub_asset) {
        Some(position) => position,
        None => return 0,
    };

    let mut cache = Cache::new_view(env);
    let market_index = cache.cached_market_index(&hub_asset);
    let decimals = cache.cached_pool_sync_data(&hub_asset).params.asset_decimals;

    // dimensional: scaled_amount * supply_index -> Token(asset), returned in asset decimals.
    position
        .scaled_amount
        .mul(env, market_index.supply_index)
        .to_asset(decimals)
}

pub fn borrow_amount_for_token(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };
    let position = match storage::try_get_debt_position(env, account_id, &hub_asset) {
        Some(position) => position,
        None => return 0,
    };

    let mut cache = Cache::new_view(env);
    let market_index = cache.cached_market_index(&hub_asset);
    let decimals = cache.cached_pool_sync_data(&hub_asset).params.asset_decimals;

    // dimensional: scaled_amount * borrow_index -> Token(asset), returned in asset decimals.
    position
        .scaled_amount
        .mul(env, market_index.borrow_index)
        .to_asset(decimals)
}

pub fn account_exists(env: &Env, account_id: u64) -> bool {
    storage::try_get_account_meta(env, account_id).is_some()
}

/// Returns raw scaled supply and debt maps for `account_id`.
pub fn get_account_positions(
    env: &Env,
    account_id: u64,
) -> (
    Map<HubAssetKey, AccountPositionRaw>,
    Map<HubAssetKey, DebtPositionRaw>,
) {
    if !account_exists(env, account_id) {
        return (Map::new(env), Map::new(env));
    }

    // dimensional: returned maps keep Ray<Share(asset, side)> raw scaled balances.
    (
        storage::get_supply_positions(env, account_id),
        storage::get_debt_positions(env, account_id),
    )
}

pub fn get_account_attributes(env: &Env, account_id: u64) -> AccountAttributes {
    let meta = storage::get_account_meta(env, account_id);
    AccountAttributes::from(&meta)
}

pub fn liquidation_collateral_available(env: &Env, account_id: u64) -> i128 {
    let account = match storage::try_get_account(env, account_id) {
        Some(account) => account,
        None => return 0,
    };
    let mut cache = Cache::new_view(env);
    // dimensional: return is Wad<USD> raw (1e18) liquidation collateral.
    helpers::calculate_account_risk_totals(
        env,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    )
    .weighted_collateral
    .raw()
}

pub fn get_pool_address(env: &Env) -> Address {
    storage::get_pool(env)
}

pub fn get_all_markets_detailed(env: &Env, assets: &Vec<Address>) -> Vec<AssetExtendedConfigView> {
    require_view_inputs_bound(env, assets);
    let mut cache = Cache::new_view(env);
    let mut result = Vec::new(env);

    for i in 0..assets.len() {
        let asset = validation::expect_invariant(env, assets.get(i));
        // Panics on unsupported assets; pool address is resolved per-row, so the
        // view is safe on empty input.
        validation::require_asset_supported(env, &mut cache, &hub0(&asset));
        let pool_address = cache.cached_pool_address();
        // dimensional: price_wad is Wad<USD/asset> raw.
        let final_price = token_price(&mut cache, &asset).price_wad;
        result.push_back(AssetExtendedConfigView {
            asset,
            pool_address,
            price_wad: final_price,
        });
    }

    result
}

pub fn get_all_market_indexes_detailed(env: &Env, assets: &Vec<Address>) -> Vec<MarketIndexView> {
    require_view_inputs_bound(env, assets);
    let mut cache = Cache::new_view(env);
    let mut hub_assets: Vec<HubAssetKey> = Vec::new(env);
    for asset in assets.iter() {
        hub_assets.push_back(hub0(&asset));
    }
    cache.prefetch_market_indexes(&hub_assets);
    let mut result = Vec::new(env);

    for i in 0..assets.len() {
        let asset = validation::expect_invariant(env, assets.get(i));
        let hub_asset = validation::expect_invariant(env, hub_assets.get(i));
        let index = cache.cached_market_index(&hub_asset);
        let components = price_components(&mut cache, &asset);
        let (safe_price_wad, aggregator_price_wad) = components.to_abi_prices();

        // dimensional: indexes are Ray<Index(asset, side)>; prices are Wad<USD/asset>.
        result.push_back(MarketIndexView {
            asset,
            supply_index_ray: index.supply_index.raw(),
            borrow_index_ray: index.borrow_index.raw(),
            price_wad: components.final_price_wad,
            safe_price_wad,
            aggregator_price_wad,
        });
    }

    result
}

pub fn liquidation_estimations_detailed(
    env: &Env,
    account_id: u64,
    debt_payments: &Vec<HubPayment>,
) -> LiquidationEstimate {
    require_view_inputs_bound(env, debt_payments);
    let mut cache = Cache::new_view(env);
    let account = storage::get_account(env, account_id);
    // dimensional: debt_payments are Token(debt_asset); result carries Token, Wad<USD>, and Bps.
    let result = execute_liquidation(env, &account, debt_payments, &mut cache);

    let mut seized_collaterals = Vec::new(env);
    let mut protocol_fees = Vec::new(env);
    // dimensional: seized collateral and protocol fees are Token(asset) amounts.
    for i in 0..result.seized.len() {
        let entry = validation::expect_invariant(env, result.seized.get(i));
        seized_collaterals.push_back(PaymentTuple {
            asset: entry.hub_asset.asset.clone(),
            amount: entry.amount,
        });
        protocol_fees.push_back(PaymentTuple {
            asset: entry.hub_asset.asset,
            amount: entry.protocol_fee,
        });
    }

    let mut refunds_view = Vec::new(env);
    // dimensional: refunds are Token(asset); max_payment_wad is Wad<USD>; bonus is Bps.
    for i in 0..result.refunds.len() {
        refunds_view.push_back(validation::expect_invariant(env, result.refunds.get(i)));
    }

    LiquidationEstimate {
        seized_collaterals,
        protocol_fees,
        refunds: refunds_view,
        max_payment_wad: result.max_debt_usd,
        bonus_rate_bps: result.bonus_bps,
    }
}

#[cfg(test)]
#[path = "../../tests/views/mod.rs"]
mod tests;
