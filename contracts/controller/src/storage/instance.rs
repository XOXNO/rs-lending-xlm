//! Instance, temporary, and protocol-shared storage for non-market controller
//! state. `ApprovedToken` is a one-use instance allow-list for pool creation.
//! `FlashLoanOngoing` blocks re-entrant controller mutations during callbacks.
//! `AssetOracle` is the token-rooted oracle config on the protocol-shared tier.

use crate::constants;
use common::errors::GenericError;
use controller_interface::types::{ControllerKey, HubConfig, MarketOracleConfig, PositionLimits};
use soroban_sdk::{assert_with_error, contracttype, panic_with_error, Address, BytesN, Env};

/// Cap on outstanding (approved but not yet consumed) token approvals.
/// Each instance key loads with each invocation, so unconsumed approvals
/// must not accumulate without bound.
const MAX_OUTSTANDING_TOKEN_APPROVALS: u32 = 16;

/// Cap on approved Blend migration source pools. Instance keys load on every
/// invocation, so the allow-list must stay bounded.
const MAX_APPROVED_BLEND_POOLS: u32 = 16;

#[contracttype]
#[derive(Clone, Debug)]
enum LocalKey {
    ApprovedToken(Address),
    ApprovedTokenCount,
    BlendPoolAllowed(Address),
    BlendPoolAllowedCount,
}

#[contracttype]
#[derive(Clone, Debug)]
enum SessionKey {
    FlashLoanOngoing,
}

pub(crate) fn is_token_approved(env: &Env, token: &Address) -> bool {
    env.storage()
        .instance()
        .get(&LocalKey::ApprovedToken(token.clone()))
        .unwrap_or(false)
}

fn approved_token_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&LocalKey::ApprovedTokenCount)
        .unwrap_or(0u32)
}

pub(crate) fn set_token_approved(env: &Env, token: &Address, approved: bool) {
    let key = LocalKey::ApprovedToken(token.clone());
    let already_approved: bool = env.storage().instance().get(&key).unwrap_or(false);

    if approved {
        if !already_approved {
            let count = approved_token_count(env);
            assert_with_error!(
                env,
                count < MAX_OUTSTANDING_TOKEN_APPROVALS,
                GenericError::InvalidPositionLimits
            );
            env.storage()
                .instance()
                .set(&LocalKey::ApprovedTokenCount, &(count + 1));
        }
        env.storage().instance().set(&key, &true);
    } else {
        if already_approved {
            // Saturate at zero: entries approved before counter bookkeeping
            // existed must still be revocable/consumable.
            let count = approved_token_count(env).saturating_sub(1);
            env.storage()
                .instance()
                .set(&LocalKey::ApprovedTokenCount, &count);
        }
        env.storage().instance().remove(&key);
    }
}

pub(crate) fn is_blend_pool_approved(env: &Env, pool: &Address) -> bool {
    env.storage()
        .instance()
        .get(&LocalKey::BlendPoolAllowed(pool.clone()))
        .unwrap_or(false)
}

fn approved_blend_pool_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&LocalKey::BlendPoolAllowedCount)
        .unwrap_or(0u32)
}

pub(crate) fn set_blend_pool_approved(env: &Env, pool: &Address, approved: bool) {
    let key = LocalKey::BlendPoolAllowed(pool.clone());
    let already_approved: bool = env.storage().instance().get(&key).unwrap_or(false);

    if approved {
        if !already_approved {
            let count = approved_blend_pool_count(env);
            assert_with_error!(
                env,
                count < MAX_APPROVED_BLEND_POOLS,
                GenericError::InvalidPositionLimits
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

pub(crate) fn get_pool_template(env: &Env) -> BytesN<32> {
    env.storage()
        .instance()
        .get(&ControllerKey::PoolTemplate)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::TemplateNotSet))
}

pub(crate) fn set_pool_template(env: &Env, hash: &BytesN<32>) {
    env.storage()
        .instance()
        .set(&ControllerKey::PoolTemplate, hash);
}

pub(crate) fn get_pool(env: &Env) -> Address {
    try_get_pool(env).unwrap_or_else(|| panic_with_error!(env, GenericError::PoolNotInitialized))
}

pub(crate) fn try_get_pool(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::Pool)
}

pub(crate) fn set_pool(env: &Env, addr: &Address) {
    env.storage().instance().set(&ControllerKey::Pool, addr);
}

pub(crate) fn get_aggregator(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&ControllerKey::Aggregator)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::AggregatorNotSet))
}

