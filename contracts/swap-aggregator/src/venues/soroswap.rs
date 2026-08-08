//! Soroswap (constant-product) hop: transfer in, then pool `swap`.

use soroban_sdk::{panic_with_error, symbol_short, token, vec, IntoVal, Symbol, Val};

use crate::errors::Error;
use crate::venues::HopContext;

/// 0.3% fee rounded up (Soroswap convention).
fn soroswap_fee(amount_in: i128) -> i128 {
    if amount_in <= 0 {
        return 0;
    }
    (amount_in * 3 + 999) / 1000
}

/// Expected out from live reserves (router still credits only measured delta).
fn soroswap_amount_out(amount_in: i128, reserve_in: i128, reserve_out: i128) -> i128 {
    if amount_in <= 0 || reserve_in <= 0 || reserve_out <= 0 {
        return 0;
    }
    let in_less = amount_in - soroswap_fee(amount_in);
    if in_less <= 0 {
        return 0;
    }
    in_less * reserve_out / (reserve_in + in_less)
}

/// Push input to the pair and request the constant-product output amount.
pub(crate) fn swap(ctx: &HopContext<'_>) -> i128 {
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

    let requested_out = soroswap_amount_out(ctx.amount_in, reserve_in, reserve_out);
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

    requested_out
}
