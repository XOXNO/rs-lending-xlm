use common::constants::{
    TTL_BUMP_INSTANCE, TTL_BUMP_SHARED, TTL_BUMP_USER, TTL_THRESHOLD_INSTANCE,
    TTL_THRESHOLD_SHARED, TTL_THRESHOLD_USER,
};
use common::errors::{EModeError, GenericError};
use common::types::{
    Account, AccountMeta, AccountPosition, ControllerKey, EModeAssetConfig, EModeCategory,
    MarketConfig, PositionLimits, POSITION_TYPE_DEPOSIT,
};
#[cfg(feature = "certora")]
use common::types::{AccountAttributes, AssetConfig, MarketIndex, MarketParams};
#[cfg(test)]
use common::types::{OracleProviderConfig, ReflectorConfig};
#[cfg(feature = "certora")]
use pool_interface::LiquidityPoolClient;
use soroban_sdk::{contracttype, panic_with_error, Address, BytesN, Env, Vec};

// Local storage keys for controller-only features that do not belong in the
// shared `ControllerKey` enum (e.g., the token-wasm allow-list for market creation).
#[contracttype]
#[derive(Clone, Debug)]
enum LocalKey {
    // Allow-list of token contract addresses eligible to back a new liquidity
    // pool. Admin approval gates this list to keep hostile or malicious token
    // implementations off the protocol.
    ApprovedToken(Address),
}

// ---------------------------------------------------------------------------
// Token allow-list (instance storage)
// ---------------------------------------------------------------------------

pub fn is_token_approved(env: &Env, token: &Address) -> bool {
    env.storage()
        .instance()
        .get(&LocalKey::ApprovedToken(token.clone()))
        .unwrap_or(false)
}

pub fn set_token_approved(env: &Env, token: &Address, approved: bool) {
    if approved {
        env.storage()
            .instance()
            .set(&LocalKey::ApprovedToken(token.clone()), &true);
    } else {
        env.storage()
            .instance()
            .remove(&LocalKey::ApprovedToken(token.clone()));
    }
}

// ---------------------------------------------------------------------------
// Tiered TTL helpers
// ---------------------------------------------------------------------------

pub fn bump_user(env: &Env, key: &ControllerKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_USER, TTL_BUMP_USER);
}

pub fn bump_shared(env: &Env, key: &ControllerKey) {
    env.storage()
        .persistent()
        .extend_ttl(key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
}

pub fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD_INSTANCE, TTL_BUMP_INSTANCE);
}

// ---------------------------------------------------------------------------
// Instance storage helpers
// Ownership and pause state are handled by support modules.
// ---------------------------------------------------------------------------

pub fn get_pool_template(env: &Env) -> BytesN<32> {
    env.storage()
        .instance()
        .get(&ControllerKey::PoolTemplate)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::TemplateNotSet))
}

pub fn set_pool_template(env: &Env, hash: &BytesN<32>) {
    env.storage()
        .instance()
        .set(&ControllerKey::PoolTemplate, hash);
}

pub fn has_pool_template(env: &Env) -> bool {
    env.storage().instance().has(&ControllerKey::PoolTemplate)
}

// ---------------------------------------------------------------------------
// Instance storage helpers
// ---------------------------------------------------------------------------

pub fn get_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&ControllerKey::Aggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

pub fn set_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::Aggregator, addr);
}

pub fn get_accumulator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&ControllerKey::Accumulator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccumulatorNotSet))
}

pub fn set_accumulator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::Accumulator, addr);
}

pub fn get_account_nonce(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&ControllerKey::AccountNonce)
        .unwrap_or(0u64)
}

pub fn increment_account_nonce(env: &Env) -> u64 {
    let current = get_account_nonce(env);
    let next = current + 1;
    env.storage()
        .instance()
        .set(&ControllerKey::AccountNonce, &next);
    next
}

pub fn get_position_limits(env: &Env) -> PositionLimits {
    env.storage()
        .instance()
        .get(&ControllerKey::PositionLimits)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PositionLimitsNotSet))
}

