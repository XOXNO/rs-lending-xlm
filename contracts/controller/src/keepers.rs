use common::errors::{CollateralError, GenericError, OracleError};
use common::math::fp::Wad;
use common::types::{AccountPosition, AssetConfig, HubAssetKey};
use common::validation::{expect_invariant, require_positive_amount};
use soroban_sdk::{assert_with_error, panic_with_error, Address, Env, Vec};

use crate::constants::THRESHOLD_UPDATE_MIN_HF_RAW;
use crate::context::Cache;
use crate::external::pool::{
    pool_claim_revenue_call, pool_recapitalize_call, pool_update_indexes_call,
};
use crate::risk::validation;
use crate::{account, events, payments, risk, storage};

/// Accrues borrow and supply indexes on the pool for each hub asset in
/// `assets`. Requires `caller`'s authorization and reverts while a flash
/// loan is in progress.
pub(crate) fn update_indexes(env: &Env, caller: Address, assets: Vec<HubAssetKey>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let mut cache = Cache::new(env);
    let pool_addr = cache.cached_pool_address();
    for hub_asset in assets {
        pool_update_indexes_call(env, &pool_addr, &hub_asset);
    }
}

/// Claims accrued protocol revenue from the pool for each asset in `assets`
/// and forwards it to the configured accumulator, returning the claimed
/// amounts in the same order. Requires `caller`'s authorization and reverts
/// while a flash loan is in progress.
pub(crate) fn claim_revenue(env: &Env, caller: Address, assets: Vec<HubAssetKey>) -> Vec<i128> {
    caller.require_auth();
    validation::require_not_flash_loaning(env);
    let mut results = Vec::new(env);
    let mut cache = Cache::new(env);
    for hub_asset in assets {
        let amount = claim_revenue_for_asset_with_cache(env, &hub_asset, &mut cache);
        results.push_back(amount);
    }
    results
}

/// Transfers up to `amount` of `hub_asset` from `payer` to the pool,
/// crediting only the amount actually delivered. Injects the received
/// amount into the pool's reserve for that market to cover a backing
/// shortfall; any unused amount is refunded. Requires `payer`'s
/// authorization, a positive `amount`, and no flash loan in progress.
pub(crate) fn recapitalize(
    env: &Env,
    payer: Address,
    hub_asset: HubAssetKey,
    amount: i128,
) -> i128 {
    payer.require_auth();
    validation::require_not_flash_loaning(env);
    require_positive_amount(env, amount);

    let mut cache = Cache::new(env);
    let pool_addr = cache.cached_pool_address();
    // Credit only tokens actually received (mirrors supply/repay measurement).
    let received = payments::transfer_amount_measured(
        env,
        &hub_asset.asset,
        &payer,
        &pool_addr,
        amount,
        GenericError::AmountMustBePositive,
    );

    pool_recapitalize_call(env, &pool_addr, &hub_asset, &payer, received).actual_amount
}

/// Refreshes the loan-to-value snapshot for each account in `account_ids`,
/// also refreshing liquidation-threshold parameters when `has_risks` is
/// true. Requires `caller`'s authorization and reverts while a flash loan
/// is in progress.
pub(crate) fn update_account_threshold(
    env: &Env,
    caller: Address,
    has_risks: bool,
    account_ids: Vec<u64>,
) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let mut cache = Cache::new(env);

    for account_id in account_ids {
        cache.reset_spoke_context();
        sync_account_thresholds(env, account_id, has_risks, &mut cache);
    }
}

/// Claims `hub_asset`'s accrued protocol revenue from the pool and, if the
/// claimed amount is positive, transfers it from the controller to the
/// configured accumulator address. Returns the claimed amount. Panics if no
/// accumulator has been configured.
fn claim_revenue_for_asset_with_cache(
    env: &Env,
    hub_asset: &HubAssetKey,
    cache: &mut Cache,
) -> i128 {
    let accumulator = storage::try_get_accumulator(env)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::NoAccumulator));

    let pool_addr = cache.cached_pool_address();

    let result = pool_claim_revenue_call(env, &pool_addr, hub_asset);
    let amount = result.actual_amount;

    if amount > 0 {
        common::token::sac_transfer(
            env,
            &hub_asset.asset,
            &env.current_contract_address(),
            &accumulator,
            amount,
        );
    }

    amount
}

/// Recomputes each of `account_id`'s supply positions' risk parameters
/// against current spoke asset config, refreshing loan-to-value always and
/// liquidation-threshold parameters only when `has_risks` is true. Skips
/// accounts with no stored metadata or no supply positions. When `has_risks`
/// is true, panics if the account's health factor falls below the minimum
/// threshold-update level after the changes are applied.
fn sync_account_thresholds(env: &Env, account_id: u64, has_risks: bool, cache: &mut Cache) {
    let Some(meta) = storage::try_get_account_meta(env, account_id) else {
        return;
    };

    let supply_positions = storage::get_supply_positions(env, account_id);
    if supply_positions.is_empty() {
        return;
    }

    // Fail closed: an id whose NFT owner cannot be resolved (never minted, burned, or the
    // NFT unconfigured) is skipped rather than treated as ownerless. Resolved after the
    // empty-positions check so a skip on that path doesn't spend a cross-contract call.
    let Some(owner) = storage::try_account_owner(env, account_id) else {
        return;
    };

    let borrow_positions = if has_risks {
        storage::get_debt_positions(env, account_id)
    } else {
        soroban_sdk::Map::new(env)
    };

    storage::renew_user_account(env, account_id);

    let mut account = storage::account_from_parts(owner, meta, supply_positions, borrow_positions);
    let assets = account.supply_positions.keys();
    let scope = if has_risks {
        risk::RiskRefreshScope::FullTuple
    } else {
        risk::RiskRefreshScope::LtvOnly
    };

    let mut any_changed = false;
    for hub_asset in assets.iter() {
        let Some(spoke_config) = cache.cached_spoke_asset(account.spoke_id, &hub_asset) else {
            continue;
        };
        let asset_config = AssetConfig::from(&spoke_config);

        let raw = expect_invariant(env, account.supply_positions.get(hub_asset.clone()));
        let mut updated = AccountPosition::from(&raw);

        let changed = risk::refresh_supply_risk_params(
            env,
            cache,
            &account,
            &hub_asset,
            &mut updated,
            &asset_config,
            scope,
        );
        if !changed {
            continue;
        }

        any_changed = true;
        account::update_or_remove_supply_position(&mut account, &hub_asset, &updated);

        let market_index = cache.cached_market_index(&hub_asset);
        cache.record_supply_position_update(
            events::PositionAction::ParamUpd,
            &hub_asset,
            market_index.supply_index.raw(),
            0,
            &updated,
        );
    }

    if any_changed {
        storage::set_supply_positions(env, account_id, &account.supply_positions);
    }

    if has_risks {
        let hf = risk::calculate_account_risk_totals(
            env,
            cache,
            &account.supply_positions,
            &account.borrow_positions,
        )
        .health_factor;
        assert_with_error!(
            env,
            hf >= Wad::from(THRESHOLD_UPDATE_MIN_HF_RAW),
            CollateralError::HealthFactorTooLow
        );
    }

    cache.emit_position_batch(account_id, &account);
}
