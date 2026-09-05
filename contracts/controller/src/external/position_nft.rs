use common::errors::GenericError;
use position_nft_interface::PositionNftClient;
use soroban_sdk::{panic_with_error, Address, BytesN, Env};

/// Mints a position NFT and widens its sequential `u32` ID to a `u64` account ID.
pub(crate) fn nft_mint_call(env: &Env, nft: &Address, to: &Address) -> u64 {
    u64::from(PositionNftClient::new(env, nft).mint(to))
}

/// Burns the NFT; IDs outside the mintable `u32` domain fail with `AccountNotFound`.
pub(crate) fn nft_burn_call(env: &Env, nft: &Address, account_id: u64) {
    let token_id = u32::try_from(account_id)
        .unwrap_or_else(|_| panic_with_error!(env, GenericError::AccountNotFound));
    PositionNftClient::new(env, nft).burn(&token_id);
}

/// Returns current NFT ownership. Out-of-range IDs, missing tokens, and failed
/// lookups return `None`, so ownership checks fail closed.
pub(crate) fn nft_try_owner_of_call(env: &Env, nft: &Address, account_id: u64) -> Option<Address> {
    let token_id = u32::try_from(account_id).ok()?;
    match PositionNftClient::new(env, nft).try_owner_of(&token_id) {
        Ok(Ok(owner)) => Some(owner),
        _ => None,
    }
}

/// Renews the NFT owner entry to the protocol user TTL window.
/// IDs outside the mintable `u32` domain fail with `AccountNotFound`.
pub(crate) fn nft_renew_call(env: &Env, nft: &Address, account_id: u64) {
    let token_id = u32::try_from(account_id)
        .unwrap_or_else(|_| panic_with_error!(env, GenericError::AccountNotFound));
    PositionNftClient::new(env, nft).renew(&token_id);
}

/// Upgrades NFT Wasm through the controller's owner-gated admin path.
pub(crate) fn nft_upgrade_call(env: &Env, nft: &Address, new_wasm_hash: &BytesN<32>) {
    PositionNftClient::new(env, nft).upgrade(new_wasm_hash);
}

#[cfg(test)]
#[path = "../../tests/external/position_nft.rs"]
mod tests;
