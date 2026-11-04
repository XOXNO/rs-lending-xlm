//! Aquarius pool queries, share checks, and shared swap invoke.

use soroban_sdk::{panic_with_error, symbol_short, vec, Address, Env, IntoVal, Symbol, Val, Vec};

use crate::errors::Error;
use crate::venues::auth::authorize_token_transfer;

/// Authorizes the input transfer and calls the pool's `swap` with a zero minimum
/// output. Panics with `IntegerOverflow` if `amount_in` is negative.
///
/// Decodes the pool's reported fill as `u128`, so a wrong return type fails, then
/// discards it: `crate::venues::dispatch_hop` measures the fill.
pub(super) fn invoke_pool_swap(
    env: &Env,
    router: &Address,
    pool: &Address,
    token_in: &Address,
    in_idx: u32,
    out_idx: u32,
    amount_in: i128,
) {
    authorize_token_transfer(env, token_in, router, pool, amount_in);
    let args: Vec<Val> = vec![
        env,
        router.into_val(env),
        in_idx.into_val(env),
        out_idx.into_val(env),
        to_u128(env, amount_in).into_val(env),
        0_u128.into_val(env),
    ];
    let _: u128 = env.invoke_contract(pool, &symbol_short!("swap"), args);
}

/// Reads the pool's constituent tokens; [`crate::venues::pool_tokens`] validates and caches
/// them.
pub(super) fn get_tokens(env: &Env, pool: &Address) -> Vec<Address> {
    env.invoke_contract(pool, &Symbol::new(env, "get_tokens"), Vec::<Val>::new(env))
}

/// Validates that `lp_token` is the pool's share token. Panics with
/// `LpTokenMismatch` if it is not.
pub(super) fn assert_share_token(env: &Env, pool: &Address, lp_token: &Address) {
    let share: Address =
        env.invoke_contract(pool, &Symbol::new(env, "share_id"), Vec::<Val>::new(env));
    if share != *lp_token {
        panic_with_error!(env, Error::LpTokenMismatch);
    }
}

/// Converts `amount` from `i128` to `u128`. Panics with `IntegerOverflow` if
/// `amount` is negative.
pub(super) fn to_u128(env: &Env, amount: i128) -> u128 {
    amount
        .try_into()
        .unwrap_or_else(|_| panic_with_error!(env, Error::IntegerOverflow))
}

#[cfg(test)]
#[path = "../../../tests/unit/venues/aquarius_pool_metadata.rs"]
mod tests;
