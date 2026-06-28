//! Owner- and role-gated configuration for markets, oracles, spokes, caps,
//! aggregator, accumulator, approved tokens, and pool templates.

use crate::events::{
    ApproveBlendPoolEvent, ApproveTokenEvent, CreateHubEvent, EventOracleProvider, EventSpoke,
    OracleDisabledEvent, RemoveSpokeAssetEvent, UpdateAccumulatorEvent, UpdateAggregatorEvent,
    UpdateAssetOracleEvent, UpdateMinBorrowCollateralEvent, UpdatePoolTemplateEvent,
    UpdatePositionLimitsEvent, UpdateSpokeAssetEvent, UpdateSpokeEvent,
};
use common::errors::{CollateralError, EModeError, GenericError, OracleError};

use controller_interface::types::{
    HubAssetKey, HubConfig, MarketOracleConfig, MarketOracleConfigOption, OraclePriceFluctuation,
    OracleSourceConfig, PositionLimits, ReflectorBase, SpokeAssetArgs, SpokeAssetConfig, SpokeConfig,
};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, xdr::ToXdr, Address, BytesN, Env,
};
use stellar_macros::only_owner;

use crate::external::pool::fetch_pool_sync_data;
use crate::helpers::utils::hub0;
use common::math::fp::Ray;

