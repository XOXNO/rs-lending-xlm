//! Exact-output hops for version 2 programs: input quotes and measured fills.

use common::math::RatioRounding;
use soroban_sdk::{panic_with_error, Address, Env, IntoVal, Map, Symbol, Vec};

use crate::errors::Error;
use crate::math::{checked_add, scale_amount};
use crate::types::{SwapHop, SwapVenue};
use crate::venues::aquarius::pool::get_tokens;
use crate::venues::curve::coins;
use crate::venues::{
    comet, curve, find_index, measure, phoenix, pool_tokens, soroswap, sushi, HopContext,
};

fn indices(
    ctx: &HopContext,
    cache: &mut Map<Address, Vec<Address>>,
    fetch: fn(&Env, &Address) -> Vec<Address>,
) -> (u32, u32) {
    let tokens = pool_tokens(ctx.env, cache, &ctx.hop.pool, fetch);
    (
        find_index(ctx.env, &tokens, &ctx.hop.token_in),
        find_index(ctx.env, &tokens, &ctx.hop.token_out),
    )
}

/// A venue that sizes its own input inside the exact-output call and has no inverse quote.
fn sizes_own_spend(venue: SwapVenue) -> bool {
    venue == SwapVenue::CometDex
}

/// Returns the input the hop's pool needs for exactly `target` of its output, or `None` when
/// the pool sizes its own spend.
pub(crate) fn reverse_input(
    env: &Env,
    router: &Address,
    hop: &SwapHop,
    target: i128,
    cache: &mut Map<Address, Vec<Address>>,
) -> Option<i128> {
    if sizes_own_spend(hop.venue) {
        return None;
    }
    let ctx = HopContext::new(env, router, hop, target);
    Some(match hop.venue {
        SwapVenue::Soroswap => soroswap::required_input(&ctx, target),
        SwapVenue::Aquarius => {
            let (i, j) = indices(&ctx, cache, get_tokens);
            let input: u128 = env.invoke_contract(
                &hop.pool,
                &Symbol::new(env, "estimate_swap_strict_receive"),
                (i, j, target as u128).into_val(env),
            );
            i128::try_from(input).unwrap_or_else(|_| panic_with_error!(env, Error::IntegerOverflow))
        }
        SwapVenue::Sushi => sushi::required_input(&ctx, target),
        SwapVenue::Curve => {
            let (i, j) = indices(&ctx, cache, coins);
            let estimate = env.invoke_contract(
                &hop.pool,
                &Symbol::new(env, "get_dx"),
                (i, j, target).into_val(env),
            );
            sufficient_input(env, target, estimate, |x| {
                env.invoke_contract(
                    &hop.pool,
                    &Symbol::new(env, "get_dy"),
                    (i, j, x).into_val(env),
                )
            })
        }
        SwapVenue::Phoenix => {
            let response: Map<Symbol, i128> = env.invoke_contract(
                &hop.pool,
                &Symbol::new(env, "simulate_reverse_swap"),
                (&hop.token_out, target).into_val(env),
            );
            let estimate = response.get_unchecked(Symbol::new(env, "offer_amount"));
            let (simulate, ask) = (
                Symbol::new(env, "simulate_swap"),
                Symbol::new(env, "ask_amount"),
            );
            sufficient_input(env, target, estimate, |x| {
                let forward: Map<Symbol, i128> =
                    env.invoke_contract(&hop.pool, &simulate, (&hop.token_in, x).into_val(env));
                forward.get_unchecked(ask.clone())
            })
        }
        SwapVenue::CometDex => panic_with_error!(env, Error::InvalidRouteXdr),
    })
}

/// Swaps for `target`, spending at most `input`, and returns the measured `(spent, received)`.
/// Spend must equal `input`, and output `target`, except where the venue rule allows less or more.
pub(crate) fn receive_output(
    env: &Env,
    router: &Address,
    hop: &SwapHop,
    input: i128,
    target: i128,
    cache: &mut Map<Address, Vec<Address>>,
) -> (i128, i128) {
    let ctx = HopContext::new(env, router, hop, input);
    let (spent, received) = measure(&ctx, || match hop.venue {
        SwapVenue::Soroswap => soroswap::swap_for(&ctx, target),
        SwapVenue::Aquarius => {
            let (i, j) = indices(&ctx, cache, get_tokens);
            ctx.authorize_pool_pull();
            let _: u128 = env.invoke_contract(
                &hop.pool,
                &Symbol::new(env, "swap_strict_receive"),
                (router, i, j, target as u128, input as u128).into_val(env),
            );
        }
        SwapVenue::Sushi => sushi::exact_output(&ctx, target),
        SwapVenue::CometDex => comet::exact_output(&ctx, target),
        SwapVenue::Phoenix => phoenix::swap(&ctx),
        SwapVenue::Curve => curve::swap(&ctx, cache),
    });
    let (Some(spent), Some(received)) = (spent, received) else {
        panic_with_error!(env, Error::InvalidAmount)
    };
    if spent <= 0
        || spent > input
        || (!sizes_own_spend(hop.venue) && spent != input)
        || received < target
        || (!matches!(hop.venue, SwapVenue::Curve | SwapVenue::Phoenix) && received != target)
    {
        panic_with_error!(env, Error::SlippageExceeded);
    }
    (spent, received)
}

/// Returns the smallest input whose forward `quote` reaches `target`, seeded by the pool's
/// inverse `estimate`. Quote calls are bounded; a far-off estimate panics `SlippageExceeded`.
fn sufficient_input(env: &Env, target: i128, estimate: i128, quote: impl Fn(i128) -> i128) -> i128 {
    let mut input = estimate.max(1);
    let (mut last_in, mut last_out) = (0, 0);
    for _ in 0..8 {
        let out = quote(input);
        if out >= target {
            // Integer units can overshoot `target`; gallop down from `input` (a rounding plateau
            // is narrow), then bisect, so one unit less underfills.
            let (mut low, mut high, mut step) = (last_in, input, 1i128);
            for _ in 0..20 {
                let candidate = (high - step).max(low);
                if candidate == low || quote(candidate) < target {
                    low = candidate;
                    while high - low > 1 {
                        let mid = low + (high - low) / 2;
                        if quote(mid) >= target {
                            high = mid;
                        } else {
                            low = mid;
                        }
                    }
                    return high;
                }
                high = candidate;
                step *= 2;
            }
            panic_with_error!(env, Error::SlippageExceeded);
        }
        // Secant step: on an imbalanced pool the average price from zero misleads.
        let step = if out > last_out {
            scale_amount(
                env,
                input - last_in,
                target - out,
                out - last_out,
                RatioRounding::Ceil,
            )
            .max(1)
        } else {
            (input - last_in).max(1)
        };
        (last_in, last_out) = (input, out);
        input = checked_add(env, input, step);
    }
    panic_with_error!(env, Error::SlippageExceeded)
}
