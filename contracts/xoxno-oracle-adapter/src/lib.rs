//! Self-hosted multi-signer price oracle. Bot wallets call `submit_price`/
//! `submit_prices` under plain `require_auth`; each signer's latest
//! submission per feed is aggregated via median, gated by an N-of-M
//! threshold.
//!
//! Exposes two read shapes from one contract: RedStone-ABI bulk reads
//! (`read_price_data_for_feed`/`read_price_data`, matching
//! `common::oracle::providers::redstone::RedStoneMultiFeed`) and SEP-40 /
//! Reflector-ABI reads (`base`/`decimals`/`resolution`/`assets`/`lastprice`/
//! `price`/`prices`). Either is a drop-in primary/anchor source for
//! rs-lending-xlm's controller. SEP-40 reads call the RedStone-ABI reads
//! directly (same contract, no cross-contract overhead).
//!
//! Owner-gated entrypoints reuse `stellar_access::ownable` +
//! `stellar_macros::only_owner` and `stellar_contract_utils::upgradeable`.
//! Ownership transfer is the 2-step `transfer_ownership`/`accept_ownership`
//! handshake so a typo'd new-owner address can't brick admin control.
//!
//! Aggregation runs at write-time (inside `submit_price`/`submit_prices`) so
//! reads stay O(1) regardless of signer count.
//!
//! Two independent staleness checks: `recompute_aggregate` gates which
//! *submissions* count toward a refreshed aggregate; `read_price_data_for_feed`
//! separately rejects a *cached* aggregate older than `MaxStaleSeconds`, so a
//! feed that stops receiving submissions doesn't serve a stale value forever.
#![no_std]

mod aggregation;
mod feed_reads;
mod sep40_reads;
mod storage;

use soroban_sdk::{contract, contracterror, contractimpl, Address, BytesN, Env, Vec};
use stellar_access::ownable::{self, Ownable};
use stellar_macros::only_owner;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotAuthorizedSigner = 1,
    InvalidPrice = 2,
    InvalidThreshold = 3,
    SignerAlreadyRegistered = 4,
    SignerNotRegistered = 5,
    CannotRemoveBelowThreshold = 6,
    NoDataForFeed = 7,
    StaleData = 8,
    PriceOutOfRange = 9,
    LengthMismatch = 10,
    FeedAlreadyMapped = 12,
    FeedNotMapped = 13,
    FeedNotKnown = 14,
}

#[contract]
pub struct XoxnoOracle;

#[contractimpl]
impl XoxnoOracle {
    /// Registers `admin` as the OZ `Ownable` owner, the initial `signers`
    /// set, the N-of-M `threshold`, and the SEP-40 `resolution`.
    /// `MaxStaleSeconds` defaults to `DEFAULT_MAX_STALE_SECONDS`.
    ///
    /// # Errors
    /// * `InvalidThreshold` - `threshold == 0`, `threshold > signers.len()`,
    ///   or `signers` contains a duplicate address.
    pub fn __constructor(
        env: Env,
        admin: Address,
        signers: Vec<Address>,
        threshold: u32,
        resolution: u32,
    ) -> Result<(), Error> {
        if threshold == 0 || threshold > signers.len() {
            return Err(Error::InvalidThreshold);
        }
        if storage::has_duplicate(&signers) {
            return Err(Error::InvalidThreshold);
        }

        ownable::set_owner(&env, &admin);

        let store = env.storage().instance();
        store.set(&storage::DataKey::Signers, &signers);
        store.set(&storage::DataKey::Threshold, &threshold);
        store.set(
            &storage::DataKey::MaxStaleSeconds,
            &storage::DEFAULT_MAX_STALE_SECONDS,
        );
        store.set(&storage::DataKey::Resolution, &resolution);
        Ok(())
    }

    /// Replaces the contract Wasm with the code at `new_wasm_hash`, keeping
    /// the contract address and all storage intact.
    #[only_owner]
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        storage::renew_oracle_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }
}

/// `#[contractimpl]` needs each method's body written out here (it can't see
/// through to `Ownable`'s trait defaults). `transfer_ownership`/
/// `renounce_ownership` already gate on owner auth internally, so no
/// `#[only_owner]` here.
#[contractimpl]
impl Ownable for XoxnoOracle {
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
