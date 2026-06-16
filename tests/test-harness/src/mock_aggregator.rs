use soroban_sdk::{
    contract, contractimpl, contracttype, xdr::FromXdr, Address, Bytes, Env, Symbol,
};

use crate::strategy::MockSwapPayload;
// MockAggregator
//
// Minimal stand-in for the strategy router. The controller treats strategy
// bytes as opaque; this mock decodes a test-only payload so harness scenarios
// can choose output token and amount.

#[contract]
pub struct MockAggregator;

#[contractimpl]
impl MockAggregator {
    pub fn __constructor(_env: Env, _admin: Address) {}

    /// Returns the total `token_out` delivered to `sender`.
    pub fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128 {
        sender.require_auth();
        let router = env.current_contract_address();
        let payload =
            MockSwapPayload::from_xdr(&env, &swap_xdr).expect("mock swap payload must decode");

        let in_client = soroban_sdk::token::Client::new(&env, &payload.token_in);
        in_client.transfer(&sender, &router, &total_in);

        if payload.min_out > 0 {
            let out_client = soroban_sdk::token::Client::new(&env, &payload.token_out);
            out_client.transfer(&router, &sender, &payload.min_out);
        }

        payload.min_out
    }
}

// Adversarial aggregator for controller-side router validation.
//
// Three misbehaviors are supported via a simple mode enum stored in instance
// storage so tests can flip the behavior between runs:
//
//   1. Refund: send extra `token_in` BACK to `sender` after the swap —
//      the controller must detect `balance_in_after > balance_in_before`
//      and panic with InternalError.
//   2. OverPull: pull MORE than `total_in` from `sender` —
//      the controller's `actual_in_spent > amount_in` guard must fire.
//   3. UnderPull: pull less than `total_in` while still returning output —
//      the controller must refund leftover input instead of stranding it.
//   4. OutputShortfall: pull input but skip the output transfer —
//      the controller's positive-output balance-delta guard must fire.

#[contracttype]
#[derive(Clone, Copy)]
pub enum BadMode {
    Refund,
    OverPull,
    UnderPull,
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

    pub fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128 {
        sender.require_auth();
        let router = env.current_contract_address();
        let payload =
            MockSwapPayload::from_xdr(&env, &swap_xdr).expect("mock swap payload must decode");
        let mode: BadMode = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "MODE"))
            .expect("mode must be set by constructor");

        let in_client = soroban_sdk::token::Client::new(&env, &payload.token_in);
        let out_client = soroban_sdk::token::Client::new(&env, &payload.token_out);

        match mode {
            BadMode::Refund => {
                if payload.min_out > 0 {
                    out_client.transfer(&router, &sender, &payload.min_out);
                }
                in_client.transfer(&router, &sender, &total_in);
            }
            BadMode::OverPull => {
                let overshoot = total_in.saturating_mul(2);
                in_client.transfer(&sender, &router, &overshoot);
                if payload.min_out > 0 {
                    out_client.transfer(&router, &sender, &payload.min_out);
                }
            }
            BadMode::UnderPull => {
                in_client.transfer(&sender, &router, &(total_in / 2));
                if payload.min_out > 0 {
                    out_client.transfer(&router, &sender, &payload.min_out);
                }
            }
            BadMode::OutputShortfall => {
                in_client.transfer(&sender, &router, &total_in);
            }
        }

        payload.min_out
    }
}
