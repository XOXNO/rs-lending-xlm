use common::errors::GenericError;
use position_nft_interface::PositionNftClient;
use soroban_sdk::{panic_with_error, Address, BytesN, Env};

/// Mints a position NFT to `to`, widening the sequential `u32` token id into
/// the controller's `u64` account-id domain. Infallible widening.
pub(crate) fn nft_mint_call(env: &Env, nft: &Address, to: &Address) -> u64 {
    u64::from(PositionNftClient::new(env, nft).mint(to))
}

/// Burns `account_id`'s NFT. Ids above `u32::MAX` can never have been minted,
/// so the narrowing failure is an unknown-account condition.
pub(crate) fn nft_burn_call(env: &Env, nft: &Address, account_id: u64) {
    let token_id = u32::try_from(account_id)
        .unwrap_or_else(|_| panic_with_error!(env, GenericError::AccountNotFound));
    PositionNftClient::new(env, nft).burn(&token_id);
}

/// Resolves the current NFT owner of `account_id`. `None` when the id is
/// outside the mintable domain or the token does not exist (never minted, or
/// burned when the account emptied). Fail closed: absence is never an owner.
pub(crate) fn nft_try_owner_of_call(env: &Env, nft: &Address, account_id: u64) -> Option<Address> {
    let token_id = u32::try_from(account_id).ok()?;
    match PositionNftClient::new(env, nft).try_owner_of(&token_id) {
        Ok(Ok(owner)) => Some(owner),
        _ => None,
    }
}

/// Extends the TTL of `account_id`'s NFT `Owner` entry to the protocol's
/// per-user renewal window. Ids above `u32::MAX` can never have been minted.
pub(crate) fn nft_renew_call(env: &Env, nft: &Address, account_id: u64) {
    let token_id = u32::try_from(account_id)
        .unwrap_or_else(|_| panic_with_error!(env, GenericError::AccountNotFound));
    PositionNftClient::new(env, nft).renew(&token_id);
}

/// Upgrades the position-NFT contract's WASM. Reached only from the
/// owner-gated `upgrade_position_nft` admin entrypoint.
pub(crate) fn nft_upgrade_call(env: &Env, nft: &Address, new_wasm_hash: &BytesN<32>) {
    PositionNftClient::new(env, nft).upgrade(new_wasm_hash);
}

#[cfg(test)]
#[path = "../../tests/external/position_nft.rs"]
mod tests;
