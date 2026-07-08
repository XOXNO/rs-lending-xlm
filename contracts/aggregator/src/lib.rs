//! Stellar swap router.
//!
//! Pulls one input amount, executes split paths, and returns measured output.
//! Storage holds only admin, referral config, whitelist, and fee buckets.

#![no_std]
// Soroban macros emit their own unsafe allowances.
#![deny(unsafe_code)]

mod errors;
mod types;
mod vault;
mod venues;

#[cfg(test)]
mod test;

use crate::errors::Error;
use crate::types::{DataKey, ReferralConfig, StrategyPayload, SwapPath};
use crate::vault::Vault;
use soroban_sdk::{
    contract, contractimpl, panic_with_error, token, xdr::FromXdr, Address, Bytes, BytesN, Env, Vec,
};

const PPM_DENOMINATOR: i128 = 1_000_000;
const TOTAL_FEE: i128 = 10_000;
const FEE_CAP: u32 = 1_000;

#[contract]
pub struct Router;

#[contractimpl]
impl Router {
    pub fn __constructor(env: Env, admin: Address) {
        let storage = env.storage().instance();
        if storage.has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialised);
        }
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::StaticFeeBps, &0u32);
        storage.set(&DataKey::ReferralCounter, &0u64);
    }

    // -----------------------------------------------------------------
    // Admin endpoints — gated by `Admin` storage entry's auth.
    // -----------------------------------------------------------------

    pub fn set_admin(env: Env, new_admin: Address) {
        require_admin(&env);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
    }

    pub fn set_static_fee(env: Env, fee_bps: u32) {
        require_admin(&env);
        if fee_bps > FEE_CAP {
            panic_with_error!(&env, Error::FeeTooHigh);
        }
        env.storage()
            .instance()
            .set(&DataKey::StaticFeeBps, &fee_bps);
    }

    pub fn add_to_whitelist(env: Env, token: Address) {
        require_admin(&env);
        let mut list = load_whitelist(&env);
        if !list.contains(&token) {
            list.push_back(token);
            env.storage()
                .instance()
                .set(&DataKey::WhitelistedTokens, &list);
        }
    }

    pub fn remove_from_whitelist(env: Env, token: Address) {
        require_admin(&env);
        let mut list = load_whitelist(&env);
        if let Some(idx) = list.iter().position(|t| t == token) {
            list.remove(idx as u32);
            env.storage()
                .instance()
                .set(&DataKey::WhitelistedTokens, &list);
        }
    }

    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        require_admin(&env);
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }

    pub fn add_referral(env: Env, owner: Address, fee_bps: u32) -> u64 {
        require_admin(&env);
        if fee_bps > FEE_CAP {
            panic_with_error!(&env, Error::FeeTooHigh);
        }
        let storage = env.storage().instance();
        let counter: u64 = storage.get(&DataKey::ReferralCounter).unwrap_or(0);
        let id = counter
            .checked_add(1)
            .unwrap_or_else(|| panic_with_error!(&env, Error::IntegerOverflow));
        storage.set(&DataKey::ReferralCounter, &id);
        env.storage().persistent().set(
            &DataKey::Referral(id),
            &ReferralConfig {
                owner,
                fee_bps,
                active: true,
            },
        );
        id
    }

    pub fn set_referral_fee(env: Env, id: u64, fee_bps: u32) {
        require_admin(&env);
        if fee_bps > FEE_CAP {
            panic_with_error!(&env, Error::FeeTooHigh);
        }
        let mut cfg = load_referral(&env, id);
        cfg.fee_bps = fee_bps;
        env.storage().persistent().set(&DataKey::Referral(id), &cfg);
    }

    pub fn set_referral_active(env: Env, id: u64, active: bool) {
        require_admin(&env);
        let mut cfg = load_referral(&env, id);
        cfg.active = active;
        env.storage().persistent().set(&DataKey::Referral(id), &cfg);
    }

    pub fn set_referral_owner(env: Env, id: u64, new_owner: Address) {
        require_admin(&env);
        let mut cfg = load_referral(&env, id);
        cfg.owner = new_owner;
        env.storage().persistent().set(&DataKey::Referral(id), &cfg);
    }

    pub fn claim_admin_fees(env: Env, recipient: Address, tokens: Vec<Address>) {
        require_admin(&env);
        let router = env.current_contract_address();
        claim_fee_bucket(&env, &router, &recipient, tokens, FeeBucket::Admin);
    }

    pub fn claim_referral_fees(env: Env, id: u64, tokens: Vec<Address>) {
        let cfg = load_referral(&env, id);
        let router = env.current_contract_address();
        claim_fee_bucket(&env, &router, &cfg.owner, tokens, FeeBucket::Referral(id));
    }

    pub fn sweep_balance(env: Env, recipient: Address, tokens: Vec<Address>) {
        require_admin(&env);
        let router = env.current_contract_address();
        let n = tokens.len();
        for i in 0..n {
            let token = tokens
                .get(i)
                .unwrap_or_else(|| panic_with_error!(&env, Error::InvalidAmount));
            let client = token::Client::new(&env, &token);
            let balance = client.balance(&router);
            let reserved = reserved_fee_balance(&env, &token);
            if balance > reserved {
                client.transfer(&router, &recipient, &(balance - reserved));
            }
        }
    }

    // -----------------------------------------------------------------
    // Views — read-only, free for off-chain callers via `simulateTransaction`.
    // -----------------------------------------------------------------

    pub fn admin(env: Env) -> Address {
        load_admin(&env)
    }

    pub fn static_fee_bps(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::StaticFeeBps)
            .unwrap_or(0)
    }

    pub fn referral(env: Env, id: u64) -> Option<ReferralConfig> {
        env.storage().persistent().get(&DataKey::Referral(id))
    }

    pub fn referral_counter(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::ReferralCounter)
            .unwrap_or(0)
    }

    pub fn is_whitelisted(env: Env, token: Address) -> bool {
        load_whitelist(&env).contains(&token)
    }

    pub fn whitelisted_tokens(env: Env) -> Vec<Address> {
        load_whitelist(&env)
    }

    pub fn admin_fee_balance(env: Env, token: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::AdminFee(token))
            .unwrap_or(0)
    }

    pub fn referral_fee_balance(env: Env, id: u64, token: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::ReferralFee(id, token))
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------
    // Main entry — execute an opaque strategy payload.
    // -----------------------------------------------------------------

    pub fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128 {
        let payload = StrategyPayload::from_xdr(&env, &swap_xdr)
            .unwrap_or_else(|_| panic_with_error!(&env, Error::InvalidRouteXdr));
        execute_payload(env, sender, total_in, payload)
    }
}

