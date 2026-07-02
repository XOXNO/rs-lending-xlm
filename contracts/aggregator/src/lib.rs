//! Stellar Router aggregator contract.
//!
//! ## Design
//! - **In-memory vault**: per-tx token balances live in `Vault` (a
//!   Soroban `Map<Address, i128>` held in a stack struct), not in
//!   storage. Zero per-hop storage I/O for swap state.
//! - **Opaque swap bytes**: callers pass `sender`, `total_in`, and a
//!   `StrategyPayload` encoded as ScVal XDR bytes. The payload carries route
//!   paths, endpoint tokens, slippage floor, and referral id.
//! - **PPM splits**: each path carries `split_ppm` (parts-per-million)
//!   instead of an absolute `amount_in`. The router pulls `total_in` ONCE
//!   from sender, then slices per path from the vault. Last path absorbs PPM
//!   rounding.
//! - **Hop forward**: within a path, output of hop N feeds hop N+1
//!   directly. Hop `amount_out` is supplied by the off-chain quote and is
//!   needed only for venues whose ABI requires a requested output.
//! - **Single slippage gate**: only `total_min_out` after all paths.
//!   Per-path mins were intentionally dropped; stale venue state is handled by
//!   the venue reverting or by the final aggregate output check.
//!
//! ## Persistent state (the only storage in the contract)
//! - `Admin` — contract admin (instance entry).
//! - `StaticFeeBps` — admin-side fee in basis points (instance).
//! - `ReferralCounter` — monotonic counter (instance).
//! - `Referral(u64)` — `ReferralConfig { owner, fee_bps, active }`.
//! - `WhitelistedTokens` — single instance entry holding `Vec<Address>`
//!   of whitelisted tokens (fee-direction hints).
//! - `AdminFee(Address)` — accumulated admin fees (claimable by admin).
//! - `ReferralFee(u64, Address)` — accumulated referral fees per
//!   (id, token), transferred to `Referral(id).owner` on claim.
//!
//! No swap state ever goes into storage.
//!
//! ## Fees
//! `referral_id == 0` → no fee. `referral_id > 0` and active → charge
//! `static_fee_bps + referral_fee_bps` from either the input token
//! (default) or the output token (when output is whitelisted but input
//! isn't). Static portion → admin accumulator. Referral portion →
//! per-referral accumulator.

#![no_std]
// Soroban's `#[contractimpl]` macro emits an internal `allow(unsafe_code)`,
// so `forbid(unsafe_code)` is incompatible with the contract toolchain. Keep
// hand-written unsafe denied while allowing the macro expansion to compile.
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
/// Hard cap on combined static fee. 1_000 bps = 10%.
const FEE_CAP: u32 = 1_000;

#[contract]
pub struct Router;

#[contractimpl]
impl Router {
    /// One-shot constructor. Called automatically when the contract is
    /// deployed via `stellar contract deploy --constructor-args ...`.
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

    /// Set the admin-side fee in basis points. Capped at `FEE_CAP`.
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

    /// Replace the WASM bytecode in place. Admin-only. Lets us ship
    /// micro-optimizations or new venues without forcing every
    /// integrator to re-pin the contract address (the controller, the
    /// quote server, the SDK and UI all keep pointing at the same id).
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        require_admin(&env);
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }

    /// Register a new referral. Admin-only (matches `rs-aggregator`'s
    /// `#[only_owner]`). Returns the assigned ID.
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

    /// Rotate the referral owner. Admin-only — matches `rs-aggregator`'s
    /// `#[only_owner]`. Owners that need rotation request it from the
    /// admin.
    pub fn set_referral_owner(env: Env, id: u64, new_owner: Address) {
        require_admin(&env);
        let mut cfg = load_referral(&env, id);
        cfg.owner = new_owner;
        env.storage().persistent().set(&DataKey::Referral(id), &cfg);
    }

    /// Claim accumulated admin fees for the listed tokens to
    /// `recipient`. Admin-only.
    pub fn claim_admin_fees(env: Env, recipient: Address, tokens: Vec<Address>) {
        require_admin(&env);
        let router = env.current_contract_address();
        claim_fee_bucket(&env, &router, &recipient, tokens, FeeBucket::Admin);
    }

    /// Claim accumulated referral fees for the listed tokens. Anyone
    /// can call; tokens are sent to the referral's `owner` of the
    /// moment of the call.
    pub fn claim_referral_fees(env: Env, id: u64, tokens: Vec<Address>) {
        let cfg = load_referral(&env, id);
        let router = env.current_contract_address();
        claim_fee_bucket(&env, &router, &cfg.owner, tokens, FeeBucket::Referral(id));
    }

    /// Recover the router's live balance of the listed tokens to
    /// `recipient`. Admin-only. The router should hold zero balance of
    /// any token between transactions — `execute_strategy` always
    /// drains its in-memory vault to zero before returning — so any
    /// non-zero balance found here is stray (direct transfers, dust
    /// from a route that never went through the vault). This ignores
    /// fee-bucket accounting: sweeping a token that also has an
    /// unclaimed `AdminFee`/`ReferralFee` entry moves the real balance
    /// out from under that entry, so claim fee buckets first if they
    /// matter.
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
            if balance > 0 {
                client.transfer(&router, &recipient, &balance);
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

    /// Return the full whitelist as a `Vec<Address>`. Lets the off-chain
    /// quote server fetch the list in one read instead of probing every
    /// known token via `is_whitelisted` individually.
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

    /// Execute a swap payload encoded as ScVal XDR bytes. This keeps callers
    /// from depending on the router's route/hop/venue types at their ABI
    /// boundary. Lending controllers pass only their sender address, concrete
    /// input amount, and these opaque bytes.
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

/// Load the whitelist (instance storage). Returns an empty `Vec` when
/// nothing has been whitelisted yet — fresh deploys hit this path so
/// the bare-metal cost of "no whitelist set" is one instance read of
/// `None`, not a per-token persistent probe.
fn load_whitelist(env: &Env) -> Vec<Address> {
    env.storage()
        .instance()
        .get(&DataKey::WhitelistedTokens)
        .unwrap_or_else(|| Vec::new(env))
}

/// Split a token's vault balance into static-fee + referral-fee
/// portions and accumulate each into its respective storage key.
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

    // Compute the combined bps once and bail before doing any vault /
    // storage work when both the admin slice and the referral slice are
    // zero. Saves the vault.withdraw + the two persistent reads / writes
    // when fees are technically enabled (active referral) but nominally
    // worthless (0 bps both sides) — typical of "tracking" referrals
    // used purely for attribution.
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
        // Same rule as before: fee on input unless output is the
        // only whitelisted side.
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
