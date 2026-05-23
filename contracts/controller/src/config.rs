use common::constants::{
    BPS, MAX_FIRST_TOLERANCE, MAX_LAST_TOLERANCE, MIN_FIRST_TOLERANCE, MIN_LAST_TOLERANCE,
};
use common::errors::{EModeError, GenericError, OracleError};
use common::events::{
    emit_approve_token, emit_remove_emode_asset, emit_update_asset_config,
    emit_update_asset_oracle, emit_update_emode_asset, emit_update_emode_category,
    ApproveTokenEvent, EventEModeCategory, EventOracleProvider, RemoveEModeAssetEvent,
    UpdateAssetConfigEvent, UpdateAssetOracleEvent, UpdateEModeAssetEvent,
    UpdateEModeCategoryEvent,
};
use common::math::fp_core;
use common::types::{
    AssetConfigRaw, EModeAssetConfig, EModeCategoryRaw, MarketOracleConfigInput, MarketStatus,
    OraclePriceFluctuation, PositionLimits,
};
use soroban_sdk::{contractimpl, panic_with_error, xdr::ToXdr, Address, BytesN, Env, Executable};
use stellar_macros::{only_owner, only_role};

use crate::oracle::validation::validate_market_oracle_sources;

use crate::{storage, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[only_owner]
    pub fn set_aggregator(env: Env, addr: Address) {
        set_aggregator(&env, addr);
    }

    #[only_owner]
    pub fn set_accumulator(env: Env, addr: Address) {
        set_accumulator(&env, addr);
    }

    #[only_owner]
    pub fn set_liquidity_pool_template(env: Env, hash: BytesN<32>) {
        set_liquidity_pool_template(&env, hash);
    }

    #[only_owner]
    pub fn edit_asset_config(env: Env, asset: Address, cfg: AssetConfigRaw) {
        edit_asset_config(&env, asset, cfg);
    }

    #[only_owner]
    pub fn set_position_limits(env: Env, limits: PositionLimits) {
        set_position_limits(&env, limits);
    }

    #[only_owner]
    pub fn add_e_mode_category(env: Env, ltv: u32, threshold: u32, bonus: u32) -> u32 {
        add_e_mode_category(&env, ltv, threshold, bonus)
    }

    #[only_owner]
    pub fn edit_e_mode_category(env: Env, id: u32, ltv: u32, threshold: u32, bonus: u32) {
        edit_e_mode_category(&env, id, ltv, threshold, bonus);
    }

    #[only_owner]
    pub fn remove_e_mode_category(env: Env, id: u32) {
        remove_e_mode_category(&env, id);
    }

    #[only_owner]
    pub fn add_asset_to_e_mode_category(
        env: Env,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
    ) {
        add_asset_to_e_mode_category(&env, asset, category_id, can_collateral, can_borrow);
    }

    #[only_owner]
    pub fn edit_asset_in_e_mode_category(
        env: Env,
        asset: Address,
        category_id: u32,
        can_collateral: bool,
        can_borrow: bool,
    ) {
        edit_asset_in_e_mode_category(&env, asset, category_id, can_collateral, can_borrow);
    }

    #[only_owner]
    pub fn remove_asset_from_e_mode(env: Env, asset: Address, category_id: u32) {
        remove_asset_from_e_mode(&env, asset, category_id);
    }

    #[only_owner]
    pub fn approve_token(env: Env, token: Address) {
        storage::set_token_approved(&env, &token, true);
        let wasm_hash = env.crypto().keccak256(&token.to_xdr(&env)).into();
        emit_approve_token(
            &env,
            ApproveTokenEvent {
                wasm_hash,
                approved: true,
            },
        );
    }

    #[only_owner]
    pub fn revoke_token(env: Env, token: Address) {
        storage::set_token_approved(&env, &token, false);
        let wasm_hash = env.crypto().keccak256(&token.to_xdr(&env)).into();
        emit_approve_token(
            &env,
            ApproveTokenEvent {
                wasm_hash,
                approved: false,
            },
        );
    }

    #[only_role(caller, "ORACLE")]
    pub fn configure_market_oracle(
        env: Env,
        caller: Address,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) {
        let _ = caller;
        configure_market_oracle(&env, asset, cfg);
    }

    #[only_role(caller, "ORACLE")]
    pub fn edit_oracle_tolerance(
        env: Env,
        caller: Address,
        asset: Address,
        first_tolerance: u32,
        last_tolerance: u32,
    ) {
        let _ = caller;
        edit_oracle_tolerance(&env, asset, first_tolerance, last_tolerance);
    }

    #[only_role(caller, "ORACLE")]
    pub fn disable_token_oracle(env: Env, caller: Address, asset: Address) {
        let _ = caller;
        disable_token_oracle(&env, asset);
    }
}

