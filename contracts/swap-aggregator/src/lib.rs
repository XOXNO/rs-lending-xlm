#![no_std]
#![deny(unsafe_code)]

mod errors;
mod types;
mod vault;
mod venues;

#[cfg(test)]
#[path = "../tests/unit/mod.rs"]
mod test;

use common::constants::{TTL_BUMP_SHARED, TTL_THRESHOLD_SHARED};

use soroban_sdk::{
    contract, contractimpl, panic_with_error, token, xdr::FromXdr, Address, Bytes, BytesN, Env,
    Map, Vec,
};

use stellar_access::ownable::{self, Ownable};
use stellar_macros::only_owner;

use crate::errors::Error;
use crate::types::{DataKey, ReferralConfig, StrategyPayload, SwapPath};
use crate::vault::Vault;

const PPM_DENOMINATOR: i128 = 1_000_000;

const RESIDUAL_DUST_FLOOR: i128 = 1_000;

const RESIDUAL_PPM: i128 = 1_000_000;

pub(crate) fn residual_allowance(credited: i128) -> i128 {
    let proportional = credited / RESIDUAL_PPM;
    if proportional > RESIDUAL_DUST_FLOOR {
        proportional
    } else {
        RESIDUAL_DUST_FLOOR
    }
}
const TOTAL_FEE: i128 = 10_000;
const FEE_CAP: u32 = 1_000;

#[contract]
pub struct Router;

#[contractimpl]
impl Router {
    pub fn __constructor(env: Env, admin: Address) {
        ownable::set_owner(&env, &admin);
        let storage = env.storage().instance();
        storage.set(&DataKey::StaticFeeBps, &0u32);
        storage.set(&DataKey::ReferralCounter, &0u64);
    }

    #[only_owner]
    pub fn set_static_fee(env: Env, fee_bps: u32) {
        if fee_bps > FEE_CAP {
            panic_with_error!(&env, Error::FeeTooHigh);
        }
        env.storage()
            .instance()
            .set(&DataKey::StaticFeeBps, &fee_bps);
    }

    #[only_owner]
    pub fn add_to_whitelist(env: Env, token: Address) {
        let mut list = load_whitelist(&env);
        if !list.contains(&token) {
            list.push_back(token);
            env.storage()
                .instance()
                .set(&DataKey::WhitelistedTokens, &list);
        }
    }

    #[only_owner]
    pub fn remove_from_whitelist(env: Env, token: Address) {
        let mut list = load_whitelist(&env);
        if let Some(idx) = list.first_index_of(&token) {
            list.remove(idx);
            env.storage()
                .instance()
                .set(&DataKey::WhitelistedTokens, &list);
        }
    }

    #[only_owner]
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }

    #[only_owner]
    pub fn add_referral(env: Env, owner: Address, fee_bps: u32) -> u64 {
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

    #[only_owner]
    pub fn set_referral_fee(env: Env, id: u64, fee_bps: u32) {
        if fee_bps > FEE_CAP {
            panic_with_error!(&env, Error::FeeTooHigh);
        }
        let mut cfg = load_referral(&env, id);
        cfg.fee_bps = fee_bps;
        env.storage().persistent().set(&DataKey::Referral(id), &cfg);
    }

    #[only_owner]
    pub fn set_referral_active(env: Env, id: u64, active: bool) {
        let mut cfg = load_referral(&env, id);
        cfg.active = active;
        env.storage().persistent().set(&DataKey::Referral(id), &cfg);
    }

    #[only_owner]
    pub fn set_referral_owner(env: Env, id: u64, new_owner: Address) {
        let mut cfg = load_referral(&env, id);
        cfg.owner = new_owner;
        env.storage().persistent().set(&DataKey::Referral(id), &cfg);
    }

    #[only_owner]
    pub fn claim_admin_fees(env: Env, recipient: Address, tokens: Vec<Address>) {
        let router = env.current_contract_address();
        claim_fee_bucket(&env, &router, &recipient, tokens, FeeBucket::Admin);
    }

    pub fn claim_referral_fees(env: Env, id: u64, tokens: Vec<Address>) {
        let cfg = load_referral(&env, id);
        let router = env.current_contract_address();
        claim_fee_bucket(&env, &router, &cfg.owner, tokens, FeeBucket::Referral(id));
    }

    #[only_owner]
    pub fn sweep_balance(env: Env, recipient: Address, tokens: Vec<Address>) {
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

    pub fn admin(env: Env) -> Address {
        ownable::get_owner(&env).unwrap_or_else(|| panic_with_error!(&env, Error::NotAdmin))
    }

    pub fn static_fee_bps(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::StaticFeeBps)
            .unwrap_or(0)
    }

    pub fn referral(env: Env, id: u64) -> Option<ReferralConfig> {
        let key = DataKey::Referral(id);
        let v: Option<ReferralConfig> = env.storage().persistent().get(&key);
        if v.is_some() {
            env.storage()
                .persistent()
                .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
        }
        v
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
        let key = DataKey::AdminFee(token);
        let v: Option<i128> = env.storage().persistent().get(&key);
        if v.is_some() {
            env.storage()
                .persistent()
                .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
        }
        v.unwrap_or(0)
    }

    pub fn referral_fee_balance(env: Env, id: u64, token: Address) -> i128 {
        let key = DataKey::ReferralFee(id, token);
        let v: Option<i128> = env.storage().persistent().get(&key);
        if v.is_some() {
            env.storage()
                .persistent()
                .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
        }
        v.unwrap_or(0)
    }

    pub fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128 {
        let payload = StrategyPayload::from_xdr(&env, &swap_xdr)
            .unwrap_or_else(|_| panic_with_error!(&env, Error::InvalidRouteXdr));
        execute_payload(env, sender, total_in, payload)
    }
}

