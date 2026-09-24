#![no_std]
#![deny(unsafe_code)]
//! # Swap Aggregator (Router)
//!
//! Executes multi-hop, multi-venue swap strategies built off-chain and passed
//! as XDR. Used by controller strategies; venues are untrusted — only measured
//! balance deltas count.
//!
//! | Layer | Role |
//! |-------|------|
//! | [`Router`] | Public entrypoints and Ownable |
//! | `execute` | Strategy run: pull, instruction stream, fees, settle |
//! | `fees` | Static + referral fee apply and claim |
//! | `storage` | Keys, TTL, fee buckets, whitelist, referrals |
//! | `vault` | Invocation-local token ledger |
//! | `venues` | Per-DEX hop adapters and Aquarius LP |

mod constants;
mod errors;
mod execute;
mod fees;
mod math;
mod program;
mod storage;
mod types;
mod vault;
mod venues;

// Test payload builders assemble registries before their length is known;
// `alloc` is available under `cargo test` and never linked into the Wasm.
#[cfg(test)]
extern crate alloc;

#[cfg(test)]
#[path = "../tests/unit/mod.rs"]
mod test;

#[cfg(test)]
pub(crate) use constants::residual_allowance;
#[cfg(test)]
pub(crate) use storage::reserved_fee_balance;

use common::ttl::renew_instance;

use soroban_sdk::{
    contract, contractimpl, panic_with_error, token, xdr::FromXdr, Address, Bytes, BytesN,
    ContractExecutable, Env, Vec,
};

use stellar_access::ownable::{self, Ownable};
use stellar_macros::only_owner;
use swap_aggregator_interface::SwapAggregatorInterface;

use crate::constants::FEE_CAP;
use crate::errors::Error;
use crate::fees::FeeBucket;
use crate::types::{ReferralConfig, StrategyPayload};

/// Deployed swap router instance.
#[contract]
pub struct Router;

#[contractimpl]
impl Router {
    /// Sets `admin` as the Ownable owner.
    ///
    /// The static fee and referral counter stay unwritten; both read as zero when unset.
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);
        renew_instance(&env);
    }
}

#[contractimpl]
impl SwapAggregatorInterface for Router {
    /// Sets the protocol static fee in BPS (`<= FEE_CAP`). Owner only.
    #[only_owner]
    fn set_static_fee(env: Env, fee_bps: u32) {
        renew_instance(&env);
        fees::set_static_fee(&env, fee_bps);
    }

    /// Adds `token` to the fee whitelist. Owner only.
    ///
    /// A referral swap takes fees on the output token only when the output token is
    /// whitelisted and the input token is not.
    #[only_owner]
    fn add_to_whitelist(env: Env, token: Address) {
        renew_instance(&env);
        let mut list = storage::load_whitelist(&env);
        if !list.contains(&token) {
            list.push_back(token);
            storage::set_whitelist(&env, &list);
        }
    }

    /// Removes `token` from the fee whitelist. Owner only.
    #[only_owner]
    fn remove_from_whitelist(env: Env, token: Address) {
        renew_instance(&env);
        let mut list = storage::load_whitelist(&env);
        if let Some(idx) = list.first_index_of(&token) {
            list.remove(idx);
            storage::set_whitelist(&env, &list);
        }
    }

