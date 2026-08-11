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
//! | `execute` | Strategy run: pull, paths, LP legs, settle |
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
pub(crate) use storage::{renew_instance, reserved_fee_balance};

use soroban_sdk::{
    contract, contractimpl, panic_with_error, token, xdr::FromXdr, Address, Bytes, BytesN, Env, Vec,
};

use stellar_access::ownable::{self, Ownable};
use stellar_macros::only_owner;
use swap_aggregator_interface::SwapAggregatorInterface;

use crate::constants::FEE_CAP;
use crate::errors::Error;
use crate::types::{ReferralConfig, StrategyPayload};

/// Deployed swap router instance.
#[contract]
pub struct Router;

#[contractimpl]
impl Router {
    /// Set `admin` as Ownable owner and zero fee config.
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);
        let storage = env.storage().instance();
        storage.set(&types::DataKey::StaticFeeBps, &0u32);
        storage.set(&types::DataKey::ReferralCounter, &0u64);
        storage::renew_instance(&env);
    }
}

#[contractimpl]
impl SwapAggregatorInterface for Router {
    /// Set the protocol static fee in bps (`<= FEE_CAP`). Owner only.
    #[only_owner]
    fn set_static_fee(env: Env, fee_bps: u32) {
        storage::renew_instance(&env);
        fees::set_static_fee(&env, fee_bps);
    }

    /// Mark `token` as fee-whitelisted (affects input-side fee selection). Owner only.
    #[only_owner]
    fn add_to_whitelist(env: Env, token: Address) {
        storage::renew_instance(&env);
        let mut list = storage::load_whitelist(&env);
        if !list.contains(&token) {
            list.push_back(token);
            storage::set_whitelist(&env, &list);
        }
    }

    /// Remove `token` from the fee whitelist. Owner only.
    #[only_owner]
    fn remove_from_whitelist(env: Env, token: Address) {
        storage::renew_instance(&env);
        let mut list = storage::load_whitelist(&env);
        if let Some(idx) = list.first_index_of(&token) {
            list.remove(idx);
            storage::set_whitelist(&env, &list);
        }
    }

