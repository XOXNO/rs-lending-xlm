use soroban_sdk::{
    auth::InvokerContractAuthEntry, panic_with_error, symbol_short, token, vec, Address, Env,
    IntoVal, Map, Symbol, Val, Vec,
};

use crate::errors::Error;

/// Aquarius states swap fees in basis points out of this denominator.
const FEE_DENOMINATOR: i128 = 10_000;

/// Hard cap on pre-swap bisection passes, matching MultiversX's zap.
const MAX_PRE_SWAP_ITERATIONS: u32 = 128;
use crate::vault::Vault;
use crate::venues::{auth_entry, authorize_as_current, authorize_token_transfer, HopContext};

/// Authorizes the pool's pull and invokes its `swap`, returning what the pool
/// claims to have paid out. Callers settle on measured balance deltas instead —
/// this figure is only good for a cheap zero check.
fn invoke_pool_swap(
    env: &Env,
    router: &Address,
    pool: &Address,
    token_in: &Address,
    in_idx: u32,
    out_idx: u32,
    amount_in: i128,
) -> u128 {
    authorize_token_transfer(env, token_in, router, pool, amount_in);
    let args: Vec<Val> = vec![
        env,
        router.into_val(env),
        in_idx.into_val(env),
        out_idx.into_val(env),
        to_u128(env, amount_in).into_val(env),
        0_u128.into_val(env),
    ];
    env.invoke_contract(pool, &symbol_short!("swap"), args)
}

pub(crate) fn swap(ctx: &HopContext<'_>, cache: &mut Map<Address, Vec<Address>>) -> i128 {
    let tokens = pool_tokens(ctx.env, cache, &ctx.hop.pool);
    let in_idx = find_index(ctx.env, &tokens, &ctx.hop.token_in);
    let out_idx = find_index(ctx.env, &tokens, &ctx.hop.token_out);

    let reported = invoke_pool_swap(
        ctx.env,
        ctx.router,
        &ctx.hop.pool,
        &ctx.hop.token_in,
        in_idx,
        out_idx,
        ctx.amount_in,
    );
    if reported == 0 {
        panic_with_error!(ctx.env, Error::ZeroOutput);
    }
    reported
        .try_into()
        .unwrap_or_else(|_| panic_with_error!(ctx.env, Error::IntegerOverflow))
}

/// Deposit every constituent balance the vault currently holds and credit the
/// minted shares back to it.
///
/// The pool pulls the FULL `desired_amounts` and refunds whatever it did not
/// consume — a constant-product pool keeps only the balanced subset, a stable
/// pool keeps everything and prices the imbalance into the shares. So the
/// authorization covers the full amounts, and both legs are settled by measured
/// balance deltas rather than by what we asked for or what the pool reports.
/// Verified on testnet: a 500000000 XLM leg was pulled in full and 1489866
/// refunded in the same call.
pub(crate) fn add_liquidity(
    env: &Env,
    router: &Address,
    vault: &mut Vault,
    pool: &Address,
    lp_token: &Address,
    min_shares: i128,
    pre_balance_fee_bps: u32,
    cache: &mut Map<Address, Vec<Address>>,
) -> i128 {
    let tokens = pool_tokens(env, cache, pool);
    assert_share_token(env, pool, lp_token);
    if min_shares <= 0 {
        panic_with_error!(env, Error::MinSharesNotMet);
    }

    pre_balance(
        env,
        router,
        vault,
        pool,
        &tokens,
        i128::from(pre_balance_fee_bps),
    );

    let n = tokens.len();
    let mut amounts: Vec<u128> = Vec::new(env);
    let mut before: Vec<i128> = Vec::new(env);
    let mut held: Vec<i128> = Vec::new(env);
    let mut total_requested: i128 = 0;
    for i in 0..n {
        let token = tokens.get_unchecked(i);
        let amount = vault.balance_of(&token);
        amounts.push_back(to_u128(env, amount));
        held.push_back(amount);
        // A constituent we are depositing none of cannot be spent, so its
        // balance never needs reading. Every skipped read is a cross-contract
        // call saved, and single-sided deposits skip one per pool. Widening this
        // to `>=` only costs a wasted read, so no test can observe it.
        before.push_back(if amount > 0 {
            token::Client::new(env, &token).balance(router)
        } else {
            0
        });
        total_requested = total_requested
            .checked_add(amount)
            .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
    }
    if total_requested <= 0 {
        panic_with_error!(env, Error::InvalidAmount);
    }

    let pulled = held.clone();
    let mut auth: Vec<InvokerContractAuthEntry> = Vec::new(env);
    for i in 0..n {
        let amount = pulled.get_unchecked(i);
        // Widening to `>=` would authorize a zero transfer the pool never makes;
        // an unused entry is ignored, so it is unobservable.
        if amount > 0 {
            auth.push_back(auth_entry(
                env,
                &tokens.get_unchecked(i),
                "transfer",
                vec![
                    env,
                    router.into_val(env),
                    pool.into_val(env),
                    amount.into_val(env),
                ],
                vec![env],
            ));
        }
    }

    let shares_before = token::Client::new(env, lp_token).balance(router);

    env.authorize_as_current_contract(auth);
    let _: (Vec<u128>, u128) = env.invoke_contract(
        pool,
        &Symbol::new(env, "deposit"),
        vec![
            env,
            router.into_val(env),
            amounts.into_val(env),
            to_u128(env, min_shares).into_val(env),
        ],
    );

    for i in 0..n {
        if held.get_unchecked(i) == 0 {
            continue;
        }
        let token = tokens.get_unchecked(i);
        let spent = before
            .get_unchecked(i)
            .checked_sub(token::Client::new(env, &token).balance(router))
            .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
        // Unreachable in practice — a deposit cannot hand the router MORE of an
        // input token than it started with — but kept because the alternative is
        // trusting a venue's arithmetic.
        if spent < 0 {
            panic_with_error!(env, Error::InvalidAmount);
        }

        vault.withdraw(&token, spent);
    }

    let shares = token::Client::new(env, lp_token)
        .balance(router)
        .checked_sub(shares_before)
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
    if shares < min_shares {
        panic_with_error!(env, Error::MinSharesNotMet);
    }
    vault.deposit(lp_token, shares);
    shares
}

