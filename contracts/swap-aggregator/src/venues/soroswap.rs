//! Soroswap (constant-product) hop: transfer in, then pool `swap`.

use common::math::fp_core::mul_div_floor;
use soroban_sdk::{panic_with_error, symbol_short, token, vec, Env, IntoVal, Symbol, Val, Vec};

use crate::errors::Error;
use crate::math::{checked_add, checked_mul};
use crate::venues::HopContext;

/// Soroswap's swap fee as `FEE_NUM / FEE_DEN`: 30 basis points. The pair hard-codes
/// this tier in its `swap` k check and exposes no fee getter, so the adapter cannot
/// read it. A pair on a cheaper tier is under-requested and a pair on a dearer tier
/// fails closed in its own k check; see [`swap`].
const FEE_NUM: i128 = 3;
const FEE_DEN: i128 = 1_000;

/// Computes the 0.3% swap fee on `amount_in`, rounded up. Returns zero if `amount_in` is not
/// positive. Panics with `Error::IntegerOverflow` if the scaled numerator does not fit in `i128`.
///
/// Panics instead of widening like [`mul_div_floor`]: an `amount_in` above `i128::MAX / 3` is a
/// corrupt amount, not a trade.
fn soroswap_fee(env: &Env, amount_in: i128) -> i128 {
    if amount_in <= 0 {
        return 0;
    }
    let scaled = checked_mul(env, amount_in, FEE_NUM);
    checked_add(env, scaled, FEE_DEN - 1) / FEE_DEN
}

/// Computes the constant-product output amount for `amount_in` against `reserve_in` and
/// `reserve_out`, net of the swap fee, rounded down. Returns zero if any input is not positive,
/// or if the fee consumes the entire input amount.
fn soroswap_amount_out(env: &Env, amount_in: i128, reserve_in: i128, reserve_out: i128) -> i128 {
    if amount_in <= 0 || reserve_in <= 0 || reserve_out <= 0 {
        return 0;
    }
    let in_less = amount_in - soroswap_fee(env, amount_in);
    if in_less <= 0 {
        return 0;
    }
    let denominator = checked_add(env, reserve_in, in_less);
    mul_div_floor(env, in_less, reserve_out, denominator)
}

/// Executes a swap against a Soroswap constant-product pair: reads the pair's `token_0` and
/// `token_1` to orient the hop, reads its live reserves, computes the expected output via
/// `soroswap_amount_out`, transfers `amount_in` of the input token from the router to the pool,
/// then invokes the pool's `swap` requesting exactly that output amount. Panics with
/// `Error::BrokenTokenChain` if the pair does not hold exactly the hop's two tokens, before any
/// transfer, and with `Error::ZeroOutput` if the computed output is not positive — unlike the
/// other venues this is a precondition on a swap *argument*, not a report of the fill; the fill
/// is measured by [`crate::venues::dispatch_hop`].
///
/// The pair's token order is checked; its fee tier is assumed:
///
/// - Orientation is read from the pair, not derived from address order. A Soroswap pair refuses
///   to initialize unless `token_0 < token_1`, so the two agree on every genuine pair; reading
///   the views costs two calls per hop and turns a pool address that does not trade the hop's
///   tokens, or one stored the other way round, into a refusal before any transfer instead of a
///   quote against the wrong reserves. The two views are therefore part of the ABI this opcode
///   requires: a fork or legacy pair that lacks them fails closed here, before any transfer.
/// - The quote uses the 30 BPS tier ([`FEE_NUM`] / [`FEE_DEN`]), with the fee rounded up in the
///   pair's favour. A pair on a cheaper tier pays exactly this quote and keeps the difference (the
///   route minimum bounds what the sender accepts); a pair on a dearer tier rejects the request in
///   its own k check and the strategy reverts.
pub(crate) fn swap(ctx: &HopContext<'_>) {
    let token_in_is_0 = ctx.direction_from_pool("token_0", "token_1");

    let no_args: Vec<Val> = vec![ctx.env];
    let (reserve_0, reserve_1): (i128, i128) = ctx.env.invoke_contract(
        &ctx.hop.pool,
        &Symbol::new(ctx.env, "get_reserves"),
        no_args,
    );
    let (reserve_in, reserve_out) = if token_in_is_0 {
        (reserve_0, reserve_1)
    } else {
        (reserve_1, reserve_0)
    };

    let requested_out = soroswap_amount_out(ctx.env, ctx.amount_in, reserve_in, reserve_out);
    if requested_out <= 0 {
        panic_with_error!(ctx.env, Error::ZeroOutput);
    }

    let token_client = token::Client::new(ctx.env, &ctx.hop.token_in);
    token_client.transfer(ctx.router, &ctx.hop.pool, &ctx.amount_in);

    let (amount_0_out, amount_1_out) = if token_in_is_0 {
        (0_i128, requested_out)
    } else {
        (requested_out, 0_i128)
    };
    let args: Vec<Val> = vec![
        ctx.env,
        amount_0_out.into_val(ctx.env),
        amount_1_out.into_val(ctx.env),
        ctx.router.into_val(ctx.env),
    ];
    let _: () = ctx
        .env
        .invoke_contract(&ctx.hop.pool, &symbol_short!("swap"), args);
}
