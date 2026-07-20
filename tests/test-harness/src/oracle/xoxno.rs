//! Test helpers for the real `xoxno-oracle-adapter` contract (registered
//! natively, no mock): signer setup, submissions, and market wiring.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, String, Vec};
use xoxno_oracle_adapter::{XoxnoOracle, XoxnoOracleClient};

use crate::core::types::LendingTest;

/// SEP-40 resolution the adapter is registered with in tests.
pub const XOXNO_TEST_RESOLUTION: u32 = 300;

/// Registers the real adapter with `threshold`-of-`signers.len()` signers and
/// submits `price_wad` for each feed from every signer (median = the price).
///
/// `feeds` is a slice of `(feed_id, price_wad)` pairs; prices are WAD and are
/// scaled down to the adapter's 8-decimal width before submission. Returns
/// the adapter address and the generated signer set.
pub fn register_xoxno_adapter(
    t: &LendingTest,
    feeds: &[(&str, i128)],
    signer_count: u32,
    threshold: u32,
) -> (Address, std::vec::Vec<Address>) {
    let signers: std::vec::Vec<Address> = (0..signer_count)
        .map(|_| Address::generate(&t.env))
        .collect();
    let mut signers_vec = Vec::new(&t.env);
    for signer in signers.iter() {
        signers_vec.push_back(signer.clone());
    }

    let adapter = t.env.register(
        XoxnoOracle,
        (
            t.admin.clone(),
            signers_vec,
            threshold,
            XOXNO_TEST_RESOLUTION,
        ),
    );

    let client = XoxnoOracleClient::new(&t.env, &adapter);
    let package_timestamp_ms = t.env.ledger().timestamp() * 1_000;
    for (feed, price_wad) in feeds {
        // WAD (18) down to the adapter's 8-decimal submission width.
        let price_raw = price_wad / 10_000_000_000;
        let feed_id = String::from_str(&t.env, feed);
        client.register_feed(&feed_id);
        for signer in signers.iter() {
            client.submit_price(signer, &feed_id, &price_raw, &package_timestamp_ms);
        }
    }

    (adapter, signers)
}
