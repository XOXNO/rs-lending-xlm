extern crate std;

use test_harness::{usdc_preset, LendingTest, ALICE};

#[test]
fn repro_supply_at_wall_clock_timestamp() {
    let mut t = LendingTest::new().with_market(usdc_preset()).build();

    t.advance_time(1_760_000_000);
    t.supply(ALICE, "USDC", 10_000.0);
}
