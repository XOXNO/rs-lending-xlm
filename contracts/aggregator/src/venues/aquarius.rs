//! Aquarius classic pool adapter.

use crate::errors::Error;
use crate::venues::HopContext;
use soroban_sdk::{panic_with_error, symbol_short, vec, Address, Env, IntoVal, Symbol, Val, Vec};

pub(crate) fn swap(ctx: &HopContext<'_>) -> i128 {
    let amount_in_u128: u128 = ctx
        .amount_in
        .try_into()
        .unwrap_or_else(|_| panic_with_error!(ctx.env, Error::IntegerOverflow));

    // Resolve in_idx / out_idx by scanning the pool's token list.
    let tokens: Vec<Address> = ctx.env.invoke_contract(
        &ctx.hop.pool,
        &Symbol::new(ctx.env, "get_tokens"),
        Vec::<Val>::new(ctx.env),
    );

    let in_idx = find_index(ctx.env, &tokens, &ctx.hop.token_in);
    let out_idx = find_index(ctx.env, &tokens, &ctx.hop.token_out);

    // Aquarius pulls `token_in` internally. Authorize only that transfer.
    ctx.authorize_pool_pull();

    let args: Vec<Val> = vec![
        ctx.env,
        ctx.router.into_val(ctx.env),
        in_idx.into_val(ctx.env),
        out_idx.into_val(ctx.env),
        amount_in_u128.into_val(ctx.env),
        // out_min = 0; the router's aggregate total_min_out gate (lib.rs)
        // enforces slippage after all paths complete.
        0_u128.into_val(ctx.env),
    ];
    let amount_out_u128: u128 =
        ctx.env
            .invoke_contract(&ctx.hop.pool, &symbol_short!("swap"), args);
    if amount_out_u128 == 0 {
        panic_with_error!(ctx.env, Error::ZeroOutput);
    }
    amount_out_u128
        .try_into()
        .unwrap_or_else(|_| panic_with_error!(ctx.env, Error::IntegerOverflow))
}

fn find_index(env: &Env, tokens: &Vec<Address>, target: &Address) -> u32 {
    let n = tokens.len();
    for i in 0..n {
        if tokens.get(i).as_ref() == Some(target) {
            return i;
        }
    }
    panic_with_error!(env, Error::BrokenTokenChain);
}