pub fn set_position_limits(env: &Env, limits: &PositionLimits) {
    env.storage()
        .instance()
        .set(&ControllerKey::PositionLimits, limits);
}

pub fn get_last_emode_category_id(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&ControllerKey::LastEModeCategoryId)
        .unwrap_or(0u32)
}

pub fn increment_emode_category_id(env: &Env) -> u32 {
    let current = get_last_emode_category_id(env);
    let next = current + 1;
    env.storage()
        .instance()
        .set(&ControllerKey::LastEModeCategoryId, &next);
    next
}

// ---------------------------------------------------------------------------
// Flash loan guard (Instance storage)
// ---------------------------------------------------------------------------

pub fn is_flash_loan_ongoing(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&ControllerKey::FlashLoanOngoing)
        .unwrap_or(false)
}

pub fn set_flash_loan_ongoing(env: &Env, ongoing: bool) {
    env.storage()
        .instance()
        .set(&ControllerKey::FlashLoanOngoing, &ongoing);
}

// ---------------------------------------------------------------------------
// Accumulator helpers
// ---------------------------------------------------------------------------

pub fn has_accumulator(env: &Env) -> bool {
    env.storage().instance().has(&ControllerKey::Accumulator)
}

// ---------------------------------------------------------------------------
// Persistent storage — MarketConfig (consolidated)
// ---------------------------------------------------------------------------

pub fn get_market_config(env: &Env, asset: &Address) -> MarketConfig {
    let key = ControllerKey::Market(asset.clone());
    match env.storage().persistent().get::<_, MarketConfig>(&key) {
        Some(config) => config,
        None => panic_with_error!(env, common::errors::GenericError::AssetNotSupported),
    }
}

pub fn set_market_config(env: &Env, asset: &Address, config: &MarketConfig) {
    let key = ControllerKey::Market(asset.clone());
    env.storage().persistent().set(&key, config);
    bump_shared(env, &key);
}

pub fn has_market_config(env: &Env, asset: &Address) -> bool {
    let key = ControllerKey::Market(asset.clone());
    env.storage().persistent().has(&key)
}

pub fn try_get_market_config(env: &Env, asset: &Address) -> Option<MarketConfig> {
    let key = ControllerKey::Market(asset.clone());
    env.storage().persistent().get(&key)
}

#[allow(dead_code)]
pub fn remove_market_config(env: &Env, asset: &Address) {
    let key = ControllerKey::Market(asset.clone());
    env.storage().persistent().remove(&key);
}

// ---------------------------------------------------------------------------
// Persistent storage — Account (split meta + per-position keys)
// ---------------------------------------------------------------------------

fn account_meta_from_account(env: &Env, account: &Account) -> AccountMeta {
    let mut supply_assets = Vec::new(env);
    for asset in account.supply_positions.keys() {
        supply_assets.push_back(asset);
    }

    let mut borrow_assets = Vec::new(env);
    for asset in account.borrow_positions.keys() {
        borrow_assets.push_back(asset);
    }

    AccountMeta {
        owner: account.owner.clone(),
        is_isolated: account.is_isolated,
        e_mode_category_id: account.e_mode_category_id,
        mode: account.mode,
        isolated_asset: account.isolated_asset.clone(),
        supply_assets,
        borrow_assets,
    }
}

fn account_from_meta_and_positions(env: &Env, account_id: u64, meta: &AccountMeta) -> Account {
    let mut supply_positions = soroban_sdk::Map::new(env);
    for asset in meta.supply_assets.iter() {
        let key = ControllerKey::SupplyPosition(account_id, asset.clone());
        if let Some(position) = env.storage().persistent().get::<_, AccountPosition>(&key) {
            supply_positions.set(asset, position);
        }
    }

    let mut borrow_positions = soroban_sdk::Map::new(env);
    for asset in meta.borrow_assets.iter() {
        let key = ControllerKey::BorrowPosition(account_id, asset.clone());
        if let Some(position) = env.storage().persistent().get::<_, AccountPosition>(&key) {
            borrow_positions.set(asset, position);
        }
    }

    Account {
        owner: meta.owner.clone(),
        is_isolated: meta.is_isolated,
        e_mode_category_id: meta.e_mode_category_id,
        mode: meta.mode,
        isolated_asset: meta.isolated_asset.clone(),
        supply_positions,
        borrow_positions,
    }
}

