//! Self-hosted multi-signer price oracle. Signers submit under `require_auth`;
//! write-time N-of-M median keeps reads O(1). RedStone-style reads fail closed;
//! SEP-40 reads soft-fail with `None`. Primary/anchor source for the
//! price-aggregator. See `docs/reference/invariants.md` §4.2.
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
    /// Caller is not in the registered signer set.
    NotAuthorizedSigner = 1,
    /// Submitted price is not strictly positive.
    InvalidPrice = 2,
    /// Threshold is zero, exceeds signer count, or signers contain duplicates.
    InvalidThreshold = 3,
    /// Signer address is already registered.
    SignerAlreadyRegistered = 4,
    /// Signer address is not in the registered set.
    SignerNotRegistered = 5,
    /// Removing the signer would leave fewer signers than the threshold.
    CannotRemoveBelowThreshold = 6,
    /// No cached aggregate (or empty history) for the requested feed.
    NoDataForFeed = 7,
    /// Cached aggregate age exceeds `MaxStaleSeconds`.
    StaleData = 8,
    /// Submitted price exceeds `MAX_SUBMITTED_PRICE`.
    PriceOutOfRange = 9,
    /// `feed_ids` and `prices` lengths differ on bulk submit.
    LengthMismatch = 10,
    /// Package timestamp is more than `MAX_FUTURE_SKEW_SECONDS` ahead of ledger time.
    FutureTimestamp = 11,
    /// Asset already mapped, or feed id already owned by another asset.
    FeedAlreadyMapped = 12,
    /// SEP-40 asset has no feed mapping.
    FeedNotMapped = 13,
    /// Feed id is not on the known-feed allowlist.
    FeedNotKnown = 14,
    /// Submission-age / stale-seconds window is below floor or inverted vs peer knob.
    InvalidSubmissionAge = 15,
    /// Package timestamp older than inclusion window or older than this signer's prior observation.
    StaleSubmission = 16,
    /// Feed id is already on the known-feed allowlist.
    FeedAlreadyRegistered = 17,
    /// Relative skew exceeds `MaxSubmissionAgeSeconds`.
    InvalidRelativeSkew = 18,
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
    /// * `InvalidThreshold` — `threshold == 0`, `threshold > signers.len()`,
    ///   or `signers` contains a duplicate address.
    pub fn __constructor(
        env: Env,
        admin: Address,
        signers: Vec<Address>,
        threshold: u32,
        resolution: u32,
    ) -> Result<(), Error> {
        if threshold == 0 || threshold > signers.len() || storage::has_duplicate(&signers) {
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
        store.set(
            &storage::DataKey::MaxRelativeSkewSeconds,
            &storage::DEFAULT_MAX_RELATIVE_SKEW_SECONDS,
        );
        store.set(&storage::DataKey::Resolution, &resolution);
        Ok(())
    }

    /// Replaces the contract Wasm with the code at `new_wasm_hash`, keeping
    /// the contract address and all storage intact. Owner only.
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
