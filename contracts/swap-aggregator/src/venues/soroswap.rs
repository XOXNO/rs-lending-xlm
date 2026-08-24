//! Soroswap (constant-product) hop: transfer in, then pool `swap`.

use common::math::fp_core::mul_div_floor;
use soroban_sdk::{panic_with_error, symbol_short, token, vec, Env, IntoVal, Symbol, Val};

use crate::errors::Error;
use crate::math::{checked_add, checked_mul};
use crate::venues::HopContext;

/// Soroswap's swap fee as `FEE_NUM / FEE_DEN`: 30 basis points.
///
/// Invariant: every deployed Soroswap pair charges 30 bps. A pair does expose
/// its own fee setting, but reading it would cost one host call per hop for a
/// value that has never differed, so hardcoding it is a deliberate trade rather
/// than an oversight. If Soroswap ever ships a pair on a different fee tier,
/// this adapter must read the fee from the pair before quoting it.
const FEE_NUM: i128 = 3;
const FEE_DEN: i128 = 1_000;

/// Computes the 0.3% swap fee on `amount_in`, rounded up. Returns zero if `amount_in` is not
/// positive. Panics with `Error::IntegerOverflow` if the scaled numerator does not fit in `i128`.
///
/// This one panics where the shared [`mul_div_floor`] widens, because the two overflows mean
/// different things. `in_less * reserve_out` overflows for pair sizes that genuinely exist (1e18
/// against 1e24 reserves), so refusing them would strand real liquidity. `amount_in * 3` only
/// overflows above `i128::MAX / 3` ≈ 5.7e37, which no token supply reaches; an input that large
/// is a corrupt amount, not a large trade, and widening it would quietly route on nonsense.
fn soroswap_fee(env: &Env, amount_in: i128) -> i128 {
    if amount_in <= 0 {
        return 0;
    }
    let scaled = checked_mul(env, amount_in, FEE_NUM);
    checked_add(env, scaled, FEE_DEN - 1) / FEE_DEN
}

/// Computes the constant-product output amount for `amount_in` against `reserve_in` and
/// `reserve_out`, net of the swap fee. Returns zero if any input is not positive, or if the fee
/// consumes the entire input amount.
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

/// Executes a swap against a Soroswap constant-product pair: reads the pair's live reserves,
/// computes the expected output via `soroswap_amount_out`, transfers `amount_in` of the input
/// token from the router to the pool, then invokes the pool's `swap` requesting exactly that
/// output amount. Panics with `Error::ZeroOutput` if the computed output is not positive —
/// unlike the other venues this is a precondition on a swap *argument*, not a report of the
/// fill; the fill is measured by [`crate::venues::dispatch_hop`].
pub(crate) fn swap(ctx: &HopContext<'_>) {
    let token_in_is_0 = ctx.hop.token_in < ctx.hop.token_out;

    let no_args: soroban_sdk::Vec<Val> = vec![ctx.env];
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
    let args: soroban_sdk::Vec<Val> = vec![
        ctx.env,
        amount_0_out.into_val(ctx.env),
        amount_1_out.into_val(ctx.env),
        ctx.router.into_val(ctx.env),
    ];
    let _: () = ctx
        .env
        .invoke_contract(&ctx.hop.pool, &symbol_short!("swap"), args);
}