pub fn try_get_account_meta(env: &Env, account_id: u64) -> Option<AccountMeta> {
    let key = ControllerKey::AccountMeta(account_id);
    env.storage().persistent().get::<_, AccountMeta>(&key)
}

pub fn get_account_meta(env: &Env, account_id: u64) -> AccountMeta {
    try_get_account_meta(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotInMarket))
}

pub fn try_get_account_position(
    env: &Env,
    account_id: u64,
    position_type: u32,
    asset: &Address,
) -> Option<AccountPosition> {
    let key = if position_type == POSITION_TYPE_DEPOSIT {
        ControllerKey::SupplyPosition(account_id, asset.clone())
    } else {
        ControllerKey::BorrowPosition(account_id, asset.clone())
    };

    env.storage().persistent().get::<_, AccountPosition>(&key)
}

pub fn get_account_position(
    env: &Env,
    account_id: u64,
    position_type: u32,
    asset: &Address,
) -> AccountPosition {
    try_get_account_position(env, account_id, position_type, asset)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotFound))
}

pub fn get_account(env: &Env, account_id: u64) -> Account {
    try_get_account(env, account_id)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AccountNotFound))
}

pub fn set_account(env: &Env, account_id: u64, account: &Account) {
    let persistent = env.storage().persistent();
    let new_meta = account_meta_from_account(env, account);
    let split_meta_key = ControllerKey::AccountMeta(account_id);
    let old_split_meta = persistent.get::<_, AccountMeta>(&split_meta_key);

    let old_supply_assets = old_split_meta
        .as_ref()
        .map(|meta| meta.supply_assets.clone())
        .unwrap_or_else(|| Vec::new(env));

    let old_borrow_assets = old_split_meta
        .as_ref()
        .map(|meta| meta.borrow_assets.clone())
        .unwrap_or_else(|| Vec::new(env));

    for asset in old_supply_assets.iter() {
        if !account.supply_positions.contains_key(asset.clone()) {
            persistent.remove(&ControllerKey::SupplyPosition(account_id, asset.clone()));
        }
    }

    for asset in old_borrow_assets.iter() {
        if !account.borrow_positions.contains_key(asset.clone()) {
            persistent.remove(&ControllerKey::BorrowPosition(account_id, asset.clone()));
        }
    }

    for asset in account.supply_positions.keys() {
        let position = account.supply_positions.get(asset.clone()).unwrap();
        let key = ControllerKey::SupplyPosition(account_id, asset.clone());
        let should_write = persistent.get::<_, AccountPosition>(&key).as_ref() != Some(&position);

        if should_write {
            persistent.set(&key, &position);
        }
        bump_user(env, &key);
    }

    for asset in account.borrow_positions.keys() {
        let position = account.borrow_positions.get(asset.clone()).unwrap();
        let key = ControllerKey::BorrowPosition(account_id, asset.clone());
        let should_write = persistent.get::<_, AccountPosition>(&key).as_ref() != Some(&position);

        if should_write {
            persistent.set(&key, &position);
        }
        bump_user(env, &key);
    }

    if old_split_meta.as_ref() != Some(&new_meta) {
        persistent.set(&split_meta_key, &new_meta);
    }
    bump_user(env, &split_meta_key);
}

pub fn try_get_account(env: &Env, account_id: u64) -> Option<Account> {
    let meta_key = ControllerKey::AccountMeta(account_id);
    if let Some(meta) = env.storage().persistent().get::<_, AccountMeta>(&meta_key) {
        return Some(account_from_meta_and_positions(env, account_id, &meta));
    }
    None
}

