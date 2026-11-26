//! Version 2: prices a swap graph backwards from live venue quotes, then executes it forwards.

use common::math::RatioRounding;
use soroban_sdk::{panic_with_error, Address, Map, Vec};

use super::{swap_hop, Ctx};
use crate::errors::Error;
use crate::math::{checked_add, scale_amount};
use crate::program::{Mode, Opcode};
use crate::types::SwapHop;
use crate::vault::Vault;
use crate::venues;

/// Prices the graph backwards from `gross` of the sink, then executes it forwards. Panics
/// with `SlippageExceeded` if the source deposit cannot cover the priced input.
pub(super) fn execute(
    ctx: &Ctx,
    vault: &mut Vault,
    gross: i128,
    cache: &mut Map<Address, Vec<Address>>,
) {
    let source = ctx.assets.get_unchecked(ctx.program.token_in);
    let sink = ctx.assets.get_unchecked(ctx.program.token_out);
    let weights = validate_graph(ctx, &source, &sink);
    let (required_input, fills) = price_backwards(ctx, &source, sink, gross, weights, cache);
    if required_input > vault.balance_of(&source) {
        panic_with_error!(ctx.env, Error::SlippageExceeded);
    }
    execute_forwards(ctx, vault, fills, cache);
}

/// Decodes op `i` as a swap and its output-weight index.
fn reverse_hop(ctx: &Ctx, i: u32) -> (SwapHop, u8) {
    let op = ctx.program.op(ctx.env, i);
    let (Opcode::Swap(venue), Mode::Fixed(index)) = (op.opcode, op.mode) else {
        panic_with_error!(ctx.env, Error::InvalidRouteXdr)
    };
    (swap_hop(ctx, &op, venue), index)
}

/// Unique pools; producers precede consumers. Returns the summed weight per output token.
fn validate_graph(ctx: &Ctx, source: &Address, sink: &Address) -> Map<Address, i128> {
    let env = ctx.env;
    let mut weights = Map::<Address, i128>::new(env);
    let mut seen_pools = Map::<Address, bool>::new(env);
    let mut consumed = Map::<Address, bool>::new(env);
    for i in 0..ctx.program.len() {
        let (hop, index) = reverse_hop(ctx, i);
        let weight = ctx.amounts.get_unchecked(u32::from(index));
        if weight <= 0
            || seen_pools.contains_key(hop.pool.clone())
            || consumed.contains_key(hop.token_out.clone())
            || &hop.token_out == source
            || &hop.token_in == sink
            || (&hop.token_in != source && !weights.contains_key(hop.token_in.clone()))
        {
            panic_with_error!(env, Error::BrokenTokenChain);
        }
        seen_pools.set(hop.pool, true);
        consumed.set(hop.token_in, true);
        let summed = checked_add(env, weights.get(hop.token_out.clone()).unwrap_or(0), weight);
        weights.set(hop.token_out, summed);
    }
    weights
}

/// Splits each token's demand across its producers by weight, sink to source; shares floor and
/// the first producer takes the remainder. Fills are in reverse program order.
fn price_backwards(
    ctx: &Ctx,
    source: &Address,
    sink: Address,
    gross: i128,
    mut weights: Map<Address, i128>,
    cache: &mut Map<Address, Vec<Address>>,
) -> (i128, Vec<(Option<i128>, i128)>) {
    let env = ctx.env;
    let mut needs = Map::<Address, i128>::new(env);
    needs.set(sink, gross);
    let mut fills = Vec::<(Option<i128>, i128)>::new(env);
    for i in (0..ctx.program.len()).rev() {
        let (hop, index) = reverse_hop(ctx, i);
        let weight = ctx.amounts.get_unchecked(u32::from(index));
        let need = needs.get(hop.token_out.clone()).unwrap_or(0);
        let remaining = weights.get_unchecked(hop.token_out.clone());
        let target = if remaining == weight {
            need
        } else {
            scale_amount(env, need, weight, remaining, RatioRounding::Floor)
        };
        if target <= 0 {
            panic_with_error!(env, Error::InvalidAmount);
        }
        weights.set(hop.token_out.clone(), remaining - weight);
        needs.set(hop.token_out.clone(), need - target);
        let input = venues::reverse_input(env, ctx.router, &hop, target, cache);
        match input {
            Some(input) if input <= 0 => panic_with_error!(env, Error::InvalidAmount),
            Some(input) => {
                let total = checked_add(env, needs.get(hop.token_in.clone()).unwrap_or(0), input);
                needs.set(hop.token_in, total);
            }
            // Capped by the remaining source deposit when it runs.
            None if &hop.token_in != source => panic_with_error!(env, Error::InvalidRouteXdr),
            None => {}
        }
        fills.push_back((input, target));
    }
    (needs.get(source.clone()).unwrap_or(0), fills)
}

/// Runs the priced fills in program order and books each measured swap.
fn execute_forwards(
    ctx: &Ctx,
    vault: &mut Vault,
    mut fills: Vec<(Option<i128>, i128)>,
    cache: &mut Map<Address, Vec<Address>>,
) {
    let env = ctx.env;
    for i in 0..ctx.program.len() {
        let (hop, _) = reverse_hop(ctx, i);
        // `fills` is in reverse program order, so its back is hop `i`.
        let (input, target) = fills.pop_back_unchecked();
        let available = vault.balance_of(&hop.token_in);
        let limit = input.unwrap_or(available);
        if limit <= 0 || limit > available {
            panic_with_error!(env, Error::SlippageExceeded);
        }
        let (spent, received) = venues::receive_output(env, ctx.router, &hop, limit, target, cache);
        vault.withdraw(&hop.token_in, spent);
        vault.deposit(&hop.token_out, received);
    }
}
