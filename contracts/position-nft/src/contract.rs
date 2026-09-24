//! Lending-position NFT: one token per controller account, and the token id is
//! the account id. The token owner (`owner_of`) is the account owner.
//! Mint, burn and upgrade are controller-only; `renew` is permissionless. The
//! rest is the stock OpenZeppelin non-fungible interface with a custom
//! `token_uri`.

use common::constants::{TTL_BUMP_USER, TTL_THRESHOLD_USER};
use common::ttl::renew_instance;
use soroban_sdk::{contract, contractimpl, contracttype, Address, BytesN, Env, String};
use stellar_contract_utils::upgradeable;
use stellar_tokens::non_fungible::burnable::emit_burn;
use stellar_tokens::non_fungible::{
    enumerable::{Enumerable, NonFungibleEnumerable},
    sequential, Base, NFTStorageKey, NonFungibleToken,
};

#[contracttype]
pub enum DataKey {
    Controller,
}

fn controller(e: &Env) -> Address {
    // Set in the constructor; the constructor cannot be skipped on Soroban.
    e.storage()
        .instance()
        .get(&DataKey::Controller)
        .expect("controller set at construction")
}

/// Extends the persistent `Owner(token_id)` and `Balance(owner)` entries to the
/// user window. The OZ enumeration entries (`NFTEnumerableStorageKey::*`) keep
/// their default windows and rely on protocol-23 auto-restore; they hold no
/// accounting state (INV-STOR-02b, INV-STOR-02d).
fn extend_user_persistent_ttl(e: &Env, owner: &Address, token_id: u32) {
    let p = e.storage().persistent();
    p.extend_ttl(
        &NFTStorageKey::Owner(token_id),
        TTL_THRESHOLD_USER,
        TTL_BUMP_USER,
    );
    p.extend_ttl(
        &NFTStorageKey::Balance(owner.clone()),
        TTL_THRESHOLD_USER,
        TTL_BUMP_USER,
    );
}

#[contract]
pub struct PositionNft;

#[contractimpl]
impl PositionNft {
    /// `controller` is the only address allowed to mint, burn and upgrade. Consumes
    /// token id 0 so the first position is id 1 — the controller ABI reserves
    /// account id 0 as the "create new account" sentinel.
    pub fn __constructor(e: &Env, controller: Address, uri: String, name: String, symbol: String) {
        e.storage()
            .instance()
            .set(&DataKey::Controller, &controller);
        Base::set_metadata(e, uri, name, symbol);
        sequential::increment_token_id(e, 1);
    }

    /// Mints the next sequential position token to `to`. Controller-only.
    ///
    /// Renews the instance TTL (controller address, collection metadata and
    /// id counter) on every account creation (INV-STOR-02a).
    pub fn mint(e: &Env, to: Address) -> u32 {
        controller(e).require_auth();
        renew_instance(e);
        let token_id = Enumerable::sequential_mint(e, &to);
        // sequential_mint writes Owner/Balance at the network minimum TTL; lift
        // them to the user window so a new position does not archive early.
        extend_user_persistent_ttl(e, &to, token_id);
        token_id
    }

    /// Burns `token_id` without the holder's authorization. Controller-only.
    ///
    /// Not the OZ `Burnable` extension: `Base::burn` calls
    /// `from.require_auth()`, and the controller must burn when liquidation
    /// empties an account without the owner's signature. This replicates
    /// `Enumerable::burn` (v0.7.1) without that auth: `Base::update` removes the
    /// owner and approval and decrements the holder's balance. The enumeration
    /// helper updates owner enumeration, total supply and global enumeration.
    ///
    /// Renews the instance TTL on every account deletion.
    pub fn burn(e: &Env, token_id: u32) {
        controller(e).require_auth();
        renew_instance(e);
        let owner = Base::owner_of(e, token_id);
        Base::update(e, Some(&owner), None, token_id);
        emit_burn(e, &owner, token_id);
        Enumerable::remove_from_enumerations(e, &owner, token_id);
    }

    /// Extends the TTL of `token_id`'s persistent `Owner` entry and its owner's
    /// `Balance` entry to the protocol's per-user renewal window, plus the
    /// instance TTL.
    /// Permissionless: a TTL extension cannot move, approve or reassign the
    /// token, and it cannot shorten a lifetime.
    ///
    /// OZ `owner_of` extends `Owner` to only 30 days. The controller calls this
    /// from `renew_account`, so account renewal keeps ownership alive for the
    /// same 120-day window as the account entries (INV-STOR-02b).
    ///
    /// Panics with the OZ `NonExistentToken` error when the token was never
    /// minted or was burned.
    pub fn renew(e: &Env, token_id: u32) {
        // Existence check first: extend_ttl on a missing key would trap with
        // a storage error; owner_of gives the standard token error instead.
        let owner = Base::owner_of(e, token_id);
        extend_user_persistent_ttl(e, &owner, token_id);
        renew_instance(e);
    }

    /// Upgrades the contract WASM to `new_wasm_hash`, extending the instance
    /// TTL first. Controller-only: the only path is the controller's owner-only
    /// `upgrade_position_nft`, behind the governance timelock.
    pub fn upgrade(e: &Env, new_wasm_hash: BytesN<32>) {
        controller(e).require_auth();
        renew_instance(e);
        upgradeable::upgrade(e, &new_wasm_hash);
    }
}

/// Query suffix that `token_uri` appends after the token id. The stock OZ
/// `token_uri` (base + id) cannot append it, so `token_uri` is overridden.
const TOKEN_URI_SUFFIX: &str = "?isStatic=true&chain=STELLAR";

#[contractimpl(contracttrait)]
impl NonFungibleToken for PositionNft {
    type ContractType = Enumerable;

    /// `{stored base_uri}{token_id}?isStatic=true&chain=STELLAR`
    ///
    /// Panics with the OZ `NonExistentToken` error for burned or never-minted
    /// ids, matching the stock behavior.
    fn token_uri(e: &Env, token_id: u32) -> String {
        let _owner = Base::owner_of(e, token_id);

        let base = Base::base_uri(e);
        let base_len = base.len() as usize;
        // OZ `set_metadata` caps the base at `MAX_BASE_URI_LEN` (200 bytes):
        // 200 + 10 digits (u32 max) + 28-byte suffix fits in 256.
        let mut buf = [0u8; 256];
        base.copy_into_slice(&mut buf[..base_len]);
        let mut len = base_len;
        // Decimal digits, most significant first. token_id >= 1 always
        // (id 0 is consumed at construction), so no zero special-case.
        let mut digits = [0u8; 10];
        let mut n = token_id;
        let mut count = 0usize;
        while n > 0 {
            digits[count] = b'0' + (n % 10) as u8;
            n /= 10;
            count += 1;
        }
        while count > 0 {
            count -= 1;
            buf[len] = digits[count];
            len += 1;
        }
        for b in TOKEN_URI_SUFFIX.bytes() {
            buf[len] = b;
            len += 1;
        }
        String::from_bytes(e, &buf[..len])
    }
}

#[contractimpl(contracttrait)]
impl NonFungibleEnumerable for PositionNft {}
