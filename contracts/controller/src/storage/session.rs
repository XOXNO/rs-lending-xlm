//! Temporary session flags (flash-loan / strategy reentrancy guard).

use soroban_sdk::{contracttype, Env};

#[contracttype]
#[derive(Clone, Debug)]
enum SessionKey {
    FlashLoanOngoing,
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

pub(crate) fn with_flash_guard<T>(env: &Env, f: impl FnOnce() -> T) -> T {
    let prev = is_flash_loan_ongoing(env);
    set_flash_loan_ongoing(env, true);
    let out = f();
    if !prev {
        set_flash_loan_ongoing(env, false);
    }
    out
}