pub fn remove_account_entry(env: &Env, account_id: u64) {
    if let Some(meta) = env
        .storage()
        .persistent()
        .get::<_, AccountMeta>(&ControllerKey::AccountMeta(account_id))
    {
        for asset in meta.supply_assets.iter() {
            env.storage()
                .persistent()
                .remove(&ControllerKey::SupplyPosition(account_id, asset));
        }
        for asset in meta.borrow_assets.iter() {
            env.storage()
                .persistent()
                .remove(&ControllerKey::BorrowPosition(account_id, asset));
        }
        env.storage()
            .persistent()
            .remove(&ControllerKey::AccountMeta(account_id));
    }
}

pub fn bump_account(env: &Env, account_id: u64) {
    if let Some(meta) = env
        .storage()
        .persistent()
        .get::<_, AccountMeta>(&ControllerKey::AccountMeta(account_id))
    {
        bump_user(env, &ControllerKey::AccountMeta(account_id));
        for asset in meta.supply_assets.iter() {
            bump_user(env, &ControllerKey::SupplyPosition(account_id, asset));
        }
        for asset in meta.borrow_assets.iter() {
            bump_user(env, &ControllerKey::BorrowPosition(account_id, asset));
        }
    }
}

// ---------------------------------------------------------------------------
// Certora compatibility helpers
// ---------------------------------------------------------------------------

#[cfg(feature = "certora")]
pub fn get_position(
    env: &Env,
    account_id: u64,
    position_type: u32,
    asset: &Address,
) -> Option<AccountPosition> {
    try_get_account_position(env, account_id, position_type, asset)
}

#[cfg(feature = "certora")]
pub fn get_position_list(env: &Env, account_id: u64, position_type: u32) -> Vec<Address> {
    if let Some(meta) = try_get_account_meta(env, account_id) {
        if position_type == POSITION_TYPE_DEPOSIT {
            return meta.supply_assets;
        }
        return meta.borrow_assets;
    }
    Vec::new(env)
}

#[cfg(feature = "certora")]
pub fn get_account_attrs(env: &Env, account_id: u64) -> AccountAttributes {
    try_get_account_meta(env, account_id)
        .map(|meta| AccountAttributes::from(&meta))
        .unwrap_or(AccountAttributes {
            is_isolated: false,
            e_mode_category_id: 0,
            mode: common::types::PositionMode::Normal,
        })
}

#[cfg(feature = "certora")]
pub fn get_asset_config(env: &Env, asset: &Address) -> asset_config::CompatAssetConfig {
    asset_config::get_asset_config(env, asset)
}

#[cfg(feature = "certora")]
pub mod asset_pool {
    use super::*;

    pub fn get_asset_pool(env: &Env, asset: &Address) -> Address {
        get_market_config(env, asset).pool_address
    }
}

#[cfg(feature = "certora")]
pub mod asset_config {
    use super::*;

    #[allow(dead_code)]
    #[derive(Clone, Debug)]
    pub struct CompatAssetConfig {
        pub loan_to_value_bps: i128,
        pub liquidation_threshold_bps: i128,
        pub liquidation_bonus_bps: i128,
        pub liquidation_fees_bps: i128,
        pub is_collateralizable: bool,
        pub is_borrowable: bool,
        pub e_mode_enabled: bool,
        pub is_isolated_asset: bool,
        pub is_siloed_borrowing: bool,
        pub is_flashloanable: bool,
        pub isolation_borrow_enabled: bool,
        pub isolation_debt_ceiling_usd_wad: i128,
        pub flashloan_fee_bps: i128,
        pub borrow_cap: i128,
        pub supply_cap: i128,
        pub reserve_factor_bps: i128,
    }

