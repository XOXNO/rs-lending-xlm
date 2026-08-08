//! Locks the cross-language wire format: the payload's XDR shape, the packed
//! program's byte layout, and the size budget the compact encoding buys.
//!
//! Off-chain encoders (`arb-algo` stellar-indexer, `sdk-js`) mirror these
//! bytes. Any change here is a breaking protocol change and needs a redeploy.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::xdr::{Limits, ReadXdr, ScVal, ToXdr};
use soroban_sdk::{Address, Bytes, Env, Vec as SVec};

use crate::program::VERSION;
use crate::types::{StrategyPayload, SwapHop, SwapVenue};

use super::support::{one_hop_path, path, Builder, SwapPath};

fn to_vec(bytes: &Bytes) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec![0u8; bytes.len() as usize];
    bytes.copy_into_slice(&mut out);
    out
}

#[test]
fn payload_wire_format_is_stable() {
    let env = Env::default();
    let payload = StrategyPayload {
        amounts: SVec::from_array(&env, [1i128]),
        assets: SVec::from_array(&env, [Address::generate(&env), Address::generate(&env)]),
        ops: Bytes::new(&env),
    };

    let raw = to_vec(&payload.to_xdr(&env));
    let decoded = ScVal::from_xdr(&raw, Limits::none()).expect("payload decodes as ScVal");
    let map = match decoded {
        ScVal::Map(Some(map)) => map,
        _ => panic!("payload must encode as an ScMap"),
    };

    const EXPECTED: [&str; 3] = ["amounts", "assets", "ops"];
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

/// Byte-for-byte layout of a two-hop, single-path program.
#[test]
fn program_bytes_match_the_documented_layout() {
    let env = Env::default();
    let token_in = Address::generate(&env);
    let mid = Address::generate(&env);
    let token_out = Address::generate(&env);
    let pool_one = Address::generate(&env);
    let pool_two = Address::generate(&env);

    let xdr = Builder::new(&env, token_in.clone(), token_out.clone(), 990_000, 7)
        .paths(alloc::vec![path(
            alloc::vec![
                SwapHop {
                    venue: SwapVenue::Soroswap,
                    pool: pool_one,
                    token_in: token_in.clone(),
                    token_out: mid.clone(),
                },
                SwapHop {
                    venue: SwapVenue::Phoenix,
                    pool: pool_two,
                    token_in: mid,
                    token_out: token_out.clone(),
                },
            ],
            1_000_000,
        )])
        .build();

    let decoded = ScVal::from_xdr(to_vec(&xdr), Limits::none()).expect("decodes");
    let ScVal::Map(Some(map)) = decoded else {
        panic!("expected a map")
    };
    let ScVal::Bytes(ops) = &map.0[2].val else {
        panic!("`ops` must be Bytes")
    };

    // Registry order follows first use: token_in, token_out, pool_one, mid, pool_two.
    #[rustfmt::skip]
    let expected: [u8; 20] = [
        VERSION, 0, 1, 0,        // version, token_in, token_out, min_out
        0, 0, 0, 7,              // referral id (u32 BE)
        2, 0,                    // 2 instructions, 0 weights
        0, 0, 2, 0, 3,           // Soroswap, All,  pool_one, token_in, mid
        2, 1, 4, 3, 1,           // Phoenix,  Prev, pool_two, mid,      token_out
    ];
    assert_eq!(ops.as_slice(), &expected, "packed program layout changed");
}

/// A three-way split lowers to relative weights with the final leg sweeping.
#[test]
fn splits_lower_to_relative_weights_with_a_sweeping_tail() {
    let env = Env::default();
    let token_in = Address::generate(&env);
    let token_out = Address::generate(&env);
    let pools: alloc::vec::Vec<Address> = (0..3).map(|_| Address::generate(&env)).collect();

    let paths: alloc::vec::Vec<SwapPath> = [500_000u32, 300_000, 200_000]
        .iter()
        .zip(pools)
        .map(|(&ppm, pool)| {
            one_hop_path(
                &env,
                SwapVenue::Soroswap,
                pool,
                token_in.clone(),
                token_out.clone(),
                ppm,
            )
        })
        .collect();

    let xdr = Builder::new(&env, token_in, token_out, 1, 0)
        .paths(paths)
        .build();
    let decoded = ScVal::from_xdr(to_vec(&xdr), Limits::none()).expect("decodes");
    let ScVal::Map(Some(map)) = decoded else {
        panic!("expected a map")
    };
    let ScVal::Bytes(ops) = &map.0[2].val else {
        panic!("`ops` must be Bytes")
    };
    let bytes = ops.as_slice();

    assert_eq!(bytes[8], 3, "three instructions");
    assert_eq!(bytes[9], 2, "two explicit weights, the third leg sweeps");

    let weights_at = 10 + 5 * 3;
    let weight = |i: usize| {
        let at = weights_at + 3 * i;
        u32::from_be_bytes([0, bytes[at], bytes[at + 1], bytes[at + 2]])
    };
    // 50% of the whole, then 30/50 of what is left, then everything remaining.
    assert_eq!(weight(0), 500_000);
    assert_eq!(weight(1), 600_000);
    assert_eq!(bytes[10 + 5 * 2 + 1], 0, "final leg uses All");
}

/// Guards the payload-size win against regression.
///
/// The pre-registry encoding spent a full XDR struct per hop (a 40-byte
/// `Address` for the pool and both tokens, plus symbol keys), so the same route
/// serialized to roughly 2 kB.
#[test]
fn a_three_way_two_hop_route_stays_well_under_a_kilobyte() {
    let env = Env::default();
    let token_in = Address::generate(&env);
    let token_out = Address::generate(&env);

    let paths: alloc::vec::Vec<SwapPath> = (0..3)
        .map(|_| {
            let mid = Address::generate(&env);
            path(
                alloc::vec![
                    SwapHop {
                        venue: SwapVenue::Soroswap,
                        pool: Address::generate(&env),
                        token_in: token_in.clone(),
                        token_out: mid.clone(),
                    },
                    SwapHop {
                        venue: SwapVenue::Aquarius,
                        pool: Address::generate(&env),
                        token_in: mid,
                        token_out: token_out.clone(),
                    },
                ],
                333_333,
            )
        })
        .collect();

    let len = Builder::new(&env, token_in, token_out, 1, 0)
        .paths(paths)
        .build()
        .len();
    assert!(
        len <= 640,
        "3x2-hop route serialized to {len} bytes, budget is 640"
    );
}
