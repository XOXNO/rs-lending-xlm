//! Curve StableSwap: indexed exchange, one-sided LP mint, and one-coin LP burn, with the router
//! as payer and receiver.

use soroban_sdk::{
    panic_with_error, symbol_short, vec, Address, Env, IntoVal, Map, Symbol, TryFromVal, Val, Vec,
};

use crate::errors::Error;
use crate::venues::{find_index, pool_tokens, HopContext};

/// Reads `coins` from the pool's `pool_info` map. The map is decoded leniently so that new
/// fields in the pool ABI do not break routing. Panics with `BrokenTokenChain` if `coins` is
/// missing or is not an address list.
fn coins(env: &Env, pool: &Address) -> Vec<Address> {
    let info: Map<Symbol, Val> = env.invoke_contract(pool, &symbol_short!("pool_info"), vec![env]);
    let coins = info
        .get(symbol_short!("coins"))
        .unwrap_or_else(|| panic_with_error!(env, Error::BrokenTokenChain));
    Vec::<Address>::try_from_val(env, &coins)
        .unwrap_or_else(|_| panic_with_error!(env, Error::BrokenTokenChain))
}

/// Burns LP for one coin when `token_in` is the pool, mints LP from one coin when `token_out`
/// is the pool, and exchanges coins otherwise. Every call passes a zero minimum; the strategy
/// floor binds the measured output.
pub(crate) fn swap(ctx: &HopContext<'_>, cache: &mut Map<Address, Vec<Address>>) {
    let env = ctx.env;
    let pool = &ctx.hop.pool;
    let tokens = pool_tokens(env, cache, pool, coins);
    let mut args = vec![env, ctx.router.into_val(env)];
    let function = if ctx.hop.token_in == *pool {
        args.push_back(ctx.amount_in.into_val(env));
        args.push_back(find_index(env, &tokens, &ctx.hop.token_out).into_val(env));
        "remove_liquidity_one_coin"
    } else {
        let i = find_index(env, &tokens, &ctx.hop.token_in);
        ctx.authorize_pool_pull();
        if ctx.hop.token_out == *pool {
            let mut amounts = Vec::<i128>::new(env);
            for index in 0..tokens.len() {
                amounts.push_back(if index == i { ctx.amount_in } else { 0 });
            }
            args.push_back(amounts.into_val(env));
            "add_liquidity"
        } else {
            args.push_back(i.into_val(env));
            args.push_back(find_index(env, &tokens, &ctx.hop.token_out).into_val(env));
            args.push_back(ctx.amount_in.into_val(env));
            "exchange"
        }
    };
    args.push_back(0_i128.into_val(env));
    args.push_back(ctx.router.into_val(env));
    let _: i128 = env.invoke_contract(pool, &Symbol::new(env, function), args);
}
