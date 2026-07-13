//! Soroswap pair adapter.

use soroban_sdk::{panic_with_error, symbol_short, token, vec, IntoVal, Symbol, Val};

use crate::errors::Error;
use crate::venues::HopContext;

// ################## LOW-LEVEL HELPERS ##################

/// Soroswap's 0.3% swap fee, ceil-rounded — mirrors the pair's k-invariant
/// `fee_in = ceil(amount_in * 3 / 1000)`.
fn soroswap_fee(amount_in: i128) -> i128 {
    if amount_in <= 0 {
        return 0;
    }
    (amount_in * 3 + 999) / 1000
}

/// Soroswap library `get_amount_out`: floor-divided output after the ceil fee.
/// Equals exactly what the pair's `swap` k-check permits for `amount_in` at the
/// supplied reserves, so requesting it cannot trip the invariant.
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

/// Execute a swap through a Soroswap pair contract.
///
/// Returns the computed output the pair is asked to honor (the router is the
/// `to` address). `dispatch_hop` re-verifies receipt via the router's
/// `token_out` balance delta, so this value is advisory.
pub(crate) fn swap(ctx: &HopContext<'_>) -> i128 {
    // 1. Soroswap pairs hold canonically sorted tokens (`token_0 < token_1`
    //    under the host's address ordering), so orientation comes from the
    //    hop's addresses — no `token_0`/`token_1` calls. `token_in < token_out`
    //    ⇒ token_in occupies the `token_0` slot.
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

    // 2. Derive the exact honorable output from the ACTUAL input + LIVE
    //    reserves. This sits on the pair's k-invariant boundary for the current
    //    state, so the swap cannot revert on reserve drift — unlike passing the
    //    stale off-chain `hop.amount_out`.
    let requested_out = soroswap_amount_out(ctx.amount_in, reserve_in, reserve_out);
    if requested_out <= 0 {
        panic_with_error!(ctx.env, Error::ZeroOutput);
    }

    // 3. Push `amount_in` into the pair; the pair sees the balance delta on
    //    entry to `swap()`.
    let token_client = token::Client::new(ctx.env, &ctx.hop.token_in);
    token_client.transfer(ctx.router, &ctx.hop.pool, &ctx.amount_in);

    // 4. Call `swap`. Zero the input slot, fill the output slot.
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

    // 5. The pair transfers exactly `requested_out` to the router or reverts on
    //    its k-check, so the honored output equals the computed amount.
    //    `dispatch_hop` credits the router's measured balance delta regardless,
    //    so this return is advisory.
    requested_out
}
