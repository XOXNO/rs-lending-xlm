//! Aquarius LP withdraw (burn).

use soroban_sdk::{panic_with_error, token, Address, Env, IntoVal, Map, Symbol, Vec};

use crate::errors::Error;
use crate::vault::Vault;
use crate::venues::aquarius::pool::{assert_share_token, pool_tokens, to_u128};
use crate::venues::auth::authorize_as_current;

/// Burn vault LP shares; credit measured constituent amounts (mins enforced).
///
/// Floors are read from `amounts[min_start .. min_start + n]`, in the pool's own
/// token order, where `n` is the pool's constituent count.
#[allow(clippy::too_many_arguments)]
pub(crate) fn remove_liquidity(
    env: &Env,
    router: &Address,
    vault: &mut Vault,
    pool: &Address,
    lp_token: &Address,
    amounts: &Vec<i128>,
    min_start: u32,
    cache: &mut Map<Address, Vec<Address>>,
) {
    let tokens = pool_tokens(env, cache, pool);
    assert_share_token(env, pool, lp_token);

    let n = tokens.len();
    // The registry must carry a full floor run for this pool's arity.
    if min_start
        .checked_add(n)
        .is_none_or(|end| end > amounts.len())
    {
        panic_with_error!(env, Error::MinAmountsNotMet);
    }

    let shares = vault.balance_of(lp_token);
    if shares <= 0 {
        panic_with_error!(env, Error::InvalidAmount);
    }

    let mut min_amounts: Vec<u128> = Vec::new(env);
    let mut before: Vec<i128> = Vec::new(env);
    for i in 0..n {
        min_amounts.push_back(to_u128(env, amounts.get_unchecked(min_start + i)));
        before.push_back(token::Client::new(env, &tokens.get_unchecked(i)).balance(router));
    }
    let shares_before = token::Client::new(env, lp_token).balance(router);

    authorize_as_current(
        env,
        lp_token,
        "burn",
        soroban_sdk::vec![env, router.into_val(env), shares.into_val(env)],
    );
    let _: Vec<u128> = env.invoke_contract(
        pool,
        &Symbol::new(env, "withdraw"),
        soroban_sdk::vec![
            env,
            router.into_val(env),
            to_u128(env, shares).into_val(env),
            min_amounts.into_val(env),
        ],
    );

    let burned = shares_before
        .checked_sub(token::Client::new(env, lp_token).balance(router))
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
    if burned != shares {
        panic_with_error!(env, Error::InvalidAmount);
    }
    vault.withdraw(lp_token, shares);

    let mut total_received: i128 = 0;
    for i in 0..n {
        let token = tokens.get_unchecked(i);
        let received = token::Client::new(env, &token)
            .balance(router)
            .checked_sub(before.get_unchecked(i))
            .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
        if received < amounts.get_unchecked(min_start + i) {
            panic_with_error!(env, Error::MinAmountsNotMet);
        }
        total_received = total_received
            .checked_add(received)
            .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
        vault.deposit(&token, received);
    }
    if total_received <= 0 {
        panic_with_error!(env, Error::ZeroOutput);
    }
}
