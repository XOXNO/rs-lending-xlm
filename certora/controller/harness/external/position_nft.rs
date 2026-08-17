//! Certora summary of the position-nft contract: ownership is a ghost map in
//! controller storage. Rules seed it via `spec::fixture::seed_account`.

use soroban_sdk::{contracttype, Address, Env};

#[contracttype]
pub enum GhostNftKey {
    Owner(u64),
    NextId,
}

pub(crate) fn nft_mint_call(env: &Env, _nft: &Address, to: &Address) -> u64 {
    let next: u64 = env
        .storage()
        .persistent()
        .get(&GhostNftKey::NextId)
        .unwrap_or(0u64)
        + 1;
    env.storage().persistent().set(&GhostNftKey::NextId, &next);
    env.storage()
        .persistent()
        .set(&GhostNftKey::Owner(next), to);
    next
}

pub(crate) fn nft_burn_call(env: &Env, _nft: &Address, account_id: u64) {
    env.storage()
        .persistent()
        .remove(&GhostNftKey::Owner(account_id));
}

pub(crate) fn nft_try_owner_of_call(env: &Env, _nft: &Address, account_id: u64) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&GhostNftKey::Owner(account_id))
}

/// TTL extension has no accounting effect. Mirrors the real contract's one
/// observable behavior: renew on a burned/never-minted token reverts.
pub(crate) fn nft_renew_call(env: &Env, _nft: &Address, account_id: u64) {
    if !env
        .storage()
        .persistent()
        .has(&GhostNftKey::Owner(account_id))
    {
        panic!("nonexistent token");
    }
}

/// Upgrades are outside the verified state space; the summary is a no-op.
pub(crate) fn nft_upgrade_call(
    _env: &Env,
    _nft: &Address,
    _new_wasm_hash: &soroban_sdk::BytesN<32>,
) {
}
