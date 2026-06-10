//! Owner- and role-gated configuration for markets, oracles, e-mode, caps,
//! aggregator, accumulator, approved tokens, and pool templates.

use common::errors::{EModeError, GenericError, OracleError};
use common::events::{
    ApproveTokenEvent, EventEModeCategory, EventOracleProvider, OracleDisabledEvent,
    RemoveEModeAssetEvent, UpdateAccumulatorEvent, UpdateAggregatorEvent, UpdateAssetConfigEvent,
    UpdateAssetOracleEvent, UpdateEModeAssetEvent, UpdateEModeCategoryEvent,
    UpdatePoolTemplateEvent, UpdatePositionLimitsEvent,
};

use common::types::{
    AssetConfigRaw, EModeAssetConfig, EModeCategoryRaw, MarketOracleConfigInput, MarketStatus,
    PositionLimits,
};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, xdr::ToXdr, Address, BytesN, Env, Executable,
};
use stellar_macros::{only_owner, only_role};

use crate::oracle::tolerance::validate_and_calculate_tolerances;
use crate::oracle::validation::validate_market_oracle_sources;

use crate::{storage, validation, Controller, ControllerArgs, ControllerClient};

/// Max supply/borrow positions configurable per account.
const POSITION_LIMIT_MAX: u32 = 10;

#[contractimpl]
impl Controller {
    #[only_owner]
    pub fn set_aggregator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        set_aggregator(&env, addr);
    }

    #[only_owner]
    pub fn set_accumulator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        set_accumulator(&env, addr);
    }

    #[only_owner]
    pub fn set_liquidity_pool_template(env: Env, hash: BytesN<32>) {
        storage::renew_controller_instance(&env);
        set_liquidity_pool_template(&env, hash);
    }

    #[only_owner]
    pub fn edit_asset_config(env: Env, asset: Address, cfg: AssetConfigRaw) {
        storage::renew_controller_instance(&env);
        edit_asset_config(&env, asset, cfg);
    }

    #[only_owner]
    pub fn set_position_limits(env: Env, limits: PositionLimits) {
        storage::renew_controller_instance(&env);
        set_position_limits(&env, limits);
    }

    #[only_owner]
    pub fn add_e_mode_category(env: Env, ltv: u32, threshold: u32, bonus: u32) -> u32 {
        storage::renew_controller_instance(&env);
        add_e_mode_category(&env, ltv, threshold, bonus)
    }

    #[only_owner]
    pub fn edit_e_mode_category(env: Env, id: u32, ltv: u32, threshold: u32, bonus: u32) {
        storage::renew_controller_instance(&env);
        edit_e_mode_category(&env, id, ltv, threshold, bonus);
    }

    #[only_owner]
    pub fn remove_e_mode_category(env: Env, id: u32) {
        storage::renew_controller_instance(&env);
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
        storage::renew_controller_instance(&env);
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
        storage::renew_controller_instance(&env);
        edit_asset_in_e_mode_category(&env, asset, category_id, can_collateral, can_borrow);
    }

    #[only_owner]
    pub fn remove_asset_from_e_mode(env: Env, asset: Address, category_id: u32) {
        storage::renew_controller_instance(&env);
        remove_asset_from_e_mode(&env, asset, category_id);
    }

    #[only_owner]
    pub fn approve_token(env: Env, token: Address) {
        set_token_approval(&env, token, true);
    }

    #[only_owner]
    pub fn revoke_token(env: Env, token: Address) {
        set_token_approval(&env, token, false);
    }

    #[only_role(caller, "ORACLE")]
    pub fn configure_market_oracle(
        env: Env,
        caller: Address,
        asset: Address,
        cfg: MarketOracleConfigInput,
    ) {
        storage::renew_controller_instance(&env);
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
        storage::renew_controller_instance(&env);
        edit_oracle_tolerance(&env, asset, first_tolerance, last_tolerance);
    }

    #[only_role(caller, "ORACLE")]
    pub fn disable_token_oracle(env: Env, caller: Address, asset: Address) {
        storage::renew_controller_instance(&env);
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
    assert_with_error!(
        env,
        hash.to_array() != [0; 32],
        GenericError::InvalidPoolTemplate
    );
}

