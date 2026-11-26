//! Comet DEX hop: approve, call `swap_exact_amount_in` or `_out`, then clear the allowance.

use soroban_sdk::{token, vec, Env, IntoVal, Symbol, Val, Vec};

use crate::venues::{authorize_token_approve, HopContext};

/// Executes an exact-in swap against a Comet pool. The function approves the router's input-token
/// allowance to the pool. It then invokes `swap_exact_amount_in` with invoker auth for the pool's
/// nested `approve` and clears the allowance. [`crate::venues::dispatch_hop`] measures the fill
/// and checks that it is positive. [`exact_output`] does the same with `swap_exact_amount_out`.
pub(crate) fn swap(ctx: &HopContext<'_>) {
    execute(ctx, "swap_exact_amount_in", 0);
}

pub(super) fn exact_output(ctx: &HopContext, target: i128) {
    execute(ctx, "swap_exact_amount_out", target);
}

fn execute(ctx: &HopContext, function: &str, output: i128) {
    let approval_ledger = comet_approval_ledger(ctx.env);
    token::Client::new(ctx.env, &ctx.hop.token_in).approve(
        ctx.router,
        &ctx.hop.pool,
        &ctx.amount_in,
        &approval_ledger,
    );

    let args = swap_args(ctx, output);
    // Comet repeats the approval inside the swap. Soroban matches the token call directly.
    authorize_token_approve(
        ctx.env,
        &ctx.hop.token_in,
        ctx.router,
        &ctx.hop.pool,
        ctx.amount_in,
        approval_ledger,
    );
    let _: (i128, i128) =
        ctx.env
            .invoke_contract(&ctx.hop.pool, &Symbol::new(ctx.env, function), args);
    clear_comet_approval(ctx);
}

/// Computes the allowance expiration ledger: the first multiple of 100,000 above the current
/// ledger sequence.
fn comet_approval_ledger(env: &Env) -> u32 {
    let seq = env.ledger().sequence();
    (seq / 100_000 + 1) * 100_000
}

/// Builds the argument vector for `swap_exact_amount_in` or `_out`: input token, `amount_in`
/// (exact or maximum input), output token, `output` (zero minimum or exact output), an unbounded
/// maximum price, and the router as `user` (payer and recipient).
fn swap_args(ctx: &HopContext<'_>, output: i128) -> Vec<Val> {
    vec![
        ctx.env,
        ctx.hop.token_in.into_val(ctx.env),
        ctx.amount_in.into_val(ctx.env),
        ctx.hop.token_out.into_val(ctx.env),
        output.into_val(ctx.env),
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
