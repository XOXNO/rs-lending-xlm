//! Coherent controller prestate for endpoint rules.
//!
//! Production allocates hub and spoke identifiers from one. Rules seed the
//! registry directly so a transition cannot pass merely by trapping on an
//! uninitialized dependency or on the obsolete hub/spoke-zero model.

use common::types::{
    AccountMeta, HubAssetKey, HubConfig, PositionLimits, PositionMode, SpokeAssetConfig,
    SpokeConfig,
};
use cvlr_soroban::nondet_address;
use soroban_sdk::{Address, Env};

pub const ACCOUNT_ID: u64 = 1;
pub const HUB_ID: u32 = 1;
pub const SPOKE_ID: u32 = 1;

pub fn hub_asset(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: HUB_ID,
        asset: asset.clone(),
    }
}

/// Seeds every protocol-global dependency read by user position flows.
pub fn seed_protocol(env: &Env) {
    crate::storage::set_pool(env, &nondet_address());
    crate::storage::set_swap_aggregator(env, &nondet_address());
    crate::storage::set_price_aggregator(env, &nondet_address());
    crate::storage::set_accumulator(env, &nondet_address());
    crate::storage::set_position_limits(
        env,
        &PositionLimits {
            max_supply_positions: common::constants::POSITION_LIMIT_MAX,
            max_borrow_positions: common::constants::POSITION_LIMIT_MAX,
        },
    );
    crate::storage::set_min_borrow_collateral_usd_wad(env, 0);
    crate::storage::set_hub(env, HUB_ID, &HubConfig { is_active: true });
    crate::storage::set_spoke(
        env,
        SPOKE_ID,
        &SpokeConfig {
            is_deprecated: false,
            liquidation_target_hf_wad: crate::constants::DEFAULT_LIQUIDATION_TARGET_HF_WAD,
            hf_for_max_bonus_wad: crate::constants::DEFAULT_HF_FOR_MAX_BONUS_WAD,
            liquidation_bonus_factor_bps: crate::constants::DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
        },
    );
}

/// Seeds a production-valid account shell while preserving any symbolic side maps.
pub fn seed_account(env: &Env, account_id: u64, owner: &Address) {
    crate::storage::set_account_meta(
        env,
        account_id,
        &AccountMeta {
            owner: owner.clone(),
            spoke_id: SPOKE_ID,
            mode: PositionMode::Normal,
        },
    );
}

/// Lists one unrestricted test market on the live hub and spoke.
pub fn seed_market(env: &Env, asset: &Address) {
    seed_protocol(env);
    crate::storage::set_spoke_asset(
        env,
        SPOKE_ID,
        &hub_asset(asset),
        &SpokeAssetConfig {
            is_collateralizable: true,
            is_borrowable: true,
            paused: false,
            frozen: false,
            loan_to_value: 7_500,
            liquidation_threshold: 8_000,
            liquidation_bonus: 500,
            liquidation_fees: 100,
            supply_cap: i128::MAX,
            borrow_cap: i128::MAX,
        },
    );
}

pub fn seed_live_account(env: &Env, account_id: u64, owner: &Address, asset: &Address) {
    seed_market(env, asset);
    seed_account(env, account_id, owner);
}