/// Burn the vault's entire LP balance and credit the released constituents.
///
/// The pool burns the shares straight off the router via the share token, so the
/// only authorization handed out is that single `burn`.
pub(crate) fn remove_liquidity(
    env: &Env,
    router: &Address,
    vault: &mut Vault,
    pool: &Address,
    lp_token: &Address,
    min_amounts_in: &Vec<i128>,
    cache: &mut Map<Address, Vec<Address>>,
) {
    let tokens = pool_tokens(env, cache, pool);
    assert_share_token(env, pool, lp_token);

    let n = tokens.len();
    if min_amounts_in.len() != n {
        panic_with_error!(env, Error::MinAmountsNotMet);
    }

    let shares = vault.balance_of(lp_token);
    if shares <= 0 {
        panic_with_error!(env, Error::InvalidAmount);
    }

    let mut min_amounts: Vec<u128> = Vec::new(env);
    let mut before: Vec<i128> = Vec::new(env);
    for i in 0..n {
        min_amounts.push_back(to_u128(env, min_amounts_in.get_unchecked(i)));
        before.push_back(token::Client::new(env, &tokens.get_unchecked(i)).balance(router));
    }
    let shares_before = token::Client::new(env, lp_token).balance(router);

    authorize_as_current(
        env,
        lp_token,
        "burn",
        vec![env, router.into_val(env), shares.into_val(env)],
    );
    let _: Vec<u128> = env.invoke_contract(
        pool,
        &Symbol::new(env, "withdraw"),
        vec![
            env,
            router.into_val(env),
            to_u128(env, shares).into_val(env),
            min_amounts.into_val(env),
        ],
    );

    let burned = shares_before
        .checked_sub(token::Client::new(env, lp_token).balance(router))
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
    if burned != shares {
        panic_with_error!(env, Error::InvalidAmount);
    }
    vault.withdraw(lp_token, shares);

    let mut total_received: i128 = 0;
    for i in 0..n {
        let token = tokens.get_unchecked(i);
        let received = token::Client::new(env, &token)
            .balance(router)
            .checked_sub(before.get_unchecked(i))
            .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
        if received < min_amounts_in.get_unchecked(i) {
            panic_with_error!(env, Error::MinAmountsNotMet);
        }
        total_received = total_received
            .checked_add(received)
            .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
        vault.deposit(&token, received);
    }
    if total_received <= 0 {
        panic_with_error!(env, Error::ZeroOutput);
    }
}

