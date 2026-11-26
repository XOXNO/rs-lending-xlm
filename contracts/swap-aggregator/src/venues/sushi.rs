//! Sushi V3-style hop: `swap` with sqrt-price limit and oracle hints.

use soroban_sdk::{
    panic_with_error, vec, Address, Bytes, Env, IntoVal, Map, Symbol, Val, Vec, U256,
};

use crate::errors::Error;
use crate::venues::HopContext;

const MIN_SQRT_RATIO_PLUS_ONE: u128 = 4_295_128_740;
const MAX_SQRT_RATIO_MINUS_ONE: [u8; 32] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xfd, 0x89, 0x63,
    0xef, 0xd1, 0xfc, 0x6a, 0x50, 0x64, 0x88, 0x49, 0x5d, 0x95, 0x1d, 0x52, 0x63, 0x98, 0x8d, 0x25,
];

/// Executes an exact-in swap against a Sushi V3-style concentrated-liquidity pool. The function
/// determines the swap direction from the pool's `token0` and `token1`, computes an extreme
/// sqrt-price limit, and forwards the pool's oracle hints. It then authorizes the pool to pull
/// the input token from the router and invokes the pool's `swap`.
/// [`crate::venues::dispatch_hop`] measures the fill and enforces that it is positive.
pub(crate) fn swap(ctx: &HopContext<'_>) {
    execute(ctx, ctx.amount_in);
}
fn direction(ctx: &HopContext) -> bool {
    let no_args: Vec<Val> = vec![ctx.env];
    let token0: Address = ctx.env.invoke_contract(
        &ctx.hop.pool,
        &Symbol::new(ctx.env, "token0"),
        no_args.clone(),
    );
    let token1: Address =
        ctx.env
            .invoke_contract(&ctx.hop.pool, &Symbol::new(ctx.env, "token1"), no_args);
    ctx.direction_for_pair(&token0, &token1)
}

pub(super) fn required_input(ctx: &HopContext, target: i128) -> i128 {
    let zero_for_one = direction(ctx);
    let result: Map<Symbol, i128> = ctx.env.invoke_contract(
        &ctx.hop.pool,
        &Symbol::new(ctx.env, "quote_exact_output"),
        (
            zero_for_one,
            target,
            sqrt_price_limit(ctx.env, zero_for_one),
        )
            .into_val(ctx.env),
    );
    let amount0 = result.get_unchecked(Symbol::new(ctx.env, "amount0"));
    let amount1 = result.get_unchecked(Symbol::new(ctx.env, "amount1"));
    let (input, out) = if zero_for_one {
        (amount0, amount1)
    } else {
        (amount1, amount0)
    };
    if out != -target {
        panic_with_error!(ctx.env, Error::SlippageExceeded);
    }
    input
}

pub(super) fn exact_output(ctx: &HopContext, target: i128) {
    execute(ctx, -target)
}
fn execute(ctx: &HopContext, amount_specified: i128) {
    let zero_for_one = direction(ctx);
    let price_limit = sqrt_price_limit(ctx.env, zero_for_one);
    let hints: Val = ctx.env.invoke_contract(
        &ctx.hop.pool,
        &Symbol::new(ctx.env, "get_oracle_hints"),
        vec![ctx.env],
    );

    ctx.authorize_pool_pull();

    let args: Vec<Val> = vec![
        ctx.env,
        ctx.router.into_val(ctx.env),
        ctx.router.into_val(ctx.env),
        zero_for_one.into_val(ctx.env),
        amount_specified.into_val(ctx.env),
        price_limit.into_val(ctx.env),
        hints,
    ];
    let _: Val = ctx
        .env
        .invoke_contract(&ctx.hop.pool, &Symbol::new(ctx.env, "swap"), args);
}

/// Returns an extreme sqrt-price limit, near the protocol minimum or maximum depending on swap
/// direction, so the swap can consume the full exact-in amount without hitting a price bound.
fn sqrt_price_limit(env: &Env, zero_for_one: bool) -> U256 {
    if zero_for_one {
        U256::from_u128(env, MIN_SQRT_RATIO_PLUS_ONE)
    } else {
        let bytes = Bytes::from_array(env, &MAX_SQRT_RATIO_MINUS_ONE);
        U256::from_be_bytes(env, &bytes)
    }
}
