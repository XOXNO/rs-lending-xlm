//! Strategy orchestration for one `execute_strategy` call.
//!
//! Order of steps: auth, decode program, pull input, optional input fee, run the
//! instruction stream, optional output fee, min-out check, trustline, payout,
//! residual accrual, swap event.

mod residual;
mod reverse;

use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{panic_with_error, token, Address, Env, Executable, Map, Vec};

use common::errors::GenericError;
use common::token::transfer_amount_measured;

use crate::constants::PPM_DENOMINATOR;
use crate::errors::Error;
use crate::events::SwapEvent;
use crate::fees;
use crate::math::{checked_add, checked_mul};
use crate::program::{Mode, Op, Opcode, Program};
use crate::storage;
use crate::types::{StrategyPayload, SwapHop, SwapVenue};
use crate::vault::Vault;
use crate::venues;

/// Token and amount produced by the previous instruction, for [`Mode::Prev`].
///
/// The value is `None` after an instruction with no single well-defined output (a burn
/// releases several constituents). A following `Prev` then fails closed.
type PrevOutput = Option<(Address, i128)>;

/// Everything the instruction loop needs that does not change between steps.
struct Ctx<'a> {
    env: &'a Env,
    router: &'a Address,
    assets: &'a Vec<Address>,
    amounts: &'a Vec<i128>,
    program: &'a Program,
}

/// Runs the strategy in `payload` for `sender` and returns the amount of `token_out` delivered.
///
/// Transfers `total_in` of the input token from `sender` and credits the vault the measured
/// balance delta. Executes the instruction stream, applies fees, pays `sender`, accrues leftover
/// vault balances as admin fee, and publishes [`SwapEvent`].
///
/// Fees apply only with an active referral (see [`fees::apply_fees_on_token`]). Version 2
/// charges them on the fixed payout. Version 1 charges on input unless only output is
/// fee-whitelisted.
///
/// Requires `sender`'s authorization. Panics if `total_in` or the declared minimum output is not
/// positive, if the delivered output falls below the minimum, or if a token's leftover balance
/// exceeds its residual allowance ([`Error::ExcessiveResidual`]).
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
    let program = Program::decode(&env, &ops, &assets, amounts.len());

    let input_token = assets.get_unchecked(program.token_in);
    let output_token = assets.get_unchecked(program.token_out);
    let total_min_out = amounts.get_unchecked(program.min_out);
    if total_min_out <= 0 {
        panic_with_error!(&env, Error::SlippageExceeded);
    }

    let router = env.current_contract_address();
    let mut vault = Vault::new(&env);
    let mut tokens_cache: Map<Address, Vec<Address>> = Map::new(&env);

    // Credit the measured delta, not the declared `total_in`. A fee-on-transfer
    // input would otherwise draw the shortfall from the fee reserve.
    let credited_in = transfer_amount_measured(
        &env,
        &input_token,
        &sender,
        &router,
        total_in,
        GenericError::AmountMustBePositive,
    );
    vault.deposit(&input_token, credited_in);

    let referral_id = program.referral_id;
    let fixed_out = program.fixed_out();
    let fee_on_input = if !fixed_out && referral_id != 0 {
        let list = storage::load_whitelist(&env);
        let in_wl = list.contains(&input_token);
        let out_wl = list.contains(&output_token);

        !out_wl || in_wl
    } else {
        false
    };

    let fee_token = if fee_on_input {
        &input_token
    } else {
        &output_token
    };
    let mut fees_charged = (0, 0);
    if fee_on_input {
        fees_charged = fees::apply_fees_on_token(&env, &mut vault, fee_token, referral_id);
    }

    let ctx = Ctx {
        env: &env,
        router: &router,
        assets: &assets,
        amounts: &amounts,
        program: &program,
    };
    let refund = if fixed_out {
        if credited_in != total_in {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        let (protocol, referral) = fees::fee_amounts(&env, total_min_out, referral_id);
        let gross = checked_add(&env, checked_add(&env, total_min_out, protocol), referral);
        reverse::execute(&ctx, &mut vault, gross, &mut tokens_cache);
        let refund = vault.balance_of(&input_token);
        if refund > 0 {
            vault.withdraw(&input_token, refund);
            if transfer_amount_measured(
                &env,
                &input_token,
                &router,
                &sender,
                refund,
                GenericError::AmountMustBePositive,
            ) != refund
            {
                panic_with_error!(&env, Error::InvalidAmount);
            }
        }
        fees_charged = fees::accrue_fixed_output_fees(
            &env,
            &mut vault,
            fee_token,
            referral_id,
            total_min_out,
            (protocol, referral),
        );
        refund
    } else {
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
            fees_charged = fees::apply_fees_on_token(&env, &mut vault, fee_token, referral_id);
        }
        0
    };

    let payout = settle_output(
        &env,
        &mut vault,
        &router,
        &sender,
        &output_token,
        total_min_out,
        fixed_out,
    );

    residual::accrue_residual_as_revenue(&env, &mut vault);

    SwapEvent {
        fee_token: fee_token.clone(),
        sender,
        token_in: input_token.clone(),
        token_out: output_token.clone(),
        amount_in: credited_in - refund,
        amount_out: payout,
        referral_id,
        static_fee: fees_charged.0,
        referral_fee: fees_charged.1,
    }
    .publish(&env);

    payout
}

