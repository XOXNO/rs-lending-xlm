//! Storage layer for the oracle contract: instance-level configuration
//! (signers, threshold, staleness, skew and spread bounds, resolution),
//! persistent feed and asset registries backed by swap-remove indexed
//! collections, per-signer submissions with aggregates and history, and TTL
//! renewal helpers. Re-exports the `config`, `index`, `prices`, and `registry`
//! submodules' items, and defines the `DataKey` storage-key enum and the
//! default configuration constants.
//!
//! Only this module constructs `DataKey`s or calls `env.storage()`. Other
//! modules use its accessors, so key encoding and TTL policy stay in one place.

mod config;
mod index;
mod prices;
mod registry;
mod ttl;

pub(crate) use config::*;
pub(crate) use index::*;
pub(crate) use prices::*;
pub(crate) use registry::*;

use common::oracle::observation::MAX_FUTURE_SKEW_SECONDS;
use common::oracle::providers::reflector::ReflectorAsset;

use soroban_sdk::{contracttype, Address, String};

/// Default maximum age, in seconds, a stored price can reach before it is considered stale.
pub(crate) const DEFAULT_MAX_STALE_SECONDS: u64 = 86_400;

/// Default maximum age, in seconds, a signer's submission timestamp can have relative to the
/// ledger time to be accepted.
pub(crate) const DEFAULT_MAX_SUBMISSION_AGE_SECONDS: u64 = 900;

/// Minimum configurable maximum submission age, in seconds. Held one second
/// above `MAX_FUTURE_SKEW_SECONDS` so the age cannot be pinned to the
/// future-skew bound and collapse the effective eviction window.
pub(crate) const MIN_SUBMISSION_AGE_SECONDS: u64 = MAX_FUTURE_SKEW_SECONDS + 1;

/// Default maximum allowed timestamp skew, in seconds, between signer submissions for the same
/// price update.
pub(crate) const DEFAULT_MAX_RELATIVE_SKEW_SECONDS: u64 = DEFAULT_MAX_SUBMISSION_AGE_SECONDS;

/// Default maximum price spread, in basis points, of a cluster smaller than
/// `2 * (signers - threshold) + 1` entries.
pub(crate) const DEFAULT_MAX_CLUSTER_SPREAD_BPS: u32 = 200;

/// Storage keys for the oracle contract. `Signers` through `Resolution` and
/// `MaxClusterSpreadBps` hold instance configuration; all other keys are
/// persistent. `LatestSubmission` holds a signer's latest submission for a
/// feed. `SignerFeeds` maps a signer to the feed ids it submits for.
/// `CurrentAggregate` and `History` hold, respectively, the latest aggregated
/// price and the price history for a feed. `FeedMapping` and `FeedOwner` link
/// a `ReflectorAsset` to its feed id and back. The
/// `AssetCount`/`AssetAt`/`AssetIndex` and `FeedCount`/`FeedAt`/`FeedIndex`
/// groups back the swap-remove indexed asset and feed collections in the
/// `index` submodule.
#[contracttype]
#[derive(Clone, Debug)]
pub(in crate::storage) enum DataKey {
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

    MaxClusterSpreadBps,
}

/// A single signer's most recent price submission for a feed: the submitted price and the
/// package timestamp (milliseconds) it was submitted with.
#[contracttype]
#[derive(Clone, Debug)]
pub(crate) struct SignerSubmission {
    pub(crate) price: i128,
    pub(crate) package_timestamp: u64,
}