/// Swaps the excess side into the scarce one so the deposit that follows lands
/// on the pool's ratio, using the balances actually in the vault.
///
/// Deciding this off-chain cannot work. A `split_ppm` only resolves to one part
/// per million, and the hop that produces the second constituent slips between
/// quote and execution, so a pre-computed split is imbalanced by construction —
/// the pool then refuses the surplus and it becomes revenue taken from the user.
/// Balancing here, against real balances and live reserves, has nothing left to
/// predict.
///
/// Only constant-product pools need it. A stable pool consumes every amount
/// offered and prices the imbalance into the shares, which costs less than the
/// swap fee a rebalance would pay, so single-sided is already its optimum.
fn pre_balance(
    env: &Env,
    router: &Address,
    vault: &mut Vault,
    pool: &Address,
    tokens: &Vec<Address>,
    fee_bps: i128,
) {
    // `fee_bps == 0` is the payload saying "do not pre-balance" — stable
    // pools consume imbalance instead of refunding it, so balancing them
    // pays a swap fee for nothing. The kind and fee travel in the payload
    // because reading them from the pool costs two cross-contract calls,
    // and the per-call VM-instantiation memory wall (the 8th call into
    // this pool trips `Budget(ExceededLimit)`, measured on testnet) leaves
    // budget for exactly one extra pool call: the pre-swap itself.
    if fee_bps <= 0 || tokens.len() != 2 {
        return;
    }
    let token_a = tokens.get_unchecked(0);
    let token_b = tokens.get_unchecked(1);
    let held_a = vault.balance_of(&token_a);
    let held_b = vault.balance_of(&token_b);

    let reserves = pool_reserves(env, pool);
    if reserves.len() != 2 {
        return;
    }
    let reserve_a = reserves.get_unchecked(0);
    let reserve_b = reserves.get_unchecked(1);
    if !pre_balance_possible(held_a, held_b, reserve_a, reserve_b) {
        return;
    }

    let (from_a, amount) = optimal_pre_swap(env, held_a, held_b, reserve_a, reserve_b, fee_bps);
    if amount <= 0 {
        return;
    }

    let (token_in, token_out, in_idx, out_idx) = if from_a {
        (token_a, token_b, 0u32, 1u32)
    } else {
        (token_b, token_a, 1u32, 0u32)
    };
    let received = swap_through_pool(
        env,
        router,
        pool,
        (&token_in, &token_out),
        (in_idx, out_idx),
        amount,
    );
    vault.withdraw(&token_in, amount);
    vault.deposit(&token_out, received);
}

/// Largest swap that still leaves the scarce side scarce.
///
/// `f(s) = (held_in - s)·(reserve_out - dy) - (held_out + dy)·(reserve_in + s)`
/// is the balance condition written as a difference; it falls monotonically in
/// `s`, so a bisection finds the crossing. Following MultiversX's zap, there is
/// deliberately no "close enough" short-circuit: the pool's own division
/// truncates, so even a nearly balanced pair has an optimum worth finding.
pub(crate) fn optimal_pre_swap(
    env: &Env,
    held_a: i128,
    held_b: i128,
    reserve_a: i128,
    reserve_b: i128,
    fee_bps: i128,
) -> (bool, i128) {
    let product_a = checked_mul(env, held_a, reserve_b);
    let product_b = checked_mul(env, held_b, reserve_a);
    if product_a == product_b {
        return (true, 0);
    }
    // Equality already returned above, so `>` and `>=` cannot differ here.
    let from_a = product_a > product_b;
    let (held_in, held_out, reserve_in, reserve_out) = if from_a {
        (held_a, held_b, reserve_a, reserve_b)
    } else {
        (held_b, held_a, reserve_b, reserve_a)
    };

    let mut low: i128 = 0;
    let mut high: i128 = held_in;
    // Bounded rather than `while high - low > 1`: the loop halves its range each
    // pass so it converges long before the cap, but leaving termination to
    // depend on the arithmetic being right is not a property worth betting a
    // transaction on. 128 covers the full i128 range.
    for _ in 0..MAX_PRE_SWAP_ITERATIONS {
        // Widening this to `+` cannot hang now the loop is bounded; it only
        // stops the early exit, and the search still settles on the same answer.
        if high - low <= 1 {
            break;
        }
        let mid = low + (high - low) / 2;
        let out = cp_swap_out(env, mid, reserve_in, reserve_out, fee_bps);
        let left = checked_mul(env, held_in - mid, reserve_out - out);
        let right = checked_mul(env, held_out + out, reserve_in + mid);
        // On exact equality either branch is correct and the answer moves by at
        // most one atomic unit — far below the dust floor.
        if left > right {
            low = mid;
        } else {
            high = mid;
        }
    }
    (from_a, low)
}

