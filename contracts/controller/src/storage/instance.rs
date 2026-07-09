//! Non-market controller storage.

use crate::constants;
use crate::storage::renew_protocol_shared_key;
use common::errors::GenericError;
use common::types::{
    ControllerKey, HubConfig, MarketOracleConfig, PositionLimits, PositionManagerConfig,
};
use soroban_sdk::{assert_with_error, contracttype, panic_with_error, Address, BytesN, Env};

/// Cap on unconsumed token approvals.
const MAX_OUTSTANDING_TOKEN_APPROVALS: u32 = 16;

/// Cap on approved Blend migration pools.
const MAX_APPROVED_BLEND_POOLS: u32 = 16;

/// Cap on registered position managers.
const MAX_POSITION_MANAGERS: u32 = 16;

#[contracttype]
#[derive(Clone, Debug)]
enum LocalKey {
    ApprovedToken(Address),
    ApprovedTokenCount,
    BlendPoolAllowed(Address),
    BlendPoolAllowedCount,
    PositionManagerCount,
}

#[contracttype]
#[derive(Clone, Debug)]
enum SessionKey {
    FlashLoanOngoing,
}

/// Returns whether `token` is on the outbound-approval allowlist.
pub(crate) fn is_token_approved(env: &Env, token: &Address) -> bool {
    env.storage()
        .instance()
        .get(&LocalKey::ApprovedToken(token.clone()))
        .unwrap_or(false)
}

/// Returns the number of currently approved tokens.
fn approved_token_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&LocalKey::ApprovedTokenCount)
        .unwrap_or(0u32)
}

/// Adds or removes `token` from the approval allowlist, maintaining the capped count.
pub(crate) fn set_token_approved(env: &Env, token: &Address, approved: bool) {
    let key = LocalKey::ApprovedToken(token.clone());
    let already_approved: bool = env.storage().instance().get(&key).unwrap_or(false);

    if approved {
        if !already_approved {
            let count = approved_token_count(env);
            assert_with_error!(
                env,
                count < MAX_OUTSTANDING_TOKEN_APPROVALS,
                GenericError::RegistryCapReached
            );
            env.storage()
                .instance()
                .set(&LocalKey::ApprovedTokenCount, &(count + 1));
        }
        env.storage().instance().set(&key, &true);
    } else {
        if already_approved {
            // Saturate at zero: an entry approved before this counter existed
            // (pre-upgrade state) must still be revocable without underflowing.
            let count = approved_token_count(env).saturating_sub(1);
            env.storage()
                .instance()
                .set(&LocalKey::ApprovedTokenCount, &count);
        }
        env.storage().instance().remove(&key);
    }
}

/// Returns whether `pool` is an approved Blend migration source.
pub(crate) fn is_blend_pool_approved(env: &Env, pool: &Address) -> bool {
    env.storage()
        .instance()
        .get(&LocalKey::BlendPoolAllowed(pool.clone()))
        .unwrap_or(false)
}

/// Returns the number of currently approved Blend pools.
fn approved_blend_pool_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&LocalKey::BlendPoolAllowedCount)
        .unwrap_or(0u32)
}

/// Adds or removes `pool` from the Blend migration allowlist, maintaining the capped count.
pub(crate) fn set_blend_pool_approved(env: &Env, pool: &Address, approved: bool) {
    let key = LocalKey::BlendPoolAllowed(pool.clone());
    let already_approved: bool = env.storage().instance().get(&key).unwrap_or(false);

    if approved {
        if !already_approved {
            let count = approved_blend_pool_count(env);
            assert_with_error!(
                env,
                count < MAX_APPROVED_BLEND_POOLS,
                GenericError::RegistryCapReached
            );
            env.storage()
                .instance()
                .set(&LocalKey::BlendPoolAllowedCount, &(count + 1));
        }
        env.storage().instance().set(&key, &true);
    } else {
        if already_approved {
            let count = approved_blend_pool_count(env).saturating_sub(1);
            env.storage()
                .instance()
                .set(&LocalKey::BlendPoolAllowedCount, &count);
        }
        env.storage().instance().remove(&key);
    }
}

