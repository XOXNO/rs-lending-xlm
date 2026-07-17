
use soroban_sdk::{panic_with_error, vec, Address, Bytes, Env, IntoVal, Symbol, Val, Vec, U256};

use crate::errors::Error;
use crate::venues::HopContext;

const MIN_SQRT_RATIO_PLUS_ONE: u128 = 4_295_128_740;
const MAX_SQRT_RATIO_MINUS_ONE: [u8; 32] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xfd, 0x89, 0x63,
    0xef, 0xd1, 0xfc, 0x6a, 0x50, 0x64, 0x88, 0x49, 0x5d, 0x95, 0x1d, 0x52, 0x63, 0x98, 0x8d, 0x25,
];

pub(crate) fn swap(ctx: &HopContext<'_>) -> i128 {
    let no_args: Vec<Val> = vec![ctx.env];
    let token0: Address = ctx.env.invoke_contract(
        &ctx.hop.pool,
        &Symbol::new(ctx.env, "token0"),
        no_args.clone(),
    );
    let token1: Address =
        ctx.env
            .invoke_contract(&ctx.hop.pool, &Symbol::new(ctx.env, "token1"), no_args);
    let zero_for_one = ctx.direction_for_pair(&token0, &token1);

    let price_limit = sqrt_price_limit(ctx.env, zero_for_one);
    let hints: Val = ctx.env.invoke_contract(
        &ctx.hop.pool,
        &Symbol::new(ctx.env, "get_oracle_hints"),
        vec![ctx.env],
    );

    let balance_before = ctx.output_balance();
    ctx.authorize_pool_pull();

    let args: Vec<Val> = vec![
        ctx.env,
        ctx.router.into_val(ctx.env),
        ctx.router.into_val(ctx.env),
        zero_for_one.into_val(ctx.env),
        ctx.amount_in.into_val(ctx.env),
        price_limit.into_val(ctx.env),
        hints,
    ];
    let _: Val = ctx
        .env
        .invoke_contract(&ctx.hop.pool, &Symbol::new(ctx.env, "swap"), args);

    let amount_out = ctx.output_balance() - balance_before;
    if amount_out <= 0 {
        panic_with_error!(ctx.env, Error::ZeroOutput);
    }
    amount_out
}

fn sqrt_price_limit(env: &Env, zero_for_one: bool) -> U256 {
    if zero_for_one {
        U256::from_u128(env, MIN_SQRT_RATIO_PLUS_ONE)
    } else {
        let bytes = Bytes::from_array(env, &MAX_SQRT_RATIO_MINUS_ONE);
        U256::from_be_bytes(env, &bytes)
    }
}
