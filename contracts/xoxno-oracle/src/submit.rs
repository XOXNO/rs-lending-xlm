//! Write entrypoints: per-signer price submission. Aggregation itself lives
//! in `aggregation`.

use soroban_sdk::{contractimpl, Address, Env, String, Vec};

use crate::aggregation::{
    recompute_aggregate, require_fresh_submission, require_monotonic_package, require_not_future,
    store_submission, MAX_SUBMITTED_PRICE,
};
use crate::storage::{renew_oracle_instance, require_known_feed, require_registered_signer};
use crate::{Error, XoxnoOracle, XoxnoOracleArgs, XoxnoOracleClient};

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
    /// Records `signer`'s latest observation for `feed_id` and recomputes
    /// the cached aggregate. Caller must auth as `signer`.
    ///
    /// # Errors
    /// * `NotAuthorizedSigner` — `signer` is not a registered signer.
    /// * `FeedNotKnown` — `feed_id` was never registered by the owner.
    /// * `InvalidPrice` — `price <= 0`.
    /// * `PriceOutOfRange` — `price > MAX_SUBMITTED_PRICE`.
    /// * `FutureTimestamp` — `package_timestamp` is more than
    ///   `MAX_FUTURE_SKEW_SECONDS` ahead of the ledger clock.
    /// * `StaleSubmission` — `package_timestamp` is already older than the
    ///   `MaxSubmissionAgeSeconds` inclusion window, or older than this
    ///   signer's previously stored observation for the feed.
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

    /// Records `signer`'s latest observations for multiple feeds in one
    /// call, sharing a single `package_timestamp` and one auth check. All
    /// inputs are validated upfront; no partial application on failure.
    ///
    /// # Errors
    /// * `NotAuthorizedSigner` — `signer` is not a registered signer.
    /// * `LengthMismatch` — `feed_ids.len() != prices.len()`.
    /// * `FeedNotKnown` — any `feed_ids[i]` was never registered.
    /// * `InvalidPrice` — any `prices[i] <= 0`.
    /// * `PriceOutOfRange` — any `prices[i] > MAX_SUBMITTED_PRICE`.
    /// * `FutureTimestamp` — the shared `package_timestamp` is more than
    ///   `MAX_FUTURE_SKEW_SECONDS` ahead of the ledger clock.
    /// * `StaleSubmission` — the shared `package_timestamp` is already older
    ///   than the `MaxSubmissionAgeSeconds` inclusion window, or older than
    ///   this signer's stored observation for any of the feeds.
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
