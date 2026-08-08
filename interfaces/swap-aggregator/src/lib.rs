#![no_std]

use soroban_sdk::{contractclient, contracttype, Address, Bytes, BytesN, Env, Vec};

/// Stored referral account.
///
/// Defined here rather than in the contract so that the trait below can name it
/// without callers depending on the router crate.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ReferralConfig {
    pub owner: Address,
    pub fee_bps: u32,
    pub active: bool,
}

/// Swap router surface.
///
/// Ownership transfer is deliberately absent: the router implements
/// `stellar_access::ownable::Ownable`, which already binds those entrypoints at
/// compile time.
#[contractclient(name = "SwapAggregatorClient")]
pub trait SwapAggregatorInterface {
    fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128;

    fn set_static_fee(env: Env, fee_bps: u32);

    fn add_to_whitelist(env: Env, token: Address);

    fn remove_from_whitelist(env: Env, token: Address);

    fn upgrade(env: Env, new_wasm_hash: BytesN<32>);

    fn add_referral(env: Env, owner: Address, fee_bps: u32) -> u64;

    fn set_referral_fee(env: Env, id: u64, fee_bps: u32);

    fn set_referral_active(env: Env, id: u64, active: bool);

    fn set_referral_owner(env: Env, id: u64, new_owner: Address);

    fn claim_admin_fees(env: Env, recipient: Address, tokens: Vec<Address>);

    fn claim_referral_fees(env: Env, id: u64, tokens: Vec<Address>);

    fn sweep_balance(env: Env, recipient: Address, tokens: Vec<Address>);

    fn admin(env: Env) -> Address;

    fn static_fee_bps(env: Env) -> u32;

    fn referral(env: Env, id: u64) -> Option<ReferralConfig>;

    fn referral_counter(env: Env) -> u64;

    fn is_whitelisted(env: Env, token: Address) -> bool;

    fn whitelisted_tokens(env: Env) -> Vec<Address>;

    fn admin_fee_balance(env: Env, token: Address) -> i128;

    fn referral_fee_balance(env: Env, id: u64, token: Address) -> i128;
}
