//! Phoenix hop: authorized pool `swap` with optional params left empty.

use soroban_sdk::{symbol_short, vec, IntoVal, Val, Vec};

use crate::venues::HopContext;

/// Executes an exact-in swap against a Phoenix pool: authorizes the pool to pull `amount_in` of
/// the input token from the router, then invokes the pool's `swap` with its four optional
/// trailing parameters left unset. The fill is measured by
/// [`crate::venues::dispatch_hop`], which also enforces that it is positive.
pub(crate) fn swap(ctx: &HopContext<'_>) {
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
    let _: i128 = ctx
        .env
        .invoke_contract(&ctx.hop.pool, &symbol_short!("swap"), args);
}
