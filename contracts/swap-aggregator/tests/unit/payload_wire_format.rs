use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::{Limits, ReadXdr, ScVal, ToXdr};
use soroban_sdk::{Address, Env, Vec as SVec};

use crate::types::StrategyPayload;

/// Pins the `StrategyPayload` wire format.
///
/// Off-chain encoders build this `ScMap` by hand and Soroban requires map keys
/// in sorted order, so a producer that emits a different order — or omits a
/// field — fails to decode. Adding or renaming a field is a breaking change for
/// every producer, and this test is where that shows up.
#[test]
fn payload_wire_format_is_stable() {
    let env = Env::default();
    let payload = StrategyPayload {
        burn_pool: None,
        burn_min_amounts: SVec::new(&env),
        mint_pool: None,
        mint_min_shares: 0,
        paths: SVec::new(&env),
        pre_balance_fee_bps: 0,
        referral_id: 0,
        token_in: Address::generate(&env),
        token_out: Address::generate(&env),
        total_min_out: 0,
    };

    let xdr = payload.to_xdr(&env);
    let len = xdr.len() as usize;
    let mut buf = [0u8; 512];
    assert!(len <= buf.len(), "payload larger than scratch buffer");
    for i in 0..xdr.len() {
        buf[i as usize] = xdr.get(i).unwrap();
    }

    let decoded = ScVal::from_xdr(&buf[..len], Limits::none()).expect("payload decodes as ScVal");
    let map = match decoded {
        ScVal::Map(Some(map)) => map,
        _ => panic!("payload must encode as an ScMap"),
    };

    const EXPECTED: [&str; 10] = [
        "burn_min_amounts",
        "burn_pool",
        "mint_min_shares",
        "mint_pool",
        "paths",
        "pre_balance_fee_bps",
        "referral_id",
        "token_in",
        "token_out",
        "total_min_out",
    ];

    assert_eq!(map.0.len(), EXPECTED.len(), "payload field count changed");
    for (entry, expected) in map.0.iter().zip(EXPECTED.iter()) {
        let key = match &entry.key {
            ScVal::Symbol(symbol) => symbol,
            _ => panic!("map key must be a symbol"),
        };
        assert_eq!(
            key.to_utf8_string_lossy().as_str(),
            *expected,
            "payload field order changed"
        );
    }
}
