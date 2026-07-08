//! Self-hosted multi-signer price oracle. Registered bot wallets submit
//! prices under plain `require_auth`; each feed's latest per-signer
//! submissions are aggregated into a median at write time, gated by an
//! N-of-M threshold, so reads stay O(1).
//!
//! One contract exposes two read ABIs: RedStone-style bulk reads
//! (`read_price_data`/`read_price_data_for_feed`) and SEP-40 / Reflector
//! reads (`base`/`decimals`/`resolution`/`assets`/`lastprice`/`price`/
//! `prices`). Either shape is a drop-in primary/anchor source for the
//! rs-lending-xlm controller.
//!
//! Two decoupled staleness windows: `MaxSubmissionAgeSeconds` bounds which
//! submissions may enter an aggregate (so one lagging signer cannot pin a
//! feed's freshness), `MaxStaleSeconds` bounds how long a cached aggregate
//! keeps serving. Keep the former `<=` every consumer's own `max_stale`.
#![no_std]

mod admin;
mod aggregation;
mod reads;
mod storage;
mod submit;

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
    FutureTimestamp = 11,
    FeedAlreadyMapped = 12,
    FeedNotMapped = 13,
    FeedNotKnown = 14,
    InvalidSubmissionAge = 15,
    StaleSubmission = 16,
}

#[contract]
pub struct XoxnoOracle;

#[contractimpl]
impl XoxnoOracle {
    /// Registers `admin` as the OZ `Ownable` owner, the initial `signers`
    /// set, the N-of-M `threshold`, and the SEP-40 `resolution`. Staleness
    /// windows start at their defaults; tune them via the owner setters.
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
        store.set(
            &storage::DataKey::MaxSubmissionAgeSeconds,
            &storage::DEFAULT_MAX_SUBMISSION_AGE_SECONDS,
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

/// `#[contractimpl]` can't see through to `Ownable`'s trait defaults, so each
/// body is written out. `transfer_ownership`/`renounce_ownership` gate on
/// owner auth internally — no `#[only_owner]` here.
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
