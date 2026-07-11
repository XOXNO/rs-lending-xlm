//! Read-only views and liquidation estimation.
//! Views use `Cache::new_view`, so non-pricing views can inspect stored/index
//! state without renewing the controller instance TTL.

use crate::constants::{MAX_VIEW_INPUTS, WAD};
use crate::risk;
use common::errors::{GenericError, SpokeError};
use common::types::{
    AccountAttributes, AccountPositionRaw, AssetExtendedConfigView, DebtPositionRaw, HubAssetKey,
    LiquidationEstimate, MarketIndexRaw, MarketIndexView, PaymentTuple, SpokeAssetConfig,
    SpokeConfig, SpokeUsageRaw,
};
use soroban_sdk::{assert_with_error, contractimpl, panic_with_error, Address, Env, Map, Vec};

#[cfg(not(feature = "certora"))]
mod aggregates;
#[cfg(feature = "certora")]
#[path = "../../../../certora/controller/harness/views/aggregates.rs"]
mod aggregates;
mod limits;
pub use aggregates::{ltv_collateral_in_usd, total_borrow_in_usd, total_collateral_in_usd};

use crate::context::Cache;
use crate::oracle::{price_components, token_price};
use crate::positions::{liquidation::execute_liquidation, HubPayment};
use crate::{storage, Controller, ControllerArgs, ControllerClient};

/// Rejects a view request whose input vector exceeds `MAX_VIEW_INPUTS`.
fn require_view_inputs_bound<T>(env: &Env, values: &Vec<T>) {
    assert_with_error!(
        env,
        values.len() <= MAX_VIEW_INPUTS,
        GenericError::InvalidPayments
    );
}

#[contractimpl]
impl Controller {
    /// Returns whether the account's health factor is below one. A debt-free or
    /// missing account is never liquidatable.
    ///
    /// # Errors
    /// * Pricing an indebted account reads oracles and can revert (e.g.
    ///   `OracleNotConfigured`, `PriceFeedStale`, `UnsafePriceNotAllowed`).
    pub fn is_liquidatable(env: Env, account_id: u64) -> bool {
        can_be_liquidated(&env, account_id)
    }

    /// Returns the account health factor in WAD; a debt-free or missing account
    /// returns `i128::MAX`.
    ///
    /// # Errors
    /// * Pricing an indebted account reads oracles and can revert (e.g.
    ///   `OracleNotConfigured`, `PriceFeedStale`, `UnsafePriceNotAllowed`).
    pub fn get_health_factor(env: Env, account_id: u64) -> i128 {
        health_factor(&env, account_id)
    }

    /// Returns total collateral value in USD WAD; `0` for a missing account or
    /// one with no supply.
    ///
    /// # Errors
    /// * Pricing supplied positions reads oracles and can revert (e.g.
    ///   `OracleNotConfigured`, `PriceFeedStale`, `UnsafePriceNotAllowed`).
    pub fn get_total_collateral_usd(env: Env, account_id: u64) -> i128 {
        total_collateral_in_usd(&env, account_id)
    }

    /// Returns total borrow value in USD WAD; `0` for a missing account or one
    /// with no debt.
    ///
    /// # Errors
    /// * Pricing debt positions reads oracles and can revert (e.g.
    ///   `OracleNotConfigured`, `PriceFeedStale`, `UnsafePriceNotAllowed`).
    pub fn get_total_borrow_usd(env: Env, account_id: u64) -> i128 {
        total_borrow_in_usd(&env, account_id)
    }

    /// Returns the current underlying collateral amount for one hub-asset; `0`
    /// when the account holds no such position. Reads no oracle.
    pub fn get_collateral_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128 {
        collateral_amount_for_hub_asset(&env, account_id, &hub_asset)
    }

