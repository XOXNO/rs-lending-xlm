//! Strategy orchestration for one `execute_strategy` call.
//!
//! Flow: auth → pull input → optional input fee → burn → paths → mint →
//! optional output fee → min-out check → payout → residual accrual.

mod path_util;
mod paths;
mod residual;
mod validate;

use soroban_sdk::{panic_with_error, token, Address, Env, Map, Vec};

use crate::errors::Error;
use crate::fees;
use crate::storage;
use crate::types::StrategyPayload;
use crate::vault::Vault;
use crate::venues;

/// Execute a decoded payload; returns delivered `token_out` to `sender`.
///
/// Fee side: with a referral, fee applies on input unless only the output token
/// is whitelisted (then on output).
pub(crate) fn run(env: Env, sender: Address, total_in: i128, payload: StrategyPayload) -> i128 {
    sender.require_auth();

    if total_in <= 0 {
        panic_with_error!(&env, Error::InvalidAmount);
    }
    if payload.total_min_out <= 0 {
        panic_with_error!(&env, Error::SlippageExceeded);
    }

    let input_token = payload.token_in.clone();
    let output_token = payload.token_out.clone();
    validate::validate_payload(&env, &payload);

    let router = env.current_contract_address();
    let mut vault = Vault::new(&env);

    let mut tokens_cache: Map<Address, Vec<Address>> = Map::new(&env);

    token::Client::new(&env, &input_token).transfer(&sender, &router, &total_in);
    vault.deposit(&input_token, total_in);

    let fee_on_input = if payload.referral_id != 0 {
        let list = storage::load_whitelist(&env);
        let in_wl = list.contains(&input_token);
        let out_wl = list.contains(&output_token);

        !out_wl || in_wl
    } else {
        false
    };

    if fee_on_input {
        fees::apply_fees_on_token(&env, &mut vault, &input_token, payload.referral_id);
    }

    if let Some(pool) = payload.burn_pool.as_ref() {
        venues::aquarius::remove_liquidity(
            &env,
            &router,
            &mut vault,
            pool,
            &input_token,
            &payload.burn_min_amounts,
            &mut tokens_cache,
        );
    }

    paths::execute_paths(&env, &router, &mut vault, &payload.paths, &mut tokens_cache);

    if let Some(pool) = payload.mint_pool.as_ref() {
        venues::aquarius::add_liquidity(
            &env,
            &router,
            &mut vault,
            venues::aquarius::MintLiquidity {
                pool,
                lp_token: &output_token,
                min_shares: payload.mint_min_shares,
                pre_swap: venues::aquarius::PreSwap {
                    from_a: payload.pre_swap_from_a,
                    amount: payload.pre_swap_amount,
                },
            },
            &mut tokens_cache,
        );
    }

    if !fee_on_input {
        fees::apply_fees_on_token(&env, &mut vault, &output_token, payload.referral_id);
    }

    let total_out = vault.balance_of(&output_token);
    if total_out < payload.total_min_out {
        panic_with_error!(&env, Error::SlippageExceeded);
    }

    vault.withdraw(&output_token, total_out);
    token::Client::new(&env, &output_token).transfer(&router, &sender, &total_out);

    residual::accrue_residual_as_revenue(&env, &mut vault);

    total_out
}
