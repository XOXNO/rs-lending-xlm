use common::types::DexDistribution;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};

#[contract]
pub struct MockAggregator;

#[contractimpl]
impl MockAggregator {
    pub fn __constructor(_env: Env, _admin: Address) {}

    #[allow(clippy::too_many_arguments)]
    pub fn swap_exact_tokens_for_tokens(
        env: Env,
        token_in: Address,
        token_out: Address,
        amount_in: i128,
        amount_out_min: i128,
        _distribution: Vec<DexDistribution>,
        to: Address,
        _deadline: u64,
    ) -> Vec<Vec<i128>> {
        // Mock execution: pull token_in, push token_out
        let client = soroban_sdk::token::Client::new(&env, &token_in);
        client.transfer_from(
            &env.current_contract_address(),
            &to,
            &env.current_contract_address(), // to aggregator
            &amount_in,
        );

        let out_client = soroban_sdk::token::Client::new(&env, &token_out);
        if amount_out_min > 0 {
            // Note: In tests, the MockAggregator contract must be seeded with token_out
            out_client.transfer(&env.current_contract_address(), &to, &amount_out_min);
        }

        Vec::new(&env)
    }
}

// ---------------------------------------------------------------------------
// BadAggregator: an adversarial swap router used to prove the controller's
// `swap_tokens` helper defends against misbehaving routers (strategy.rs:433).
//
// Three misbehaviors are supported via a simple mode enum stored in instance
// storage so tests can flip the behavior between runs:
//
//   1. Refund: after transferring `amount_out_min` to `to`, also transfer
//      some token_in BACK to `to` -- the controller must detect
//      `balance_in_after > balance_in_before` and panic with InternalError.
//   2. OverPull: pull MORE than `amount_in` from the controller (relying on
//      a lingering allowance or the controller having pre-approved extra).
//   3. OutputShortfall: transfer less than `amount_out_min` -- the ledger
//      delta still verifies, but since nothing is pushed the controller's
//      downstream deposit/repay path surfaces the zero-output failure.
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Copy)]
pub enum BadMode {
    /// After the swap, refund half of `amount_in` back to the caller,
    /// violating the "balance_in must monotonically decrease" invariant.
    Refund,
    /// Pull MORE than `amount_in` from the caller -- the controller's
    /// `actual_in_spent > amount_in` guard must fire.
    OverPull,
    /// Transfer ZERO tokens out even though `amount_out_min > 0`.
    OutputShortfall,
}

#[contract]
pub struct BadAggregator;

#[contractimpl]
impl BadAggregator {
    pub fn __constructor(env: Env, _admin: Address, mode: BadMode) {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "MODE"), &mode);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn swap_exact_tokens_for_tokens(
        env: Env,
        token_in: Address,
        token_out: Address,
        amount_in: i128,
        amount_out_min: i128,
        _distribution: Vec<DexDistribution>,
        to: Address,
        _deadline: u64,
    ) -> Vec<Vec<i128>> {
        let mode: BadMode = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "MODE"))
            .expect("mode must be set by constructor");

        let in_client = soroban_sdk::token::Client::new(&env, &token_in);
        let out_client = soroban_sdk::token::Client::new(&env, &token_out);

        match mode {
            BadMode::Refund => {
                // Send token_in TO the caller without pulling anything.
                // This must make `balance_in_after > balance_in_before`
                // and trigger the controller's InternalError guard. Any
                // excess token_in on the aggregator (test-seeded) is used.
                if amount_out_min > 0 {
                    out_client.transfer(&env.current_contract_address(), &to, &amount_out_min);
                }
                // Net-positive refund: send token_in to the caller.
                let refund = amount_in;
                if refund > 0 {
                    in_client.transfer(&env.current_contract_address(), &to, &refund);
                }
            }
            BadMode::OverPull => {
                // Pull `amount_in * 2` -- the controller's `actual_in_spent`
                // check must fire.
                let overshoot = amount_in.saturating_mul(2);
                in_client.transfer_from(
                    &env.current_contract_address(),
                    &to,
                    &env.current_contract_address(),
                    &overshoot,
                );
                if amount_out_min > 0 {
                    out_client.transfer(&env.current_contract_address(), &to, &amount_out_min);
                }
            }
            BadMode::OutputShortfall => {
                in_client.transfer_from(
                    &env.current_contract_address(),
                    &to,
                    &env.current_contract_address(),
                    &amount_in,
                );
                // Deliberately skip transferring token_out.
            }
        }

        Vec::new(&env)
    }
}
