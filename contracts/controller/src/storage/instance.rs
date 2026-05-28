//! Instance and temporary storage for non-market controller state.
//!
//! `ApprovedToken` is a one-use instance allow-list for pool creation.
//! `FlashLoanOngoing` is a temporary transaction guard that blocks re-entrant
//! controller mutations during flash-loan and strategy callbacks.

use common::errors::GenericError;
use common::types::{ControllerKey, PositionLimits};
use soroban_sdk::{contracttype, panic_with_error, Address, BytesN, Env};

#[contracttype]
#[derive(Clone, Debug)]
enum LocalKey {
    ApprovedToken(Address),
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

pub(crate) fn set_token_approved(env: &Env, token: &Address, approved: bool) {
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

pub(crate) fn get_last_emode_category_id(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&ControllerKey::LastEModeCategoryId)
        .unwrap_or(0u32)
}

pub(crate) fn increment_emode_category_id(env: &Env) -> u32 {
    let current = get_last_emode_category_id(env);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage()
        .instance()
        .set(&ControllerKey::LastEModeCategoryId, &next);
    next
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
