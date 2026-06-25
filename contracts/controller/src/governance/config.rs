//! Owner- and role-gated configuration for markets, oracles, e-mode, caps,
//! aggregator, accumulator, approved tokens, and pool templates.

use crate::events::{
    ApproveBlendPoolEvent, ApproveTokenEvent, EventEModeCategory, EventOracleProvider,
    OracleDisabledEvent, RemoveEModeAssetEvent, UpdateAccumulatorEvent, UpdateAggregatorEvent,
    UpdateAssetConfigEvent, UpdateAssetOracleEvent, UpdateEModeAssetEvent,
    UpdateEModeCategoryEvent, UpdateMinBorrowCollateralEvent, UpdatePoolTemplateEvent,
    UpdatePositionLimitsEvent,
};
use common::errors::{CollateralError, EModeError, GenericError, OracleError};

use controller_interface::types::{
    AssetConfigRaw, EModeAssetArgs, EModeAssetConfig, EModeCategoryRaw, EModeSpokeUsageRaw,
    MarketOracleConfig, MarketStatus, OraclePriceFluctuation, OracleSourceConfig, PositionLimits,
    ReflectorBase,
};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, xdr::ToXdr, Address, BytesN, Env,
};
use stellar_macros::only_owner;

use crate::external::pool::fetch_pool_sync_data;
use common::math::fp::Ray;