    /// Upgrade contract WASM. Owner only.
    #[only_owner]
    fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        storage::renew_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    /// Create a referral; returns the new id. Owner only.
    #[only_owner]
    fn add_referral(env: Env, owner: Address, fee_bps: u32) -> u64 {
        storage::renew_instance(&env);
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

    /// Update a referral's fee bps. Owner only.
    #[only_owner]
    fn set_referral_fee(env: Env, id: u64, fee_bps: u32) {
        storage::renew_instance(&env);
        if fee_bps > FEE_CAP {
            panic_with_error!(&env, Error::FeeTooHigh);
        }
        let mut cfg = storage::load_referral(&env, id);
        cfg.fee_bps = fee_bps;
        storage::set_referral(&env, id, &cfg);
    }

    /// Activate or deactivate a referral. Owner only.
    #[only_owner]
    fn set_referral_active(env: Env, id: u64, active: bool) {
        storage::renew_instance(&env);
        let mut cfg = storage::load_referral(&env, id);
        cfg.active = active;
        storage::set_referral(&env, id, &cfg);
    }

    /// Transfer claim rights for a referral. Owner only.
    #[only_owner]
    fn set_referral_owner(env: Env, id: u64, new_owner: Address) {
        storage::renew_instance(&env);
        let mut cfg = storage::load_referral(&env, id);
        cfg.owner = new_owner;
        storage::set_referral(&env, id, &cfg);
    }

    /// Pay out accrued admin fee balances for `tokens`. Owner only.
    #[only_owner]
    fn claim_admin_fees(env: Env, recipient: Address, tokens: Vec<Address>) {
        storage::renew_instance(&env);
        let router = env.current_contract_address();
        fees::claim_admin_fees(&env, &router, &recipient, tokens);
    }

    /// Pay out accrued fees for referral `id` to its configured owner.
    fn claim_referral_fees(env: Env, id: u64, tokens: Vec<Address>) {
        storage::renew_instance(&env);
        let router = env.current_contract_address();
        fees::claim_referral_fees(&env, &router, id, tokens);
    }

    /// Recover non-fee token balances to `recipient`. Leaves fee buckets intact. Owner only.
    #[only_owner]
    fn sweep_balance(env: Env, recipient: Address, tokens: Vec<Address>) {
        storage::renew_instance(&env);
        let router = env.current_contract_address();
        let n = tokens.len();
        for i in 0..n {
            let token = tokens
                .get(i)
                .unwrap_or_else(|| panic_with_error!(&env, Error::InvalidAmount));
            let client = token::Client::new(&env, &token);
            let balance = client.balance(&router);
            let reserved = storage::reserved_fee_balance(&env, &token);
            if balance > reserved {
                client.transfer(&router, &recipient, &(balance - reserved));
            }
        }
    }

    /// Current Ownable owner.
    fn admin(env: Env) -> Address {
        ownable::get_owner(&env).unwrap_or_else(|| panic_with_error!(&env, Error::NotAdmin))
    }

    /// Protocol static fee in basis points.
    fn static_fee_bps(env: Env) -> u32 {
        storage::static_fee_bps(&env)
    }

    /// Referral config if `id` exists.
    fn referral(env: Env, id: u64) -> Option<ReferralConfig> {
        storage::try_load_referral(&env, id)
    }

    /// Highest referral id issued so far.
    fn referral_counter(env: Env) -> u64 {
        storage::referral_counter(&env)
    }

    /// Whether `token` is on the fee whitelist.
    fn is_whitelisted(env: Env, token: Address) -> bool {
        storage::load_whitelist(&env).contains(&token)
    }

    /// Full fee-whitelist token list.
    fn whitelisted_tokens(env: Env) -> Vec<Address> {
        storage::load_whitelist(&env)
    }

    /// Accrued admin fee balance for `token`.
    fn admin_fee_balance(env: Env, token: Address) -> i128 {
        storage::fee_balance(&env, &types::DataKey::AdminFee(token))
    }

    /// Accrued referral fee balance for `(id, token)`.
    fn referral_fee_balance(env: Env, id: u64, token: Address) -> i128 {
        storage::fee_balance(&env, &types::DataKey::ReferralFee(id, token))
    }

    /// Decode `swap_xdr` as `StrategyPayload` and execute it.
    ///
    /// Pulls `total_in` from `sender`, runs optional LP burn/paths/mint, applies
    /// fees, enforces `total_min_out`, and returns delivered output.
    fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128 {
        storage::renew_instance(&env);
        let payload = StrategyPayload::from_xdr(&env, &swap_xdr)
            .unwrap_or_else(|_| panic_with_error!(&env, Error::InvalidRouteXdr));
        execute::run(env, sender, total_in, payload)
    }
}

#[contractimpl]
impl Ownable for Router {
    /// Current owner, or `None` if ownership has been renounced or was never set.
    fn get_owner(e: &Env) -> Option<Address> {
        ownable::get_owner(e)
    }

    /// Starts a two-step ownership transfer to `new_owner`, acceptable until ledger
    /// `live_until_ledger`. Requires current-owner authorization; overrides any
    /// pending transfer.
    fn transfer_ownership(e: &Env, new_owner: Address, live_until_ledger: u32) {
        ownable::transfer_ownership(e, &new_owner, live_until_ledger);
    }

    /// Completes a pending ownership transfer. Requires authorization from the
    /// pending owner.
    fn accept_ownership(e: &Env) {
        ownable::accept_ownership(e);
    }

    /// Clears the current owner. Requires current-owner authorization and panics
    /// if a transfer is pending.
    fn renounce_ownership(e: &Env) {
        ownable::renounce_ownership(e);
    }
}