fn require_contract_address(
    env: &Env,
    addr: &Address,
    error: impl Into<soroban_sdk::Error> + soroban_sdk::SpecShakingMarker,
) {
    if !addr.exists() || !matches!(addr.executable(), Some(Executable::Wasm(_))) {
        panic_with_error!(env, error);
    }
}

fn require_nonzero_wasm_hash(env: &Env, hash: &BytesN<32>) {
    if hash.to_array() == [0; 32] {
        panic_with_error!(env, GenericError::InvalidPoolTemplate);
    }
}

pub fn set_aggregator(env: &Env, addr: Address) {
    require_contract_address(env, &addr, OracleError::InvalidAggregator);
    storage::set_aggregator(env, &addr);
}

pub fn set_accumulator(env: &Env, addr: Address) {
    require_contract_address(env, &addr, GenericError::NotSmartContract);
    storage::set_accumulator(env, &addr);
}

pub fn set_liquidity_pool_template(env: &Env, hash: BytesN<32>) {
    require_nonzero_wasm_hash(env, &hash);
    storage::set_pool_template(env, &hash);
}

pub fn edit_asset_config(env: &Env, asset: Address, mut next_config: AssetConfigRaw) {
    validation::validate_asset_config(env, &next_config);

    let mut market = storage::get_market_config(env, &asset);
    next_config.e_mode_categories = market.asset_config.e_mode_categories.clone();
    market.asset_config = next_config.clone();
    storage::set_market_config(env, &asset, &market);

    emit_update_asset_config(
        env,
        UpdateAssetConfigEvent {
            asset,
            config: next_config,
        },
    );
}

const POSITION_LIMIT_MAX: u32 = 10;

pub fn set_position_limits(env: &Env, limits: PositionLimits) {
    if limits.max_supply_positions == 0
        || limits.max_borrow_positions == 0
        || limits.max_supply_positions > POSITION_LIMIT_MAX
        || limits.max_borrow_positions > POSITION_LIMIT_MAX
    {
        panic_with_error!(env, GenericError::InvalidPositionLimits);
    }
    storage::set_position_limits(env, &limits);
}

pub fn add_e_mode_category(env: &Env, ltv: u32, threshold: u32, bonus: u32) -> u32 {
    validation::validate_risk_bounds(env, ltv, threshold, bonus);

    let id = storage::increment_emode_category_id(env);
    let cat = EModeCategoryRaw {
        loan_to_value_bps: ltv,
        liquidation_threshold_bps: threshold,
        liquidation_bonus_bps: bonus,
        is_deprecated: false,
        assets: soroban_sdk::Map::new(env),
    };
    storage::set_emode_category(env, id, &cat);

    emit_update_emode_category(
        env,
        UpdateEModeCategoryEvent {
            category: EventEModeCategory::new(id, &cat),
        },
    );

    id
}