use crate::helpers::emode_caps::{
    validate_spoke_caps_against_hub, validate_spoke_caps_against_usage,
};
use crate::{storage, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[only_owner]
    pub fn set_aggregator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        storage::set_aggregator(&env, &addr);
        UpdateAggregatorEvent { aggregator: addr }.publish(&env);
    }

    #[only_owner]
    pub fn set_accumulator(env: Env, addr: Address) {
        storage::renew_controller_instance(&env);
        storage::set_accumulator(&env, &addr);
        UpdateAccumulatorEvent { accumulator: addr }.publish(&env);
    }

    #[only_owner]
    pub fn set_liquidity_pool_template(env: Env, hash: BytesN<32>) {
        storage::renew_controller_instance(&env);
        storage::set_pool_template(&env, &hash);
        UpdatePoolTemplateEvent { wasm_hash: hash }.publish(&env);
    }

    #[only_owner]
    pub fn set_position_limits(env: Env, limits: PositionLimits) {
        storage::renew_controller_instance(&env);
        storage::set_position_limits(&env, &limits);
        UpdatePositionLimitsEvent {
            max_supply_positions: limits.max_supply_positions,
            max_borrow_positions: limits.max_borrow_positions,
        }
        .publish(&env);
    }

    #[only_owner]
    pub fn set_min_borrow_collateral_usd(env: Env, floor_wad: i128) {
        storage::renew_controller_instance(&env);
        assert_with_error!(env, floor_wad >= 0, CollateralError::InvalidBorrowParams);
        storage::set_min_borrow_collateral_usd_wad(&env, floor_wad);
        UpdateMinBorrowCollateralEvent {
            min_borrow_collateral_usd_wad: floor_wad,
        }
        .publish(&env);
    }

    pub fn get_min_borrow_collateral_usd(env: Env) -> i128 {
        storage::get_min_borrow_collateral_usd_wad(&env)
    }

    #[only_owner]
    pub fn create_hub(env: Env) -> u32 {
        storage::renew_controller_instance(&env);
        create_hub(&env)
    }

    #[only_owner]
    pub fn add_spoke(env: Env) -> u32 {
        storage::renew_controller_instance(&env);
        add_spoke(&env)
    }

    #[only_owner]
    pub fn remove_spoke(env: Env, id: u32) {
        storage::renew_controller_instance(&env);
        remove_spoke(&env, id);
    }

    #[only_owner]
    pub fn add_asset_to_spoke(env: Env, input: SpokeAssetArgs) {
        storage::renew_controller_instance(&env);
        add_asset_to_spoke(&env, &input);
    }

    #[only_owner]
    pub fn edit_asset_in_spoke(env: Env, input: SpokeAssetArgs) {
        storage::renew_controller_instance(&env);
        edit_asset_in_spoke(&env, &input);
    }

    #[only_owner]
    pub fn remove_asset_from_spoke(env: Env, asset: Address, spoke_id: u32) {
        storage::renew_controller_instance(&env);
        remove_asset_from_spoke(&env, asset, spoke_id);
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
        storage::renew_controller_instance(&env);
        storage::set_blend_pool_approved(&env, &pool, true);
        ApproveBlendPoolEvent {
            pool,
            approved: true,
        }
        .publish(&env);
    }

    #[only_owner]
    pub fn revoke_blend_pool(env: Env, pool: Address) {
        storage::renew_controller_instance(&env);
        storage::set_blend_pool_approved(&env, &pool, false);
        ApproveBlendPoolEvent {
            pool,
            approved: false,
        }
        .publish(&env);
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

pub fn create_hub(env: &Env) -> u32 {
    let id = storage::increment_hub_id(env);
    storage::set_hub(env, id, &HubConfig { is_active: true });

    CreateHubEvent { hub_id: id }.publish(env);

    id
}

/// Gates use of a hub. Hub 0 is the implicit default and is always active
/// without a registry read; any higher id must resolve to an active `Hub`
/// entry. Consumed by the (hub, asset) market and position flows in a later
/// phase, hence currently uncalled outside tests.
#[allow(dead_code)] // Wired into market/position flows in a later phase.
pub(crate) fn require_hub_active(env: &Env, hub_id: u32) {
    if hub_id == 0 {
        return;
    }
    let active = storage::get_hub(env, hub_id).is_some_and(|hub| hub.is_active);
    assert_with_error!(env, active, GenericError::HubNotActive);
}

pub fn add_spoke(env: &Env) -> u32 {
    let id = storage::increment_spoke_id(env);
    // The liquidation-curve fields default to zero; they stay inert until a
    // later phase reads them.
    let spoke = SpokeConfig {
        is_deprecated: false,
        liquidation_target_hf_wad: 0,
        hf_for_max_bonus_wad: 0,
        liquidation_bonus_factor_bps: 0,
    };
    storage::set_spoke(env, id, &spoke);

    UpdateSpokeEvent {
        spoke: EventSpoke::new(id, &spoke),
    }
    .publish(env);

    id
}

pub fn remove_spoke(env: &Env, id: u32) {
    let mut spoke = storage::get_spoke(env, id);
    assert_with_error!(env, !spoke.is_deprecated, EModeError::EModeCategoryDeprecated);
    // Deprecation gates every spoke read (overlay, `active_spoke`, asset edits).
    // Discrete `SpokeAsset` keys are not enumerable, so member assets and their
    // market backlinks are left in place; the deprecation flag keeps them
    // unreachable.
    spoke.is_deprecated = true;
    storage::set_spoke(env, id, &spoke);

    UpdateSpokeEvent {
        spoke: EventSpoke::new(id, &spoke),
    }
    .publish(env);
}

pub fn add_asset_to_spoke(env: &Env, args: &SpokeAssetArgs) {
    common::validation::validate_risk_bounds(env, args.ltv, args.threshold, args.bonus);
    assert_with_error!(
        env,
        args.supply_cap >= 0 && args.borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );
    let spoke = storage::get_spoke(env, args.spoke_id);
    assert_with_error!(env, !spoke.is_deprecated, EModeError::EModeCategoryDeprecated);

    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: args.asset.clone(),
    };
    // The asset must be listed (base `SpokeAsset(0)`) before a named spoke lists it.
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, 0, &hub_asset).is_some(),
        GenericError::AssetNotSupported
    );
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_none(),
        EModeError::AssetAlreadyInEmode
    );

    let pool_addr = storage::get_pool(env);
    let hub = fetch_pool_sync_data(env, &pool_addr, &hub_asset);
    validate_spoke_caps_against_hub(
        env,
        hub.params.supply_cap,
        hub.params.borrow_cap,
        args.supply_cap,
        args.borrow_cap,
    );
    // Spoke caps feed the same Ray::from_asset rescale as hub caps; reject any
    // that would overflow it so a misconfig fails here, not at view time.
    common::validation::require_cap_within_asset_domain(
        env,
        args.supply_cap,
        hub.params.asset_decimals,
    );
    common::validation::require_cap_within_asset_domain(
        env,
        args.borrow_cap,
        hub.params.asset_decimals,
    );

    let config = SpokeAssetConfig {
        is_collateralizable: args.can_collateral,
        is_borrowable: args.can_borrow,
        paused: false,
        frozen: false,
        loan_to_value_bps: args.ltv,
        liquidation_threshold_bps: args.threshold,
        liquidation_bonus_bps: args.bonus,
        liquidation_fees_bps: 0,
        supply_cap: args.supply_cap,
        borrow_cap: args.borrow_cap,
        oracle_override: MarketOracleConfigOption::None,
    };
    storage::set_spoke_asset(env, args.spoke_id, &hub_asset, &config);

    UpdateSpokeAssetEvent {
        asset: args.asset.clone(),
        config,
        spoke_id: args.spoke_id,
    }
    .publish(env);
}