// ---------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------

fn load_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .unwrap_or_else(|| panic_with_error!(env, Error::NotAdmin))
}

fn require_admin(env: &Env) {
    load_admin(env).require_auth();
}

fn load_referral(env: &Env, id: u64) -> ReferralConfig {
    env.storage()
        .persistent()
        .get(&DataKey::Referral(id))
        .unwrap_or_else(|| panic_with_error!(env, Error::ReferralNotFound))
}

fn load_whitelist(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::WhitelistedTokens)
        .unwrap_or_else(|| Vec::new(env))
}

fn apply_fees_on_token(env: &Env, vault: &mut Vault, token: &Address, referral_id: u64) {
    if referral_id == 0 {
        return;
    }
    // Look up referral; silently no-op if missing or inactive so a stale
    // referral id doesn't brick the user's swap.
    let cfg: ReferralConfig = match env
        .storage()
        .persistent()
        .get(&DataKey::Referral(referral_id))
    {
        Some(c) => c,
        None => return,
    };
    if !cfg.active {
        return;
    }

    let balance = vault.balance_of(token);
    if balance <= 0 {
        return;
    }
    let static_fee_bps: u32 = env
        .storage()
        .instance()
        .get(&DataKey::StaticFeeBps)
        .unwrap_or(0);

    // Compute the combined bps once and bail before computing the
    // per-side fee amounts when both the admin slice and the referral
    // slice are zero — the `total <= 0` check below would also catch
    // this case before any vault/storage work, but this skips the two
    // `fee_amount` calls up front. Typical of "tracking" referrals
    // (active but 0 bps both sides) used purely for attribution.
    let combined_bps = static_fee_bps
        .checked_add(cfg.fee_bps)
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
    if combined_bps == 0 {
        return;
    }

    let static_fee = fee_amount(env, balance, static_fee_bps);
    let referral_fee = fee_amount(env, balance, cfg.fee_bps);
    let total = checked_add(env, static_fee, referral_fee);
    if total <= 0 {
        return;
    }

    vault.withdraw(token, total);

    if static_fee > 0 {
        accumulate_fee(env, DataKey::AdminFee(token.clone()), static_fee);
    }
    if referral_fee > 0 {
        accumulate_fee(
            env,
            DataKey::ReferralFee(referral_id, token.clone()),
            referral_fee,
        );
    }
}

