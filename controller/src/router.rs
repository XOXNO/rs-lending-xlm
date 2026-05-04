use common::errors::{GenericError, OracleError};
use common::events::{emit_create_market, CreateMarketEvent};
use common::types::{
    AssetConfig, ControllerKey, InterestRateModel, MarketConfig, MarketParams, MarketStatus,
    OracleProviderConfig, ReflectorAssetKind,
};
use soroban_sdk::{
    contractimpl, panic_with_error, token, xdr::ToXdr, Address, BytesN, Env, Symbol, Vec,
};
use stellar_macros::{only_owner, only_role, when_not_paused};

use crate::cache::ControllerCache;
use crate::{storage, utils, validation, Controller, ControllerArgs, ControllerClient};

#[contractimpl]
impl Controller {
    #[when_not_paused]
    #[only_role(caller, "KEEPER")]
    pub fn update_indexes(env: Env, caller: Address, assets: Vec<Address>) {
        validation::require_not_flash_loaning(&env);

        let mut cache = ControllerCache::new(&env, true);
        utils::sync_market_indexes(&env, &mut cache, &assets);

        storage::bump_pools_list(&env);
    }

    #[only_role(caller, "KEEPER")]
    pub fn keepalive_shared_state(env: Env, caller: Address, assets: Vec<Address>) {
        let _ = caller;
        keepalive_shared_state(&env, &assets);
    }

    #[only_role(caller, "KEEPER")]
    pub fn keepalive_accounts(env: Env, caller: Address, account_ids: Vec<u64>) {
        let _ = caller;
        keepalive_accounts(&env, &account_ids);
    }

    #[only_role(caller, "KEEPER")]
    pub fn keepalive_pools(env: Env, caller: Address, assets: Vec<Address>) {
        let _ = caller;
        keepalive_pools(&env, &assets);
    }

    #[only_owner]
    pub fn create_liquidity_pool(
        env: Env,
        asset: Address,
        params: MarketParams,
        config: AssetConfig,
    ) -> Address {
        create_liquidity_pool(&env, &asset, &params, &config)
    }

    #[only_owner]
    pub fn upgrade_pool_params(env: Env, asset: Address, params: InterestRateModel) {
        upgrade_liquidity_pool_params(&env, &asset, &params);
    }

    #[only_owner]
    pub fn upgrade_liquidity_pool_params(env: Env, asset: Address, params: InterestRateModel) {
        upgrade_liquidity_pool_params(&env, &asset, &params);
    }

    #[only_owner]
    pub fn upgrade_pool(env: Env, asset: Address, new_wasm_hash: BytesN<32>) {
        upgrade_liquidity_pool(&env, &asset, new_wasm_hash);
    }

    #[only_owner]
    pub fn upgrade_liquidity_pool(env: Env, asset: Address, new_wasm_hash: BytesN<32>) {
        upgrade_liquidity_pool(&env, &asset, new_wasm_hash);
    }

    #[when_not_paused]
    #[only_role(caller, "REVENUE")]
    pub fn claim_revenue(env: Env, caller: Address, assets: Vec<Address>) -> Vec<i128> {
        let _ = caller;
        validation::require_not_flash_loaning(&env);
        claim_revenue(&env, assets)
    }

    #[only_role(caller, "REVENUE")]
    pub fn add_rewards(env: Env, caller: Address, rewards: Vec<(Address, i128)>) {
        validation::require_not_flash_loaning(&env);
        add_rewards_batch(&env, &caller, rewards);
    }
}

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

    // Deploy the pool with the controller as owner. Revenue routing is
    // anchored to that ownership: `claim_revenue` transfers to the pool
    // owner (controller), which then forwards to the accumulator. The
    // accumulator address itself is NOT passed in here; the controller is
    // the single source of truth and resolves it at claim time.
    let pool_address = env
        .deployer()
        .with_current_contract(salt)
        .deploy_v2(wasm_hash, (env.current_contract_address(), params.clone()));

    // `IsolatedDebt(asset)` is created lazily on the first isolated
    // borrow against this asset (see `handle_isolated_debt` →
    // `cache.flush_isolated_debts`). Reads default to 0 via
    // `storage::get_isolated_debt`'s `unwrap_or(0)`, so non-isolated
    // markets never need a placeholder entry.

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

    // Approval is single-use: the gate at the top of this function
    // verified the caller had pre-approved this asset; consume it now
    // so the instance footprint doesn't accumulate stale approvals
    // and any future `create_liquidity_pool` call requires a fresh
    // admin review.
    storage::set_token_approved(env, asset, false);

    pool_address
}

// ---------------------------------------------------------------------------
// Pool upgrades
// ---------------------------------------------------------------------------

