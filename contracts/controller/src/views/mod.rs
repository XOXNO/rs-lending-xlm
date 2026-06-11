//! Read-only views and liquidation estimation.
//!
//! All views deliberately use `Cache::new_view` (no instance TTL
//! bump, permissive `OraclePolicy::View`) so that front-ends and indexers
//! can call them without rent side-effects and without being blocked by
//! stale oracles on disabled markets.
//!
//! Aggregates are harnessed under certora.

use common::constants::WAD;
use common::types::{
    AccountAttributes, AccountPositionRaw, AssetExtendedConfigView, DebtPositionRaw,
    EModeCategoryRaw, LiquidationEstimate, MarketConfig, MarketIndexRaw, MarketIndexView, Payment,
    PaymentTuple,
};
use soroban_sdk::{contractimpl, Address, Env, Map, Vec};

#[cfg(not(feature = "certora"))]
mod aggregates;
#[cfg(feature = "certora")]
#[path = "../../../../verification/certora/controller/harness/views/aggregates.rs"]
mod aggregates;
mod limits;
// Certora swaps the pure position-iteration aggregates for summary re-exports
// defined in shared/summaries/mod.rs.

pub use aggregates::{ltv_collateral_in_usd, total_borrow_in_usd, total_collateral_in_usd};

use crate::cache::Cache;
use crate::oracle::{price_components, token_price};
use crate::positions::liquidation::execute_liquidation;
use crate::{helpers, storage, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    pub fn can_be_liquidated(env: Env, account_id: u64) -> bool {
        can_be_liquidated(&env, account_id)
    }

    pub fn health_factor(env: Env, account_id: u64) -> i128 {
        health_factor(&env, account_id)
    }

    pub fn total_collateral_in_usd(env: Env, account_id: u64) -> i128 {
        total_collateral_in_usd(&env, account_id)
    }

    pub fn total_borrow_in_usd(env: Env, account_id: u64) -> i128 {
        total_borrow_in_usd(&env, account_id)
    }

    pub fn collateral_amount_for_token(env: Env, account_id: u64, asset: Address) -> i128 {
        collateral_amount_for_token(&env, account_id, &asset)
    }

    pub fn borrow_amount_for_token(env: Env, account_id: u64, asset: Address) -> i128 {
        borrow_amount_for_token(&env, account_id, &asset)
    }

    pub fn get_account_positions(
        env: Env,
        account_id: u64,
    ) -> (
        Map<Address, AccountPositionRaw>,
        Map<Address, DebtPositionRaw>,
    ) {
        get_account_positions(&env, account_id)
    }

    pub fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes {
        get_account_attributes(&env, account_id)
    }

    pub fn get_market_config(env: Env, asset: Address) -> MarketConfig {
        storage::get_market_config(&env, &asset)
    }

    pub fn get_e_mode_category(env: Env, category_id: u32) -> EModeCategoryRaw {
        storage::get_emode_category(&env, category_id)
    }

    pub fn get_isolated_debt(env: Env, asset: Address) -> i128 {
        storage::get_isolated_debt(&env, &asset)
    }

    pub fn get_all_markets_detailed(
        env: Env,
        assets: Vec<Address>,
    ) -> Vec<AssetExtendedConfigView> {
        get_all_markets_detailed(&env, &assets)
    }

    pub fn get_all_market_indexes_detailed(env: Env, assets: Vec<Address>) -> Vec<MarketIndexView> {
        get_all_market_indexes_detailed(&env, &assets)
    }

    pub fn liquidation_estimations_detailed(
        env: Env,
        account_id: u64,
        debt_payments: Vec<(Address, i128)>,
    ) -> LiquidationEstimate {
        liquidation_estimations_detailed(&env, account_id, &debt_payments)
    }

    pub fn liquidation_collateral_available(env: Env, account_id: u64) -> i128 {
        liquidation_collateral_available(&env, account_id)
    }

    pub fn ltv_collateral_in_usd(env: Env, account_id: u64) -> i128 {
        ltv_collateral_in_usd(&env, account_id)
    }

    /// Largest currently executable `withdraw` amount; `0` while paused.
    pub fn max_withdraw(env: Env, account_id: u64, asset: Address) -> i128 {
        limits::max_withdraw(&env, account_id, &asset)
    }

    /// Supply-cap headroom; `i128::MAX` uncapped, `0` paused or inactive.
    pub fn max_supply(env: Env, asset: Address) -> i128 {
        limits::max_supply(&env, &asset)
    }

    /// Current indexes accrued to now; reads no oracle.
    pub fn get_market_index(env: Env, asset: Address) -> MarketIndexRaw {
        let mut cache = Cache::new_view(&env);
        MarketIndexRaw::from(&cache.cached_market_index(&asset))
    }
}

