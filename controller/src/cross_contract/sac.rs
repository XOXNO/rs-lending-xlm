//! Thin wrapper around the Soroban Asset Contract (SAC) cross-contract
//! ABI. Lifting the cross-contract call into its own module gives a
//! clean substitution boundary under `--features certora`; the
//! sibling [`crate::cross_contract::pool`] uses the same pattern.

use soroban_sdk::{Address, Env};

pub(crate) fn sac_transfer_call(env: &Env, token: &Address, from: &Address, to: &Address, amount: &i128) {
    soroban_sdk::token::Client::new(env, token).transfer(from, to, amount)
}