pub fn edit_asset_in_spoke(env: &Env, args: &SpokeAssetArgs) {
    common::validation::validate_risk_bounds(env, args.ltv, args.threshold, args.bonus);
    assert_with_error!(
        env,
        args.supply_cap >= 0 && args.borrow_cap >= 0,
        CollateralError::InvalidBorrowParams
    );
    let spoke = storage::get_spoke(env, args.spoke_id);
    assert_with_error!(env, !spoke.is_deprecated, EModeError::EModeCategoryDeprecated);
    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: args.asset.clone(),
    };
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_some(),
        EModeError::AssetNotInEmode
    );

    let pool_addr = storage::get_pool(env);
    let hub = fetch_pool_sync_data(env, &pool_addr, &hub_asset);
    validate_spoke_caps_against_hub(
        env,
        hub.params.supply_cap,
        hub.params.borrow_cap,
        args.supply_cap,
        args.borrow_cap,
    );
    // Spoke caps feed the same Ray::from_asset rescale as hub caps; reject any
    // that would overflow it so a misconfig fails here, not at view time.
    common::validation::require_cap_within_asset_domain(
        env,
        args.supply_cap,
        hub.params.asset_decimals,
    );
    common::validation::require_cap_within_asset_domain(
        env,
        args.borrow_cap,
        hub.params.asset_decimals,
    );
    let usage = storage::get_spoke_usage(env, args.spoke_id, &hub_asset).unwrap_or_default();
    validate_spoke_caps_against_usage(
        env,
        &usage,
        args.supply_cap,
        args.borrow_cap,
        Ray::from(hub.state.supply_index_ray),
        Ray::from(hub.state.borrow_index_ray),
        hub.params.asset_decimals,
    );

    let config = SpokeAssetConfig {
        is_collateralizable: args.can_collateral,
        is_borrowable: args.can_borrow,
        paused: false,
        frozen: false,
        loan_to_value_bps: args.ltv,
        liquidation_threshold_bps: args.threshold,
        liquidation_bonus_bps: args.bonus,
        liquidation_fees_bps: 0,
        supply_cap: args.supply_cap,
        borrow_cap: args.borrow_cap,
        oracle_override: MarketOracleConfigOption::None,
    };
    storage::set_spoke_asset(env, args.spoke_id, &hub_asset, &config);

    UpdateSpokeAssetEvent {
        asset: args.asset.clone(),
        config,
        spoke_id: args.spoke_id,
    }
    .publish(env);
}

pub fn remove_asset_from_spoke(env: &Env, asset: Address, spoke_id: u32) {
    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, spoke_id, &hub_asset).is_some(),
        EModeError::AssetNotInEmode
    );

    storage::remove_spoke_asset(env, spoke_id, &hub_asset);

    RemoveSpokeAssetEvent { asset, spoke_id }.publish(env);
}

/// Activates a listed asset by writing its token-rooted `AssetOracle` entry.
/// Re-running it replaces the oracle config; presence of the entry is the
/// "active" signal that price resolution and `require_market_active` read.
pub fn set_market_oracle_config(env: &Env, asset: Address, mut config: MarketOracleConfig) {
    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };
    // Only a listed asset (base `SpokeAsset(0)`) can be activated.
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, 0, &hub_asset).is_some(),
        GenericError::AssetNotSupported
    );

    // Re-validate the sanity band at the controller boundary. Governance
    // validates the proposal; execution rejects unset or invalid bands before
    // activation.
    common::validation::validate_sanity_bounds(
        env,
        config.min_sanity_price_wad,
        config.max_sanity_price_wad,
    );

    // Re-assert quote-market USD/active invariant at execution time.
    // Timelock delay can make the proposed quote market stale.
    require_quote_markets_active_usd(env, &asset, &config);

    // Test markets register pools with preset decimals that may diverge from
    // the live token probe; keep the pool-registered value authoritative.
    if cfg!(feature = "testing") {
        let pool_addr = storage::get_pool(env);
        let hub_asset = hub0(&asset);
        let pool_decimals = fetch_pool_sync_data(env, &pool_addr, &hub_asset).params.asset_decimals;
        if pool_decimals != 0 {
            config.asset_decimals = pool_decimals;
        }
    }

    storage::set_asset_oracle(env, &asset, &config);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_oracle(env, &asset, &config),
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

    // The quote must be active: a token-rooted `AssetOracle` entry must exist.
    let quote_oracle = match storage::get_asset_oracle(env, quote) {
        Some(oracle) => oracle,
        None => panic_with_error!(env, OracleError::InvalidOracleBase),
    };

    // The quote's primary must itself be USD-based: keeps the conversion exactly
    // one hop, forbidding a quote chain.
    match &quote_oracle.primary {
        OracleSourceConfig::RedStone(_) => {}
        OracleSourceConfig::Reflector(quote_primary) => assert_with_error!(
            env,
            matches!(quote_primary.base, ReflectorBase::Usd),
            OracleError::InvalidOracleBase
        ),
    }
}

pub fn set_oracle_tolerance(env: &Env, asset: Address, tolerance: OraclePriceFluctuation) {
    let mut oracle = storage::get_asset_oracle(env, &asset)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PairNotActive));
    oracle.tolerance = tolerance;
    storage::set_asset_oracle(env, &asset, &oracle);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_oracle(env, &asset, &oracle),
    }
    .publish(env);
}

/// Disables an active asset by removing its `AssetOracle` entry. Absence is the
/// disabled signal: price resolution then reverts for the asset.
pub fn disable_token_oracle(env: &Env, asset: Address) {
    assert_with_error!(
        env,
        storage::get_asset_oracle(env, &asset).is_some(),
        GenericError::PairNotActive
    );
    storage::remove_asset_oracle(env, &asset);
    OracleDisabledEvent { asset }.publish(env);
}

#[cfg(test)]
#[path = "../../tests/governance/config.rs"]
mod tests;
