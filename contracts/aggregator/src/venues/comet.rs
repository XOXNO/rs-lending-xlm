//! Comet weighted-pool adapter.

use soroban_sdk::{panic_with_error, token, vec, Env, IntoVal, Symbol, Val, Vec};

use crate::errors::Error;
use crate::venues::{auth_entry, authorize_token_approve, HopContext};

// ################## EXECUTION ##################

pub(crate) fn swap(ctx: &HopContext<'_>) -> i128 {
    let approval_ledger = comet_approval_ledger(ctx.env);
    authorize_token_approve(
        ctx.env,
        &ctx.hop.token_in,
        ctx.router,
        &ctx.hop.pool,
        ctx.amount_in,
        approval_ledger,
    );
    token::Client::new(ctx.env, &ctx.hop.token_in).approve(
        ctx.router,
        &ctx.hop.pool,
        &ctx.amount_in,
        &approval_ledger,
    );

    let args = swap_args(ctx);
    authorize_comet_swap(ctx, args.clone());
    let (amount_out, _): (i128, i128) = ctx.env.invoke_contract(
        &ctx.hop.pool,
        &Symbol::new(ctx.env, "swap_exact_amount_in"),
        args,
    );
    clear_comet_approval(ctx);
    if amount_out <= 0 {
        panic_with_error!(ctx.env, Error::ZeroOutput);
    }
    amount_out
}

fn comet_approval_ledger(env: &Env) -> u32 {
    let seq = env.ledger().sequence();
    (seq / 100_000 + 1) * 100_000
}

fn swap_args(ctx: &HopContext<'_>) -> Vec<Val> {
    vec![
        ctx.env,
        ctx.hop.token_in.into_val(ctx.env),
        ctx.amount_in.into_val(ctx.env),
        ctx.hop.token_out.into_val(ctx.env),
        0_i128.into_val(ctx.env),
        i128::MAX.into_val(ctx.env),
        ctx.router.into_val(ctx.env),
    ]
}

fn clear_comet_approval(ctx: &HopContext<'_>) {
    authorize_token_approve(ctx.env, &ctx.hop.token_in, ctx.router, &ctx.hop.pool, 0, 0);
    token::Client::new(ctx.env, &ctx.hop.token_in).approve(ctx.router, &ctx.hop.pool, &0, &0);
}

fn authorize_comet_swap(ctx: &HopContext<'_>, swap_args: Vec<Val>) {
    ctx.env.authorize_as_current_contract(vec![
        ctx.env,
        auth_entry(
            ctx.env,
            &ctx.hop.pool,
            "swap_exact_amount_in",
            swap_args,
            vec![
                ctx.env,
                auth_entry(
                    ctx.env,
                    &ctx.hop.token_in,
                    "transfer_from",
                    vec![
                        ctx.env,
                        ctx.hop.pool.into_val(ctx.env),
                        ctx.router.into_val(ctx.env),
                        ctx.hop.pool.into_val(ctx.env),
                        ctx.amount_in.into_val(ctx.env),
                    ],
                    vec![ctx.env],
                ),
            ],
        ),
    ]);
}