#[derive(Clone, Copy)]
enum FeeBucket {
    Admin,
    Referral(u64),
}

impl FeeBucket {
    fn key(self, token: Address) -> DataKey {
        match self {
            FeeBucket::Admin => DataKey::AdminFee(token),
            FeeBucket::Referral(id) => DataKey::ReferralFee(id, token),
        }
    }
}

fn claim_fee_bucket(
    env: &Env,
    router: &Address,
    recipient: &Address,
    tokens: Vec<Address>,
    bucket: FeeBucket,
) {
    let n = tokens.len();
    for i in 0..n {
        let token = tokens
            .get(i)
            .unwrap_or_else(|| panic_with_error!(env, Error::InvalidAmount));
        let key = bucket.key(token.clone());
        let amount: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        if amount > 0 {
            env.storage().persistent().remove(&key);
            token::Client::new(env, &token).transfer(router, recipient, &amount);
        }
    }
}

fn reserved_fee_balance(env: &Env, token: &Address) -> i128 {
    let mut total: i128 = env
        .storage()
        .persistent()
        .get(&DataKey::AdminFee(token.clone()))
        .unwrap_or(0);
    let counter: u64 = env
        .storage()
        .instance()
        .get(&DataKey::ReferralCounter)
        .unwrap_or(0);
    for id in 1..=counter {
        let amount: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::ReferralFee(id, token.clone()))
            .unwrap_or(0);
        total = checked_add(env, total, amount);
    }
    total
}

fn accumulate_fee(env: &Env, key: DataKey, amount: i128) {
    let cur: i128 = env.storage().persistent().get(&key).unwrap_or(0);
    let next = checked_add(env, cur, amount);
    env.storage().persistent().set(&key, &next);
}

fn fee_amount(env: &Env, balance: i128, fee_bps: u32) -> i128 {
    checked_mul(env, balance, fee_bps as i128) / TOTAL_FEE
}

fn checked_add(env: &Env, lhs: i128, rhs: i128) -> i128 {
    lhs.checked_add(rhs)
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow))
}

fn checked_mul(env: &Env, lhs: i128, rhs: i128) -> i128 {
    lhs.checked_mul(rhs)
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow))
}

fn execute_payload(env: Env, sender: Address, total_in: i128, payload: StrategyPayload) -> i128 {
    sender.require_auth();

    if payload.paths.is_empty() {
        panic_with_error!(&env, Error::EmptyBatch);
    }
    if total_in <= 0 {
        panic_with_error!(&env, Error::InvalidAmount);
    }
    if payload.total_min_out <= 0 {
        panic_with_error!(&env, Error::SlippageExceeded);
    }

    let (input_token, output_token) = validate_batch_shape(&env, &payload.paths);
    if input_token != payload.token_in || output_token != payload.token_out {
        panic_with_error!(&env, Error::BrokenTokenChain);
    }

    let router = env.current_contract_address();
    let mut vault = Vault::new(&env);

    // Pull total_in once into the router's vault.
    token::Client::new(&env, &input_token).transfer(&sender, &router, &total_in);
    vault.deposit(&input_token, total_in);

    // Fee direction is only meaningful when there's actually a fee
    // to charge. `referral_id == 0` means "no fee" (matches MVX
    // semantics). Skipping this block on the no-fee path saves
    // 2 instance-storage reads and avoids touching the whitelist
    // entirely — the lending controller's only path.
    let fee_on_input = if payload.referral_id != 0 {
        let list = load_whitelist(&env);
        let in_wl = list.contains(&input_token);
        let out_wl = list.contains(&output_token);
        // Fee is charged on input unless output is the only whitelisted side.
        !out_wl || in_wl
    } else {
        false
    };

    // Apply input-side fee BEFORE walking paths so per-path slicing
    // is on the post-fee vault balance. The function early-returns
    // when there's nothing to charge (referral_id == 0, missing /
    // inactive referral, or zero combined bps).
    if fee_on_input {
        apply_fees_on_token(&env, &mut vault, &input_token, payload.referral_id);
    }

    let total_after_fee = vault.balance_of(&input_token);
    if total_after_fee <= 0 {
        panic_with_error!(&env, Error::InvalidAmount);
    }

    // Walk paths.
    let n = payload.paths.len();
    let mut consumed: i128 = 0;
    for i in 0..n {
        let path = payload
            .paths
            .get(i)
            .unwrap_or_else(|| panic_with_error!(&env, Error::EmptyPath));
        let path_input = if i + 1 == n {
            total_after_fee - consumed
        } else {
            let allocated =
                checked_mul(&env, total_after_fee, path.split_ppm as i128) / PPM_DENOMINATOR;
            consumed = checked_add(&env, consumed, allocated);
            allocated
        };
        if path_input <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        execute_path(&env, &router, &mut vault, &path, path_input);
    }

    // Apply output-side fee AFTER paths complete.
    if !fee_on_input {
        apply_fees_on_token(&env, &mut vault, &output_token, payload.referral_id);
    }

    let total_out = vault.balance_of(&output_token);
    if total_out < payload.total_min_out {
        panic_with_error!(&env, Error::SlippageExceeded);
    }

    vault.withdraw(&output_token, total_out);
    token::Client::new(&env, &output_token).transfer(&router, &sender, &total_out);

    total_out
}