/// Returns the pool WASM template hash, panicking if unset.
pub(crate) fn get_pool_template(env: &Env) -> BytesN<32> {
    env.storage()
        .instance()
        .get(&ControllerKey::PoolTemplate)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::TemplateNotSet))
}

/// Stores the pool WASM template hash used to deploy new pools.
pub(crate) fn set_pool_template(env: &Env, hash: &BytesN<32>) {
    env.storage()
        .instance()
        .set(&ControllerKey::PoolTemplate, hash);
}

/// Returns the pool address, panicking if the pool is not yet initialized.
pub(crate) fn get_pool(env: &Env) -> Address {
    try_get_pool(env).unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

/// Returns the pool address if set.
pub(crate) fn try_get_pool(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::Pool)
}

/// Stores the pool contract address.
pub(crate) fn set_pool(env: &Env, addr: &Address) {
    env.storage().instance().set(&ControllerKey::Pool, addr);
}

/// Returns the aggregator address, panicking if unset.
pub(crate) fn get_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&ControllerKey::Aggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

/// Stores the aggregator contract address.
pub(crate) fn set_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::Aggregator, addr);
}

/// Returns the fee-accumulator address if set.
pub(crate) fn try_get_accumulator(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::Accumulator)
}

/// Stores the fee-accumulator contract address.
pub(crate) fn set_accumulator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::Accumulator, addr);
}

/// Token-rooted oracle config under `AssetOracle(asset)`. Persistent,
/// protocol-shared tier, renewed via `renew_protocol_shared_key` like the
/// rest of that tier.
pub(crate) fn get_asset_oracle(env: &Env, asset: &Address) -> Option<MarketOracleConfig> {
    let key = ControllerKey::AssetOracle(asset.clone());
    let config: Option<MarketOracleConfig> = env.storage().persistent().get(&key);
    if config.is_some() {
        renew_protocol_shared_key(env, &key);
    }
    config
}

/// Stores the token-rooted oracle config and renews its shared-tier TTL.
pub(crate) fn set_asset_oracle(env: &Env, asset: &Address, config: &MarketOracleConfig) {
    let key = ControllerKey::AssetOracle(asset.clone());
    env.storage().persistent().set(&key, config);
    renew_protocol_shared_key(env, &key);
}

// Persistent, not instance: the nonce changes on every account creation, and
// an instance write rewrites (and re-rents) the whole instance envelope.
/// Returns the current account-id nonce, or 0 before any account is created.
pub(crate) fn get_account_nonce(env: &Env) -> u64 {
    env.storage()
        .persistent()
        .get(&ControllerKey::AccountNonce)
        .unwrap_or(0u64)
}

/// Increments and returns the next account-id nonce, panicking on overflow.
pub(crate) fn increment_account_nonce(env: &Env) -> u64 {
    let current = get_account_nonce(env);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let key = ControllerKey::AccountNonce;
    env.storage().persistent().set(&key, &next);
    renew_protocol_shared_key(env, &key);
    next
}

/// Returns the configured position limits, panicking if unset.
pub(crate) fn get_position_limits(env: &Env) -> PositionLimits {
    env.storage()
        .instance()
        .get(&ControllerKey::PositionLimits)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PositionLimitsNotSet))
}

/// Stores the position limits config.
pub(crate) fn set_position_limits(env: &Env, limits: &PositionLimits) {
    env.storage()
        .instance()
        .set(&ControllerKey::PositionLimits, limits);
}

/// Returns the highest allocated spoke id, or 0 when none exist.
pub(crate) fn get_last_spoke_id(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&ControllerKey::LastSpokeId)
        .unwrap_or(0u32)
}

/// Returns the minimum borrow-collateral USD floor, defaulting to the constant when unset.
pub(crate) fn get_min_borrow_collateral_usd_wad(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&ControllerKey::MinBorrowCollateralUsd)
        .unwrap_or(constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD)
}

/// Stores the minimum borrow-collateral USD floor.
pub(crate) fn set_min_borrow_collateral_usd_wad(env: &Env, floor_wad: i128) {
    env.storage()
        .instance()
        .set(&ControllerKey::MinBorrowCollateralUsd, &floor_wad);
}

/// Allocates and returns the next spoke id, panicking on overflow.
pub(crate) fn increment_spoke_id(env: &Env) -> u32 {
    let current = get_last_spoke_id(env);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage()
        .instance()
        .set(&ControllerKey::LastSpokeId, &next);
    next
}

