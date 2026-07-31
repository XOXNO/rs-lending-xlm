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

    FeedAlreadyRegistered = 17,

    InvalidRelativeSkew = 18,
}

#[contract]
pub struct XoxnoOracle;

#[contractimpl]
impl XoxnoOracle {
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

    #[only_owner]
    pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
        storage::renew_oracle_instance(&env);
        stellar_contract_utils::upgradeable::upgrade(&env, &new_wasm_hash);
    }
}

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