pub fn edit_e_mode_category(env: &Env, id: u32, ltv: u32, threshold: u32, bonus: u32) {
    validation::validate_risk_bounds(env, ltv, threshold, bonus);
    let mut cat = storage::try_get_emode_category(env, id)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound));
    if cat.is_deprecated {
        panic_with_error!(env, EModeError::EModeCategoryDeprecated);
    }
    cat.loan_to_value_bps = ltv;
    cat.liquidation_threshold_bps = threshold;
    cat.liquidation_bonus_bps = bonus;
    storage::set_emode_category(env, id, &cat);

    emit_update_emode_category(
        env,
        UpdateEModeCategoryEvent {
            category: EventEModeCategory::new(id, &cat),
        },
    );
}

pub fn remove_e_mode_category(env: &Env, id: u32) {
    let mut cat = storage::try_get_emode_category(env, id)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound));
    if cat.is_deprecated {
        panic_with_error!(env, EModeError::EModeCategoryDeprecated);
    }
    cat.is_deprecated = true;

    let members = cat.assets.clone();
    cat.assets = soroban_sdk::Map::new(env);
    storage::set_emode_category(env, id, &cat);

    for asset in members.keys() {
        remove_emode_category_from_market_config(env, &asset, id);
    }

    emit_update_emode_category(
        env,
        UpdateEModeCategoryEvent {
            category: EventEModeCategory::new(id, &cat),
        },
    );
}

pub fn add_asset_to_e_mode_category(
    env: &Env,
    asset: Address,
    category_id: u32,
    can_collateral: bool,
    can_borrow: bool,
) {
    let cat = storage::try_get_emode_category(env, category_id)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound));
    if cat.is_deprecated {
        panic_with_error!(env, EModeError::EModeCategoryDeprecated);
    }

    if !storage::has_market_config(env, &asset) {
        panic_with_error!(env, GenericError::AssetNotSupported);
    }

    if storage::get_emode_asset(env, category_id, &asset).is_some() {
        panic_with_error!(env, EModeError::AssetAlreadyInEmode);
    }

    let config = EModeAssetConfig {
        is_collateralizable: can_collateral,
        is_borrowable: can_borrow,
    };
    storage::set_emode_asset(env, category_id, &asset, &config);

    let mut market = storage::get_market_config(env, &asset);
    if !market.asset_config.e_mode_categories.contains(category_id) {
        market.asset_config.e_mode_categories.push_back(category_id);
        storage::set_market_config(env, &asset, &market);
    }

    emit_update_emode_asset(
        env,
        UpdateEModeAssetEvent {
            asset,
            config,
            category_id,
        },
    );
}

pub fn edit_asset_in_e_mode_category(
    env: &Env,
    asset: Address,
    category_id: u32,
    can_collateral: bool,
    can_borrow: bool,
) {
    let cat = storage::try_get_emode_category(env, category_id)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound));
    if cat.is_deprecated {
        panic_with_error!(env, EModeError::EModeCategoryDeprecated);
    }
    if !cat.assets.contains_key(asset.clone()) {
        panic_with_error!(env, EModeError::AssetNotInEmode);
    }

    let config = EModeAssetConfig {
        is_collateralizable: can_collateral,
        is_borrowable: can_borrow,
    };
    storage::set_emode_asset(env, category_id, &asset, &config);

    emit_update_emode_asset(
        env,
        UpdateEModeAssetEvent {
            asset,
            config,
            category_id,
        },
    );
}

pub fn remove_asset_from_e_mode(env: &Env, asset: Address, category_id: u32) {
    storage::remove_emode_asset(env, category_id, &asset);

    remove_emode_category_from_market_config(env, &asset, category_id);

    emit_remove_emode_asset(env, RemoveEModeAssetEvent { asset, category_id });
}

/// i128 to u32 (checked).
fn bps_i128_to_u32(env: &Env, v: i128) -> u32 {
    u32::try_from(v).unwrap_or_else(|_| panic_with_error!(env, GenericError::MathOverflow))
}

