//! Storage layer for the oracle contract: instance-level configuration
//! (signers, threshold, staleness and skew bounds, resolution), persistent
//! feed and asset registries backed by swap-remove indexed collections, and
//! TTL renewal helpers. Re-exports the `config`, `index`, `registry`, and
//! `ttl` submodules' public items, and defines the `DataKey` storage-key
//! enum and default configuration constants shared across them.

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

/// Default maximum age, in seconds, a stored price can reach before it is considered stale.
pub(crate) const DEFAULT_MAX_STALE_SECONDS: u64 = 86_400;

/// Default maximum age, in seconds, a signer's submission timestamp can have relative to the
/// ledger time to be accepted.
pub(crate) const DEFAULT_MAX_SUBMISSION_AGE_SECONDS: u64 = 900;

/// Minimum allowed value, in seconds, for the configured maximum submission age.
pub(crate) const MIN_SUBMISSION_AGE_SECONDS: u64 = 60;

/// Default maximum allowed timestamp skew, in seconds, between signer submissions for the same
/// price update.
pub(crate) const DEFAULT_MAX_RELATIVE_SKEW_SECONDS: u64 = DEFAULT_MAX_SUBMISSION_AGE_SECONDS;

/// Storage keys for the oracle contract. `Signers` through `Resolution` hold
/// instance configuration. `LatestSubmission` is **persistent** per-signer
/// latest package state. `SignerFeeds` maps a signer to the feed ids it
/// submits for. `CurrentAggregate` and `History` hold, respectively, the latest
/// aggregated price and the price history for a feed. `FeedMapping` and
/// `FeedOwner` link a `ReflectorAsset` to its feed id and back. The
/// `AssetCount`/`AssetAt`/`AssetIndex` and `FeedCount`/`FeedAt`/`FeedIndex`
/// groups back the swap-remove indexed asset and feed collections in the
/// `index` submodule.
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

/// A single signer's most recent price submission for a feed: the submitted price and the
/// package timestamp it was submitted with.
#[contracttype]
#[derive(Clone, Debug)]
pub(crate) struct SignerSubmission {
    pub(crate) price: i128,
    pub(crate) package_timestamp: u64,
}