#[contractimpl]
impl Ownable for Router {
    fn get_owner(e: &Env) -> Option<Address> {
        ownable::get_owner(e)
    }

    fn transfer_ownership(e: &Env, new_owner: Address, live_until_ledger: u32) {
        ownable::transfer_ownership(e, &new_owner, live_until_ledger);
    }

    fn accept_ownership(e: &Env) {
        ownable::accept_ownership(e);
    }

    fn renounce_ownership(e: &Env) {
        ownable::renounce_ownership(e);
    }
}

fn load_referral(env: &Env, id: u64) -> ReferralConfig {
    let key = DataKey::Referral(id);
    let v: ReferralConfig = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, Error::ReferralNotFound));
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    v
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

    let combined_bps = static_fee_bps
        .checked_add(cfg.fee_bps)
        .unwrap_or_else(|| panic_with_error!(env, Error::IntegerOverflow));
    if combined_bps == 0 {
        return;
    }

    if combined_bps > FEE_CAP {
        panic_with_error!(env, Error::FeeTooHigh);
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
    let admin_key = DataKey::AdminFee(token.clone());
    let mut total: i128 = env.storage().persistent().get(&admin_key).unwrap_or(0);
    if total > 0 {
        env.storage()
            .persistent()
            .extend_ttl(&admin_key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
    }

    let counter: u64 = env
        .storage()
        .instance()
        .get(&DataKey::ReferralCounter)
        .unwrap_or(0);
    for id in 1..=counter {
        let key = DataKey::ReferralFee(id, token.clone());
        let amount: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        if amount > 0 {
            env.storage()
                .persistent()
                .extend_ttl(&key, TTL_THRESHOLD_SHARED, TTL_BUMP_SHARED);
        }
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

    if total_in <= 0 {
        panic_with_error!(&env, Error::InvalidAmount);
    }
    if payload.total_min_out <= 0 {
        panic_with_error!(&env, Error::SlippageExceeded);
    }

    let input_token = payload.token_in.clone();
    let output_token = payload.token_out.clone();
    validate_payload(&env, &payload);

    let router = env.current_contract_address();
    let mut vault = Vault::new(&env);

    let mut tokens_cache: Map<Address, Vec<Address>> = Map::new(&env);

    token::Client::new(&env, &input_token).transfer(&sender, &router, &total_in);
    vault.deposit(&input_token, total_in);

    let fee_on_input = if payload.referral_id != 0 {
        let list = load_whitelist(&env);
        let in_wl = list.contains(&input_token);
        let out_wl = list.contains(&output_token);

        !out_wl || in_wl
    } else {
        false
    };

    if fee_on_input {
        apply_fees_on_token(&env, &mut vault, &input_token, payload.referral_id);
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

    execute_paths(&env, &router, &mut vault, &payload.paths, &mut tokens_cache);

    if let Some(pool) = payload.mint_pool.as_ref() {
        venues::aquarius::add_liquidity(
            &env,
            &router,
            &mut vault,
            venues::aquarius::MintLiquidity {
                pool,
                lp_token: &output_token,
                min_shares: payload.mint_min_shares,
                pre_balance_fee_bps: payload.pre_balance_fee_bps,
            },
            &mut tokens_cache,
        );
    }

    if !fee_on_input {
        apply_fees_on_token(&env, &mut vault, &output_token, payload.referral_id);
    }

    let total_out = vault.balance_of(&output_token);
    if total_out < payload.total_min_out {
        panic_with_error!(&env, Error::SlippageExceeded);
    }

    vault.withdraw(&output_token, total_out);
    token::Client::new(&env, &output_token).transfer(&router, &sender, &total_out);

    accrue_residual_as_revenue(&env, &mut vault);

    total_out
}

fn execute_paths(
    env: &Env,
    router: &Address,
    vault: &mut Vault,
    paths: &Vec<SwapPath>,
    tokens_cache: &mut Map<Address, Vec<Address>>,
) {
    let n = paths.len();
    for i in 0..n {
        let token = path_token_in(env, paths, i);
        if i != first_index_for_token(env, paths, &token) {
            continue;
        }

        let available = vault.balance_of(&token);
        if available <= 0 {
            panic_with_error!(env, Error::InvalidAmount);
        }

        let mut consumed: i128 = 0;
        let mut last = i;
        for j in i..n {
            if path_token_in(env, paths, j) == token {
                last = j;
            }
        }

        let routes_everything = group_split_ppm(env, paths, &token) == PPM_DENOMINATOR as u32;
        for j in i..n {
            if path_token_in(env, paths, j) != token {
                continue;
            }
            let path = paths
                .get(j)
                .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath));
            let path_input = if j == last && routes_everything {
                available - consumed
            } else {
                let allocated =
                    checked_mul(env, available, path.split_ppm as i128) / PPM_DENOMINATOR;
                consumed = checked_add(env, consumed, allocated);
                allocated
            };
            if path_input <= 0 {
                panic_with_error!(env, Error::InvalidAmount);
            }
            execute_path(env, router, vault, &path, path_input, tokens_cache);
        }
    }
}

fn accrue_residual_as_revenue(env: &Env, vault: &mut Vault) {
    let tokens = vault.tokens();
    let n = tokens.len();
    for i in 0..n {
        let token = tokens.get_unchecked(i);
        let amount = vault.balance_of(&token);
        if amount <= 0 {
            continue;
        }

        if amount > residual_allowance(vault.credited_of(&token)) {
            panic_with_error!(env, Error::ExcessiveResidual);
        }
        vault.withdraw(&token, amount);
        accumulate_fee(env, DataKey::AdminFee(token), amount);
    }
}

fn execute_path(
    env: &Env,
    router: &Address,
    vault: &mut Vault,
    path: &SwapPath,
    path_input: i128,
    tokens_cache: &mut Map<Address, Vec<Address>>,
) {
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
        let out = venues::dispatch_hop(env, router, &hop, current, tokens_cache);
        if out <= 0 {
            panic_with_error!(env, Error::ZeroOutput);
        }
        vault.deposit(&hop.token_out, out);
        current = out;
    }
}

