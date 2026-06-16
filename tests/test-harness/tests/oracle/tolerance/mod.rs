use soroban_sdk::Address;
use test_harness::LendingTest;

pub(super) fn setup() -> LendingTest {
    LendingTest::new().standard_two_asset_dust_disabled()
}

pub(super) fn enable_dual_source(t: &LendingTest, asset_name: &str) {
    t.enable_dual_source_oracle(asset_name);
}

pub(super) fn set_dual_oracle_dex(t: &LendingTest, asset_name: &str, dex_oracle: Address) {
    t.set_dual_oracle_dex_anchor(asset_name, dex_oracle);
}

mod bands;
mod config;
mod dual_source;
mod edge;
mod staleness;
