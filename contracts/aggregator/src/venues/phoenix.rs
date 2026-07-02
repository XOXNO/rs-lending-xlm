//! Phoenix DEX venue dispatcher.
//!
//! Phoenix XYK and Stable pools share a single swap entry with an
//! `Option`-heavy signature:
//! ```text
//! fn swap(
//!     env,
//!     sender: Address,
//!     offer_asset: Address,
//!     offer_amount: i128,
//!     ask_asset_min_amount: Option<i128>,
//!     max_spread_bps: Option<i64>,
//!     deadline: Option<u64>,
//!     max_allowed_fee_bps: Option<i64>,
//! ) -> i128
//! ```
//!
//! Unlike Aquarius, Phoenix identifies tokens by ADDRESS, so we don't need
//! an extra `get_tokens` round-trip.
//!
//! Like Aquarius, the pool pulls tokens internally. We pass the router as
//! `sender` and explicitly authorize the exact token transfer the pool may
//! perform.
//!
//! We pass `None` for every Option — the aggregate and per-path slippage
//! guards in `lib.rs` cover our risk; passing `None` here avoids double-
//! checking and gives Phoenix the freedom to route optimally.

use crate::errors::Error;
use crate::venues::HopContext;
use soroban_sdk::{panic_with_error, symbol_short, vec, IntoVal, Val, Vec};

pub(crate) fn swap(ctx: &HopContext<'_>) -> i128 {
    // `Option<T>` in Soroban is represented at the `Val` boundary via the
    // SDK's `IntoVal`. An explicit `None` is `Option::<i128>::None.into_val`.
    let none_i128: Option<i128> = None;
    let none_i64: Option<i64> = None;
    let none_u64: Option<u64> = None;
    let none_fee: Option<i64> = None;

    let args: Vec<Val> = vec![
        ctx.env,
        ctx.router.into_val(ctx.env),
        ctx.hop.token_in.into_val(ctx.env),
        ctx.amount_in.into_val(ctx.env),
        none_i128.into_val(ctx.env),
        none_i64.into_val(ctx.env),
        none_u64.into_val(ctx.env),
        none_fee.into_val(ctx.env),
    ];
    ctx.authorize_pool_pull();
    let amount_out: i128 = ctx
        .env
        .invoke_contract(&ctx.hop.pool, &symbol_short!("swap"), args);
    if amount_out <= 0 {
        panic_with_error!(ctx.env, Error::ZeroOutput);
    }
    amount_out
}