/// Constant-product output with the fee taken on the way in, which is how
/// Aquarius charges it — calibrated against a live trade to the unit.
pub(crate) fn cp_swap_out(
    env: &Env,
    amount_in: i128,
    reserve_in: i128,
    reserve_out: i128,
    fee_bps: i128,
) -> i128 {
    if amount_in <= 0 {
        return 0;
    }
    let net = checked_mul(env, amount_in, FEE_DENOMINATOR - fee_bps) / FEE_DENOMINATOR;
    if net <= 0 {
        return 0;
    }
    checked_mul(env, net, reserve_out) / (reserve_in + net)
}




/// Runs one swap through the pool, crediting only the measured output delta.
fn swap_through_pool(
    env: &Env,
    router: &Address,
    pool: &Address,
    tokens: (&Address, &Address),
    indices: (u32, u32),
    amount_in: i128,
) -> i128 {
    let (token_in, token_out) = tokens;
    let (in_idx, out_idx) = indices;
    let before = token::Client::new(env, token_out).balance(router);
    invoke_pool_swap(env, router, pool, token_in, in_idx, out_idx, amount_in);
    let received = token::Client::new(env, token_out)
        .balance(router)
        .checked_sub(before)
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
    if received <= 0 {
        panic_with_error!(env, Error::ZeroOutput);
    }
    received
}

fn pool_reserves(env: &Env, pool: &Address) -> Vec<i128> {
    let reserves: Vec<u128> = env.invoke_contract(
        pool,
        &Symbol::new(env, "get_reserves"),
        Vec::<Val>::new(env),
    );
    let mut out: Vec<i128> = Vec::new(env);
    for value in reserves.iter() {
        out.push_back(to_i128(env, value));
    }
    out
}

fn to_i128(env: &Env, amount: u128) -> i128 {
    amount
        .try_into()
        .unwrap_or_else(|_| panic_with_error!(env, Error::IntegerOverflow))
}

fn checked_mul(env: &Env, lhs: i128, rhs: i128) -> i128 {
    lhs.checked_mul(rhs)
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow))
}

/// Whether a pre-balance swap can run at all: something to balance, and a pool
/// with reserves on both sides to balance against.
pub(crate) fn pre_balance_possible(
    held_a: i128,
    held_b: i128,
    reserve_a: i128,
    reserve_b: i128,
) -> bool {
    (held_a > 0 || held_b > 0) && reserve_a > 0 && reserve_b > 0
}

/// Pool constituents, read once per transaction. A route's hop and its LP leg
/// address the same pool, and every cross-contract call re-instantiates the
/// pool's VM against the transaction memory budget — the measured wall sits
/// within one call of a pre-balanced mint, so the duplicate `get_tokens` is
/// the difference between fitting and `Budget(ExceededLimit)`.
fn pool_tokens(env: &Env, cache: &mut Map<Address, Vec<Address>>, pool: &Address) -> Vec<Address> {
    if let Some(tokens) = cache.get(pool.clone()) {
        return tokens;
    }
    let tokens: Vec<Address> =
        env.invoke_contract(pool, &Symbol::new(env, "get_tokens"), Vec::<Val>::new(env));
    if tokens.is_empty() {
        panic_with_error!(env, Error::BrokenTokenChain);
    }
    cache.set(pool.clone(), tokens.clone());
    tokens
}

/// Bind the declared LP token to the pool's own `share_id`, so a payload cannot
/// point the mint/burn accounting at a token the pool does not actually issue.
fn assert_share_token(env: &Env, pool: &Address, lp_token: &Address) {
    let share: Address =
        env.invoke_contract(pool, &Symbol::new(env, "share_id"), Vec::<Val>::new(env));
    if share != *lp_token {
        panic_with_error!(env, Error::LpTokenMismatch);
    }
}

fn to_u128(env: &Env, amount: i128) -> u128 {
    amount
        .try_into()
        .unwrap_or_else(|_| panic_with_error!(env, Error::IntegerOverflow))
}

fn find_index(env: &Env, tokens: &Vec<Address>, target: &Address) -> u32 {
    let n = tokens.len();
    for i in 0..n {
        if tokens.get(i).as_ref() == Some(target) {
            return i;
        }
    }
    panic_with_error!(env, Error::BrokenTokenChain);
}
