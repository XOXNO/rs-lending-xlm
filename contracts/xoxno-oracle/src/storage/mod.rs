//! Storage keys, types, and raw storage helpers. No entrypoints live here.
//!
//! Assets and known feed ids are each kept as a swap-remove indexed set
//! (count + slot-array + reverse lookup) in persistent storage, so add/remove
//! cost O(1) persistent writes instead of rewriting a growing instance blob.

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

/// Cache TTL (24h, RedStone's own convention): how long a feed keeps serving
/// after submissions stop entirely. Deliberately looser than the aggregation
/// inclusion window below.
pub(crate) const DEFAULT_MAX_STALE_SECONDS: u64 = 86_400;

/// Aggregation inclusion window (15 min): a submission older than this counts
/// toward neither the median nor the reported observation time. Must be kept
/// `<=` every consumer's own `max_stale`.
pub(crate) const DEFAULT_MAX_SUBMISSION_AGE_SECONDS: u64 = 900;

/// Floor for `MaxSubmissionAgeSeconds`, so the window can't be set so tight
/// that ordinary propagation delay drops the quorum on every recompute.
pub(crate) const MIN_SUBMISSION_AGE_SECONDS: u64 = 60;

/// Default relative cluster skew equals the absolute inclusion window so a
/// desynced-but-still-fresh signer is not dropped beyond what
/// `MaxSubmissionAgeSeconds` already excludes. Tighten via
/// `set_max_relative_skew_seconds` when bots submit in a tight wave.
pub(crate) const DEFAULT_MAX_RELATIVE_SKEW_SECONDS: u64 = DEFAULT_MAX_SUBMISSION_AGE_SECONDS;

#[contracttype]
#[derive(Clone, Debug)]
pub(crate) enum DataKey {
    Signers,
    Threshold,
    MaxStaleSeconds,
    MaxSubmissionAgeSeconds,
    /// Max package-time lag behind the freshest absolute-fresh peer that may
    /// still enter the median cluster.
    MaxRelativeSkewSeconds,
    Resolution,
    LatestSubmission(String, Address),
    /// Per-signer index of feed ids the signer has submitted to. Lets
    /// `remove_signer` clean up in O(feeds-this-signer-touched).
    SignerFeeds(Address),
    CurrentAggregate(String),
    History(String),
    FeedMapping(ReflectorAsset),
    /// Reverse of `FeedMapping`: at most one asset may own a feed id.
    FeedOwner(String),
    /// Enumerable asset index: count, slot, and reverse lookup.
    AssetCount,
    AssetAt(u32),
    AssetIndex(ReflectorAsset),
    /// Enumerable known-feed allowlist: only registered feeds accept submits.
    /// Populated by `register_feed` / `add_feed`, not by raw submissions.
    FeedCount,
    FeedAt(u32),
    FeedIndex(String),
}

/// A single signer's latest raw submission for one feed. `price` stays `i128`
/// (not `U256`): per-signer submissions never leave the contract.
#[contracttype]
#[derive(Clone, Debug)]
pub(crate) struct SignerSubmission {
    pub(crate) price: i128,
    pub(crate) package_timestamp: u64,
}
