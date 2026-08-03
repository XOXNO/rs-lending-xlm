mod config;
mod index;
mod registry;
mod ttl;

pub(crate) use config::*;
pub(crate) use index::*;
pub(crate) use registry::*;
pub(crate) use ttl::*;

use common::oracle::providers::reflector::ReflectorAsset;

use soroban_sdk::{contracttype, Address, String};

pub(crate) const DEFAULT_MAX_STALE_SECONDS: u64 = 86_400;

pub(crate) const DEFAULT_MAX_SUBMISSION_AGE_SECONDS: u64 = 900;

pub(crate) const MIN_SUBMISSION_AGE_SECONDS: u64 = 60;

pub(crate) const DEFAULT_MAX_RELATIVE_SKEW_SECONDS: u64 = DEFAULT_MAX_SUBMISSION_AGE_SECONDS;

#[contracttype]
#[derive(Clone, Debug)]
pub(crate) enum DataKey {
    Signers,
    Threshold,
    MaxStaleSeconds,
    MaxSubmissionAgeSeconds,

    MaxRelativeSkewSeconds,
    Resolution,
    LatestSubmission(String, Address),

    SignerFeeds(Address),
    CurrentAggregate(String),
    History(String),
    FeedMapping(ReflectorAsset),

    FeedOwner(String),

    AssetCount,
    AssetAt(u32),
    AssetIndex(ReflectorAsset),

    FeedCount,
    FeedAt(u32),
    FeedIndex(String),
}

#[contracttype]
#[derive(Clone, Debug)]
pub(crate) struct SignerSubmission {
    pub(crate) price: i128,
    pub(crate) package_timestamp: u64,
}
