//! Aquarius LP deposit (mint) and optional pre-balance swap.

use soroban_sdk::{
    auth::InvokerContractAuthEntry, panic_with_error, token, vec, Address, Env, IntoVal, Map,
    Symbol, Vec,
};

use crate::errors::Error;
use crate::vault::Vault;
use crate::venues::aquarius::pool::{
    assert_share_token, invoke_pool_swap, pool_reserves, pool_tokens, to_u128,
};
use crate::venues::auth::auth_entry;

/// Off-chain sized pre-swap before mint: move `amount` A→B when `from_a`, else B→A.
pub(crate) struct PreSwap {
    pub from_a: bool,
    pub amount: i128,
}

/// Mint parameters; router plumbing (`env` / vault / cache) is passed separately.
pub(crate) struct MintLiquidity<'a> {
    pub pool: &'a Address,
    pub lp_token: &'a Address,
    pub min_shares: i128,
    pub pre_swap: PreSwap,
}

/// Deposit vault holdings of pool constituents; credit measured LP shares.
///
/// Optional pre-swap rebalances a 2-token pool before deposit.
pub(crate) fn add_liquidity(
    env: &Env,
    router: &Address,
    vault: &mut Vault,
    mint: MintLiquidity<'_>,
    cache: &mut Map<Address, Vec<Address>>,
) -> i128 {
    let MintLiquidity {
        pool,
        lp_token,
        min_shares,
        pre_swap,
    } = mint;

    let tokens = pool_tokens(env, cache, pool);
    assert_share_token(env, pool, lp_token);
    if min_shares <= 0 {
        panic_with_error!(env, Error::MinSharesNotMet);
    }

    pre_balance(env, router, vault, pool, &tokens, pre_swap);

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

    let pulled = held.clone();
    let mut auth: Vec<InvokerContractAuthEntry> = Vec::new(env);
    for i in 0..n {
        let amount = pulled.get_unchecked(i);

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

/// Run caller-provided pre-swap when the pool is a 2-asset book with positive reserves.
fn pre_balance(
    env: &Env,
    router: &Address,
    vault: &mut Vault,
    pool: &Address,
    tokens: &Vec<Address>,
    pre_swap: PreSwap,
) {
    if pre_swap.amount <= 0 || tokens.len() != 2 {
        return;
    }
    let token_a = tokens.get_unchecked(0);
    let token_b = tokens.get_unchecked(1);
    let held_a = vault.balance_of(&token_a);
    let held_b = vault.balance_of(&token_b);

    let reserves = pool_reserves(env, pool);
    if reserves.len() != 2 {
        return;
    }
    let reserve_a = reserves.get_unchecked(0);
    let reserve_b = reserves.get_unchecked(1);
    if !pre_balance_possible(held_a, held_b, reserve_a, reserve_b) {
        return;
    }

    let (from_a, amount) = (pre_swap.from_a, pre_swap.amount);
    let held_in = if from_a { held_a } else { held_b };
    if amount > held_in {
        panic_with_error!(env, Error::InvalidAmount);
    }

    let (token_in, token_out, in_idx, out_idx) = if from_a {
        (token_a, token_b, 0u32, 1u32)
    } else {
        (token_b, token_a, 1u32, 0u32)
    };
    let received = swap_through_pool(
        env,
        router,
        pool,
        (&token_in, &token_out),
        (in_idx, out_idx),
        amount,
    );
    vault.withdraw(&token_in, amount);
    vault.deposit(&token_out, received);
}

/// Swap via pool and credit vault with measured output delta.
fn swap_through_pool(
    env: &Env,
    router: &Address,
    pool: &Address,
    tokens: (&Address, &Address),
    indices: (u32, u32),
    amount_in: i128,
) -> i128 {
    let (token_in, token_out) = tokens;
    let (in_idx, out_idx) = indices;
    let before = token::Client::new(env, token_out).balance(router);
    invoke_pool_swap(env, router, pool, token_in, in_idx, out_idx, amount_in);
    let received = token::Client::new(env, token_out)
        .balance(router)
        .checked_sub(before)
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
    if received <= 0 {
        panic_with_error!(env, Error::ZeroOutput);
    }
    received
}

/// True when held inventory and pool reserves allow a pre-balance swap.
pub(crate) fn pre_balance_possible(
    held_a: i128,
    held_b: i128,
    reserve_a: i128,
    reserve_b: i128,
) -> bool {
    (held_a > 0 || held_b > 0) && reserve_a > 0 && reserve_b > 0
}
