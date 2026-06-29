//! Owner- and role-gated configuration for markets, oracles, spokes, caps,
//! aggregator, accumulator, approved tokens, and pool templates.

use crate::events::{
    ApproveBlendPoolEvent, ApproveTokenEvent, CreateHubEvent, EventOracleProvider, EventSpoke,
    OracleDisabledEvent, RemoveSpokeAssetEvent, UpdateAccumulatorEvent, UpdateAggregatorEvent,
    UpdateAssetOracleEvent, UpdateMinBorrowCollateralEvent, UpdatePoolTemplateEvent,
    UpdatePositionLimitsEvent, UpdateSpokeAssetEvent, UpdateSpokeEvent,
};
use common::errors::{CollateralError, SpokeError, GenericError, OracleError};

use controller_interface::types::{
    HubAssetKey, HubConfig, MarketOracleConfig, MarketOracleConfigOption, OraclePriceFluctuation,
    OracleSourceConfig, PositionLimits, PositionManagerConfig, ReflectorBase, SpokeAssetArgs,
    SpokeAssetConfig, SpokeConfig,
};
use soroban_sdk::{
    assert_with_error, contractimpl, panic_with_error, xdr::ToXdr, Address, BytesN, Env,
};
use stellar_macros::only_owner;

use crate::external::pool::fetch_pool_sync_data;
use common::math::fp::Ray;

