//! LendingTest context builders for libFuzzer protocol targets.

pub use test_harness::{eth_preset, usdc_preset, xlm_preset, LendingTest};

pub fn build_min_context() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build()
}

pub fn build_wide_context() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(xlm_preset())
        .build()
}
