//! Aquarius single-hop swap.

use soroban_sdk::{Address, Map, Vec};

use crate::venues::aquarius::pool::{get_tokens, invoke_pool_swap};
use crate::venues::{find_index, pool_tokens, HopContext};

/// Executes a single-hop swap of `amount_in` through the hop's pool.
/// [`crate::venues::dispatch_hop`] measures the fill and enforces that it is positive. The
/// function decodes the pool's own report but never trusts it.
pub(crate) fn swap(ctx: &HopContext<'_>, cache: &mut Map<Address, Vec<Address>>) {
    let tokens = pool_tokens(ctx.env, cache, &ctx.hop.pool, get_tokens);
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
