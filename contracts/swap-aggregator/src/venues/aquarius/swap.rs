//! Aquarius single-hop swap.

use soroban_sdk::{panic_with_error, Address, Map, Vec};

use crate::errors::Error;
use crate::venues::aquarius::pool::{find_index, invoke_pool_swap, pool_tokens};
use crate::venues::HopContext;

/// Executes a single-hop swap of `amount_in` through the hop's pool and returns
/// the pool-reported output amount. Panics with `ZeroOutput` if the pool reports
/// zero output, or with `IntegerOverflow` if the reported amount does not fit
/// in `i128`.
pub(crate) fn swap(ctx: &HopContext<'_>, cache: &mut Map<Address, Vec<Address>>) -> i128 {
    let tokens = pool_tokens(ctx.env, cache, &ctx.hop.pool);
    let in_idx = find_index(ctx.env, &tokens, &ctx.hop.token_in);
    let out_idx = find_index(ctx.env, &tokens, &ctx.hop.token_out);

    let reported = invoke_pool_swap(
        ctx.env,
        ctx.router,
        &ctx.hop.pool,
        &ctx.hop.token_in,
        in_idx,
        out_idx,
        ctx.amount_in,
    );
    if reported == 0 {
        panic_with_error!(ctx.env, Error::ZeroOutput);
    }
    reported
        .try_into()
        .unwrap_or_else(|_| panic_with_error!(ctx.env, Error::IntegerOverflow))
}
