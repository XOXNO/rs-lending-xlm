//! Aquarius single-hop swap.

use soroban_sdk::{Address, Map, Vec};

use crate::venues::aquarius::pool::{find_index, invoke_pool_swap, pool_tokens};
use crate::venues::HopContext;

/// Executes a single-hop swap of `amount_in` through the hop's pool. The fill is
/// measured by [`crate::venues::dispatch_hop`], which also enforces that it is
/// positive; the pool's own report is decoded but never trusted.
pub(crate) fn swap(ctx: &HopContext<'_>, cache: &mut Map<Address, Vec<Address>>) {
    let tokens = pool_tokens(ctx.env, cache, &ctx.hop.pool);
    let in_idx = find_index(ctx.env, &tokens, &ctx.hop.token_in);
    let out_idx = find_index(ctx.env, &tokens, &ctx.hop.token_out);

    invoke_pool_swap(
        ctx.env,
        ctx.router,
        &ctx.hop.pool,
        &ctx.hop.token_in,
        in_idx,
        out_idx,
        ctx.amount_in,
    );
}
