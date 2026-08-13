use common::types::{
    AccountMeta, AccountPositionRaw, HubAssetKey, HubConfig, PositionLimits, PositionMode,
    SpokeAssetConfig, SpokeConfig,
};
use cvlr_soroban::nondet_address;
use soroban_sdk::{Address, Env};

pub const ACCOUNT_ID: u64 = 1;
pub const HUB_ID: u32 = 1;
pub const SPOKE_ID: u32 = 1;

pub const UNCONSTRAINED_CAP: i128 = i128::MAX
    / 10i128.pow(common::constants::RAY_DECIMALS)
    / (common::constants::RAY / common::constants::SUPPLY_INDEX_FLOOR_RAW);

pub fn hub_asset(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: HUB_ID,
        asset: asset.clone(),
    }
}

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
            supply_cap: UNCONSTRAINED_CAP,
            borrow_cap: UNCONSTRAINED_CAP,
        },
    );
}

pub fn seed_live_account(env: &Env, account_id: u64, owner: &Address, asset: &Address) {
    seed_market(env, asset);
    seed_account(env, account_id, owner);
}

/// Writes a concrete supply position for `asset`. The caller must have already
/// seeded the account (e.g. `seed_live_account`), so rules can control the
/// owner/caller relationship; this helper only writes the position map.
pub fn seed_supply_position(env: &Env, account_id: u64, asset: &Address, scaled_amount: i128) {
    let mut map = crate::storage::get_supply_positions(env, account_id);
    map.set(
        hub_asset(asset),
        AccountPositionRaw {
            scaled_amount,
            liquidation_threshold: 8_000,
            liquidation_bonus: 500,
            loan_to_value: 7_500,
            liquidation_fees: 100,
        },
    );
    crate::storage::set_supply_positions(env, account_id, &map);
}

/// Writes a concrete debt position for `asset`. The caller must have already
/// seeded the account (e.g. `seed_live_account`).
pub fn seed_debt_position(env: &Env, account_id: u64, asset: &Address, scaled_amount: i128) {
    let mut map = crate::storage::get_debt_positions(env, account_id);
    map.set(
        hub_asset(asset),
        common::types::DebtPositionRaw { scaled_amount },
    );
    crate::storage::set_debt_positions(env, account_id, &map);
}

/// Writes `assets.len()` concrete supply positions, returning the count that
/// was actually persisted (distinct keys only). Callers must assume the assets
/// pairwise distinct to reach exactly `assets.len()` entries.
pub fn seed_supply_positions(env: &Env, account_id: u64, assets: &[Address]) -> u32 {
    let mut map = crate::storage::get_supply_positions(env, account_id);
    for asset in assets {
        map.set(
            hub_asset(asset),
            AccountPositionRaw {
                scaled_amount: 1,
                liquidation_threshold: 8_000,
                liquidation_bonus: 500,
                loan_to_value: 7_500,
                liquidation_fees: 100,
            },
        );
    }
    let count = map.len();
    crate::storage::set_supply_positions(env, account_id, &map);
    count
}

/// Writes `assets.len()` concrete debt positions, returning the count that was
/// actually persisted (distinct keys only).
pub fn seed_debt_positions(env: &Env, account_id: u64, assets: &[Address]) -> u32 {
    let mut map = crate::storage::get_debt_positions(env, account_id);
    for asset in assets {
        map.set(
            hub_asset(asset),
            common::types::DebtPositionRaw { scaled_amount: 1 },
        );
    }
    let count = map.len();
    crate::storage::set_debt_positions(env, account_id, &map);
    count
}