fn validate_payload(env: &Env, payload: &StrategyPayload) {
    let paths = &payload.paths;
    let n = paths.len();
    // A pure burn or pure mint carries no paths; anything else with no path is empty.
    if n == 0 && payload.burn_pool.is_none() && payload.mint_pool.is_none() {
        panic_with_error!(env, Error::EmptyBatch);
    }

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

        let path_in = path_token_in(env, paths, i);
        if payload.burn_pool.is_none() && path_in != payload.token_in {
            panic_with_error!(env, Error::BrokenTokenChain);
        }
        if payload.mint_pool.is_none() && last_token_out(env, &path) != payload.token_out {
            panic_with_error!(env, Error::BrokenTokenChain);
        }

        if i != first_index_for_token(env, paths, &path_in) {
            continue;
        }
        let sum_ppm = group_split_ppm(env, paths, &path_in);
        if sum_ppm > PPM_DENOMINATOR as u32 {
            panic_with_error!(env, Error::SplitPpmMismatch);
        }

        if payload.mint_pool.is_none() && sum_ppm != PPM_DENOMINATOR as u32 {
            panic_with_error!(env, Error::SplitPpmMismatch);
        }
    }

    if payload.token_in == payload.token_out {
        panic_with_error!(env, Error::SameToken);
    }
}

fn path_token_in(env: &Env, paths: &Vec<SwapPath>, index: u32) -> Address {
    paths
        .get(index)
        .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath))
        .hops
        .get(0)
        .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath))
        .token_in
}

fn group_split_ppm(env: &Env, paths: &Vec<SwapPath>, token: &Address) -> u32 {
    let n = paths.len();
    let mut sum_ppm: u32 = 0;
    for i in 0..n {
        if path_token_in(env, paths, i) != *token {
            continue;
        }
        let path = paths
            .get(i)
            .unwrap_or_else(|| panic_with_error!(env, Error::EmptyPath));
        sum_ppm = sum_ppm
            .checked_add(path.split_ppm)
            .unwrap_or_else(|| panic_with_error!(env, Error::SplitPpmMismatch));
    }
    sum_ppm
}

fn first_index_for_token(env: &Env, paths: &Vec<SwapPath>, token: &Address) -> u32 {
    let n = paths.len();
    for i in 0..n {
        if path_token_in(env, paths, i) == *token {
            return i;
        }
    }
    panic_with_error!(env, Error::EmptyPath)
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