pub fn health_factor(env: &Env, account_id: u64) -> i128 {
    let mut cache = Cache::new_view(env);
    match storage::try_get_account(env, account_id) {
        Some(account) => helpers::calculate_health_factor(
            env,
            &mut cache,
            &account.supply_positions,
            &account.borrow_positions,
        )
        .raw(),
        None => i128::MAX,
    }
}

pub fn can_be_liquidated(env: &Env, account_id: u64) -> bool {
    health_factor(env, account_id) < WAD
}

pub fn collateral_amount_for_token(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let position = match storage::try_get_supply_position(env, account_id, asset) {
        Some(position) => position,
        None => return 0,
    };

    let mut cache = Cache::new_view(env);
    let market_index = cache.cached_market_index(asset);
    // Decimals come from pool params, not the price feed, so balance reads
    // never depend on oracle liveness.
    let decimals = cache.cached_pool_sync_data(asset).params.asset_decimals;

    position
        .scaled_amount
        .mul(env, market_index.supply_index)
        .to_asset(decimals)
}

pub fn borrow_amount_for_token(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let position = match storage::try_get_debt_position(env, account_id, asset) {
        Some(position) => position,
        None => return 0,
    };

    let mut cache = Cache::new_view(env);
    let market_index = cache.cached_market_index(asset);
    // Decimals come from pool params, not the price feed, so debt reads
    // never depend on oracle liveness.
    let decimals = cache.cached_pool_sync_data(asset).params.asset_decimals;

    position
        .scaled_amount
        .mul(env, market_index.borrow_index)
        .to_asset(decimals)
}

/// Returns raw scaled supply and debt maps for `account_id`.
pub fn get_account_positions(
    env: &Env,
    account_id: u64,
) -> (
    Map<Address, AccountPositionRaw>,
    Map<Address, DebtPositionRaw>,
) {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return (Map::new(env), Map::new(env));
    }

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
    let (_, _, weighted_coll) = helpers::calculate_account_totals(
        env,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    weighted_coll.raw()
}

pub fn get_all_markets_detailed(env: &Env, assets: &Vec<Address>) -> Vec<AssetExtendedConfigView> {
    let mut cache = Cache::new_view(env);
    let mut result = Vec::new(env);

    for i in 0..assets.len() {
        let asset = validation::expect_invariant(env, assets.get(i));
        // Discarded read panics on unsupported assets; pool address is
        // resolved per-row, so the view is safe on empty input.
        cache.cached_market_config(&asset);
        let pool_address = cache.cached_pool_address();
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
    let mut cache = Cache::new_view(env);
    cache.prefetch_market_indexes(assets);
    let mut result = Vec::new(env);

    for i in 0..assets.len() {
        let asset = validation::expect_invariant(env, assets.get(i));
        let index = cache.cached_market_index(&asset);
        let components = price_components(&mut cache, &asset);
        let (safe_price_wad, aggregator_price_wad) = components.to_abi_prices();

        result.push_back(MarketIndexView {
            asset,
            supply_index_ray: index.supply_index.raw(),
            borrow_index_ray: index.borrow_index.raw(),
            price_wad: components.final_price_wad,
            safe_price_wad,
            aggregator_price_wad,
            within_first_tolerance: components.within_first_tolerance,
            within_second_tolerance: components.within_second_tolerance,
        });
    }

    result
}

pub fn liquidation_estimations_detailed(
    env: &Env,
    account_id: u64,
    debt_payments: &Vec<Payment>,
) -> LiquidationEstimate {
    let mut cache = Cache::new_view(env);
    let account = storage::get_account(env, account_id);
    let result = execute_liquidation(env, &account, debt_payments, &mut cache);

    let mut seized_collaterals = Vec::new(env);
    let mut protocol_fees = Vec::new(env);
    for i in 0..result.seized.len() {
        let entry = validation::expect_invariant(env, result.seized.get(i));
        seized_collaterals.push_back(PaymentTuple {
            asset: entry.asset.clone(),
            amount: entry.amount,
        });
        protocol_fees.push_back(PaymentTuple {
            asset: entry.asset,
            amount: entry.protocol_fee,
        });
    }

    let mut refunds_view = Vec::new(env);
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