fn set_token_approval(env: &Env, token: Address, approved: bool) {
    storage::renew_controller_instance(env);
    storage::set_token_approved(env, &token, approved);
    let wasm_hash = env.crypto().keccak256(&token.to_xdr(env)).into();
    ApproveTokenEvent {
        wasm_hash,
        approved,
    }
    .publish(env);
}

pub fn set_aggregator(env: &Env, addr: Address) {
    require_contract_address(env, &addr, OracleError::InvalidAggregator);
    storage::set_aggregator(env, &addr);
    UpdateAggregatorEvent { aggregator: addr }.publish(env);
}

pub fn set_accumulator(env: &Env, addr: Address) {
    require_contract_address(env, &addr, GenericError::NotSmartContract);
    storage::set_accumulator(env, &addr);
    UpdateAccumulatorEvent { accumulator: addr }.publish(env);
}

pub fn set_liquidity_pool_template(env: &Env, hash: BytesN<32>) {
    require_nonzero_wasm_hash(env, &hash);
    storage::set_pool_template(env, &hash);
    UpdatePoolTemplateEvent { wasm_hash: hash }.publish(env);
}

pub fn edit_asset_config(env: &Env, asset: Address, mut next_config: AssetConfigRaw) {
    validation::validate_asset_config(env, &next_config);

    let mut market = storage::get_market_config(env, &asset);
    next_config.e_mode_categories = market.asset_config.e_mode_categories.clone();
    market.asset_config = next_config.clone();
    storage::set_market_config(env, &asset, &market);

    UpdateAssetConfigEvent {
        asset,
        config: next_config,
    }
    .publish(env);
}

pub fn set_position_limits(env: &Env, limits: PositionLimits) {
    if limits.max_supply_positions == 0
        || limits.max_borrow_positions == 0
        || limits.max_supply_positions > POSITION_LIMIT_MAX
        || limits.max_borrow_positions > POSITION_LIMIT_MAX
    {
        panic_with_error!(env, GenericError::InvalidPositionLimits);
    }
    storage::set_position_limits(env, &limits);
    UpdatePositionLimitsEvent {
        max_supply_positions: limits.max_supply_positions,
        max_borrow_positions: limits.max_borrow_positions,
    }
    .publish(env);
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

    UpdateEModeCategoryEvent {
        category: EventEModeCategory::new(id, &cat),
    }
    .publish(env);

    id
}

pub fn edit_e_mode_category(env: &Env, id: u32, ltv: u32, threshold: u32, bonus: u32) {
    validation::validate_risk_bounds(env, ltv, threshold, bonus);
    let mut cat = storage::get_emode_category(env, id);
    assert_with_error!(env, !cat.is_deprecated, EModeError::EModeCategoryDeprecated);
    cat.loan_to_value_bps = ltv;
    cat.liquidation_threshold_bps = threshold;
    cat.liquidation_bonus_bps = bonus;
    storage::set_emode_category(env, id, &cat);

    UpdateEModeCategoryEvent {
        category: EventEModeCategory::new(id, &cat),
    }
    .publish(env);
}

pub fn remove_e_mode_category(env: &Env, id: u32) {
    let mut cat = storage::get_emode_category(env, id);
    assert_with_error!(env, !cat.is_deprecated, EModeError::EModeCategoryDeprecated);
    cat.is_deprecated = true;

    let members = cat.assets.clone();
    cat.assets = soroban_sdk::Map::new(env);
    storage::set_emode_category(env, id, &cat);

    for asset in members.keys() {
        remove_emode_category_from_market_config(env, &asset, id);
    }

    UpdateEModeCategoryEvent {
        category: EventEModeCategory::new(id, &cat),
    }
    .publish(env);
}

