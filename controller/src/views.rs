use common::constants::WAD;
use common::fp::{Ray, Wad};
use common::types::{
    AccountAttributes, AccountPosition, AssetExtendedConfigView, EModeCategory,
    LiquidationEstimate, MarketConfig, MarketIndexView, Payment, PaymentTuple,
    POSITION_TYPE_BORROW, POSITION_TYPE_DEPOSIT,
};
use soroban_sdk::{contractimpl, Address, Env, Map, Vec};

use crate::cache::ControllerCache;
use crate::{helpers, storage, Controller, ControllerArgs, ControllerClient};

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
    ) -> (Map<Address, AccountPosition>, Map<Address, AccountPosition>) {
        get_account_positions(&env, account_id)
    }

    pub fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes {
        get_account_attributes(&env, account_id)
    }

    pub fn get_market_config(env: Env, asset: Address) -> MarketConfig {
        get_market_config_view(&env, &asset)
    }

    pub fn get_e_mode_category(env: Env, category_id: u32) -> EModeCategory {
        get_emode_category_view(&env, category_id)
    }

    pub fn get_isolated_debt(env: Env, asset: Address) -> i128 {
        get_isolated_debt_view(&env, &asset)
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
        debt_payments: Vec<Payment>,
    ) -> LiquidationEstimate {
        liquidation_estimations_detailed(&env, account_id, &debt_payments)
    }

    pub fn liquidation_collateral_available(env: Env, account_id: u64) -> i128 {
        liquidation_collateral_available(&env, account_id)
    }

    pub fn ltv_collateral_in_usd(env: Env, account_id: u64) -> i128 {
        ltv_collateral_in_usd(&env, account_id)
    }
}

pub fn health_factor(env: &Env, account_id: u64) -> i128 {
    let mut cache = ControllerCache::new_view(env);
    match storage::try_get_account(env, account_id) {
        Some(account) => helpers::calculate_health_factor(
            env,
            &mut cache,
            &account.supply_positions,
            &account.borrow_positions,
        ),
        None => i128::MAX,
    }
}

pub fn can_be_liquidated(env: &Env, account_id: u64) -> bool {
    health_factor(env, account_id) < WAD
}

crate::summarized!(
    total_collateral_in_usd_summary,
    pub fn total_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
        if storage::try_get_account_meta(env, account_id).is_none() {
            return 0;
        }
        let supply = storage::get_supply_positions(env, account_id);
        if supply.is_empty() {
            return 0;
        }

        let mut cache = ControllerCache::new_view(env);
        let mut total_collateral = Wad::ZERO;

        for (asset, position) in supply.iter() {
            let feed = cache.cached_price(&asset);
            let market_index = cache.cached_market_index(&asset);

            let value = helpers::position_value(
                env,
                Ray::from_raw(position.scaled_amount_ray),
                Ray::from_raw(market_index.supply_index_ray),
                Wad::from_raw(feed.price_wad),
            );
            total_collateral += value;
        }

        total_collateral.raw()
    }
);

crate::summarized!(
    total_borrow_in_usd_summary,
    pub fn total_borrow_in_usd(env: &Env, account_id: u64) -> i128 {
        if storage::try_get_account_meta(env, account_id).is_none() {
            return 0;
        }
        let borrow = storage::get_borrow_positions(env, account_id);
        if borrow.is_empty() {
            return 0;
        }

        let mut cache = ControllerCache::new_view(env);
        let mut total_borrow = Wad::ZERO;

        for (asset, position) in borrow.iter() {
            let feed = cache.cached_price(&asset);
            let market_index = cache.cached_market_index(&asset);

            let value = helpers::position_value(
                env,
                Ray::from_raw(position.scaled_amount_ray),
                Ray::from_raw(market_index.borrow_index_ray),
                Wad::from_raw(feed.price_wad),
            );
            total_borrow += value;
        }

        total_borrow.raw()
    }
);

pub fn collateral_amount_for_token(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let position = match storage::try_get_position(env, account_id, POSITION_TYPE_DEPOSIT, asset) {
        Some(position) => position,
        None => return 0,
    };

    let mut cache = ControllerCache::new_view(env);
    let market_index = cache.cached_market_index(asset);
    let feed = cache.cached_price(asset);

    Ray::from_raw(position.scaled_amount_ray)
        .mul(env, Ray::from_raw(market_index.supply_index_ray))
        .to_asset(feed.asset_decimals)
}

pub fn borrow_amount_for_token(env: &Env, account_id: u64, asset: &Address) -> i128 {
    let position = match storage::try_get_position(env, account_id, POSITION_TYPE_BORROW, asset) {
        Some(position) => position,
        None => return 0,
    };

    let mut cache = ControllerCache::new_view(env);
    let market_index = cache.cached_market_index(asset);
    let feed = cache.cached_price(asset);

    Ray::from_raw(position.scaled_amount_ray)
        .mul(env, Ray::from_raw(market_index.borrow_index_ray))
        .to_asset(feed.asset_decimals)
}

