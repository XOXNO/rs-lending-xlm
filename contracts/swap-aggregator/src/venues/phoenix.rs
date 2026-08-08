//! Phoenix hop: authorized pool `swap` with optional params left empty.

use soroban_sdk::{panic_with_error, symbol_short, vec, IntoVal, Val, Vec};

use crate::errors::Error;
use crate::venues::HopContext;

/// Exact-in swap; pool-reported out must be positive (delta checked upstream).
pub(crate) fn swap(ctx: &HopContext<'_>) -> i128 {
    let args: Vec<Val> = vec![
        ctx.env,
        ctx.router.into_val(ctx.env),
        ctx.hop.token_in.into_val(ctx.env),
        ctx.amount_in.into_val(ctx.env),
        Option::<i128>::None.into_val(ctx.env),
        Option::<i64>::None.into_val(ctx.env),
        Option::<u64>::None.into_val(ctx.env),
        Option::<i64>::None.into_val(ctx.env),
    ];
    ctx.authorize_pool_pull();
    let amount_out: i128 = ctx
        .env
        .invoke_contract(&ctx.hop.pool, &symbol_short!("swap"), args);
    if amount_out <= 0 {
        panic_with_error!(ctx.env, Error::ZeroOutput);
    }
    amount_out
}
