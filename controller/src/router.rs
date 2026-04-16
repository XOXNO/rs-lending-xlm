use common::errors::GenericError;
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

// Deploys a new liquidity pool for `asset` using the stored WASM template.
//
// The pool is owned by the controller (current contract) and initialized with
// the provided market parameters. The pool address is persisted so that
// subsequent supply/borrow calls can find it.
pub fn create_liquidity_pool(
    env: &Env,
    asset: &Address,
    params: &MarketParams,
    config: &AssetConfig,
) -> Address {
    // Guard: asset must be a valid token contract (checks decimals and symbol)
    let token_client = token::Client::new(env, asset);
    let token_decimals = token_client
        .try_decimals()
        .unwrap_or_else(|_| panic_with_error!(env, GenericError::InvalidAsset))
        .unwrap_or_else(|_| panic_with_error!(env, GenericError::InvalidAsset));
    if token_client.try_symbol().is_err() {
        panic_with_error!(env, GenericError::InvalidAsset);
    }

    // Guard: asset must not already have a pool (fire BEFORE allow-list so
    // double-listing gives a specific error, not the generic "not approved").
    if storage::has_market_config(env, asset) {
        panic_with_error!(env, GenericError::AssetAlreadySupported);
    }

    // Guard: token contract address must be on the admin allow-list. Soroban
    // SDK 25 does not expose a runtime lookup of a deployed contract's Wasm
    // hash, so we gate by token address (see `approve_token_wasm` admin endpoint).
    if !storage::is_token_approved(env, asset) {
        panic_with_error!(env, GenericError::TokenNotApproved);
    }

    validate_market_creation(env, asset, params, config, token_decimals);

    if !storage::has_pool_template(env) {
        panic_with_error!(env, GenericError::TemplateEmpty);
    }
    let wasm_hash = storage::get_pool_template(env);

    // Deterministic salt from asset address — ensures exactly one pool per asset.
    let salt = env.crypto().keccak256(&asset.to_xdr(env));

    // Deploy pool contract with controller as admin
    let pool_address = env
        .deployer()
        .with_current_contract(salt)
        .deploy_v2(wasm_hash, (env.current_contract_address(), params.clone()));

    // Initialize the isolated debt tracker to zero explicitly
    storage::set_isolated_debt(env, asset, 0);

    // Oracle is optional at this stage; the market starts pending until a
    // subsequent configure_market_oracle call populates the flat oracle fields.
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
        dex_decimals: 0,
        twap_records: 0,
    };
    storage::set_market_config(env, asset, &market);

    // Track in pools list for enumeration
    storage::add_to_pools_list(env, asset, &pool_address);
    storage::bump_instance(env);

    // Emit creation event
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

// Upgrades a pool's rate model without redeployment.
//
// Syncs the pool indexes before updating the rate model so accrued
// interest is preserved.
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
        &common::types::MarketParams {
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

    // Sync indexes at current price before changing rate model so any
    // accrued interest is rolled into the stored indexes.
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

// Upgrades the pool contract's WASM code.
pub fn upgrade_liquidity_pool(env: &Env, asset: &Address, new_wasm_hash: BytesN<32>) {
    validation::require_asset_supported(env, asset);

    let market = storage::get_market_config(env, asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(env, &market.pool_address);
    pool_client.upgrade(&new_wasm_hash);
}

// ---------------------------------------------------------------------------
// Revenue management
// ---------------------------------------------------------------------------

// Claims accrued protocol revenue from a pool and forwards it to the
// configured accumulator. Returns the claimed amount.
fn claim_revenue_for_asset(env: &Env, asset: &Address) -> i128 {
    validation::require_asset_supported(env, asset);

    if !storage::has_accumulator(env) {
        panic_with_error!(env, common::errors::OracleError::NoAccumulator);
    }

    let mut cache = ControllerCache::new(env, true); // revenue claim is safe
    let pool_addr = cache.cached_pool_address(asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
    let feed = cache.cached_price(asset);

    let amount = pool_client.claim_revenue(&env.current_contract_address(), &feed.price_wad);
    if amount <= 0 {
        return 0;
    }

    // Forward tokens from the controller to the accumulator.
    let tok = soroban_sdk::token::Client::new(env, asset);
    let acc = storage::get_accumulator(env);
    tok.transfer(&env.current_contract_address(), &acc, &amount);

    amount
}

// Claims accrued protocol revenue from multiple pools in a single call.
pub fn claim_revenue(env: &Env, assets: soroban_sdk::Vec<Address>) -> soroban_sdk::Vec<i128> {
    let mut results = soroban_sdk::Vec::new(env);
    for i in 0..assets.len() {
        let asset = assets.get(i).unwrap();
        results.push_back(claim_revenue_for_asset(env, &asset));
    }
    results
}

// Adds rewards to a pool's supply index. The caller must hold the reward
// tokens; they are pulled from the caller and credited to the pool.
pub fn add_reward(env: &Env, caller: &Address, asset: &Address, amount: i128) {
    validation::require_asset_supported(env, asset);
    validation::require_amount_positive(env, amount);

    let mut cache = ControllerCache::new(env, true); // reward credit is safe
    let pool_addr = cache.cached_pool_address(asset);
    let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
    let feed = cache.cached_price(asset);

    // Transfer reward tokens from the caller to the pool, then signal the
    // pool to bump its supply index.
    let tok = soroban_sdk::token::Client::new(env, asset);
    tok.transfer(caller, &pool_addr, &amount);
    pool_client.add_rewards(&feed.price_wad, &amount);
}

// Adds rewards to multiple pools in a single call.
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
