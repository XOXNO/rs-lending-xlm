//! Lending-position NFT: one token per controller account. Token id ==
//! controller account id. Mint and burn are controller-only; everything else
//! is the stock OpenZeppelin non-fungible interface so external indexers and
//! marketplaces can track positions without protocol-specific tooling.

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

/// Extends the reachable persistent entries — `Owner(token_id)` and
/// `Balance(owner)` — to the user window. The OZ enumeration entries
/// (`NFTEnumerableStorageKey::*`) are deliberately left on their default
/// windows and rely on protocol-23 auto-restore: renewing them would need
/// extra index reads per token for no accounting-critical state; see
/// F-11 / INV-STOR-02d.
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
    /// `controller` is the only address allowed to mint and burn. Consumes
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
    /// Renews the instance TTL: the instance holds the controller address,
    /// collection metadata, and the sequential id counter, and `mint` runs on
    /// every controller account creation, so this is the natural cadence to
    /// keep it alive. See `docs/reference/invariants.md` (INV-STOR-02) for
    /// the resulting TTL asymmetry with the per-token `Owner` entry.
    pub fn mint(e: &Env, to: Address) -> u32 {
        controller(e).require_auth();
        renew_instance(e);
        let token_id = Enumerable::sequential_mint(e, &to);
        // sequential_mint writes Owner/Balance at the network minimum; lift them
        // to the user window so a fresh position does not archive early (F-7).
        extend_user_persistent_ttl(e, &to, token_id);
        token_id
    }

    /// Burns `token_id` without the holder's authorization. Controller-only.
    ///
    /// Deliberately NOT the OZ `Burnable` extension: `Base::burn` calls
    /// `from.require_auth()`, but the controller must burn when an account
    /// empties through liquidation, where the owner never signed. This
    /// replicates `Enumerable::burn` (v0.7.1) minus that auth:
    /// `Base::update` clears owner/balance/approval, then the enumeration
    /// helper maintains owner enumeration, total supply, and global
    /// enumeration.
    ///
    /// Renews the instance TTL for the same reason as `mint` — this runs on
    /// every controller account deletion (e.g. liquidation cleanup).
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
    /// Permissionless: extending a TTL is pure rent charity — it cannot move,
    /// approve, or reassign the token, and it cannot shorten a lifetime.
    ///
    /// This closes the renewal asymmetry with OZ's `owner_of`, which extends
    /// the `Owner` entry by only its own 30-day default: the controller calls
    /// this from `renew_account`, so an account renewal keeps the ownership
    /// leg alive for the same 120-day window as the controller's own entries
    /// (INV-STOR-02).
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
    /// TTL first. Controller-only — reachable on mainnet solely through the
    /// controller's `upgrade_position_nft`, which sits behind the same
    /// governance ownership and timelock as every other upgrade.
    pub fn upgrade(e: &Env, new_wasm_hash: BytesN<32>) {
        controller(e).require_auth();
        renew_instance(e);
        upgradeable::upgrade(e, &new_wasm_hash);
    }
}

/// The query suffix rides AFTER the token id, which the stock OZ
/// base-and-append composition cannot produce — hence the `token_uri`
/// override below. The base itself is the STORED `base_uri` (set at
/// construction), so raw storage stays the single source of truth and
/// metadata changes are constructor- or upgrade-driven only.
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
        // Stored base + 10 digits (u32 max) + suffix; 256 leaves headroom
        // for any plausible future base_uri.
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
