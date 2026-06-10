//! Repro for the live-testnet supply trap: pool-as-wasm `supply` after the
//! ledger clock moves to wall-clock scale (testnet timestamps ≈ 1.78e9 s).

extern crate std;

use test_harness::{usdc_preset, LendingTest, ALICE};

#[test]
fn repro_supply_at_wall_clock_timestamp() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();
    // Jump from the builder's t=1000 to real wall-clock scale, as on testnet.
    t.advance_time(1_760_000_000);
    t.supply(ALICE, "USDC", 10_000.0);
}