    pub fn get_asset_config(env: &Env, asset: &Address) -> CompatAssetConfig {
        let market = get_market_config(env, asset);
        let sync = LiquidityPoolClient::new(env, &market.pool_address).get_sync_data();
        let cfg: AssetConfig = market.asset_config;
        CompatAssetConfig {
            loan_to_value_bps: cfg.loan_to_value_bps,
            liquidation_threshold_bps: cfg.liquidation_threshold_bps,
            liquidation_bonus_bps: cfg.liquidation_bonus_bps,
            liquidation_fees_bps: cfg.liquidation_fees_bps,
            is_collateralizable: cfg.is_collateralizable,
            is_borrowable: cfg.is_borrowable,
            e_mode_enabled: cfg.e_mode_enabled,
            is_isolated_asset: cfg.is_isolated_asset,
            is_siloed_borrowing: cfg.is_siloed_borrowing,
            is_flashloanable: cfg.is_flashloanable,
            isolation_borrow_enabled: cfg.isolation_borrow_enabled,
            isolation_debt_ceiling_usd_wad: cfg.isolation_debt_ceiling_usd_wad,
            flashloan_fee_bps: cfg.flashloan_fee_bps,
            borrow_cap: cfg.borrow_cap,
            supply_cap: cfg.supply_cap,
            reserve_factor_bps: sync.params.reserve_factor_bps,
        }
    }
}

#[cfg(feature = "certora")]
pub mod market_index {
    use super::*;

    pub fn get_market_index(env: &Env, asset: &Address) -> MarketIndex {
        let market = get_market_config(env, asset);
        let state = LiquidityPoolClient::new(env, &market.pool_address)
            .get_sync_data()
            .state;
        MarketIndex {
            borrow_index_ray: state.borrow_index_ray,
            supply_index_ray: state.supply_index_ray,
        }
    }
}

#[cfg(feature = "certora")]
pub mod market_params {
    use super::*;

    pub fn get_market_params(env: &Env, asset: &Address) -> MarketParams {
        let market = get_market_config(env, asset);
        LiquidityPoolClient::new(env, &market.pool_address)
            .get_sync_data()
            .params
    }
}

#[cfg(feature = "certora")]
pub mod isolation {
    use super::*;

    pub fn get_isolated_debt(env: &Env, asset: &Address) -> i128 {
        super::get_isolated_debt(env, asset)
    }
}

#[cfg(feature = "certora")]
pub mod accounts {
    use super::*;

    #[derive(Clone, Debug)]
    pub struct AccountData {
        pub is_isolated: bool,
        pub e_mode_category: u32,
        pub isolated_asset: Address,
    }

    pub fn get_account_data(env: &Env, account_id: u64) -> AccountData {
        let meta = get_account_meta(env, account_id);
        let isolated_asset = meta.isolated_asset.unwrap_or_else(|| meta.owner.clone());
        AccountData {
            is_isolated: meta.is_isolated,
            e_mode_category: meta.e_mode_category_id,
            isolated_asset,
        }
    }
}

#[cfg(feature = "certora")]
pub mod positions {
    use super::*;

    pub fn get_scaled_amount(
        env: &Env,
        account_id: u64,
        position_type: u32,
        asset: &Address,
    ) -> i128 {
        try_get_account_position(env, account_id, position_type, asset)
            .map(|position| position.scaled_amount_ray)
            .unwrap_or(0)
    }

    pub fn count_positions(env: &Env, account_id: u64, position_type: u32) -> u32 {
        get_position_list(env, account_id, position_type).len()
    }

    pub fn get_position_list(env: &Env, account_id: u64, position_type: u32) -> Vec<Address> {
        super::get_position_list(env, account_id, position_type)
    }
}

// ---------------------------------------------------------------------------
// Persistent storage — E-Mode
// ---------------------------------------------------------------------------

pub fn get_emode_category(env: &Env, id: u32) -> EModeCategory {
    let key = ControllerKey::EModeCategory(id);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, EModeError::EModeCategoryNotFound))
}

pub fn try_get_emode_category(env: &Env, id: u32) -> Option<EModeCategory> {
    let key = ControllerKey::EModeCategory(id);
    env.storage().persistent().get(&key)
}

pub fn set_emode_category(env: &Env, id: u32, cat: &EModeCategory) {
    let key = ControllerKey::EModeCategory(id);
    env.storage().persistent().set(&key, cat);
    bump_shared(env, &key);
}

pub fn get_emode_asset(env: &Env, category_id: u32, asset: &Address) -> Option<EModeAssetConfig> {
    let key = ControllerKey::EModeAsset(category_id, asset.clone());
    env.storage().persistent().get(&key)
}

