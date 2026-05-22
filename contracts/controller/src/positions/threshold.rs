use common::errors::CollateralError;
use common::math::fp::Wad;
use common::types::{Account, AccountPosition, AccountPositionType, AssetConfig, PriceFeed};
use soroban_sdk::{contractimpl, panic_with_error, symbol_short, Address, Env, Vec};
use stellar_macros::{only_role, when_not_paused};

use super::{emode, update};
use crate::cache::ControllerCache;
use crate::oracle::policy::OraclePolicy;
use crate::{helpers, storage, validation, Controller, ControllerArgs, ControllerClient};

const THRESHOLD_UPDATE_MIN_HF_RAW: i128 = 1_050_000_000_000_000_000;

#[contractimpl]
impl Controller {
    #[when_not_paused]
    #[only_role(caller, "KEEPER")]
    pub fn update_account_threshold(
        env: Env,
        caller: Address,
        asset: Address,
        has_risks: bool,
        account_ids: Vec<u64>,
    ) {
        validation::require_not_flash_loaning(&env);

        // Propagates threshold updates with safety buffer.
        let mut cache = ControllerCache::new(&env, OraclePolicy::RiskIncreasing);
        validation::require_asset_supported(&env, &mut cache, &asset);

        let base_config = cache.cached_asset_config(&asset);
        let price_feed = cache.cached_price(&asset);

        for account_id in account_ids {
            let mut account_asset_config = base_config.clone();

            update_position_threshold(
                &env,
                account_id,
                ThresholdUpdate {
                    asset: &asset,
                    has_risks,
                    asset_config: &mut account_asset_config,
                    feed: &price_feed,
                },
                &mut cache,
            );
        }
    }
}

/// Per-account inputs for a keeper threshold propagation.
struct ThresholdUpdate<'a> {
    asset: &'a Address,
    has_risks: bool,
    asset_config: &'a mut AssetConfig,
    feed: &'a PriceFeed,
}

// Keeper-driven risk parameter propagation.
fn update_position_threshold(
    env: &Env,
    account_id: u64,
    update_req: ThresholdUpdate<'_>,
    cache: &mut ControllerCache,
) {
    let ThresholdUpdate {
        asset,
        has_risks,
        asset_config,
        feed,
    } = update_req;

    // No-op when the account is gone (bad-debt cleanup, full exit).
    let Some(meta) = storage::try_get_account_meta(env, account_id) else {
        return;
    };

    let supply_positions = storage::get_positions(env, account_id, AccountPositionType::Deposit);

    // No-op when the account has no supply position for this asset.
    let Some(position) = supply_positions.get(asset.clone()) else {
        return;
    };

    // Load borrow positions only when the health-factor gate requires them.
    let borrow_positions = if has_risks {
        storage::get_positions(env, account_id, AccountPositionType::Borrow)
    } else {
        soroban_sdk::Map::new(env)
    };

    storage::renew_user_account(env, account_id);

    // Apply e-mode overrides.
    let e_mode_category = emode::e_mode_category(env, meta.e_mode_category_id);
    let asset_emode_config = cache.cached_emode_asset(meta.e_mode_category_id, asset);
    emode::apply_e_mode_to_asset_config(env, asset_config, &e_mode_category, asset_emode_config);

    let mut updated_pos = position;

    let cfg_lt = asset_config.liquidation_threshold.raw() as u32;
    let cfg_ltv = asset_config.loan_to_value.raw() as u32;
    let cfg_bonus = asset_config.liquidation_bonus.raw() as u32;
    let cfg_fees = asset_config.liquidation_fees.raw() as u32;
    if has_risks {
        if updated_pos.liquidation_threshold_bps != cfg_lt {
            updated_pos.liquidation_threshold_bps = cfg_lt;
        }
    } else {
        if updated_pos.loan_to_value_bps != cfg_ltv {
            updated_pos.loan_to_value_bps = cfg_ltv;
        }
        if updated_pos.liquidation_bonus_bps != cfg_bonus {
            updated_pos.liquidation_bonus_bps = cfg_bonus;
        }
        if updated_pos.liquidation_fees_bps != cfg_fees {
            updated_pos.liquidation_fees_bps = cfg_fees;
        }
    }

    let mut account = Account {
        owner: meta.owner.clone(),
        is_isolated: meta.is_isolated,
        e_mode_category_id: meta.e_mode_category_id,
        mode: meta.mode,
        isolated_asset: meta.isolated_asset.clone(),
        supply_positions,
        borrow_positions,
    };
    update::update_or_remove_position(
        &mut account,
        AccountPositionType::Deposit,
        asset,
        &AccountPosition::from(&updated_pos),
    );

    // Persist only the supply side; borrow stays as-is.
    storage::set_positions(
        env,
        account_id,
        AccountPositionType::Deposit,
        &account.supply_positions,
    );

    // Enforce safety buffer on risky updates.
    if has_risks {
        let hf = helpers::calculate_health_factor(
            env,
            cache,
            &account.supply_positions,
            &account.borrow_positions,
        );
        if hf < Wad::from_raw(THRESHOLD_UPDATE_MIN_HF_RAW) {
            panic_with_error!(env, CollateralError::HealthFactorTooLow);
        }
    }

    // Record a position update with amount = 0; no deposit or withdraw
    // occurred, only a parameter change.
    let market_index = cache.cached_market_index(asset);
    cache.record_position_update(
        symbol_short!("param_upd"),
        AccountPositionType::Deposit,
        asset,
        market_index.supply_index.raw(),
        0,
        &AccountPosition::from(&updated_pos),
        Some(feed.price.raw()),
    );
    cache.emit_position_batch(account_id, &account);
}