pub(crate) fn set_aggregator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::Aggregator, addr);
}

pub(crate) fn try_get_accumulator(env: &Env) -> Option<Address> {
    env.storage().instance().get(&ControllerKey::Accumulator)
}

pub(crate) fn set_accumulator(env: &Env, addr: &Address) {
    env.storage()
        .instance()
        .set(&ControllerKey::Accumulator, addr);
}

/// Token-rooted oracle config under `AssetOracle(asset)`. Persistent
/// protocol-shared tier, mirroring the `Market` key's TTL so the two never
/// archive on divergent schedules while both hold the oracle config.
pub(crate) fn get_asset_oracle(env: &Env, asset: &Address) -> Option<MarketOracleConfig> {
    let key = ControllerKey::AssetOracle(asset.clone());
    let config: Option<MarketOracleConfig> = env.storage().persistent().get(&key);
    if config.is_some() {
        super::renew_protocol_shared_key(env, &key);
    }
    config
}

pub(crate) fn set_asset_oracle(env: &Env, asset: &Address, config: &MarketOracleConfig) {
    let key = ControllerKey::AssetOracle(asset.clone());
    env.storage().persistent().set(&key, config);
    super::renew_protocol_shared_key(env, &key);
}

/// Removes the token-rooted oracle config. Absence is the protocol's
/// disabled/pending signal: price resolution and `require_market_active` reject
/// assets with no `AssetOracle` entry.
pub(crate) fn remove_asset_oracle(env: &Env, asset: &Address) {
    env.storage()
        .persistent()
        .remove(&ControllerKey::AssetOracle(asset.clone()));
}

pub(crate) fn get_account_nonce(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&ControllerKey::AccountNonce)
        .unwrap_or(0u64)
}

pub(crate) fn increment_account_nonce(env: &Env) -> u64 {
    let current = get_account_nonce(env);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage()
        .instance()
        .set(&ControllerKey::AccountNonce, &next);
    next
}

pub(crate) fn get_position_limits(env: &Env) -> PositionLimits {
    env.storage()
        .instance()
        .get(&ControllerKey::PositionLimits)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::PositionLimitsNotSet))
}

pub(crate) fn set_position_limits(env: &Env, limits: &PositionLimits) {
    env.storage()
        .instance()
        .set(&ControllerKey::PositionLimits, limits);
}

pub(crate) fn get_last_spoke_id(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&ControllerKey::LastSpokeId)
        .unwrap_or(0u32)
}

pub(crate) fn get_min_borrow_collateral_usd_wad(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&ControllerKey::MinBorrowCollateralUsd)
        .unwrap_or(constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD)
}

pub(crate) fn set_min_borrow_collateral_usd_wad(env: &Env, floor_wad: i128) {
    env.storage()
        .instance()
        .set(&ControllerKey::MinBorrowCollateralUsd, &floor_wad);
}

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

pub(crate) fn get_last_hub_id(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&ControllerKey::LastHubId)
        .unwrap_or(0u32)
}

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

/// Reads a hub registry entry. Hub 0 is the implicit default and is never
/// stored, so this returns `None` for it; `require_hub_active` treats that
/// absence as always-active.
pub(crate) fn get_hub(env: &Env, hub_id: u32) -> Option<HubConfig> {
    env.storage().instance().get(&ControllerKey::Hub(hub_id))
}

pub(crate) fn set_hub(env: &Env, hub_id: u32, config: &HubConfig) {
    env.storage()
        .instance()
        .set(&ControllerKey::Hub(hub_id), config);
}

pub(crate) fn is_flash_loan_ongoing(env: &Env) -> bool {
    env.storage()
        .temporary()
        .get(&SessionKey::FlashLoanOngoing)
        .unwrap_or(false)
}

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