pub fn set_emode_asset(env: &Env, category_id: u32, asset: &Address, config: &EModeAssetConfig) {
    let key = ControllerKey::EModeAsset(category_id, asset.clone());
    env.storage().persistent().set(&key, config);
    bump_shared(env, &key);
}

pub fn remove_emode_asset(env: &Env, category_id: u32, asset: &Address) {
    let key = ControllerKey::EModeAsset(category_id, asset.clone());
    env.storage().persistent().remove(&key);
}

pub fn get_asset_emodes(env: &Env, asset: &Address) -> Vec<u32> {
    let key = ControllerKey::AssetEModes(asset.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn set_asset_emodes(env: &Env, asset: &Address, categories: &Vec<u32>) {
    let key = ControllerKey::AssetEModes(asset.clone());
    env.storage().persistent().set(&key, categories);
    bump_shared(env, &key);
}

// ---------------------------------------------------------------------------
// Persistent storage — Isolated debt
// ---------------------------------------------------------------------------

pub fn get_isolated_debt(env: &Env, asset: &Address) -> i128 {
    let key = ControllerKey::IsolatedDebt(asset.clone());
    env.storage().persistent().get(&key).unwrap_or(0i128)
}

pub fn set_isolated_debt(env: &Env, asset: &Address, debt: i128) {
    let key = ControllerKey::IsolatedDebt(asset.clone());
    env.storage().persistent().set(&key, &debt);
    // IsolatedDebt is global per-asset state (debt-ceiling tracker), not user data.
    bump_shared(env, &key);
}

// ---------------------------------------------------------------------------
// Pools list helpers (indexed by sequential u32 key)
// ---------------------------------------------------------------------------

pub fn get_pools_count(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&ControllerKey::PoolsCount)
        .unwrap_or(0u32)
}

pub fn set_pools_count(env: &Env, count: u32) {
    let key = ControllerKey::PoolsCount;
    env.storage().persistent().set(&key, &count);
    bump_shared(env, &key);
}

#[cfg(test)]
pub fn get_pools_list_entry(env: &Env, idx: u32) -> (Address, Address) {
    let key = ControllerKey::PoolsList(idx);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PoolsListNotFound))
}

pub fn bump_pools_list(env: &Env) {
    let count = get_pools_count(env);
    bump_shared(env, &ControllerKey::PoolsCount);
    for i in 0..count {
        bump_shared(env, &ControllerKey::PoolsList(i));
    }
}

pub fn add_to_pools_list(env: &Env, asset: &Address, pool: &Address) {
    let count = get_pools_count(env);
    let key = ControllerKey::PoolsList(count);
    env.storage()
        .persistent()
        .set(&key, &(asset.clone(), pool.clone()));
    bump_shared(env, &key);
    set_pools_count(env, count + 1);
}

#[cfg(test)]
pub fn set_reflector_config(env: &Env, asset: &Address, config: &ReflectorConfig) {
    let mut market = get_market_config(env, asset);
    market.cex_oracle = Some(config.cex_oracle.clone());
    market.cex_asset_kind = config.cex_asset_kind.clone();
    market.cex_symbol = config.cex_symbol.clone();
    market.cex_decimals = config.cex_decimals;
    market.dex_oracle = config.dex_oracle.clone();
    market.dex_asset_kind = config.dex_asset_kind.clone();
    market.dex_decimals = config.dex_decimals;
    market.twap_records = config.twap_records;
    set_market_config(env, asset, &market);
}

