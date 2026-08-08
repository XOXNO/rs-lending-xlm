//! Strategy orchestration for one `execute_strategy` call.
//!
//! Flow: auth → decode program → pull input → optional input fee → run the
//! instruction stream → optional output fee → min-out check → payout →
//! residual accrual.

mod residual;

use soroban_sdk::{panic_with_error, token, Address, Env, Map, Vec};

use crate::constants::PPM_DENOMINATOR;
use crate::errors::Error;
use crate::fees;
use crate::math::checked_mul;
use crate::program::{Mode, Op, Opcode, Program};
use crate::storage;
use crate::types::{StrategyPayload, SwapHop};
use crate::vault::Vault;
use crate::venues;

/// Token and amount produced by the previous instruction, for [`Mode::Prev`].
///
/// `None` after an instruction with no single well-defined output (a burn
/// releases several constituents), which makes a following `Prev` fail closed.
type PrevOutput = Option<(Address, i128)>;

/// Everything the instruction loop needs that does not change between steps.
struct Ctx<'a> {
    env: &'a Env,
    router: &'a Address,
    assets: &'a Vec<Address>,
    amounts: &'a Vec<i128>,
    program: &'a Program,
}

/// Execute a decoded payload; returns delivered `token_out` to `sender`.
///
/// Fee side: with a referral, fee applies on input unless only the output token
/// is whitelisted (then on output).
pub(crate) fn run(env: Env, sender: Address, total_in: i128, payload: StrategyPayload) -> i128 {
    sender.require_auth();

    if total_in <= 0 {
        panic_with_error!(&env, Error::InvalidAmount);
    }

    let StrategyPayload {
        amounts,
        assets,
        ops,
    } = payload;
    let program = Program::decode(&env, &ops, assets.len(), amounts.len());

    let input_token = assets.get_unchecked(program.token_in);
    let output_token = assets.get_unchecked(program.token_out);
    let total_min_out = amounts.get_unchecked(program.min_out);
    if total_min_out <= 0 {
        panic_with_error!(&env, Error::SlippageExceeded);
    }

    let router = env.current_contract_address();
    let mut vault = Vault::new(&env);
    let mut tokens_cache: Map<Address, Vec<Address>> = Map::new(&env);

    token::Client::new(&env, &input_token).transfer(&sender, &router, &total_in);
    vault.deposit(&input_token, total_in);

    let referral_id = program.referral_id;
    let fee_on_input = if referral_id != 0 {
        let list = storage::load_whitelist(&env);
        let in_wl = list.contains(&input_token);
        let out_wl = list.contains(&output_token);

        !out_wl || in_wl
    } else {
        false
    };

    if fee_on_input {
        fees::apply_fees_on_token(&env, &mut vault, &input_token, referral_id);
    }

    let ctx = Ctx {
        env: &env,
        router: &router,
        assets: &assets,
        amounts: &amounts,
        program: &program,
    };
    let mut prev: PrevOutput = None;
    for i in 0..program.len() {
        prev = execute_op(
            &ctx,
            &mut vault,
            program.op(&env, i),
            prev,
            &mut tokens_cache,
        );
    }

    if !fee_on_input {
        fees::apply_fees_on_token(&env, &mut vault, &output_token, referral_id);
    }

    let total_out = vault.balance_of(&output_token);
    if total_out < total_min_out {
        panic_with_error!(&env, Error::SlippageExceeded);
    }

    vault.withdraw(&output_token, total_out);
    token::Client::new(&env, &output_token).transfer(&router, &sender, &total_out);

    residual::accrue_residual_as_revenue(&env, &mut vault);

    total_out
}

/// Run one instruction; returns its output for a following [`Mode::Prev`].
fn execute_op(
    ctx: &Ctx<'_>,
    vault: &mut Vault,
    op: Op,
    prev: PrevOutput,
    tokens_cache: &mut Map<Address, Vec<Address>>,
) -> PrevOutput {
    match op.opcode {
        Opcode::Swap(venue) => {
            let hop = SwapHop {
                pool: ctx.assets.get_unchecked(op.idx_a),
                token_in: ctx.assets.get_unchecked(op.idx_b),
                token_out: ctx.assets.get_unchecked(op.idx_c),
                venue,
            };
            let amount_in = resolve_amount(ctx, vault, op.mode, &hop.token_in, prev);
            if amount_in <= 0 {
                panic_with_error!(ctx.env, Error::InvalidAmount);
            }

            vault.withdraw(&hop.token_in, amount_in);
            let out = venues::dispatch_hop(ctx.env, ctx.router, &hop, amount_in, tokens_cache);
            if out <= 0 {
                panic_with_error!(ctx.env, Error::ZeroOutput);
            }
            vault.deposit(&hop.token_out, out);
            Some((hop.token_out, out))
        }
        Opcode::Burn => {
            venues::aquarius::remove_liquidity(
                ctx.env,
                ctx.router,
                vault,
                &ctx.assets.get_unchecked(op.idx_a),
                &ctx.assets.get_unchecked(op.idx_b),
                ctx.amounts,
                op.idx_c,
                tokens_cache,
            );
            // A burn releases every constituent; there is no single "previous
            // output" for the next instruction to chain onto.
            None
        }
        Opcode::Mint => {
            let lp_token = ctx.assets.get_unchecked(op.idx_b);
            let shares = venues::aquarius::add_liquidity(
                ctx.env,
                ctx.router,
                vault,
                venues::aquarius::MintLiquidity {
                    pool: &ctx.assets.get_unchecked(op.idx_a),
                    lp_token: &lp_token,
                    min_shares: ctx.amounts.get_unchecked(op.idx_c),
                },
                tokens_cache,
            );
            Some((lp_token, shares))
        }
    }
}

/// Size an instruction's input from its [`Mode`].
fn resolve_amount(
    ctx: &Ctx<'_>,
    vault: &Vault,
    mode: Mode,
    token_in: &Address,
    prev: PrevOutput,
) -> i128 {
    match mode {
        Mode::All => vault.balance_of(token_in),
        Mode::Prev => {
            let Some((prev_token, prev_amount)) = prev else {
                panic_with_error!(ctx.env, Error::BrokenTokenChain);
            };
            if prev_token != *token_in {
                panic_with_error!(ctx.env, Error::BrokenTokenChain);
            }
            prev_amount
        }
        Mode::Fixed(idx) => ctx.amounts.get_unchecked(idx as u32),
        Mode::Ppm(idx) => {
            let available = vault.balance_of(token_in);
            checked_mul(ctx.env, available, ctx.program.weight(idx as u32) as i128)
                / PPM_DENOMINATOR
        }
    }
}
