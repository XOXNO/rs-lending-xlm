//! Instance and temporary storage for non-market controller state.
//!
//! `ApprovedToken` is a one-use instance allow-list for pool creation.
//! `FlashLoanOngoing` is a temporary transaction guard that blocks re-entrant
//! controller mutations during flash-loan and strategy callbacks.

use common::errors::GenericError;
use controller_interface::types::{ControllerKey, PositionLimits};
use soroban_sdk::{assert_with_error, contracttype, panic_with_error, Address, BytesN, Env};

/// Cap on outstanding (approved but not yet consumed) token approvals.
/// Each instance key loads with each invocation, so unconsumed approvals
/// must not accumulate without bound.
const MAX_OUTSTANDING_TOKEN_APPROVALS: u32 = 16;

#[contracttype]
#[derive(Clone, Debug)]
enum LocalKey {
    ApprovedToken(Address),
    ApprovedTokenCount,
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

pub(crate) fn get_min_borrow_collateral_usd_wad(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&ControllerKey::MinBorrowCollateralUsd)
        .unwrap_or(crate::constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD)
}

pub(crate) fn set_min_borrow_collateral_usd_wad(env: &Env, floor_wad: i128) {
    env.storage()
        .instance()
        .set(&ControllerKey::MinBorrowCollateralUsd, &floor_wad);
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

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    // Approve/revoke/consume keeps the outstanding counter exact: re-approval
    // of the same token cannot double-count, and revocation frees a slot.
    #[test]
    fn test_token_approval_counter_tracks_outstanding_set() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(crate::Controller, (admin,));
        env.as_contract(&contract_id, || {
            let token = Address::generate(&env);
            set_token_approved(&env, &token, true);
            set_token_approved(&env, &token, true); // idempotent re-approve
            assert_eq!(approved_token_count(&env), 1);
            assert!(is_token_approved(&env, &token));

            set_token_approved(&env, &token, false);
            assert_eq!(approved_token_count(&env), 0);
            assert!(!is_token_approved(&env, &token));

            // Revoking an unapproved token cannot underflow the counter.
            set_token_approved(&env, &token, false);
            assert_eq!(approved_token_count(&env), 0);
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #36)")]
    fn test_token_approval_cap_rejects_overflowing_approval() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(crate::Controller, (admin,));
        env.as_contract(&contract_id, || {
            for _ in 0..MAX_OUTSTANDING_TOKEN_APPROVALS {
                set_token_approved(&env, &Address::generate(&env), true);
            }
            set_token_approved(&env, &Address::generate(&env), true);
        });
    }
}
