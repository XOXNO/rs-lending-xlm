//! Signer-facing entry points that submit prices to the oracle, one at a time
//! or in a batch. Each submission passes a validation chain before the contract
//! stores it and recomputes the aggregate of the feed.

use common::ttl::renew_instance;

use soroban_sdk::{contractimpl, Address, Env, String, Vec};

use crate::aggregation::{
    recompute_aggregate, require_fresh_submission, require_monotonic_package, require_not_future,
    store_submission, QuorumMiss, MAX_SUBMITTED_PRICE,
};
use crate::storage::{require_known_feed, require_registered_signer};
use crate::{Error, XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

/// Rejects `price` with `InvalidPrice` if it is not strictly positive, or with
/// `PriceOutOfRange` if it exceeds `MAX_SUBMITTED_PRICE`.
fn validate_price(price: i128) -> Result<(), Error> {
    if price <= 0 {
        return Err(Error::InvalidPrice);
    }
    if price > MAX_SUBMITTED_PRICE {
        return Err(Error::PriceOutOfRange);
    }
    Ok(())
}

#[contractimpl]
impl XoxnoOracle {
    /// Submits `price` for `feed_id` on behalf of `signer`. Requires the
    /// authorization of `signer`. Validates that `signer` is registered,
    /// `feed_id` is known, and the price is within bounds. Also validates that
    /// `package_timestamp` (milliseconds) is not in the future, is not stale,
    /// and is not older than the previous submission of the signer for the
    /// feed. The staleness check compares the age against the maximum
    /// submission age in seconds. Stores the submission and recomputes the
    /// aggregate of the feed.
    pub fn submit_price(
        env: Env,
        signer: Address,
        feed_id: String,
        price: i128,
        package_timestamp: u64,
    ) -> Result<(), Error> {
        renew_instance(&env);
        signer.require_auth();
        require_registered_signer(&env, &signer)?;
        require_known_feed(&env, &feed_id)?;
        validate_price(price)?;
        require_not_future(&env, package_timestamp)?;
        require_fresh_submission(&env, package_timestamp)?;
        require_monotonic_package(&env, &feed_id, &signer, package_timestamp)?;

        store_submission(&env, &feed_id, &signer, price, package_timestamp);
        recompute_aggregate(&env, &feed_id, QuorumMiss::Retain);
        Ok(())
    }

    /// Submits `prices` for `feed_ids` on behalf of `signer`. Requires the
    /// authorization of `signer` and uses the same `package_timestamp`
    /// (milliseconds) for every entry. Validates every entry before it stores
    /// any submission. Then stores each submission and recomputes the aggregate
    /// of each feed.
    ///
    /// Returns the first failure in this order: `NotAuthorizedSigner`,
    /// `LengthMismatch`, `FutureTimestamp`, `StaleSubmission` (age), then per
    /// entry `FeedNotKnown` or `StaleSubmission` (not monotonic), then
    /// `InvalidPrice` or `PriceOutOfRange` over all prices. A later entry's
    /// feed fault therefore wins over an earlier entry's price fault.
    pub fn submit_prices(
        env: Env,
        signer: Address,
        feed_ids: Vec<String>,
        prices: Vec<i128>,
        package_timestamp: u64,
    ) -> Result<(), Error> {
        renew_instance(&env);
        signer.require_auth();
        require_registered_signer(&env, &signer)?;
        if feed_ids.len() != prices.len() {
            return Err(Error::LengthMismatch);
        }
        require_not_future(&env, package_timestamp)?;
        require_fresh_submission(&env, package_timestamp)?;
        for feed_id in feed_ids.iter() {
            require_known_feed(&env, &feed_id)?;
            require_monotonic_package(&env, &feed_id, &signer, package_timestamp)?;
        }
        for price in prices.iter() {
            validate_price(price)?;
        }

        for (feed_id, price) in feed_ids.iter().zip(prices.iter()) {
            store_submission(&env, &feed_id, &signer, price, package_timestamp);
            recompute_aggregate(&env, &feed_id, QuorumMiss::Retain);
        }
        Ok(())
    }
}
