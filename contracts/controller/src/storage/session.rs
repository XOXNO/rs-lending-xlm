//! Temporary-storage tracking of whether a flash loan is currently in progress within the
//! executing transaction.

use soroban_sdk::{contracttype, Env};

/// Storage key for the flash-loan-ongoing flag, held in temporary storage so it does not
/// persist beyond the current ledger.
#[contracttype]
#[derive(Clone, Debug)]
enum SessionKey {
    FlashLoanOngoing,
}

/// Returns whether a flash loan is currently marked as ongoing. Defaults to `false` if unset.
pub(crate) fn is_flash_loan_ongoing(env: &Env) -> bool {
    env.storage()
        .temporary()
        .get(&SessionKey::FlashLoanOngoing)
        .unwrap_or(false)
}

/// Sets the flash-loan-ongoing flag. Setting `ongoing` to `false` removes the storage
/// key instead of storing a negative flag.
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

/// Runs `f` with the flash-loan-ongoing flag set to `true`. If the flag was already set
/// before the call, leaves it set to `true` after `f` returns; otherwise clears it.
pub(crate) fn with_flash_guard<T>(env: &Env, f: impl FnOnce() -> T) -> T {
    let prev = is_flash_loan_ongoing(env);
    set_flash_loan_ongoing(env, true);
    let out = f();
    if !prev {
        set_flash_loan_ongoing(env, false);
    }
    out
}