use crate::helpers::spoke_caps::{
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
    pub fn remove_asset_from_spoke(env: Env, hub_asset: HubAssetKey, spoke_id: u32) {
        storage::renew_controller_instance(&env);
        remove_asset_from_spoke(&env, hub_asset, spoke_id);
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
    pub fn set_market_oracle_config(env: Env, hub_asset: HubAssetKey, config: MarketOracleConfig) {
        storage::renew_controller_instance(&env);
        set_market_oracle_config(&env, hub_asset, config);
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

    /// Registers (or updates) a position manager. Owners opt in per account via
    /// `add_delegate`; a manager only acts on an account while active and listed
    /// among that account's delegates.
    #[only_owner]
    pub fn set_position_manager(env: Env, manager: Address, is_active: bool) {
        storage::renew_controller_instance(&env);
        storage::set_position_manager(&env, &manager, &PositionManagerConfig { is_active });
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

/// Gates use of a hub. Every hub must resolve to an active `Hub` registry entry;
/// an unknown or deactivated hub reverts `HubNotActive`. No hub is seeded, so
/// hub 0 (and any uncreated id) reverts — there is no implicit default hub.
/// Wired into market creation (`create_liquidity_pool`) and the supply/borrow
/// validate paths via `validation::require_hub_active`.
pub(crate) fn require_hub_active(env: &Env, hub_id: u32) {
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
    assert_with_error!(env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
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
    assert_with_error!(env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);

    let hub_asset = HubAssetKey {
        hub_id: args.hub_id,
        asset: args.asset.clone(),
    };
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_none(),
        SpokeError::AssetAlreadyInSpoke
    );

    // The pool owns the market record; `fetch_pool_sync_data` reverts
    // `PoolNotInitialized` when the (hub, asset) market was never created, so the
    // asset must already have a created market before a spoke lists it.
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
        loan_to_value: args.ltv,
        liquidation_threshold: args.threshold,
        liquidation_bonus: args.bonus,
        liquidation_fees: args.liquidation_fees,
        supply_cap: args.supply_cap,
        borrow_cap: args.borrow_cap,
        oracle_override: resolve_spoke_oracle_override(
            env,
            &args.asset,
            hub.params.asset_decimals,
            &args.oracle_override,
        ),
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
    assert_with_error!(env, !spoke.is_deprecated, SpokeError::SpokeDeprecated);
    let hub_asset = HubAssetKey {
        hub_id: args.hub_id,
        asset: args.asset.clone(),
    };
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, args.spoke_id, &hub_asset).is_some(),
        SpokeError::AssetNotInSpoke
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
        Ray::from(hub.state.supply_index),
        Ray::from(hub.state.borrow_index),
        hub.params.asset_decimals,
    );

    let config = SpokeAssetConfig {
        is_collateralizable: args.can_collateral,
        is_borrowable: args.can_borrow,
        paused: false,
        frozen: false,
        loan_to_value: args.ltv,
        liquidation_threshold: args.threshold,
        liquidation_bonus: args.bonus,
        liquidation_fees: args.liquidation_fees,
        supply_cap: args.supply_cap,
        borrow_cap: args.borrow_cap,
        oracle_override: resolve_spoke_oracle_override(
            env,
            &args.asset,
            hub.params.asset_decimals,
            &args.oracle_override,
        ),
    };
    storage::set_spoke_asset(env, args.spoke_id, &hub_asset, &config);

    UpdateSpokeAssetEvent {
        asset: args.asset.clone(),
        config,
        spoke_id: args.spoke_id,
    }
    .publish(env);
}

/// Validates a per-spoke `oracle_override`. `None` passes through; `Some(cfg)`
/// is validated exactly like the token-rooted `set_market_oracle_config` base
/// (sanity band + quote markets active and USD-based), and the pool-registered
/// decimals override the input under `testing` for parity with test markets.
fn resolve_spoke_oracle_override(
    env: &Env,
    asset: &Address,
    pool_decimals: u32,
    input: &MarketOracleConfigOption,
) -> MarketOracleConfigOption {
    match input {
        MarketOracleConfigOption::None => MarketOracleConfigOption::None,
        MarketOracleConfigOption::Some(cfg) => {
            let mut cfg = cfg.clone();
            validate_market_oracle_config(env, asset, &cfg);
            if cfg!(feature = "testing") && pool_decimals != 0 {
                cfg.asset_decimals = pool_decimals;
            }
            MarketOracleConfigOption::Some(cfg)
        }
    }
}

pub fn remove_asset_from_spoke(env: &Env, hub_asset: HubAssetKey, spoke_id: u32) {
    assert_with_error!(
        env,
        storage::get_spoke_asset(env, spoke_id, &hub_asset).is_some(),
        SpokeError::AssetNotInSpoke
    );

    storage::remove_spoke_asset(env, spoke_id, &hub_asset);

    RemoveSpokeAssetEvent {
        asset: hub_asset.asset,
        spoke_id,
    }
    .publish(env);
}

/// Activates a listed asset by writing its token-rooted `AssetOracle` entry.
/// Re-running it replaces the oracle config; presence of the entry is the
/// "active" signal that price resolution and `require_market_active` read.
pub fn set_market_oracle_config(env: &Env, hub_asset: HubAssetKey, mut config: MarketOracleConfig) {
    let asset = &hub_asset.asset;
    // Only an existing `(hub, asset)` market can be activated. The pool owns the
    // market record; `fetch_pool_sync_data` reverts `PoolNotInitialized` when the
    // market was never created.
    let pool_addr = storage::get_pool(env);
    let pool_decimals = fetch_pool_sync_data(env, &pool_addr, &hub_asset).params.asset_decimals;

    // Re-validate the sanity band and quote-market USD/active invariant at the
    // controller boundary. Governance validates the proposal; execution rejects
    // unset or invalid bands, and timelock delay can make a quote market stale.
    validate_market_oracle_config(env, asset, &config);

    // Test markets register pools with preset decimals that may diverge from
    // the live token probe; keep the pool-registered value authoritative.
    if cfg!(feature = "testing") && pool_decimals != 0 {
        config.asset_decimals = pool_decimals;
    }

    // The oracle is token-rooted (hub-independent), keyed by the bare asset.
    storage::set_asset_oracle(env, asset, &config);

    UpdateAssetOracleEvent {
        asset: asset.clone(),
        oracle: EventOracleProvider::from_oracle(env, asset, &config),
    }
    .publish(env);
}

/// Validates a resolved `MarketOracleConfig` at the controller boundary: the
/// sanity band must be set and ordered, and every quote source must point at an
/// active, USD-based market. Shared by the token-rooted `set_market_oracle_config`
/// and the per-spoke `oracle_override`.
fn validate_market_oracle_config(env: &Env, asset: &Address, config: &MarketOracleConfig) {
    common::validation::validate_sanity_bounds(
        env,
        config.min_sanity_price_wad,
        config.max_sanity_price_wad,
    );
    require_quote_markets_active_usd(env, asset, config);
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
