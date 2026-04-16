//! Bootstrap helpers for contract-level libFuzzer targets.
//!
//! Contract-level (Env-based) fuzzing requires instantiating a `LendingTest`
//! and running one or more operations. We expose a single minimal helper so
//! targets stay tiny and focused on their scenario.

pub use test_harness::{usdc_preset, eth_preset, LendingTest, ALICE};

/// Build a minimal two-market (USDC + ETH) lending context for fuzz targets.
pub fn build_min_context() -> LendingTest {
    LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build()
}