#[cfg(test)]
pub fn set_oracle_config(env: &Env, asset: &Address, config: &OracleProviderConfig) {
    let mut market = get_market_config(env, asset);
    market.oracle_config = config.clone();
    set_market_config(env, asset, &market);
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use common::errors::{EModeError, GenericError};
    use common::types::{
        Account, AssetConfig, MarketConfig, MarketStatus, OraclePriceFluctuation,
        OracleProviderConfig, ReflectorAssetKind,
    };
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, BytesN, Env, Map, Symbol};

    struct TestSetup {
        env: Env,
        contract: Address,
        asset: Address,
    }

    impl TestSetup {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let contract = env.register(crate::Controller, (admin.clone(),));
            let asset = Address::generate(&env);

            Self {
                env,
                contract,
                asset,
            }
        }

        fn as_contract<T>(&self, f: impl FnOnce() -> T) -> T {
            self.env.as_contract(&self.contract, f)
        }

        fn sample_asset_config(&self) -> AssetConfig {
            AssetConfig {
                loan_to_value_bps: 7_500,
                liquidation_threshold_bps: 8_000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                is_collateralizable: true,
                is_borrowable: true,
                e_mode_enabled: true,
                is_isolated_asset: false,
                is_siloed_borrowing: false,
                is_flashloanable: true,
                isolation_borrow_enabled: true,
                isolation_debt_ceiling_usd_wad: 1_000_000,
                flashloan_fee_bps: 9,
                borrow_cap: 2_000_000,
                supply_cap: 3_000_000,
            }
        }

        fn sample_oracle_config(&self) -> OracleProviderConfig {
            OracleProviderConfig {
                base_asset: self.asset.clone(),
                oracle_type: common::types::OracleType::Normal,
                exchange_source: common::types::ExchangeSource::SpotVsTwap,
                asset_decimals: 7,
                tolerance: OraclePriceFluctuation {
                    first_upper_ratio_bps: 10_200,
                    first_lower_ratio_bps: 9_800,
                    last_upper_ratio_bps: 11_000,
                    last_lower_ratio_bps: 9_000,
                },
                max_price_stale_seconds: 900,
            }
        }

        fn sample_market_config(&self) -> MarketConfig {
            MarketConfig {
                status: MarketStatus::Active,
                asset_config: self.sample_asset_config(),
                pool_address: Address::generate(&self.env),
                oracle_config: self.sample_oracle_config(),
                cex_oracle: Some(Address::generate(&self.env)),
                cex_asset_kind: ReflectorAssetKind::Other,
                cex_symbol: Symbol::new(&self.env, "XLM"),
                cex_decimals: 14,
                dex_oracle: Some(Address::generate(&self.env)),
                dex_asset_kind: ReflectorAssetKind::Stellar,
                dex_symbol: Symbol::new(&self.env, ""),
                dex_decimals: 14,
                twap_records: 3,
            }
        }

        fn sample_account(&self) -> Account {
            Account {
                owner: Address::generate(&self.env),
                is_isolated: false,
                e_mode_category_id: 1,
                mode: common::types::PositionMode::Normal,
                isolated_asset: None,
                supply_positions: Map::new(&self.env),
                borrow_positions: Map::new(&self.env),
            }
        }
    }

    #[test]
    fn test_instance_storage_round_trip_and_counters() {
        let t = TestSetup::new();

        t.as_contract(|| {
            let template = BytesN::from_array(&t.env, &[7; 32]);
            let aggregator = Address::generate(&t.env);
            let accumulator = Address::generate(&t.env);
            let limits = PositionLimits {
                max_supply_positions: 6,
                max_borrow_positions: 3,
            };

            set_pool_template(&t.env, &template);
            set_aggregator(&t.env, &aggregator);
            set_accumulator(&t.env, &accumulator);
            set_position_limits(&t.env, &limits);
            set_flash_loan_ongoing(&t.env, true);
            bump_instance(&t.env);

            assert_eq!(get_pool_template(&t.env), template);
            assert_eq!(get_aggregator(&t.env), aggregator);
            assert_eq!(get_accumulator(&t.env), accumulator);
            assert_eq!(get_position_limits(&t.env).max_supply_positions, 6);
            assert!(is_flash_loan_ongoing(&t.env));
            assert!(has_accumulator(&t.env));
            assert_eq!(get_account_nonce(&t.env), 0);
            assert_eq!(increment_account_nonce(&t.env), 1);
            assert_eq!(increment_account_nonce(&t.env), 2);
            assert_eq!(get_last_emode_category_id(&t.env), 0);
            assert_eq!(increment_emode_category_id(&t.env), 1);
            assert_eq!(increment_emode_category_id(&t.env), 2);
        });
    }

    #[test]
    fn test_market_account_and_emode_round_trips() {
        let t = TestSetup::new();

        t.as_contract(|| {
            let market = t.sample_market_config();
            let mut account = t.sample_account();
            let emode = EModeCategory {
                category_id: 1,
                loan_to_value_bps: 8_500,
                liquidation_threshold_bps: 9_000,
                liquidation_bonus_bps: 200,
                is_deprecated: false,
            };
            let emode_asset = EModeAssetConfig {
                is_collateralizable: true,
                is_borrowable: false,
            };
            let categories = Vec::from_array(&t.env, [1u32, 2u32]);

            set_market_config(&t.env, &t.asset, &market);
            set_account(&t.env, 9, &account);
            set_emode_category(&t.env, 1, &emode);
            set_emode_asset(&t.env, 1, &t.asset, &emode_asset);
            set_asset_emodes(&t.env, &t.asset, &categories);
            set_isolated_debt(&t.env, &t.asset, 42);
            add_to_pools_list(&t.env, &t.asset, &market.pool_address);

            assert!(has_market_config(&t.env, &t.asset));
            assert_eq!(
                get_market_config(&t.env, &t.asset).pool_address,
                market.pool_address
            );
            assert_eq!(
                try_get_market_config(&t.env, &t.asset)
                    .unwrap()
                    .oracle_config
                    .oracle_type,
                common::types::OracleType::Normal
            );

            assert_eq!(get_account(&t.env, 9).owner, account.owner);
            account.is_isolated = true;
            set_account(&t.env, 9, &account);
            assert!(try_get_account(&t.env, 9).unwrap().is_isolated);
            bump_account(&t.env, 9);
            remove_account_entry(&t.env, 9);
            assert!(try_get_account(&t.env, 9).is_none());

            assert_eq!(get_emode_category(&t.env, 1).loan_to_value_bps, 8_500);
            assert_eq!(
                try_get_emode_category(&t.env, 1)
                    .unwrap()
                    .liquidation_threshold_bps,
                9_000
            );
            assert!(!get_emode_asset(&t.env, 1, &t.asset).unwrap().is_borrowable);
            assert_eq!(get_asset_emodes(&t.env, &t.asset).len(), 2);
            remove_emode_asset(&t.env, 1, &t.asset);
            assert!(get_emode_asset(&t.env, 1, &t.asset).is_none());

            assert_eq!(get_isolated_debt(&t.env, &t.asset), 42);
            assert_eq!(get_pools_count(&t.env), 1);
            assert_eq!(get_pools_list_entry(&t.env, 0).0, t.asset);
            bump_pools_list(&t.env);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #26)")]
    fn test_get_pool_template_panics_when_missing() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let _ = get_pool_template(&t.env);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #27)")]
    fn test_get_aggregator_panics_when_missing() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let _ = get_aggregator(&t.env);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #28)")]
    fn test_get_accumulator_panics_when_missing() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let _ = get_accumulator(&t.env);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #29)")]
    fn test_get_position_limits_panics_when_missing() {
        let t = TestSetup::new();
        t.as_contract(|| {
            t.env
                .storage()
                .instance()
                .remove(&ControllerKey::PositionLimits);
            let _ = get_position_limits(&t.env);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #1)")]
    fn test_get_market_config_panics_when_missing() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let _ = get_market_config(&t.env, &t.asset);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #24)")]
    fn test_get_account_panics_when_missing() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let _ = get_account(&t.env, 404);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #300)")]
    fn test_get_emode_category_panics_when_missing() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let _ = get_emode_category(&t.env, 77);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #31)")]
    fn test_get_pools_list_entry_panics_when_missing() {
        let t = TestSetup::new();
        t.as_contract(|| {
            let _ = get_pools_list_entry(&t.env, 0);
        });
    }

    #[test]
    fn test_error_codes_match_expected_contract_ranges() {
        assert_eq!(GenericError::TemplateNotSet as u32, 26);
        assert_eq!(EModeError::EModeCategoryNotFound as u32, 300);
    }
}