/// Transfers the output remaining after fee accrual and verifies a fixed payout.
fn settle_output(
    env: &Env,
    vault: &mut Vault,
    router: &Address,
    sender: &Address,
    output_token: &Address,
    min_out: i128,
    fixed_out: bool,
) -> i128 {
    let output = vault.balance_of(output_token);
    if output < min_out {
        panic_with_error!(env, Error::SlippageExceeded);
    }

    vault.withdraw(output_token, output);
    ensure_trustline(env, output_token, sender);
    if !fixed_out {
        token::Client::new(env, output_token).transfer(router, sender, &output);
        return output;
    }

    let received = transfer_amount_measured(
        env,
        output_token,
        router,
        sender,
        min_out,
        GenericError::AmountMustBePositive,
    );
    if received != min_out {
        panic_with_error!(env, Error::SlippageExceeded);
    }

    received
}

/// Builds the swap hop of `op` from its registry indices.
fn swap_hop(ctx: &Ctx<'_>, op: &Op, venue: SwapVenue) -> SwapHop {
    SwapHop {
        pool: ctx.assets.get_unchecked(op.idx_a),
        token_in: ctx.assets.get_unchecked(op.idx_b),
        token_out: ctx.assets.get_unchecked(op.idx_c),
        venue,
    }
}

/// Opens `receiver`'s missing trustline via the SAC `trust` call (CAP-73).
///
/// Skips non-SAC tokens, so custom token code never runs here. The SAC is a no-auth no-op for
/// a contract receiver or an existing trustline. The function ignores failures. The payout
/// transfer reverts if the trustline is still missing.
pub(super) fn ensure_trustline(env: &Env, token: &Address, receiver: &Address) {
    if token.executable() != Some(Executable::StellarAsset) {
        return;
    }
    let _ = StellarAssetClient::new(env, token).try_trust(receiver);
}

/// Executes one instruction and returns its output token and amount for a
/// following [`Mode::Prev`], or `None` when the instruction has no single
/// well-defined output (a burn releases every pool constituent at once).
///
/// Panics if a swap's resolved input amount is not positive, or if the venue
/// dispatch returns a non-positive output amount.
fn execute_op(
    ctx: &Ctx<'_>,
    vault: &mut Vault,
    op: Op,
    prev: PrevOutput,
    tokens_cache: &mut Map<Address, Vec<Address>>,
) -> PrevOutput {
    match op.opcode {
        Opcode::Swap(venue) => {
            let hop = swap_hop(ctx, &op, venue);
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
            None
        }
        Opcode::Mint => {
            let lp_token = ctx.assets.get_unchecked(op.idx_b);
            let shares = venues::aquarius::add_liquidity(
                ctx.env,
                ctx.router,
                vault,
                &ctx.assets.get_unchecked(op.idx_a),
                &lp_token,
                ctx.amounts.get_unchecked(op.idx_c),
                tokens_cache,
            );
            Some((lp_token, shares))
        }
    }
}

/// Resolves an instruction's input amount according to `mode`: the vault's
/// full balance of `token_in`, the previous instruction's output, a fixed
/// amount from the payload, or a parts-per-million share of the available
/// balance.
///
/// Panics if `mode` is [`Mode::Prev`] and there is no previous output, or its
/// token does not match `token_in`.
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
