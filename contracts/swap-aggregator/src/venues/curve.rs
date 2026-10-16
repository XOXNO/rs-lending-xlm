//! Curve StableSwap: indexed exchange with the router as payer and receiver.

use soroban_sdk::{
    panic_with_error, symbol_short, vec, Address, IntoVal, Map, Symbol, TryFromVal, Val, Vec,
};

use crate::{errors::Error, program::MAX_ASSETS, venues::HopContext};

pub(crate) fn swap(ctx: &HopContext<'_>, cache: &mut Map<Address, Vec<Address>>) {
    let env = ctx.env;
    let pool = &ctx.hop.pool;
    let tokens = if let Some(tokens) = cache.get(pool.clone()) {
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
            if seen.contains_key(token.clone()) {
                panic_with_error!(env, Error::BrokenTokenChain);
            }
            seen.set(token, true);
        }
        cache.set(pool.clone(), tokens.clone());
        tokens
    };
    let i = tokens
        .first_index_of(ctx.hop.token_in.clone())
        .unwrap_or_else(|| panic_with_error!(env, Error::BrokenTokenChain));
    let j = tokens
        .first_index_of(ctx.hop.token_out.clone())
        .unwrap_or_else(|| panic_with_error!(env, Error::BrokenTokenChain));

    // Metadata calls must precede this authorization; the next invocation consumes it.
    ctx.authorize_pool_pull();
    let _: i128 = env.invoke_contract(
        pool,
        &symbol_short!("exchange"),
        vec![
            env,
            ctx.router.into_val(env),
            i.into_val(env),
            j.into_val(env),
            ctx.amount_in.into_val(env),
            0_i128.into_val(env),
            ctx.router.into_val(env),
        ],
    );
}