/// Returns the supply and borrow position maps keyed by asset so the SDK
/// receives the asset alongside the snapshot — the stored value no longer
/// carries it.
pub fn get_account_positions(
    env: &Env,
    account_id: u64,
) -> (Map<Address, AccountPosition>, Map<Address, AccountPosition>) {
    if storage::try_get_account_meta(env, account_id).is_none() {
        return (Map::new(env), Map::new(env));
    }

    (
        storage::get_supply_positions(env, account_id),
        storage::get_borrow_positions(env, account_id),
    )
}

pub fn get_account_attributes(env: &Env, account_id: u64) -> AccountAttributes {
    let meta = storage::get_account_meta(env, account_id);
    AccountAttributes::from(&meta)
}

pub fn get_market_config_view(env: &Env, asset: &Address) -> MarketConfig {
    storage::get_market_config(env, asset)
}

pub fn get_emode_category_view(env: &Env, category_id: u32) -> EModeCategory {
    storage::get_emode_category(env, category_id)
}

pub fn get_isolated_debt_view(env: &Env, asset: &Address) -> i128 {
    storage::get_isolated_debt(env, asset)
}

pub fn liquidation_collateral_available(env: &Env, account_id: u64) -> i128 {
    let account = match storage::try_get_account(env, account_id) {
        Some(account) => account,
        None => return 0,
    };
    let mut cache = ControllerCache::new_view(env);
    let (_, _, weighted_coll) = helpers::calculate_account_totals(
        env,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );
    weighted_coll.raw()
}

crate::summarized!(
    ltv_collateral_in_usd_summary,
    pub fn ltv_collateral_in_usd(env: &Env, account_id: u64) -> i128 {
        let account = match storage::try_get_account(env, account_id) {
            Some(account) => account,
            None => return 0,
        };
        let mut cache = ControllerCache::new_view(env);
        helpers::calculate_ltv_collateral_wad(env, &mut cache, &account.supply_positions).raw()
    }
);

// ---------------------------------------------------------------------------
// Market index views
// ---------------------------------------------------------------------------

pub fn get_all_markets_detailed(env: &Env, assets: &Vec<Address>) -> Vec<AssetExtendedConfigView> {
    let mut cache = ControllerCache::new_view(env);
    let mut result = Vec::new(env);

    for i in 0..assets.len() {
        let asset = assets.get(i).unwrap();
        let market = cache.cached_market_config(&asset);
        let final_price = crate::oracle::token_price(&mut cache, &asset).price_wad;
        result.push_back(AssetExtendedConfigView {
            asset,
            pool_address: market.pool_address,
            price_wad: final_price,
        });
    }

    result
}

pub fn get_all_market_indexes_detailed(env: &Env, assets: &Vec<Address>) -> Vec<MarketIndexView> {
    let mut cache = ControllerCache::new_view(env);
    let mut result = Vec::new(env);

    for i in 0..assets.len() {
        let asset = assets.get(i).unwrap();
        let index = cache.cached_market_index(&asset);
        let (aggregator_price, safe_price, final_price, within_first, within_second) =
            crate::oracle::price_components(&mut cache, &asset);
        let safe_price_wad = safe_price.unwrap_or(final_price);
        let aggregator_price_wad = aggregator_price.unwrap_or(final_price);

        result.push_back(MarketIndexView {
            asset,
            supply_index_ray: index.supply_index_ray,
            borrow_index_ray: index.borrow_index_ray,
            price_wad: final_price,
            safe_price_wad,
            aggregator_price_wad,
            within_first_tolerance: within_first,
            within_second_tolerance: within_second,
        });
    }

    result
}

// ---------------------------------------------------------------------------
// Liquidation estimation view
// ---------------------------------------------------------------------------

pub fn liquidation_estimations_detailed(
    env: &Env,
    account_id: u64,
    debt_payments: &Vec<Payment>,
) -> LiquidationEstimate {
    let mut cache = ControllerCache::new_view(env);
    let account = storage::get_account(env, account_id);
    let result = crate::positions::liquidation::execute_liquidation(
        env,
        &account,
        debt_payments,
        &mut cache,
    );

    let mut seized_collaterals = Vec::new(env);
    let mut protocol_fees = Vec::new(env);
    for i in 0..result.seized.len() {
        let entry = result.seized.get(i).unwrap();
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
        let (asset, amount) = result.refunds.get(i).unwrap();
        refunds_view.push_back(PaymentTuple { asset, amount });
    }

    LiquidationEstimate {
        seized_collaterals,
        protocol_fees,
        refunds: refunds_view,
        max_payment_wad: result.max_debt_usd,
        bonus_rate_bps: result.bonus_bps,
    }
}