fn execute_path(env: &Env, router: &Address, vault: &mut Vault, path: &SwapPath, path_input: i128) {
    if path.hops.is_empty() {
        panic_with_error!(env, Error::EmptyPath);
    }

    let n = path.hops.len();
    let mut current = path_input;
    for idx in 0..n {
        let hop = path
            .hops
            .get(idx)
            .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath));
        if idx + 1 < n {
            let next_hop = path
                .hops
                .get(idx + 1)
                .unwrap_or_else(|| panic_with_error!(env, Error::BrokenTokenChain));
            if hop.token_out != next_hop.token_in {
                panic_with_error!(env, Error::BrokenTokenChain);
            }
        }
        vault.withdraw(&hop.token_in, current);
        let out = venues::dispatch_hop(env, router, &hop, current);
        if out <= 0 {
            panic_with_error!(env, Error::ZeroOutput);
        }
        vault.deposit(&hop.token_out, out);
        current = out;
    }
}

fn validate_batch_shape(env: &Env, paths: &Vec<SwapPath>) -> (Address, Address) {
    let first_path = paths
        .get(0)
        .unwrap_or_else(|| panic_with_error!(env, Error::EmptyBatch));
    if first_path.hops.is_empty() {
        panic_with_error!(env, Error::EmptyPath);
    }
    let input_token = first_path
        .hops
        .get(0)
        .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath))
        .token_in
        .clone();
    let output_token = last_token_out(env, &first_path);
    if input_token == output_token {
        panic_with_error!(env, Error::SameToken);
    }

    let mut sum_ppm: u32 = 0;
    let n = paths.len();
    for i in 0..n {
        let path = paths
            .get(i)
            .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath));
        if path.hops.is_empty() {
            panic_with_error!(env, Error::EmptyPath);
        }
        if path.split_ppm == 0 {
            panic_with_error!(env, Error::ZeroSplitPpm);
        }
        sum_ppm = sum_ppm
            .checked_add(path.split_ppm)
            .unwrap_or_else(|| panic_with_error!(env, Error::SplitPpmMismatch));

        let path_in = path
            .hops
            .get(0)
            .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath))
            .token_in
            .clone();
        let path_out = last_token_out(env, &path);
        if path_in != input_token || path_out != output_token {
            panic_with_error!(env, Error::BrokenTokenChain);
        }
    }
    if sum_ppm != PPM_DENOMINATOR as u32 {
        panic_with_error!(env, Error::SplitPpmMismatch);
    }
    (input_token, output_token)
}

fn last_token_out(env: &Env, path: &SwapPath) -> Address {
    let n = path.hops.len();
    if n == 0 {
        panic_with_error!(env, Error::EmptyPath);
    }
    path.hops
        .get(n - 1)
        .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath))
        .token_out
}
