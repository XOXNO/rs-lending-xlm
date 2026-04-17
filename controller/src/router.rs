use common::errors::{GenericError, OracleError};
use common::events::{emit_create_market, CreateMarketEvent};
use common::types::{
    AssetConfig, ControllerKey, MarketConfig, MarketParams, MarketStatus, OracleProviderConfig,
    ReflectorAssetKind,
};
use soroban_sdk::{panic_with_error, token, xdr::ToXdr, Address, BytesN, Env, Symbol};

use crate::cache::ControllerCache;
use crate::{storage, validation};

fn validate_market_creation(
    env: &Env,
    asset: &Address,
    params: &MarketParams,
    config: &AssetConfig,
    _token_decimals: u32,
) {
    if params.asset_id != *asset {
        panic_with_error!(env, GenericError::WrongToken);
    }
    #[cfg(not(feature = "testing"))]
    if params.asset_decimals != _token_decimals {
        panic_with_error!(env, GenericError::InvalidAsset);
    }

    validation::validate_asset_config(env, config);
    validation::validate_interest_rate_model(env, params);
}

/// Deploys a liquidity pool for `asset` from the stored WASM template and
/// persists the resulting market config. The controller owns the pool.
pub fn create_liquidity_pool(
    env: &Env,
    asset: &Address,
    params: &MarketParams,
    config: &AssetConfig,
) -> Address {
    // Asset must be a valid token contract (probes decimals and symbol).
    let token_client = token::Client::new(env, asset);
    let token_decimals = token_client
        .try_decimals()
        .unwrap_or_else(|_| panic_with_error!(env, GenericError::InvalidAsset))
        .unwrap_or_else(|_| panic_with_error!(env, GenericError::InvalidAsset));
    if token_client.try_symbol().is_err() {
        panic_with_error!(env, GenericError::InvalidAsset);
    }

    // Reject double-listing before the allow-list check so the error is
    // specific instead of the generic `TokenNotApproved`.
    if storage::has_market_config(env, asset) {
        panic_with_error!(env, GenericError::AssetAlreadySupported);
    }

    // Token contract address must be on the admin allow-list; gate by
    // address because the Soroban SDK exposes no runtime Wasm-hash lookup.
    if !storage::is_token_approved(env, asset) {
        panic_with_error!(env, GenericError::TokenNotApproved);
    }

    validate_market_creation(env, asset, params, config, token_decimals);

    if !storage::has_pool_template(env) {
        panic_with_error!(env, GenericError::TemplateEmpty);
    }
    let wasm_hash = storage::get_pool_template(env);

    // Deterministic salt from the asset address enforces one pool per asset.
    let salt = env.crypto().keccak256(&asset.to_xdr(env));

    // Accumulator address is passed at pool construction so `claim_revenue`
    // transfers to a pool-stored destination rather than a caller-supplied
    // one. Accumulator must be configured first.
    if !storage::has_accumulator(env) {
        panic_with_error!(env, GenericError::AccumulatorNotSet);
    }
    let accumulator = storage::get_accumulator(env);

    // Deploy the pool with the controller as admin.
    let pool_address = env.deployer().with_current_contract(salt).deploy_v2(
        wasm_hash,
        (env.current_contract_address(), params.clone(), accumulator),
    );

    storage::set_isolated_debt(env, asset, 0);

    // Market starts in PendingOracle; `configure_market_oracle` populates
    // the flat oracle fields and transitions to Active.
    let market = MarketConfig {
        status: MarketStatus::PendingOracle,
        asset_config: config.clone(),
        pool_address: pool_address.clone(),
        oracle_config: OracleProviderConfig::default_for(asset.clone(), params.asset_decimals),
        cex_oracle: None,
        cex_asset_kind: ReflectorAssetKind::Stellar,
        cex_symbol: Symbol::new(env, ""),
        cex_decimals: 0,
        dex_oracle: None,
        dex_asset_kind: ReflectorAssetKind::Stellar,
        dex_symbol: Symbol::new(env, ""),
        dex_decimals: 0,
        twap_records: 0,
    };
    storage::set_market_config(env, asset, &market);

    // Track in the pools list for enumeration.
    storage::add_to_pools_list(env, asset, &pool_address);
    storage::bump_instance(env);

    emit_create_market(
        env,
        CreateMarketEvent {
            base_asset: asset.clone(),
            max_borrow_rate: params.max_borrow_rate_ray,
            base_borrow_rate: params.base_borrow_rate_ray,
            slope1: params.slope1_ray,
            slope2: params.slope2_ray,
            slope3: params.slope3_ray,
            mid_utilization: params.mid_utilization_ray,
            optimal_utilization: params.optimal_utilization_ray,
            reserve_factor: params.reserve_factor_bps,
            market_address: pool_address.clone(),
            config: config.clone(),
        },
    );

    pool_address
}

// ---------------------------------------------------------------------------
// Pool upgrades
// ---------------------------------------------------------------------------

