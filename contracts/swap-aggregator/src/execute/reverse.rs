//! Reverse a topologically ordered swap graph. Each edge's encoded amount is
//! its relative share among edges producing the same token. Live venue quotes
//! determine intermediate requirements before any pool is traded.
use super::*;
use crate::math::{checked_add, mul_div_floor};
use crate::types::SwapVenue;

pub(super) fn execute(
    ctx: &Ctx,
    vault: &mut Vault,
    gross: i128,
    cache: &mut Map<Address, Vec<Address>>,
) {
    let env = ctx.env;
    let source = ctx.assets.get_unchecked(ctx.program.token_in);
    let sink = ctx.assets.get_unchecked(ctx.program.token_out);
    let weights = validate_graph(ctx, &source, &sink);
    let (required_input, fills) = price_backwards(ctx, &source, sink, gross, weights, cache);
    if required_input > vault.balance_of(&source) {
        panic_with_error!(env, Error::SlippageExceeded);
    }
    execute_forwards(ctx, vault, fills, cache);
}

/// A reverse record is a swap and an index into the output-weight table.
fn reverse_hop(ctx: &Ctx, i: u32) -> (SwapHop, u8) {
    let op = ctx.program.op(ctx.env, i);
    let (Opcode::Swap(venue), Mode::Fixed(index)) = (op.opcode, op.mode) else {
        panic_with_error!(ctx.env, Error::InvalidRouteXdr)
    };
    let hop = SwapHop {
        venue,
        pool: ctx.assets.get_unchecked(op.idx_a),
        token_in: ctx.assets.get_unchecked(op.idx_b),
        token_out: ctx.assets.get_unchecked(op.idx_c),
    };
    (hop, index)
}

/// Require unique pools and all producers before their consumers; sum weights by output token.
fn validate_graph(ctx: &Ctx, source: &Address, sink: &Address) -> Map<Address, i128> {
    let env = ctx.env;
    let mut weights = Map::<Address, i128>::new(env);
    let mut seen_pools = Map::<Address, bool>::new(env);
    let mut consumed = Map::<Address, bool>::new(env);
    for i in 0..ctx.program.len() {
        let (hop, index) = reverse_hop(ctx, i);
        let SwapHop {
            pool,
            token_in: input,
            token_out: output,
            ..
        } = hop;
        let weight = ctx.amounts.get_unchecked(u32::from(index));
        if weight <= 0
            || seen_pools.contains_key(pool.clone())
            || consumed.contains_key(output.clone())
            || &output == source
            || &input == sink
            || (&input != source && !weights.contains_key(input.clone()))
        {
            panic_with_error!(env, Error::BrokenTokenChain);
        }
        seen_pools.set(pool, true);
        consumed.set(input, true);
        weights.set(
            output.clone(),
            checked_add(env, weights.get(output).unwrap_or(0), weight),
        );
    }
    weights
}

/// Allocate gross demand across producers. The last producer covers the rounding remainder.
fn price_backwards(
    ctx: &Ctx,
    source: &Address,
    sink: Address,
    gross: i128,
    mut weights: Map<Address, i128>,
    cache: &mut Map<Address, Vec<Address>>,
) -> (i128, Vec<(i128, i128)>) {
    let env = ctx.env;
    let mut needs = Map::<Address, i128>::new(env);
    needs.set(sink, gross);
    let mut fills = Vec::<(i128, i128)>::new(env);
    for i in (0..ctx.program.len()).rev() {
        let (hop, index) = reverse_hop(ctx, i);
        let need = needs.get(hop.token_out.clone()).unwrap_or(0);
        let remaining = weights.get_unchecked(hop.token_out.clone());
        let weight = ctx.amounts.get_unchecked(u32::from(index));
        let target = if remaining == weight {
            need
        } else {
            mul_div_floor(env, need, weight, remaining)
        };
        if target <= 0 {
            panic_with_error!(env, Error::InvalidAmount);
        }
        weights.set(hop.token_out.clone(), remaining - weight);
        needs.set(hop.token_out.clone(), need - target);
        let input = if hop.venue == SwapVenue::CometDex {
            // This pool has no inverse view. Let it pull its measured spend,
            // bounded by the remaining source deposit.
            if &hop.token_in != source {
                panic_with_error!(env, Error::InvalidRouteXdr);
            }
            0
        } else {
            let input = venues::reverse_input(env, ctx.router, &hop, target, cache);
            if input <= 0 {
                panic_with_error!(env, Error::InvalidAmount);
            }
            needs.set(
                hop.token_in.clone(),
                checked_add(env, needs.get(hop.token_in).unwrap_or(0), input),
            );
            input
        };
        fills.push_back((input, target));
    }
    (needs.get(source.clone()).unwrap_or(0), fills)
}

/// Consume the backwards-priced fills in program order and book each measured swap.
fn execute_forwards(
    ctx: &Ctx,
    vault: &mut Vault,
    mut fills: Vec<(i128, i128)>,
    cache: &mut Map<Address, Vec<Address>>,
) {
    let env = ctx.env;
    for i in 0..ctx.program.len() {
        let (hop, _) = reverse_hop(ctx, i);
        let (input, target) = fills.pop_back_unchecked();
        let available = vault.balance_of(&hop.token_in);
        let limit = if hop.venue == SwapVenue::CometDex {
            available
        } else {
            input
        };
        if limit <= 0 || limit > available {
            panic_with_error!(env, Error::SlippageExceeded);
        }
        let (spent, received) = venues::receive_output(env, ctx.router, &hop, limit, target, cache);
        vault.withdraw(&hop.token_in, spent);
        vault.deposit(&hop.token_out, received);
    }
}