    /// Returns the current underlying debt amount for one hub-asset; `0` when the
    /// account holds no such position. Reads no oracle.
    pub fn get_borrow_amount(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128 {
        borrow_amount_for_hub_asset(&env, account_id, &hub_asset)
    }

    /// Returns the raw scaled supply and debt maps for an account; empty maps
    /// when the account does not exist.
    pub fn get_account_positions(
        env: Env,
        account_id: u64,
    ) -> (
        Map<HubAssetKey, AccountPositionRaw>,
        Map<HubAssetKey, DebtPositionRaw>,
    ) {
        get_account_positions(&env, account_id)
    }

    /// Returns the account's mode and spoke attributes.
    ///
    /// # Errors
    /// * `AccountNotInMarket` - no account metadata exists for `account_id`.
    pub fn get_account_attributes(env: Env, account_id: u64) -> AccountAttributes {
        get_account_attributes(&env, account_id)
    }

    /// Whether `account_id` still has on-chain account metadata.
    pub fn account_exists(env: Env, account_id: u64) -> bool {
        account_exists(&env, account_id)
    }

    /// Returns the per-spoke risk listing for `hub_asset`; each spoke (id `>= 1`)
    /// holds its own config.
    ///
    /// # Errors
    /// * `AssetNotInSpoke` - the asset is not listed on the spoke.
    pub fn get_spoke_asset(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeAssetConfig {
        storage::get_spoke_asset(&env, spoke_id, &hub_asset)
            .unwrap_or_else(|| panic_with_error!(&env, SpokeError::AssetNotInSpoke))
    }

    /// Returns the spoke config for `spoke_id`.
    ///
    /// # Errors
    /// * `SpokeNotFound` - no spoke exists for `spoke_id`.
    pub fn get_spoke(env: Env, spoke_id: u32) -> SpokeConfig {
        storage::get_spoke(&env, spoke_id)
    }

    /// Returns the listing's scaled usage totals; zero when no row exists.
    /// Token-space usage = `scaled * index`; cap headroom = `cap - usage`.
    pub fn get_spoke_usage(env: Env, spoke_id: u32, hub_asset: HubAssetKey) -> SpokeUsageRaw {
        storage::get_spoke_usage(&env, spoke_id, &hub_asset).unwrap_or_default()
    }

    /// Central liquidity pool for all markets; reads instance storage only.
    pub fn get_pool_address(env: Env) -> Address {
        get_pool_address(&env)
    }

    /// Returns config and USD price for each requested hub-asset market.
    ///
    /// # Errors
    /// * `InvalidPayments` - `hub_assets` exceeds the view input bound.
    /// * `OracleNotConfigured` - a requested asset has no configured oracle.
    pub fn get_markets_detailed(
        env: Env,
        hub_assets: Vec<HubAssetKey>,
    ) -> Vec<AssetExtendedConfigView> {
        get_all_markets_detailed(&env, &hub_assets)
    }

    /// Returns accrued indexes and price components for each requested hub-asset
    /// market.
    ///
    /// # Errors
    /// * `InvalidPayments` - `hub_assets` exceeds the view input bound.
    /// * `PoolNotInitialized` - a requested `(hub, asset)` market was never created.
    /// * Price-component resolution reads oracles and can revert (e.g.
    ///   `OracleNotConfigured`, `PriceFeedStale`, `UnsafePriceNotAllowed`).
    pub fn get_market_indexes_detailed(
        env: Env,
        hub_assets: Vec<HubAssetKey>,
    ) -> Vec<MarketIndexView> {
        get_all_market_indexes_detailed(&env, &hub_assets)
    }

    /// Estimates the seize, repay, refund, and bonus data for liquidating the
    /// account with the supplied debt payments.
    ///
    /// # Errors
    /// * `InvalidPayments` - `debt_payments` exceeds the view input bound.
    /// * `AccountNotFound` - no account exists for `account_id`.
    /// * The liquidation engine reverts on oracle resolution or when the account
    ///   is not liquidatable; refer to the liquidation flow errors.
    pub fn get_liquidation_estimate(
        env: Env,
        account_id: u64,
        debt_payments: Vec<(HubAssetKey, i128)>,
    ) -> LiquidationEstimate {
        liquidation_estimations_detailed(&env, account_id, &debt_payments)
    }

    /// Returns total collateral value available for liquidation in USD WAD; `0`
    /// for a missing account.
    ///
    /// # Errors
    /// * Pricing supplied positions reads oracles and can revert (e.g.
    ///   `OracleNotConfigured`, `PriceFeedStale`, `UnsafePriceNotAllowed`).
    pub fn get_liquidation_collateral(env: Env, account_id: u64) -> i128 {
        liquidation_collateral_available(&env, account_id)
    }

    /// Returns collateral value counted toward LTV in USD WAD; `0` for a missing
    /// account.
    ///
    /// # Errors
    /// * Pricing supplied positions reads oracles and can revert (e.g.
    ///   `OracleNotConfigured`, `PriceFeedStale`, `UnsafePriceNotAllowed`).
    pub fn get_ltv_collateral_usd(env: Env, account_id: u64) -> i128 {
        ltv_collateral_in_usd(&env, account_id)
    }

    /// Largest executable `withdraw` amount.
    pub fn max_withdraw(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128 {
        limits::max_withdraw(&env, account_id, &hub_asset)
    }

    /// Supply-cap headroom for `account_id`; `i128::MAX` uncapped, `0` paused or inactive.
    pub fn max_supply(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128 {
        limits::max_supply(&env, account_id, &hub_asset)
    }

    /// Largest executable `borrow` amount of `hub_asset`; `0` while
    /// paused, on an inactive/non-borrowable market, or when the asset is
    /// structurally not borrowable for the account.
    pub fn max_borrow(env: Env, account_id: u64, hub_asset: HubAssetKey) -> i128 {
        limits::max_borrow(&env, account_id, &hub_asset)
    }

    /// Accrued indexes; reads no oracle.
    pub fn get_market_index(env: Env, hub_asset: HubAssetKey) -> MarketIndexRaw {
        let mut cache = Cache::new_view(&env);
        MarketIndexRaw::from(&cache.cached_market_index(&hub_asset))
    }
}

/// Returns the account health factor in raw WAD; `i128::MAX` when the account
/// has no debt or does not exist.
pub fn health_factor(env: &Env, account_id: u64) -> i128 {
    let mut cache = Cache::new_view(env);
    match storage::try_get_account(env, account_id) {
        // A debt-free account has an infinite health factor regardless of collateral,
        // so short-circuit before pricing: calculate_account_risk_totals would
        // otherwise read every supplied asset's oracle only to saturate to MAX,
        // making a debt-free view fail on missing/broken collateral feeds.
        Some(account) if !account.borrow_positions.is_empty() => {
            risk::calculate_account_risk_totals(
                env,
                &mut cache,
                account.spoke_id,
                &account.supply_positions,
                &account.borrow_positions,
            )
            .health_factor
            .raw()
        }
        _ => i128::MAX,
    }
}

/// Returns whether the account's health factor is below one.
pub fn can_be_liquidated(env: &Env, account_id: u64) -> bool {
    // dimensional: raw WAD HealthFactor is compared to WAD-scaled 1.0.
    health_factor(env, account_id) < WAD
}

/// Returns the account's current underlying collateral for one hub-asset; `0`
/// when no such supply position exists.
pub fn collateral_amount_for_hub_asset(
    env: &Env,
    account_id: u64,
    hub_asset: &HubAssetKey,
) -> i128 {
    let Some(position) = storage::try_get_supply_position(env, account_id, hub_asset) else {
        return 0;
    };

    let mut cache = Cache::new_view(env);
    let market_index = cache.cached_market_index(hub_asset);
    let decimals = cache.cached_pool_sync_data(hub_asset).params.asset_decimals;

    position
        .scaled_amount
        .mul(env, market_index.supply_index)
        .to_asset(decimals)
}

/// Returns the account's current underlying debt for one hub-asset; `0` when no
/// such debt position exists.
pub fn borrow_amount_for_hub_asset(env: &Env, account_id: u64, hub_asset: &HubAssetKey) -> i128 {
    let Some(position) = storage::try_get_debt_position(env, account_id, hub_asset) else {
        return 0;
    };

    let mut cache = Cache::new_view(env);
    let market_index = cache.cached_market_index(hub_asset);
    let decimals = cache.cached_pool_sync_data(hub_asset).params.asset_decimals;

    position
        .scaled_amount
        .mul(env, market_index.borrow_index)
        .to_asset(decimals)
}

/// Returns whether on-chain account metadata still exists for `account_id`.
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

    (
        storage::get_supply_positions(env, account_id),
        storage::get_debt_positions(env, account_id),
    )
}

/// Returns the account's mode and spoke attributes.
pub fn get_account_attributes(env: &Env, account_id: u64) -> AccountAttributes {
    let meta = storage::get_account_meta(env, account_id);
    AccountAttributes::from(&meta)
}

/// Returns the account's liquidation-threshold weighted collateral in USD WAD;
/// `0` for a missing account.
pub fn liquidation_collateral_available(env: &Env, account_id: u64) -> i128 {
    let Some(account) = storage::try_get_account(env, account_id) else {
        return 0;
    };
    let mut cache = Cache::new_view(env);
    // dimensional: return is Wad<USD> raw (1e18) liquidation collateral.
    risk::calculate_account_risk_totals(
        env,
        &mut cache,
        account.spoke_id,
        &account.supply_positions,
        &account.borrow_positions,
    )
    .weighted_collateral
    .raw()
}

/// Returns the central liquidity pool address.
pub fn get_pool_address(env: &Env) -> Address {
    storage::get_pool(env)
}

/// Returns config and token-rooted USD price for each requested hub-asset market.
pub fn get_all_markets_detailed(
    env: &Env,
    hub_assets: &Vec<HubAssetKey>,
) -> Vec<AssetExtendedConfigView> {
    require_view_inputs_bound(env, hub_assets);
    let mut cache = Cache::new_view(env);
    let mut result = Vec::new(env);

    for hub_asset in hub_assets.iter() {
        // Pool address is resolved per-row, so the view is safe on empty input.
        // `token_price` panics `OracleNotConfigured` for an unpriced asset.
        let pool_address = cache.cached_pool_address();
        // Price is token-rooted.
        let final_price = token_price(&mut cache, &hub_asset.asset).price_wad;
        result.push_back(AssetExtendedConfigView {
            asset: hub_asset.asset,
            pool_address,
            price_wad: final_price,
        });
    }

    result
}

/// Returns accrued indexes and price components for each requested hub-asset market.
pub fn get_all_market_indexes_detailed(
    env: &Env,
    hub_assets: &Vec<HubAssetKey>,
) -> Vec<MarketIndexView> {
    require_view_inputs_bound(env, hub_assets);
    let mut cache = Cache::new_view(env);
    cache.prefetch_market_indexes(hub_assets);
    let mut result = Vec::new(env);

    for hub_asset in hub_assets.iter() {
        let index = cache.cached_market_index(&hub_asset);
        let components = price_components(&mut cache, &hub_asset);
        let (safe_price_wad, aggregator_price_wad) = components.to_abi_prices();

        result.push_back(MarketIndexView {
            asset: hub_asset.asset,
            supply_index: index.supply_index.raw(),
            borrow_index: index.borrow_index.raw(),
            price_wad: components.final_price_wad,
            safe_price_wad,
            aggregator_price_wad,
        });
    }

    result
}

/// Simulates liquidating `account_id` with `debt_payments` and returns the seize,
/// fee, refund, and bonus estimate.
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
    for entry in result.seized {
        seized_collaterals.push_back(PaymentTuple {
            asset: entry.hub_asset.asset.clone(),
            amount: entry.amount,
        });
        protocol_fees.push_back(PaymentTuple {
            asset: entry.hub_asset.asset,
            amount: entry.protocol_fee,
        });
    }

    LiquidationEstimate {
        seized_collaterals,
        protocol_fees,
        refunds: result.refunds,
        max_payment_wad: result.max_debt_usd,
        bonus_rate_bps: result.bonus_bps,
    }
}

#[cfg(test)]
#[path = "../../tests/views/mod.rs"]
mod tests;
