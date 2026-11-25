use super::*;
fn aquarius_indices(ctx: &HopContext, cache: &mut Map<Address, Vec<Address>>) -> (u32, u32) {
    let tokens = pool_tokens(ctx.env, cache, &ctx.hop.pool, aquarius::pool::get_tokens);
    (
        find_index(ctx.env, &tokens, &ctx.hop.token_in),
        find_index(ctx.env, &tokens, &ctx.hop.token_out),
    )
}

pub(crate) fn reverse_input(
    env: &Env,
    router: &Address,
    hop: &SwapHop,
    target: i128,
    cache: &mut Map<Address, Vec<Address>>,
) -> i128 {
    use soroban_sdk::{IntoVal, Symbol};
    let ctx = HopContext::new(env, router, hop, target);
    match hop.venue {
        SwapVenue::Soroswap => soroswap::required_input(&ctx, target),
        SwapVenue::Aquarius => {
            let (i, j) = aquarius_indices(&ctx, cache);
            let input: u128 = env.invoke_contract(
                &hop.pool,
                &Symbol::new(env, "estimate_swap_strict_receive"),
                (i, j, target as u128).into_val(env),
            );
            i128::try_from(input).unwrap_or_else(|_| panic_with_error!(env, Error::IntegerOverflow))
        }
        SwapVenue::Sushi => sushi::required_input(&ctx, target),
        SwapVenue::Curve => {
            let tokens = pool_tokens(env, cache, &hop.pool, curve::coins);
            let i = find_index(env, &tokens, &hop.token_in);
            let j = find_index(env, &tokens, &hop.token_out);
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
            sufficient_input(env, target, estimate, |x| {
                let forward: Map<Symbol, i128> = env.invoke_contract(
                    &hop.pool,
                    &Symbol::new(env, "simulate_swap"),
                    (&hop.token_in, x).into_val(env),
                );
                forward.get_unchecked(Symbol::new(env, "ask_amount"))
            })
        }
        // Comet sizes its source debit inside swap_exact_amount_out.
        SwapVenue::CometDex => panic_with_error!(env, Error::InvalidRouteXdr),
    }
}

pub(crate) fn receive_output(
    env: &Env,
    router: &Address,
    hop: &SwapHop,
    input: i128,
    target: i128,
    cache: &mut Map<Address, Vec<Address>>,
) -> (i128, i128) {
    use soroban_sdk::{IntoVal, Symbol};
    let ctx = HopContext::new(env, router, hop, input);
    let before_in = ctx.input_balance();
    let before_out = ctx.output_balance();
    match hop.venue {
        SwapVenue::Soroswap => soroswap::swap_for(&ctx, target),
        SwapVenue::Aquarius => {
            let (i, j) = aquarius_indices(&ctx, cache);
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
    }
    let spent = before_in
        .checked_sub(ctx.input_balance())
        .unwrap_or_else(|| panic_with_error!(env, Error::InvalidAmount));
    let received = ctx
        .output_balance()
        .checked_sub(before_out)
        .unwrap_or_else(|| panic_with_error!(env, Error::InvalidAmount));
    if spent <= 0
        || spent > input
        || (hop.venue != SwapVenue::CometDex && spent != input)
        || received < target
        || (!matches!(hop.venue, SwapVenue::Curve | SwapVenue::Phoenix) && received != target)
    {
        panic_with_error!(env, Error::SlippageExceeded);
    }
    (spent, received)
}

/// Verify pool inverse estimates against the forward quote. Bound RPC-like
/// contract calls per hop; a large discrepancy makes the route unexecutable.
fn sufficient_input(env: &Env, target: i128, estimate: i128, quote: impl Fn(i128) -> i128) -> i128 {
    let mut input = estimate.max(1);
    let (mut last_in, mut last_out) = (0, 0);
    for _ in 0..8 {
        let out = quote(input);
        if out >= target {
            // With integer token units the closest attainable output can
            // exceed the target. Only accept that excess when one less input
            // unit would underfill against this pool's forward quote.
            let mut high = input;
            let mut low = last_in;
            let mut step = 1i128;
            // Search outwards from the estimate first: a rounding plateau
            // is usually much narrower than the interval back to zero.
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
        // Use the local slope on imbalanced pools; the average price from
        // zero can be far from the price for the final units.
        let step = if out > last_out {
            crate::math::mul_div_ceil(env, input - last_in, target - out, out - last_out).max(1)
        } else {
            (input - last_in).max(1)
        };
        (last_in, last_out) = (input, out);
        input = crate::math::checked_add(env, input, step);
    }
    panic_with_error!(env, Error::SlippageExceeded)
}
