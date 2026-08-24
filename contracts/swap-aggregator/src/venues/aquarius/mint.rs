//! Aquarius LP deposit (mint).

use soroban_sdk::{
    auth::InvokerContractAuthEntry, panic_with_error, token, vec, Address, Env, IntoVal, Map,
    Symbol, Vec,
};

use crate::errors::Error;
use crate::vault::Vault;
use crate::venues::aquarius::pool::{assert_share_token, pool_tokens, to_u128};
use crate::venues::auth::auth_entry;

/// Deposits the vault's full balance of each pool constituent token into the pool
/// and credits the vault with the measured LP shares received. Returns the minted
/// share amount.
///
/// Panics if `min_shares` is not positive, if the vault holds none of the pool's
/// constituent tokens, or if the shares received fall below `min_shares`.
pub(crate) fn add_liquidity(
    env: &Env,
    router: &Address,
    vault: &mut Vault,
    pool: &Address,
    lp_token: &Address,
    min_shares: i128,
    cache: &mut Map<Address, Vec<Address>>,
) -> i128 {
    let tokens = pool_tokens(env, cache, pool);
    assert_share_token(env, pool, lp_token);
    if min_shares <= 0 {
        panic_with_error!(env, Error::MinSharesNotMet);
    }

    let n = tokens.len();
    let mut amounts: Vec<u128> = Vec::new(env);
    let mut before: Vec<i128> = Vec::new(env);
    let mut held: Vec<i128> = Vec::new(env);
    let mut total_requested: i128 = 0;
    for i in 0..n {
        let token = tokens.get_unchecked(i);
        let amount = vault.balance_of(&token);
        amounts.push_back(to_u128(env, amount));
        held.push_back(amount);

        before.push_back(if amount > 0 {
            token::Client::new(env, &token).balance(router)
        } else {
            0
        });
        total_requested = total_requested
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
    }
    if total_requested <= 0 {
        panic_with_error!(env, Error::InvalidAmount);
    }

    let mut auth: Vec<InvokerContractAuthEntry> = Vec::new(env);
    for i in 0..n {
        let amount = held.get_unchecked(i);

        if amount > 0 {
            auth.push_back(auth_entry(
                env,
                &tokens.get_unchecked(i),
                "transfer",
                vec![
                    env,
                    router.into_val(env),
                    pool.into_val(env),
                    amount.into_val(env),
                ],
                vec![env],
            ));
        }
    }

    let shares_before = token::Client::new(env, lp_token).balance(router);

    env.authorize_as_current_contract(auth);
    let _: (Vec<u128>, u128) = env.invoke_contract(
        pool,
        &Symbol::new(env, "deposit"),
        vec![
            env,
            router.into_val(env),
            amounts.into_val(env),
            to_u128(env, min_shares).into_val(env),
        ],
    );

    for i in 0..n {
        if held.get_unchecked(i) == 0 {
            continue;
        }
        let token = tokens.get_unchecked(i);
        let spent = before
            .get_unchecked(i)
            .checked_sub(token::Client::new(env, &token).balance(router))
            .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));

        if spent < 0 {
            panic_with_error!(env, Error::InvalidAmount);
        }

        vault.withdraw(&token, spent);
    }

    let shares = token::Client::new(env, lp_token)
        .balance(router)
        .checked_sub(shares_before)
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
    if shares < min_shares {
        panic_with_error!(env, Error::MinSharesNotMet);
    }
    vault.deposit(lp_token, shares);
    shares
}