use crate::helpers::emode_caps::{
    empty_usage_map, validate_spoke_caps_against_hub, validate_spoke_caps_against_usage,
};
use crate::{storage, Controller, ControllerArgs, ControllerClient};

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
    pub fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128) {
        storage::renew_controller_instance(&env);
        set_min_borrow_collateral_usd(&env, floor_wad);
    }

    pub fn get_min_borrow_collateral_usd(env: Env) -> i128 {
        storage::get_min_borrow_collateral_usd_wad(&env)
    }

    #[only_owner]
    pub fn add_e_mode_category(env: Env) -> u32 {
        storage::renew_controller_instance(&env);
        add_e_mode_category(&env)
    }

    #[only_owner]
    pub fn remove_e_mode_category(env: Env, id: u32) {
        storage::renew_controller_instance(&env);
        remove_e_mode_category(&env, id);
    }

    #[only_owner]
    pub fn add_asset_to_e_mode_category(env: Env, input: EModeAssetArgs) {
        storage::renew_controller_instance(&env);
        add_asset_to_e_mode_category(
            &env,
            input.asset,
            input.category_id,
            input.can_collateral,
            input.can_borrow,
            input.ltv,
            input.threshold,
            input.bonus,
            input.supply_cap,
            input.borrow_cap,
        );
    }

    #[only_owner]
    pub fn edit_asset_in_e_mode_category(env: Env, input: EModeAssetArgs) {
        storage::renew_controller_instance(&env);
        edit_asset_in_e_mode_category(
            &env,
            input.asset,
            input.category_id,
            input.can_collateral,
            input.can_borrow,
            input.ltv,
            input.threshold,
            input.bonus,
            input.supply_cap,
            input.borrow_cap,
        );
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

    /// View: whether `pool` is on the Blend-pool allow-list (migration source).
    pub fn is_blend_pool_approved(env: Env, pool: Address) -> bool {
        storage::is_blend_pool_approved(&env, &pool)
    }

    #[only_owner]
    pub fn approve_blend_pool(env: Env, pool: Address) {
        set_blend_pool_approval(&env, pool, true);
    }

    #[only_owner]
    pub fn revoke_blend_pool(env: Env, pool: Address) {
        set_blend_pool_approval(&env, pool, false);
    }

    #[only_owner]
    pub fn set_market_oracle_config(env: Env, asset: Address, config: MarketOracleConfig) {
        storage::renew_controller_instance(&env);
        set_market_oracle_config(&env, asset, config);
    }

    #[only_owner]
    pub fn set_oracle_tolerance(env: Env, asset: Address, tolerance: OraclePriceFluctuation) {
        storage::renew_controller_instance(&env);
        set_oracle_tolerance(&env, asset, tolerance);
    }

    #[only_owner]
    pub fn disable_token_oracle(env: Env, asset: Address) {
        storage::renew_controller_instance(&env);
        disable_token_oracle(&env, asset);
    }
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

fn set_blend_pool_approval(env: &Env, pool: Address, approved: bool) {
    storage::renew_controller_instance(env);
    storage::set_blend_pool_approved(env, &pool, approved);
    ApproveBlendPoolEvent { pool, approved }.publish(env);
}

pub fn set_aggregator(env: &Env, addr: Address) {
    storage::set_aggregator(env, &addr);
    UpdateAggregatorEvent { aggregator: addr }.publish(env);
}

pub fn set_accumulator(env: &Env, addr: Address) {
    storage::set_accumulator(env, &addr);
    UpdateAccumulatorEvent { accumulator: addr }.publish(env);
}

pub fn set_liquidity_pool_template(env: &Env, hash: BytesN<32>) {
    storage::set_pool_template(env, &hash);
    UpdatePoolTemplateEvent { wasm_hash: hash }.publish(env);
}

pub fn edit_asset_config(env: &Env, asset: Address, mut next_config: AssetConfigRaw) {
    common::validation::validate_risk_bounds(
        env,
        next_config.loan_to_value_bps,
        next_config.liquidation_threshold_bps,
        next_config.liquidation_bonus_bps,
    );
    let mut market = storage::get_market_config(env, &asset);
    next_config.e_mode_categories = market.asset_config.e_mode_categories.clone();
    next_config.asset_decimals = market.asset_config.asset_decimals;
    market.asset_config = next_config.clone();
    storage::set_market_config(env, &asset, &market);

    UpdateAssetConfigEvent {
        asset,
        config: next_config,
    }
    .publish(env);
}

pub fn set_position_limits(env: &Env, limits: PositionLimits) {
    storage::set_position_limits(env, &limits);
    UpdatePositionLimitsEvent {
        max_supply_positions: limits.max_supply_positions,
        max_borrow_positions: limits.max_borrow_positions,
    }
    .publish(env);
}

pub fn set_min_borrow_collateral_usd(env: &Env, floor_wad: i128) {
    assert_with_error!(env, floor_wad >= 0, CollateralError::InvalidBorrowParams);
    storage::set_min_borrow_collateral_usd_wad(env, floor_wad);
    UpdateMinBorrowCollateralEvent {
        min_borrow_collateral_usd_wad: floor_wad,
    }
    .publish(env);
}

pub fn add_e_mode_category(env: &Env) -> u32 {
    let id = storage::increment_emode_category_id(env);
    let cat = EModeCategoryRaw {
        is_deprecated: false,
        assets: soroban_sdk::Map::new(env),
        usage: empty_usage_map(env),
    };
    storage::set_emode_category(env, id, &cat);

    UpdateEModeCategoryEvent {
        category: EventEModeCategory::new(id, &cat),
    }
    .publish(env);

    id
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

#[allow(clippy::too_many_arguments)]
pub fn add_asset_to_e_mode_category(
    env: &Env,
    asset: Address,
    category_id: u32,
    can_collateral: bool,
    can_borrow: bool,
    ltv: u32,
    threshold: u32,
    bonus: u32,
    supply_cap: i128,
    borrow_cap: i128,
) {
    common::validation::validate_risk_bounds(env, ltv, threshold, bonus);
    assert_with_error!(
        env,
        supply_cap >= 0 && borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );
    let cat = storage::get_emode_category(env, category_id);
    assert_with_error!(env, !cat.is_deprecated, EModeError::EModeCategoryDeprecated);

    let mut market = storage::get_market_config(env, &asset);

    assert_with_error!(
        env,
        !cat.assets.contains_key(asset.clone()),
        EModeError::AssetAlreadyInEmode
    );

    let pool_addr = storage::get_pool(env);
    let hub = fetch_pool_sync_data(env, &pool_addr, &asset);
    validate_spoke_caps_against_hub(
        env,
        hub.params.supply_cap,
        hub.params.borrow_cap,
        supply_cap,
        borrow_cap,
    );
    // Spoke caps feed the same Ray::from_asset rescale as hub caps; reject any
    // that would overflow it so a misconfig fails here, not at view time.
    common::validation::require_cap_within_asset_domain(env, supply_cap, hub.params.asset_decimals);
    common::validation::require_cap_within_asset_domain(env, borrow_cap, hub.params.asset_decimals);

    let config = EModeAssetConfig {
        is_collateralizable: can_collateral,
        is_borrowable: can_borrow,
        loan_to_value_bps: ltv,
        liquidation_threshold_bps: threshold,
        liquidation_bonus_bps: bonus,
        supply_cap,
        borrow_cap,
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

#[allow(clippy::too_many_arguments)]
pub fn edit_asset_in_e_mode_category(
    env: &Env,
    asset: Address,
    category_id: u32,
    can_collateral: bool,
    can_borrow: bool,
    ltv: u32,
    threshold: u32,
    bonus: u32,
    supply_cap: i128,
    borrow_cap: i128,
) {
    common::validation::validate_risk_bounds(env, ltv, threshold, bonus);
    assert_with_error!(
        env,
        supply_cap >= 0 && borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );
    let cat = storage::get_emode_category(env, category_id);
    assert_with_error!(env, !cat.is_deprecated, EModeError::EModeCategoryDeprecated);
    assert_with_error!(
        env,
        cat.assets.contains_key(asset.clone()),
        EModeError::AssetNotInEmode
    );

    let pool_addr = storage::get_pool(env);
    let hub = fetch_pool_sync_data(env, &pool_addr, &asset);
    validate_spoke_caps_against_hub(
        env,
        hub.params.supply_cap,
        hub.params.borrow_cap,
        supply_cap,
        borrow_cap,
    );
    // Spoke caps feed the same Ray::from_asset rescale as hub caps; reject any
    // that would overflow it so a misconfig fails here, not at view time.
    common::validation::require_cap_within_asset_domain(env, supply_cap, hub.params.asset_decimals);
    common::validation::require_cap_within_asset_domain(env, borrow_cap, hub.params.asset_decimals);
    let usage = cat
        .usage
        .get(asset.clone())
        .unwrap_or(EModeSpokeUsageRaw {
            supplied_scaled_ray: 0,
            borrowed_scaled_ray: 0,
        });
    validate_spoke_caps_against_usage(
        env,
        &usage,
        supply_cap,
        borrow_cap,
        Ray::from(hub.state.supply_index_ray),
        Ray::from(hub.state.borrow_index_ray),
        hub.params.asset_decimals,
    );

    let config = EModeAssetConfig {
        is_collateralizable: can_collateral,
        is_borrowable: can_borrow,
        loan_to_value_bps: ltv,
        liquidation_threshold_bps: threshold,
        liquidation_bonus_bps: bonus,
        supply_cap,
        borrow_cap,
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

pub fn set_market_oracle_config(env: &Env, asset: Address, mut config: MarketOracleConfig) {
    let mut market = storage::get_market_config(env, &asset);

    assert_with_error!(
        env,
        matches!(
            market.status,
            MarketStatus::PendingOracle | MarketStatus::Active | MarketStatus::Disabled
        ),
        GenericError::PairNotActive
    );

    // Re-validate the sanity band at the controller boundary. Governance
    // validates the proposal; execution rejects unset or invalid bands before
    // activation.
    common::validation::validate_sanity_bounds(
        env,
        config.min_sanity_price_wad,
        config.max_sanity_price_wad,
    );

    // Re-assert quote-market USD/Active invariant at execution time.
    // Timelock delay can make the proposed quote market stale.
    require_quote_markets_active_usd(env, &asset, &config);

    // Test markets register pools with preset decimals that may diverge from
    // the live token probe; keep the registered value authoritative.
    if cfg!(feature = "testing") && market.oracle_config.asset_decimals != 0 {
        config.asset_decimals = market.oracle_config.asset_decimals;
    }

    market.oracle_config = config;
    market.status = MarketStatus::Active;
    storage::set_market_config(env, &asset, &market);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_market(env, &asset, &market),
    }
    .publish(env);
}

/// Checks that quote sources point at active USD-based markets.
/// Direct USD and Other sources pass without lookup.
fn require_quote_markets_active_usd(env: &Env, asset: &Address, config: &MarketOracleConfig) {
    require_source_quote_active_usd(env, asset, &config.primary);
    if let Some(anchor) = config.anchor.as_ref() {
        require_source_quote_active_usd(env, asset, anchor);
    }
}

fn require_source_quote_active_usd(env: &Env, asset: &Address, source: &OracleSourceConfig) {
    let OracleSourceConfig::Reflector(reflector) = source else {
        return;
    };
    let ReflectorBase::Quoted(quote) = &reflector.base else {
        return;
    };

    // A market quoted in itself would chain forever at read time; reject it here.
    assert_with_error!(env, quote != asset, OracleError::InvalidOracleBase);

    let quote_market = match storage::try_get_market_config(env, quote) {
        Some(market) => market,
        None => panic_with_error!(env, OracleError::InvalidOracleBase),
    };
    assert_with_error!(
        env,
        matches!(quote_market.status, MarketStatus::Active),
        OracleError::InvalidOracleBase
    );

    // The quote market's primary must itself be USD-based: keeps the conversion
    // exactly one hop, forbidding a quote chain.
    match &quote_market.oracle_config.primary {
        OracleSourceConfig::RedStone(_) => {}
        OracleSourceConfig::Reflector(quote_primary) => assert_with_error!(
            env,
            matches!(quote_primary.base, ReflectorBase::Usd),
            OracleError::InvalidOracleBase
        ),
    }
}

pub fn set_oracle_tolerance(env: &Env, asset: Address, tolerance: OraclePriceFluctuation) {
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