    /// Replaces the contract Wasm with `new_wasm_hash`. Owner only.
    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        renew_instance(&env);
        env.deployer()
            .update_current_contract(ContractExecutable::Wasm(new_wasm_hash));
    }

    /// Creates an active referral and returns its id. Owner only.
    ///
    /// Panics with `Error::FeeTooHigh` if `fee_bps > FEE_CAP`.
    #[only_owner]
    fn add_referral(env: Env, owner: Address, fee_bps: u32) -> u64 {
        renew_instance(&env);
        if fee_bps > FEE_CAP {
            panic_with_error!(&env, Error::FeeTooHigh);
        }
        let counter = storage::referral_counter(&env);
        let id = counter
            .checked_add(1)
            .unwrap_or_else(|| panic_with_error!(&env, Error::IntegerOverflow));
        storage::set_referral_counter(&env, id);
        storage::set_referral(
            &env,
            id,
            &ReferralConfig {
                owner,
                fee_bps,
                active: true,
            },
        );
        id
    }

    /// Sets referral `id`'s fee in BPS (`<= FEE_CAP`). Owner only.
    #[only_owner]
    fn set_referral_fee(env: Env, id: u64, fee_bps: u32) {
        renew_instance(&env);
        if fee_bps > FEE_CAP {
            panic_with_error!(&env, Error::FeeTooHigh);
        }
        let mut cfg = storage::load_referral(&env, id);
        cfg.fee_bps = fee_bps;
        storage::set_referral(&env, id, &cfg);
    }

    /// Activates or deactivates referral `id`. Owner only.
    #[only_owner]
    fn set_referral_active(env: Env, id: u64, active: bool) {
        renew_instance(&env);
        let mut cfg = storage::load_referral(&env, id);
        cfg.active = active;
        storage::set_referral(&env, id, &cfg);
    }

    /// Sets the address that receives referral `id`'s fee claims. Owner only.
    #[only_owner]
    fn set_referral_owner(env: Env, id: u64, new_owner: Address) {
        renew_instance(&env);
        let mut cfg = storage::load_referral(&env, id);
        cfg.owner = new_owner;
        storage::set_referral(&env, id, &cfg);
    }

    /// Pays the admin fee balances for `tokens` to `recipient`. Owner only.
    #[only_owner]
    fn claim_admin_fees(env: Env, recipient: Address, tokens: Vec<Address>) {
        renew_instance(&env);
        let router = env.current_contract_address();
        fees::claim_fee_bucket(&env, &router, &recipient, tokens, FeeBucket::Admin);
    }

    /// Pays referral `id`'s fee balances for `tokens` to its stored owner. Callable by anyone.
    fn claim_referral_fees(env: Env, id: u64, tokens: Vec<Address>) {
        renew_instance(&env);
        let router = env.current_contract_address();
        fees::claim_referral_fees(&env, &router, id, tokens);
    }

    /// Transfers each token's balance above its reserved fee total to `recipient`. Owner only.
    #[only_owner]
    fn sweep_balance(env: Env, recipient: Address, tokens: Vec<Address>) {
        renew_instance(&env);
        let router = env.current_contract_address();
        let n = tokens.len();
        for i in 0..n {
            // `i < n == tokens.len()`, so the index is in range by construction.
            let token = tokens.get_unchecked(i);
            let client = token::Client::new(&env, &token);
            let balance = client.balance(&router);
            let reserved = storage::reserved_fee_balance(&env, &token);
            if balance > reserved {
                client.transfer(&router, &recipient, &(balance - reserved));
            }
        }
    }

    /// Returns the current Ownable owner; panics with `Error::NotAdmin` if unset.
    fn admin(env: Env) -> Address {
        ownable::get_owner(&env).unwrap_or_else(|| panic_with_error!(&env, Error::NotAdmin))
    }

    /// Returns the protocol static fee in basis points.
    fn static_fee_bps(env: Env) -> u32 {
        storage::static_fee_bps(&env)
    }

    /// Returns the referral config for `id`, or `None` if it does not exist.
    fn referral(env: Env, id: u64) -> Option<ReferralConfig> {
        storage::try_load_referral(&env, id)
    }

    /// Returns the highest referral id issued so far.
    fn referral_counter(env: Env) -> u64 {
        storage::referral_counter(&env)
    }

    /// Returns whether `token` is on the fee whitelist.
    fn is_whitelisted(env: Env, token: Address) -> bool {
        storage::load_whitelist(&env).contains(&token)
    }

    /// Returns the full fee-whitelist token list.
    fn whitelisted_tokens(env: Env) -> Vec<Address> {
        storage::load_whitelist(&env)
    }

    /// Returns the accrued admin fee balance for `token`.
    fn admin_fee_balance(env: Env, token: Address) -> i128 {
        storage::fee_balance(&env, &types::DataKey::AdminFee(token))
    }

    /// Returns the accrued referral fee balance for `(id, token)`.
    fn referral_fee_balance(env: Env, id: u64, token: Address) -> i128 {
        storage::fee_balance(&env, &types::DataKey::ReferralFee(id, token))
    }

    /// Decodes `swap_xdr` as a `StrategyPayload` and runs it for `sender`.
    ///
    /// Requires `sender` authorization. Pulls `total_in` of the input token, runs the
    /// instruction stream, applies fees, checks the minimum output, and returns the amount
    /// delivered to `sender`. Panics with `Error::InvalidRouteXdr` if the XDR does not decode.
    fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128 {
        renew_instance(&env);
        let payload = StrategyPayload::from_xdr(&env, &swap_xdr)
            .unwrap_or_else(|_| panic_with_error!(&env, Error::InvalidRouteXdr));
        execute::run(env, sender, total_in, payload)
    }
}

#[contractimpl]
impl Ownable for Router {
    /// Returns the current owner, or `None` if it was never set.
    fn get_owner(e: &Env) -> Option<Address> {
        ownable::get_owner(e)
    }

    /// Starts a two-step ownership transfer to `new_owner`, acceptable until ledger
    /// `live_until_ledger`. Requires current-owner authorization; overrides any
    /// pending transfer. A `live_until_ledger` of 0 cancels the pending transfer.
    fn transfer_ownership(e: &Env, new_owner: Address, live_until_ledger: u32) {
        ownable::transfer_ownership(e, &new_owner, live_until_ledger);
    }

    /// Completes a pending ownership transfer. Requires authorization from the
    /// pending owner.
    fn accept_ownership(e: &Env) {
        ownable::accept_ownership(e);
    }
}
