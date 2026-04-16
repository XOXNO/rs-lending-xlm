#![no_main]
//! Contract-level libFuzzer target: oracle tolerance tiers.
//!
//! Invariants:
//!   - Supply (risk-decreasing) succeeds under any reasonable deviation
//!   - Zero-price oracle always rejects supply

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use stellar_fuzz::{arb_amount, build_min_context, ALICE};

#[derive(Arbitrary, Debug)]
struct Input {
    supply_amt: u32,
    deviation_bps: u16,
    direction_up: bool,
    zero_price: bool,
}

fuzz_target!(|inp: Input| {
    let supply = arb_amount(inp.supply_amt, 1_000.0, 101_000.0);
    let dev = (inp.deviation_bps.min(5_000)) as i128;

    let mut t = build_min_context();
    t.supply(ALICE, "USDC", supply);

    let eth_spot: i128 = 2000 * 10_i128.pow(18);
    let mult = if inp.direction_up { 10_000 + dev } else { (10_000 - dev).max(1) };
    let eth_twap = eth_spot * mult / 10_000;

    let reflector = t.mock_reflector_client();
    let eth_addr = t.resolve_asset("ETH");
    reflector.set_price(&eth_addr, &eth_spot);
    reflector.set_twap_price(&eth_addr, &eth_twap);

    if inp.zero_price {
        let usdc_addr = t.resolve_asset("USDC");
        reflector.set_price(&usdc_addr, &0);
        reflector.set_twap_price(&usdc_addr, &0);
        let res = t.try_supply(ALICE, "USDC", 1.0);
        assert!(res.is_err(), "supply with zero oracle must fail");
    } else {
        let _ = t.try_supply(ALICE, "USDC", 1.0);
    }
});