fn calculate_tolerance_range(env: &Env, tolerance: i128) -> (i128, i128) {
    let upper_bound = BPS
        .checked_add(tolerance)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let lower_bound = fp_core::mul_div_half_up(env, BPS, BPS, upper_bound);
    (upper_bound, lower_bound)
}

fn validate_and_calculate_tolerances(
    env: &Env,
    first_tolerance: u32,
    last_tolerance: u32,
) -> OraclePriceFluctuation {
    let first = i128::from(first_tolerance);
    let last = i128::from(last_tolerance);
    if !(MIN_FIRST_TOLERANCE..=MAX_FIRST_TOLERANCE).contains(&first) {
        panic_with_error!(env, OracleError::BadFirstTolerance);
    }
    if !(MIN_LAST_TOLERANCE..=MAX_LAST_TOLERANCE).contains(&last) {
        panic_with_error!(env, OracleError::BadLastTolerance);
    }

    validation::validate_oracle_bounds(env, first, last);

    let (first_upper, first_lower) = calculate_tolerance_range(env, first);
    let (last_upper, last_lower) = calculate_tolerance_range(env, last);

    OraclePriceFluctuation {
        first_upper_ratio_bps: bps_i128_to_u32(env, first_upper),
        first_lower_ratio_bps: bps_i128_to_u32(env, first_lower),
        last_upper_ratio_bps: bps_i128_to_u32(env, last_upper),
        last_lower_ratio_bps: bps_i128_to_u32(env, last_lower),
    }
}

pub fn configure_market_oracle(env: &Env, asset: Address, config: MarketOracleConfigInput) {
    let mut market = match storage::try_get_market_config(env, &asset) {
        Some(m) => m,
        None => panic_with_error!(env, GenericError::AssetNotSupported),
    };

    if !matches!(
        market.status,
        MarketStatus::PendingOracle | MarketStatus::Active | MarketStatus::Disabled
    ) {
        panic_with_error!(env, GenericError::PairNotActive);
    }

    let tolerance = validate_and_calculate_tolerances(
        env,
        config.first_tolerance_bps,
        config.last_tolerance_bps,
    );
    let mut oracle_config = validate_market_oracle_sources(env, &asset, &config, tolerance);
    // Persists config.
    if cfg!(feature = "testing") && market.oracle_config.asset_decimals != 0 {
        oracle_config.asset_decimals = market.oracle_config.asset_decimals;
    }

    market.oracle_config = oracle_config;
    market.status = MarketStatus::Active;
    storage::set_market_config(env, &asset, &market);

    emit_update_asset_oracle(
        env,
        UpdateAssetOracleEvent {
            asset: asset.clone(),
            oracle: EventOracleProvider::from_market(env, &asset, &market),
        },
    );
}

pub fn edit_oracle_tolerance(env: &Env, asset: Address, first_tolerance: u32, last_tolerance: u32) {
    let tolerance = validate_and_calculate_tolerances(env, first_tolerance, last_tolerance);

    let mut market = storage::get_market_config(env, &asset);
    market.oracle_config.tolerance = tolerance;
    storage::set_market_config(env, &asset, &market);

    emit_update_asset_oracle(
        env,
        UpdateAssetOracleEvent {
            asset: asset.clone(),
            oracle: EventOracleProvider::from_market(env, &asset, &market),
        },
    );
}

pub fn disable_token_oracle(env: &Env, asset: Address) {
    let mut market = storage::get_market_config(env, &asset);
    market.status = MarketStatus::Disabled;
    storage::set_market_config(env, &asset, &market);
}

// Helper to remove an E-mode category ID from a market configuration's categories list.
fn remove_emode_category_from_market_config(env: &Env, asset: &Address, category_id: u32) {
    if let Some(mut market) = storage::try_get_market_config(env, asset) {
        if let Some(idx) = market
            .asset_config
            .e_mode_categories
            .iter()
            .position(|id| id == category_id)
        {
            market.asset_config.e_mode_categories.remove(idx as u32);
            storage::set_market_config(env, asset, &market);
        }
    }
}
