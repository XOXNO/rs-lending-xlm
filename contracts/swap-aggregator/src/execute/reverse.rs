//! Reverse a topologically ordered swap graph. Each edge's encoded amount is
//! its relative share among edges producing the same token. Live venue quotes
//! determine intermediate requirements before any pool is traded.
use super::*;
use crate::math::checked_add;
use crate::math::mul_div_floor;

pub(super) fn execute(
    ctx: &Ctx,
    vault: &mut Vault,
    gross: i128,
    cache: &mut Map<Address, Vec<Address>>,
) {
    let env = ctx.env;
    let source = ctx.assets.get_unchecked(ctx.program.token_in);
    let sink = ctx.assets.get_unchecked(ctx.program.token_out);
    let mut weights = Map::<Address, i128>::new(env);
    let mut seen_pools = Map::<Address, bool>::new(env);
    let mut consumed = Map::<Address, bool>::new(env);
    for i in 0..ctx.program.len() {
        let op = ctx.program.op(env, i);
        let (Opcode::Swap(_), Mode::Fixed(index)) = (op.opcode, op.mode) else {
            panic_with_error!(env, Error::InvalidRouteXdr)
        };
        let input = ctx.assets.get_unchecked(op.idx_b);
        let output = ctx.assets.get_unchecked(op.idx_c);
        let pool = ctx.assets.get_unchecked(op.idx_a);
        let weight = ctx.amounts.get_unchecked(u32::from(index));
        if weight <= 0
            || seen_pools.contains_key(pool.clone())
            || consumed.contains_key(output.clone())
            || output == source
            || input == sink
            || (input != source && !weights.contains_key(input.clone()))
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
    let mut needs = Map::<Address, i128>::new(env);
    needs.set(sink, gross);
    let mut fills = Vec::<(i128, i128)>::new(env);
    for i in (0..ctx.program.len()).rev() {
        let op = ctx.program.op(env, i);
        let (Opcode::Swap(venue), Mode::Fixed(index)) = (op.opcode, op.mode) else {
            panic_with_error!(env, Error::InvalidRouteXdr)
        };
        let hop = SwapHop {
            venue,
            pool: ctx.assets.get_unchecked(op.idx_a),
            token_in: ctx.assets.get_unchecked(op.idx_b),
            token_out: ctx.assets.get_unchecked(op.idx_c),
        };
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
        let input = if venue == crate::types::SwapVenue::CometDex {
            // This pool has no inverse view. Let it pull its measured spend,
            // bounded by the remaining source deposit.
            if hop.token_in != source {
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
    if needs.get(source.clone()).unwrap_or(0) > vault.balance_of(&source) {
        panic_with_error!(env, Error::SlippageExceeded);
    }
    for i in 0..ctx.program.len() {
        let op = ctx.program.op(env, i);
        let Opcode::Swap(venue) = op.opcode else {
            panic_with_error!(env, Error::InvalidRouteXdr)
        };
        let hop = SwapHop {
            venue,
            pool: ctx.assets.get_unchecked(op.idx_a),
            token_in: ctx.assets.get_unchecked(op.idx_b),
            token_out: ctx.assets.get_unchecked(op.idx_c),
        };
        let (input, target) = fills.pop_back_unchecked();
        let available = vault.balance_of(&hop.token_in);
        let limit = if venue == crate::types::SwapVenue::CometDex {
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
