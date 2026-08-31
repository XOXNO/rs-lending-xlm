use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, String, Vec};
use xoxno_oracle::{XoxnoOracle, XoxnoOracleClient};

use crate::core::LendingTest;

pub const XOXNO_TEST_RESOLUTION: u32 = 300;

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
        let price_raw = price_wad / 10_000_000_000;
        let feed_id = String::from_str(&t.env, feed);
        client.register_feed(&feed_id);
        for signer in signers.iter() {
            client.submit_price(signer, &feed_id, &price_raw, &package_timestamp_ms);
        }
    }

    (adapter, signers)
}
