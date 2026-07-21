//! Shared black-box test helpers for the `xoxno-oracle-adapter` integration
//! tests. Every test binary under `tests/` pulls this in via `mod common;`.

#![allow(dead_code)]
extern crate std;

use xoxno_oracle::{Error, XoxnoOracle, XoxnoOracleClient};

use common::oracle::providers::reflector::ReflectorAsset;
use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
use soroban_sdk::{contracttype, Address, ConversionError, Env, InvokeError, String, Symbol};

/// Mirror of the crate-private `DataKey` variants that tests assert on
/// directly (storage-invariant and TTL checks). `#[contracttype]` enum keys
/// serialize as `[variant-name, payload...]`, so a variant with the same name
/// and payload produces the identical storage key regardless of the enum's
/// Rust-side name.
#[contracttype]
pub enum MirrorKey {
    LatestSubmission(String, Address),
    SignerFeeds(Address),
}

pub const FEED: &str = "XLM/USD";
pub const TEST_RESOLUTION: u32 = 300;

pub fn setup(
    env: &Env,
    signer_count: u32,
    threshold: u32,
) -> (XoxnoOracleClient<'static>, Address, std::vec::Vec<Address>) {
    // Owner-gated `register_feed` for the default allowlist entry needs auth;
    // tests that tighten mocks (e.g. only-owner checks) overwrite this after setup.
    env.mock_all_auths();
    let admin = Address::generate(env);
    let signers: std::vec::Vec<Address> =
        (0..signer_count).map(|_| Address::generate(env)).collect();
    let mut signers_vec = soroban_sdk::Vec::new(env);
    for s in signers.iter() {
        signers_vec.push_back(s.clone());
    }
    let contract_id = env.register(
        XoxnoOracle,
        (admin.clone(), signers_vec, threshold, TEST_RESOLUTION),
    );
    let client = XoxnoOracleClient::new(env, &contract_id);
    // Default allowlist entry so RedStone-path tests can submit without an
    // extra admin step. Custom feed ids still need `register_feed` / `add_feed`.
    client.register_feed(&feed_id(env));
    (client, admin, signers)
}

pub fn feed_id(env: &Env) -> String {
    String::from_str(env, FEED)
}

pub fn xlm_asset(env: &Env) -> ReflectorAsset {
    ReflectorAsset::Other(Symbol::new(env, "XLM"))
}

pub fn register_extra_feeds(client: &XoxnoOracleClient<'static>, env: &Env, feeds: &[&str]) {
    for feed in feeds {
        client.register_feed(&String::from_str(env, feed));
    }
}

pub fn advance_ledger_seconds(env: &Env, seconds: u64) {
    let current = env.ledger().timestamp();
    let sequence_number = env.ledger().sequence();
    env.ledger().set(LedgerInfo {
        timestamp: current + seconds,
        protocol_version: 26,
        sequence_number,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 16,
        min_persistent_entry_ttl: 16,
        max_entry_ttl: 6_312_000,
    });
}

/// `RedStonePriceData` (defined in the `common` crate) does not implement
/// `PartialEq`, so `try_read_price_data_for_feed`/`try_read_price_data`
/// results can't be compared with `assert_eq!` directly. This extracts just
/// the contract error variant for assertions.
pub fn expect_error<T>(
    result: Result<Result<T, ConversionError>, Result<Error, InvokeError>>,
) -> Error {
    match result {
        Err(Ok(e)) => e,
        Ok(_) => panic!("expected contract error, got Ok"),
        Err(Err(_)) => panic!("expected contract error, got InvokeError"),
    }
}
