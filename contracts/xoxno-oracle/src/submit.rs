//! Signer-facing entry points for submitting prices to the oracle, single
//! and batched, with the validation chain that gates each submission before
//! it is stored and the feed's aggregate is recomputed.

use soroban_sdk::{contractimpl, Address, Env, String, Vec};

use crate::aggregation::{
    recompute_aggregate, require_fresh_submission, require_monotonic_package, require_not_future,
    store_submission, MAX_SUBMITTED_PRICE,
};
use crate::storage::{renew_oracle_instance, require_known_feed, require_registered_signer};
use crate::{Error, XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

/// Rejects `price` if it is not strictly positive or exceeds
/// `MAX_SUBMITTED_PRICE`.
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
    /// Submits `price` for `feed_id` on behalf of `signer`, requiring
    /// `signer`'s authorization. Validates that `signer` is registered,
    /// `feed_id` is known, the price is within bounds, and
    /// `package_timestamp` (milliseconds) is not in the future, not stale
    /// (age vs max submission age in seconds), and not older than the
    /// signer's previous submission for the feed. Stores the submission and
    /// recomputes the feed's aggregate.
    pub fn submit_price(
        env: Env,
        signer: Address,
        feed_id: String,
        price: i128,
        package_timestamp: u64,
    ) -> Result<(), Error> {
        renew_oracle_instance(&env);
        signer.require_auth();
        require_registered_signer(&env, &signer)?;
        require_known_feed(&env, &feed_id)?;
        validate_price(price)?;
        require_not_future(&env, package_timestamp)?;
        require_fresh_submission(&env, package_timestamp)?;
        require_monotonic_package(&env, &feed_id, &signer, package_timestamp)?;

        store_submission(&env, &feed_id, &signer, price, package_timestamp);
        recompute_aggregate(&env, &feed_id);
        Ok(())
    }

    /// Submits `prices` for `feed_ids` on behalf of `signer`, requiring
    /// `signer`'s authorization and using the same `package_timestamp`
    /// (milliseconds) for every entry; fails with `LengthMismatch` if the two
    /// lists differ in length. Validates that `signer` is registered, the
    /// timestamp is not in the future or stale (age vs max submission age in
    /// seconds), and each feed is known, monotonic for `signer`, and its
    /// price within bounds, before storing any submission. Stores each
    /// submission and recomputes each feed's aggregate.
    pub fn submit_prices(
        env: Env,
        signer: Address,
        feed_ids: Vec<String>,
        prices: Vec<i128>,
        package_timestamp: u64,
    ) -> Result<(), Error> {
        renew_oracle_instance(&env);
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
            recompute_aggregate(&env, &feed_id);
        }
        Ok(())
    }
}
