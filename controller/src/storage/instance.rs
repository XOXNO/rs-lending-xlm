use common::errors::GenericError;
use common::types::{ControllerKey, PositionLimits};
use soroban_sdk::{contracttype, panic_with_error, Address, BytesN, Env};

#[contracttype]
#[derive(Clone, Debug)]
enum LocalKey {
    ApprovedToken(Address),
}

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

pub fn has_accumulator(env: &Env) -> bool {
    env.storage().instance().has(&ControllerKey::Accumulator)
}

pub fn get_account_nonce(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&ControllerKey::AccountNonce)
        .unwrap_or(0u64)
}

pub fn increment_account_nonce(env: &Env) -> u64 {
    let current = get_account_nonce(env);
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
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
    let next = current
        .checked_add(1)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    env.storage()
        .instance()
        .set(&ControllerKey::LastEModeCategoryId, &next);
    next
}

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