/// Returns the highest allocated hub id, or 0 when none exist.
pub(crate) fn get_last_hub_id(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&ControllerKey::LastHubId)
        .unwrap_or(0u32)
}

/// Allocates and returns the next hub id, panicking on overflow.
pub(crate) fn increment_hub_id(env: &Env) -> u32 {
    let current = get_last_hub_id(env);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage()
        .instance()
        .set(&ControllerKey::LastHubId, &next);
    next
}

/// Reads a hub registry entry. No hub is seeded; hubs are created on demand
/// (ids from 1). Returns `None` for any uncreated id.
/// Persistent, not instance: the registry grows with the hub count, and the
/// instance envelope is read (and rent-extended) on every invocation.
pub(crate) fn get_hub(env: &Env, hub_id: u32) -> Option<HubConfig> {
    let key = ControllerKey::Hub(hub_id);
    let hub: Option<HubConfig> = env.storage().persistent().get(&key);
    // Read-renewal policy: active hubs must not archive while markets use them.
    if hub.is_some() {
        renew_protocol_shared_key(env, &key);
    }
    hub
}

/// Stores a hub registry entry and renews its shared-tier TTL.
pub(crate) fn set_hub(env: &Env, hub_id: u32, config: &HubConfig) {
    let key = ControllerKey::Hub(hub_id);
    env.storage().persistent().set(&key, config);
    renew_protocol_shared_key(env, &key);
}

/// Reads a position-manager registry entry. Absence means the address is not a
/// registered manager; `require_owner_or_delegate` then grants it no access.
pub(crate) fn get_position_manager(env: &Env, addr: &Address) -> Option<PositionManagerConfig> {
    env.storage()
        .instance()
        .get(&ControllerKey::PositionManager(addr.clone()))
}

/// Returns the number of active position managers.
fn position_manager_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&LocalKey::PositionManagerCount)
        .unwrap_or(0u32)
}

/// Capped registry of active managers; deactivation removes the entry
/// (absence == inactive for the delegate-auth check).
pub(crate) fn set_position_manager(env: &Env, addr: &Address, config: &PositionManagerConfig) {
    let key = ControllerKey::PositionManager(addr.clone());
    let already_registered = env.storage().instance().has(&key);

    if config.is_active {
        if !already_registered {
            let count = position_manager_count(env);
            assert_with_error!(
                env,
                count < MAX_POSITION_MANAGERS,
                GenericError::RegistryCapReached
            );
            env.storage()
                .instance()
                .set(&LocalKey::PositionManagerCount, &(count + 1));
        }
        env.storage().instance().set(&key, config);
    } else {
        if already_registered {
            // Saturate at zero: an entry registered before this counter existed
            // (pre-upgrade state) must still be deactivatable without underflowing.
            let count = position_manager_count(env).saturating_sub(1);
            env.storage()
                .instance()
                .set(&LocalKey::PositionManagerCount, &count);
        }
        env.storage().instance().remove(&key);
    }
}

/// Returns whether a flash loan is currently in progress.
pub(crate) fn is_flash_loan_ongoing(env: &Env) -> bool {
    env.storage()
        .temporary()
        .get(&SessionKey::FlashLoanOngoing)
        .unwrap_or(false)
}

/// Sets or clears the flash-loan reentrancy flag.
pub(crate) fn set_flash_loan_ongoing(env: &Env, ongoing: bool) {
    if ongoing {
        env.storage()
            .temporary()
            .set(&SessionKey::FlashLoanOngoing, &true);
    } else {
        env.storage()
            .temporary()
            .remove(&SessionKey::FlashLoanOngoing);
    }
}

/// Runs `f` with the flash-loan reentrancy flag set, then restores the saved value.
pub(crate) fn with_flash_guard<T>(env: &Env, f: impl FnOnce() -> T) -> T {
    let prev = is_flash_loan_ongoing(env);
    set_flash_loan_ongoing(env, true);
    let out = f();
    if !prev {
        set_flash_loan_ongoing(env, false);
    }
    out
}

#[cfg(test)]
#[path = "../../tests/storage/instance.rs"]
mod tests;
