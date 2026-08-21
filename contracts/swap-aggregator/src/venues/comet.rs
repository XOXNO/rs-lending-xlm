//! Comet DEX hop: approve → `swap_exact_amount_in` → clear allowance.

use soroban_sdk::{token, vec, Env, IntoVal, Symbol, Val, Vec};

use crate::venues::{auth_entry, authorize_token_approve, HopContext};

/// Executes an exact-in swap against a Comet pool: approves the router's input-token allowance to
/// the pool, invokes `swap_exact_amount_in` under a nested invoker auth entry covering the pool's
/// `transfer_from` of the input token, then clears the allowance back to zero. The fill is
/// measured by [`crate::venues::dispatch_hop`], which also enforces that it is positive.
pub(crate) fn swap(ctx: &HopContext<'_>) {
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
    let _: (i128, i128) = ctx.env.invoke_contract(
        &ctx.hop.pool,
        &Symbol::new(ctx.env, "swap_exact_amount_in"),
        args,
    );
    clear_comet_approval(ctx);
}

/// Computes the allowance expiration ledger: the current ledger sequence rounded up to the next
/// 100,000-ledger boundary.
fn comet_approval_ledger(env: &Env) -> u32 {
    let seq = env.ledger().sequence();
    (seq / 100_000 + 1) * 100_000
}

/// Builds the argument vector for the pool's `swap_exact_amount_in` call: input token, input
/// amount, output token, a zero minimum output amount, an unbounded maximum price, and the router
/// as the recipient.
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

/// Resets the router's input-token allowance to the pool to zero, authorizing the token
/// `approve` call as the current contract.
fn clear_comet_approval(ctx: &HopContext<'_>) {
    authorize_token_approve(ctx.env, &ctx.hop.token_in, ctx.router, &ctx.hop.pool, 0, 0);
    token::Client::new(ctx.env, &ctx.hop.token_in).approve(ctx.router, &ctx.hop.pool, &0, &0);
}

/// Authorizes the pool's `swap_exact_amount_in` call as the current contract, with a nested
/// invoker-auth entry covering the pool's `transfer_from` of the input token from the router.
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
