#![no_std]

//! XOXNO oracle contract. Aggregates prices from a configured set of authorized
//! signers: each signer posts their latest per-feed submission (with auth), and
//! the contract forms a median once at least `threshold` fresh, skew-clustered
//! submissions exist. History is bounded; prices are exposed via RedStone-shaped
//! feed APIs and a Reflector-compatible asset API. Owner-only admin configures
//! signers, threshold, feeds, and staleness/skew bounds.

mod admin;
mod aggregation;
mod reads;
mod storage;
mod submit;

use soroban_sdk::{contract, contracterror, contractimpl, Address, BytesN, Env, Vec};

use stellar_access::ownable::{self, Ownable};
use stellar_macros::only_owner;

/// Error conditions returned by the oracle's contract calls.
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

    FeedAlreadyRegistered = 17,

    InvalidRelativeSkew = 18,
}

/// The XOXNO oracle contract type.
#[contract]
pub struct XoxnoOracle;

#[contractimpl]
impl XoxnoOracle {
    /// Initializes the contract: sets `admin` as owner and stores the
    /// initial signer set, submission threshold, price resolution, and
    /// default staleness, submission-age, and skew bounds. Fails with
    /// `InvalidThreshold` if `threshold` is zero, exceeds the number of
    /// signers, or `signers` contains a duplicate address.
    pub fn __constructor(
        env: Env,
        admin: Address,
        signers: Vec<Address>,
        threshold: u32,
        resolution: u32,
    ) -> Result<(), Error> {
        if threshold == 0 || threshold > signers.len() || has_duplicate(&signers) {
            return Err(Error::InvalidThreshold);
        }

        ownable::set_owner(&env, &admin);

        storage::store_signers(&env, &signers);
        storage::store_threshold(&env, threshold);
        storage::store_max_stale_seconds(&env, storage::DEFAULT_MAX_STALE_SECONDS);
        storage::store_max_submission_age(&env, storage::DEFAULT_MAX_SUBMISSION_AGE_SECONDS);
        storage::store_max_relative_skew(&env, storage::DEFAULT_MAX_RELATIVE_SKEW_SECONDS);
        storage::store_resolution(&env, resolution);
        Ok(())
    }

    /// Renews the contract's instance storage TTL and upgrades the contract
    /// to the WASM code at `new_wasm_hash`.
    #[only_owner]
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        storage::renew_oracle_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }
}

#[contractimpl]
impl Ownable for XoxnoOracle {
    /// Returns the current contract owner, if set.
    fn get_owner(e: &Env) -> Option<Address> {
        ownable::get_owner(e)
    }

    /// Starts an ownership transfer to `new_owner`, valid for acceptance
    /// until `live_until_ledger`.
    fn transfer_ownership(e: &Env, new_owner: Address, live_until_ledger: u32) {
        ownable::transfer_ownership(e, &new_owner, live_until_ledger);
    }

    /// Accepts a pending ownership transfer, making the caller the new owner.
    fn accept_ownership(e: &Env) {
        ownable::accept_ownership(e);
    }

    /// Renounces ownership, leaving the contract without an owner.
    fn renounce_ownership(e: &Env) {
        ownable::renounce_ownership(e);
    }
}

/// Returns true if `signers` contains any address more than once.
fn has_duplicate(signers: &Vec<Address>) -> bool {
    for i in 0..signers.len() {
        for j in (i + 1)..signers.len() {
            if signers.get_unchecked(i) == signers.get_unchecked(j) {
                return true;
            }
        }
    }
    false
}
