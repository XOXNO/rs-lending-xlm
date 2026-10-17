//! Curve StableSwap: indexed exchange with the router as payer and receiver.

use soroban_sdk::{
    panic_with_error, symbol_short, vec, Address, Env, IntoVal, Map, Symbol, TryFromVal, Val, Vec,
};

use crate::{errors::Error, program::MAX_ASSETS, venues::HopContext};

fn pool_tokens(env: &Env, pool: &Address, cache: &mut Map<Address, Vec<Address>>) -> Vec<Address> {
    if let Some(tokens) = cache.get(pool.clone()) {
        tokens
    } else {
        let info: Map<Symbol, Val> =
            env.invoke_contract(pool, &Symbol::new(env, "pool_info"), vec![env]);
        let coins = info
            .get(symbol_short!("coins"))
            .unwrap_or_else(|| panic_with_error!(env, Error::BrokenTokenChain));
        let tokens = Vec::<Address>::try_from_val(env, &coins)
            .unwrap_or_else(|_| panic_with_error!(env, Error::BrokenTokenChain));
        if tokens.len() < 2 || tokens.len() > MAX_ASSETS {
            panic_with_error!(env, Error::BrokenTokenChain);
        }
        let mut seen = Map::<Address, bool>::new(env);
        for token in tokens.iter() {
            if token == *pool || seen.contains_key(token.clone()) {
                panic_with_error!(env, Error::BrokenTokenChain);
            }
            seen.set(token, true);
        }
        cache.set(pool.clone(), tokens.clone());
        tokens
    }
}

pub(crate) fn swap(ctx: &HopContext<'_>, cache: &mut Map<Address, Vec<Address>>) {
    let env = ctx.env;
    let pool = &ctx.hop.pool;
    let tokens = pool_tokens(env, pool, cache);
    let (function, args) = if ctx.hop.token_in == *pool {
        let j = tokens
            .first_index_of(ctx.hop.token_out.clone())
            .unwrap_or_else(|| panic_with_error!(env, Error::BrokenTokenChain));
        (
            "remove_liquidity_one_coin",
            vec![
                env,
                ctx.router.into_val(env),
                ctx.amount_in.into_val(env),
                j.into_val(env),
                0_i128.into_val(env),
                ctx.router.into_val(env),
            ],
        )
    } else {
        let i = tokens
            .first_index_of(ctx.hop.token_in.clone())
            .unwrap_or_else(|| panic_with_error!(env, Error::BrokenTokenChain));
        if ctx.hop.token_out == *pool {
            let mut amounts = Vec::<i128>::new(env);
            for index in 0..tokens.len() {
                amounts.push_back(if index == i { ctx.amount_in } else { 0 });
            }
            (
                "add_liquidity",
                vec![
                    env,
                    ctx.router.into_val(env),
                    amounts.into_val(env),
                    0_i128.into_val(env),
                    ctx.router.into_val(env),
                ],
            )
        } else {
            let j = tokens
                .first_index_of(ctx.hop.token_out.clone())
                .unwrap_or_else(|| panic_with_error!(env, Error::BrokenTokenChain));
            (
                "exchange",
                vec![
                    env,
                    ctx.router.into_val(env),
                    i.into_val(env),
                    j.into_val(env),
                    ctx.amount_in.into_val(env),
                    0_i128.into_val(env),
                    ctx.router.into_val(env),
                ],
            )
        }
    };
    // Curve burns its own LP internally; only coin inputs need a nested transfer auth.
    if ctx.hop.token_in != *pool {
        ctx.authorize_pool_pull();
    }
    let _: i128 = env.invoke_contract(pool, &Symbol::new(env, function), args);
}