/// Upgrades a pool's interest-rate model in place. Indexes are synced at
/// the current oracle price before the new parameters are applied so
/// accrued interest rolls into the stored indexes.
pub fn upgrade_liquidity_pool_params(env: &Env, asset: &Address, params: &InterestRateModel) {
    validation::require_asset_supported(env, asset);

    let market = storage::get_market_config(env, asset);

    validation::validate_interest_rate_model(
        env,
        &MarketParams {
            max_borrow_rate_ray: params.max_borrow_rate_ray,
            base_borrow_rate_ray: params.base_borrow_rate_ray,
            slope1_ray: params.slope1_ray,
            slope2_ray: params.slope2_ray,
            slope3_ray: params.slope3_ray,
            mid_utilization_ray: params.mid_utilization_ray,
            optimal_utilization_ray: params.optimal_utilization_ray,
            reserve_factor_bps: params.reserve_factor_bps,
            asset_id: asset.clone(),
            asset_decimals: market.oracle_config.asset_decimals,
        },
    );

    let pool_client = pool_interface::LiquidityPoolClient::new(env, &market.pool_address);

    let mut cache = ControllerCache::new(env, true);
    let feed = cache.cached_price(asset);
    pool_update_indexes_call(env, &market.pool_address, feed.price_wad);

    pool_client.update_params(
        &params.max_borrow_rate_ray,
        &params.base_borrow_rate_ray,
        &params.slope1_ray,
        &params.slope2_ray,
        &params.slope3_ray,
        &params.mid_utilization_ray,
        &params.optimal_utilization_ray,
        &params.reserve_factor_bps,
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
/// the claimed amount to its owner (this controller); the controller then
/// forwards to the configured accumulator. Both transfers must succeed in
/// the same transaction or the whole claim reverts (no try/catch on either
/// hop), so partial state is impossible.
fn claim_revenue_for_asset(env: &Env, asset: &Address) -> i128 {
    validation::require_asset_supported(env, asset);

    if !storage::has_accumulator(env) {
        panic_with_error!(env, OracleError::NoAccumulator);
    }

    // Safe-price cache: revenue claim cannot liquidate positions.
    let mut cache = ControllerCache::new(env, true);
    let pool_addr = cache.cached_pool_address(asset);
    let feed = cache.cached_price(asset);

    let amount = pool_claim_revenue_call(env, &pool_addr, feed.price_wad);

    if amount > 0 {
        let accumulator = storage::get_accumulator(env);
        utils::sac_transfer_call(
            env,
            asset,
            &env.current_contract_address(),
            &accumulator,
            &amount,
        );
    }

    amount
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
    let feed = cache.cached_price(asset);

    let actual_received = utils::transfer_and_measure_received(
        env,
        asset,
        caller,
        &pool_addr,
        amount,
        GenericError::AmountMustBePositive,
    );

    pool_add_rewards_call(env, &pool_addr, feed.price_wad, actual_received);
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
        // `IsolatedDebt(asset)` is created lazily on the first isolated
        // borrow — non-isolated markets have no entry to bump.
        let isolated_key = ControllerKey::IsolatedDebt(asset.clone());
        if env.storage().persistent().has(&isolated_key) {
            storage::bump_shared(env, &isolated_key);
        }

        let categories = storage::get_asset_emodes(env, &asset);
        if !categories.is_empty() {
            storage::bump_shared(env, &ControllerKey::AssetEModes(asset.clone()));
        }
        for category_id in categories {
            // Single ledger entry per category — params + member-asset map.
            storage::bump_shared(env, &ControllerKey::EModeCategory(category_id));
        }
    }
}

pub fn keepalive_accounts(env: &Env, account_ids: &soroban_sdk::Vec<u64>) {
    for i in 0..account_ids.len() {
        let account_id = account_ids.get(i).unwrap();
        // `bump_account` per-key `has` checks already make missing accounts a
        // no-op, so a separate existence read up front is wasted I/O.
        storage::bump_account(env, account_id);
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

// ---------------------------------------------------------------------------
// Summarised pool wrappers (used by router + helpers; the macro is a no-op
// outside `--features certora`).
// ---------------------------------------------------------------------------

crate::summarized!(
    crate::spec::summaries::pool::update_indexes_summary,
    pub(crate) fn pool_update_indexes_call(
        env: &Env,
        pool_addr: &Address,
        price_wad: i128,
    ) -> common::types::MarketIndex {
        pool_interface::LiquidityPoolClient::new(env, pool_addr).update_indexes(&price_wad)
    }
);

crate::summarized!(
    crate::spec::summaries::pool::claim_revenue_summary,
    pub(crate) fn pool_claim_revenue_call(
        env: &Env,
        pool_addr: &Address,
        price_wad: i128,
    ) -> i128 {
        pool_interface::LiquidityPoolClient::new(env, pool_addr).claim_revenue(&price_wad)
    }
);

crate::summarized!(
    crate::spec::summaries::pool::add_rewards_summary,
    pub(crate) fn pool_add_rewards_call(
        env: &Env,
        pool_addr: &Address,
        price_wad: i128,
        amount: i128,
    ) {
        pool_interface::LiquidityPoolClient::new(env, pool_addr).add_rewards(&price_wad, &amount)
    }
);
