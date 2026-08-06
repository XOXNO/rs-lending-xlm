use common::types::{
    AccountMeta, HubAssetKey, HubConfig, PositionLimits, PositionMode, SpokeAssetConfig,
    SpokeConfig,
};
use cvlr_soroban::nondet_address;
use soroban_sdk::{Address, Env};

pub const ACCOUNT_ID: u64 = 1;
pub const HUB_ID: u32 = 1;
pub const SPOKE_ID: u32 = 1;

/// Supply/borrow cap for fixtures that are not exercising cap behaviour.
///
/// Caps are always enforced and carry no "unlimited" sentinel, so a fixture
/// has to name a finite ceiling. It must be one `enforce_spoke_cap` can scale
/// without overflowing for every market shape the pool summaries can invent:
/// `get_sync_data_summary` draws `asset_decimals` anywhere in `0..=27`, and
/// `nondet_market_index` draws the supply index as low as
/// `SUPPLY_INDEX_FLOOR_RAW`. Both worst cases can land together, and
/// `cap_to_scaled` hits them in order:
///
/// 1. `Ray::from_asset(cap, 0)` upscales by `10^RAY_DECIMALS` through a
///    `checked_mul` that panics rather than wrapping;
/// 2. `div_floor(index)` then scales by `RAY / SUPPLY_INDEX_FLOOR_RAW` before
///    narrowing the `I256` result back to `i128`.
///
/// Dividing `i128::MAX` by both factors yields the largest cap that survives
/// the pair. Note that no finite cap is unconditionally non-binding here: the
/// pool summaries hand back an unbounded nondeterministic scaled position, so
/// the prover can always pick a leg that trips any ceiling. Taking the maximum
/// keeps the space of entries the prover may still take as wide as possible.
///
/// A fixture that means "this market is closed" writes `0` instead.
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