/// Upgrades a pool's interest-rate model in place. Indexes are synced at
/// the current oracle price before the new parameters are applied so
/// accrued interest rolls into the stored indexes.
#[allow(clippy::too_many_arguments)]
pub fn upgrade_liquidity_pool_params(
    env: &Env,
    asset: &Address,
    max_borrow_rate: i128,
    base_borrow_rate: i128,
    slope1: i128,
    slope2: i128,
    slope3: i128,
    mid_utilization: i128,
    optimal_utilization: i128,
    reserve_factor: i128,
) {
    validation::require_asset_supported(env, asset);

    let market = storage::get_market_config(env, asset);

    validation::validate_interest_rate_model(
        env,
        &MarketParams {
            max_borrow_rate_ray: max_borrow_rate,
            base_borrow_rate_ray: base_borrow_rate,
            slope1_ray: slope1,
            slope2_ray: slope2,
            slope3_ray: slope3,
            mid_utilization_ray: mid_utilization,
            optimal_utilization_ray: optimal_utilization,
            reserve_factor_bps: reserve_factor,
            asset_id: asset.clone(),
            asset_decimals: market.oracle_config.asset_decimals,
        },
    );

    let pool_client = pool_interface::LiquidityPoolClient::new(env, &market.pool_address);

    let mut cache = ControllerCache::new(env, true);
    let feed = cache.cached_price(asset);
    pool_client.update_indexes(&feed.price_wad);

    pool_client.update_params(
        &max_borrow_rate,
        &base_borrow_rate,
        &slope1,
        &slope2,
        &slope3,
        &mid_utilization,
        &optimal_utilization,
        &reserve_factor,
    );
}

/// Upgrades the pool contract's WASM code.
pub fn upgrade_liquidity_pool(env: &Env, asset: &Address, new_wasm_hash: BytesN<32>) {
    validation::require_asset_supported(env, asset);

    let market = storage::get_market_config(env, asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(env, &market.pool_address);
    pool_client.upgrade(&new_wasm_hash);
}

// ---------------------------------------------------------------------------
// Revenue management
// ---------------------------------------------------------------------------

/// Claims accrued protocol revenue from a single pool. The pool transfers
/// directly to the accumulator address it stored at construction.
fn claim_revenue_for_asset(env: &Env, asset: &Address) -> i128 {
    validation::require_asset_supported(env, asset);

    if !storage::has_accumulator(env) {
        panic_with_error!(env, OracleError::NoAccumulator);
    }

    // Safe-price cache: revenue claim cannot liquidate positions.
    let mut cache = ControllerCache::new(env, true);
    let pool_addr = cache.cached_pool_address(asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
    let feed = cache.cached_price(asset);

    pool_client.claim_revenue(&feed.price_wad)
}

/// Claims accrued protocol revenue from multiple pools in one call.
pub fn claim_revenue(env: &Env, assets: soroban_sdk::Vec<Address>) -> soroban_sdk::Vec<i128> {
    let mut results = soroban_sdk::Vec::new(env);
    for i in 0..assets.len() {
        let asset = assets.get(i).unwrap();
        results.push_back(claim_revenue_for_asset(env, &asset));
    }
    results
}

/// Transfers reward tokens from the caller into the pool and bumps the
/// pool's supply index to distribute the rewards to suppliers.
pub fn add_reward(env: &Env, caller: &Address, asset: &Address, amount: i128) {
    validation::require_asset_supported(env, asset);
    validation::require_amount_positive(env, amount);

    // Safe-price cache: reward credit cannot liquidate positions.
    let mut cache = ControllerCache::new(env, true);
    let pool_addr = cache.cached_pool_address(asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
    let feed = cache.cached_price(asset);

    let tok = soroban_sdk::token::Client::new(env, asset);
    tok.transfer(caller, &pool_addr, &amount);
    pool_client.add_rewards(&feed.price_wad, &amount);
}

pub fn add_rewards_batch(env: &Env, caller: &Address, rewards: soroban_sdk::Vec<(Address, i128)>) {
    for i in 0..rewards.len() {
        let (asset, amount) = rewards.get(i).unwrap();
        add_reward(env, caller, &asset, amount);
    }
}

// ---------------------------------------------------------------------------
// TTL maintenance
// ---------------------------------------------------------------------------

pub fn keepalive_shared_state(env: &Env, assets: &soroban_sdk::Vec<Address>) {
    storage::bump_instance(env);
    storage::bump_pools_list(env);

    for i in 0..assets.len() {
        let asset = assets.get(i).unwrap();
        if !storage::has_market_config(env, &asset) {
            continue;
        }

        storage::bump_shared(env, &ControllerKey::Market(asset.clone()));
        storage::bump_shared(env, &ControllerKey::IsolatedDebt(asset.clone()));

        let categories = storage::get_asset_emodes(env, &asset);
        if !categories.is_empty() {
            storage::bump_shared(env, &ControllerKey::AssetEModes(asset.clone()));
        }
        for category_id in categories {
            storage::bump_shared(env, &ControllerKey::EModeCategory(category_id));
            if storage::get_emode_asset(env, category_id, &asset).is_some() {
                storage::bump_shared(env, &ControllerKey::EModeAsset(category_id, asset.clone()));
            }
        }
    }
}

pub fn keepalive_accounts(env: &Env, account_ids: &soroban_sdk::Vec<u64>) {
    for i in 0..account_ids.len() {
        let account_id = account_ids.get(i).unwrap();
        if storage::try_get_account(env, account_id).is_some() {
            storage::bump_account(env, account_id);
        }
    }
}

pub fn keepalive_pools(env: &Env, assets: &soroban_sdk::Vec<Address>) {
    for i in 0..assets.len() {
        let asset = assets.get(i).unwrap();
        if !storage::has_market_config(env, &asset) {
            continue;
        }
        let market = storage::get_market_config(env, &asset);
        let pool_client = pool_interface::LiquidityPoolClient::new(env, &market.pool_address);
        pool_client.keepalive();
    }
}