pub fn add_asset_to_e_mode_category(
    env: &Env,
    asset: Address,
    category_id: u32,
    can_collateral: bool,
    can_borrow: bool,
) {
    let cat = storage::get_emode_category(env, category_id);
    assert_with_error!(env, !cat.is_deprecated, EModeError::EModeCategoryDeprecated);

    let mut market = storage::get_market_config(env, &asset);

    assert_with_error!(
        env,
        !cat.assets.contains_key(asset.clone()),
        EModeError::AssetAlreadyInEmode
    );

    let config = EModeAssetConfig {
        is_collateralizable: can_collateral,
        is_borrowable: can_borrow,
    };
    storage::set_emode_asset(env, category_id, &asset, &config);

    if !market.asset_config.e_mode_categories.contains(category_id) {
        market.asset_config.e_mode_categories.push_back(category_id);
        storage::set_market_config(env, &asset, &market);
    }

    UpdateEModeAssetEvent {
        asset,
        config,
        category_id,
    }
    .publish(env);
}

pub fn edit_asset_in_e_mode_category(
    env: &Env,
    asset: Address,
    category_id: u32,
    can_collateral: bool,
    can_borrow: bool,
) {
    let cat = storage::get_emode_category(env, category_id);
    assert_with_error!(env, !cat.is_deprecated, EModeError::EModeCategoryDeprecated);
    assert_with_error!(
        env,
        cat.assets.contains_key(asset.clone()),
        EModeError::AssetNotInEmode
    );

    let config = EModeAssetConfig {
        is_collateralizable: can_collateral,
        is_borrowable: can_borrow,
    };
    storage::set_emode_asset(env, category_id, &asset, &config);

    UpdateEModeAssetEvent {
        asset,
        config,
        category_id,
    }
    .publish(env);
}

pub fn remove_asset_from_e_mode(env: &Env, asset: Address, category_id: u32) {
    assert_with_error!(
        env,
        storage::get_emode_asset(env, category_id, &asset).is_some(),
        EModeError::AssetNotInEmode
    );

    storage::remove_emode_asset(env, category_id, &asset);

    remove_emode_category_from_market_config(env, &asset, category_id);

    RemoveEModeAssetEvent { asset, category_id }.publish(env);
}

pub fn configure_market_oracle(env: &Env, asset: Address, config: MarketOracleConfigInput) {
    let mut market = storage::get_market_config(env, &asset);

    assert_with_error!(
        env,
        matches!(
            market.status,
            MarketStatus::PendingOracle | MarketStatus::Active | MarketStatus::Disabled
        ),
        GenericError::PairNotActive
    );

    let tolerance = validate_and_calculate_tolerances(
        env,
        config.first_tolerance_bps,
        config.last_tolerance_bps,
    );
    let mut oracle_config = validate_market_oracle_sources(env, &asset, &config, tolerance);
    if cfg!(feature = "testing") && market.oracle_config.asset_decimals != 0 {
        oracle_config.asset_decimals = market.oracle_config.asset_decimals;
    }

    market.oracle_config = oracle_config;
    market.status = MarketStatus::Active;
    storage::set_market_config(env, &asset, &market);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_market(env, &asset, &market),
    }
    .publish(env);
}

pub fn edit_oracle_tolerance(env: &Env, asset: Address, first_tolerance: u32, last_tolerance: u32) {
    let tolerance = validate_and_calculate_tolerances(env, first_tolerance, last_tolerance);

    let mut market = storage::get_market_config(env, &asset);
    market.oracle_config.tolerance = tolerance;
    storage::set_market_config(env, &asset, &market);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_market(env, &asset, &market),
    }
    .publish(env);
}

pub fn disable_token_oracle(env: &Env, asset: Address) {
    let mut market = storage::get_market_config(env, &asset);
    assert_with_error!(
        env,
        matches!(market.status, MarketStatus::Active),
        GenericError::PairNotActive
    );
    market.status = MarketStatus::Disabled;
    storage::set_market_config(env, &asset, &market);
    OracleDisabledEvent { asset }.publish(env);
}

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
