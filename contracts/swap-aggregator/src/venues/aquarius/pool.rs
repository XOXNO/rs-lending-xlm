//! Aquarius pool queries, share checks, and shared swap invoke.

use soroban_sdk::{panic_with_error, symbol_short, Address, Env, IntoVal, Map, Symbol, Val, Vec};

use crate::errors::Error;
use crate::venues::auth::authorize_token_transfer;

/// Authorize transfer and call pool `swap`; returns reported out amount.
pub(super) fn invoke_pool_swap(
    env: &Env,
    router: &Address,
    pool: &Address,
    token_in: &Address,
    in_idx: u32,
    out_idx: u32,
    amount_in: i128,
) -> u128 {
    authorize_token_transfer(env, token_in, router, pool, amount_in);
    let args: Vec<Val> = soroban_sdk::vec![
        env,
        router.into_val(env),
        in_idx.into_val(env),
        out_idx.into_val(env),
        to_u128(env, amount_in).into_val(env),
        0_u128.into_val(env),
    ];
    env.invoke_contract(pool, &symbol_short!("swap"), args)
}

/// Pool constituent tokens, cached per invocation in `cache`.
pub(super) fn pool_tokens(
    env: &Env,
    cache: &mut Map<Address, Vec<Address>>,
    pool: &Address,
) -> Vec<Address> {
    if let Some(tokens) = cache.get(pool.clone()) {
        return tokens;
    }
    let tokens: Vec<Address> =
        env.invoke_contract(pool, &Symbol::new(env, "get_tokens"), Vec::<Val>::new(env));
    if tokens.is_empty() {
        panic_with_error!(env, Error::BrokenTokenChain);
    }
    cache.set(pool.clone(), tokens.clone());
    tokens
}

/// Require `lp_token` is the pool share token.
pub(super) fn assert_share_token(env: &Env, pool: &Address, lp_token: &Address) {
    let share: Address =
        env.invoke_contract(pool, &Symbol::new(env, "share_id"), Vec::<Val>::new(env));
    if share != *lp_token {
        panic_with_error!(env, Error::LpTokenMismatch);
    }
}

pub(super) fn to_u128(env: &Env, amount: i128) -> u128 {
    amount
        .try_into()
        .unwrap_or_else(|_| panic_with_error!(env, Error::IntegerOverflow))
}

/// Index of `target` in `tokens`, or panic.
pub(super) fn find_index(env: &Env, tokens: &Vec<Address>, target: &Address) -> u32 {
    let n = tokens.len();
    for i in 0..n {
        if tokens.get(i).as_ref() == Some(target) {
            return i;
        }
    }
    panic_with_error!(env, Error::BrokenTokenChain);
}
