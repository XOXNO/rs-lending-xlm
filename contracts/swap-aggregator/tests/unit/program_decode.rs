//! Structural decoding of the packed program.
//!
//! `Program::decode` is the only thing standing between an attacker-supplied
//! byte string and a venue call, so every registry bound, length cap, and
//! instruction-level range check gets a rejecting case *and* — where the guard
//! is an inequality — the exact boundary value it must still accept.
//!
//! The tests drive `Program::decode` through a probe contract rather than
//! through `execute_strategy`: a payload can then be malformed in exactly one
//! way at a time, and the contract error it produces is observed precisely
//! (several of these guards differ only in *which* error they raise).

use soroban_sdk::{contract, contractimpl, Address, Bytes, Env};

use crate::constants::PPM_DENOMINATOR;
use crate::errors::Error;
use crate::program::encode::{self, RawOp};
use crate::program::{Program, VERSION};

/// Opcode bytes, mirroring `Opcode::from_u8`.
const OP_SWAP_SOROSWAP: u8 = 0;
const OP_BURN: u8 = 5;
const OP_MINT: u8 = 6;

/// Structural caps the wire format is built around.
const HEADER_LEN: u32 = 10;
const MAX_OPS: u32 = 48;
const MAX_WEIGHTS: u32 = 32;
const MAX_PROGRAM_BYTES: u32 = HEADER_LEN + 5 * MAX_OPS + 3 * MAX_WEIGHTS;
const MAX_ASSETS: u32 = 256;
const MAX_AMOUNTS: u32 = 126;

/// Decodes a payload and reports the header, so tests can assert on a
/// successful decode as well as on the error a rejected one raises.
#[contract]
pub struct DecodeProbe;

#[contractimpl]
impl DecodeProbe {
    /// `(op_count, token_in, token_out, min_out, referral_id)`.
    pub fn header(
        env: Env,
        ops: Bytes,
        assets_len: u32,
        amounts_len: u32,
    ) -> (u32, u32, u32, u32, u64) {
        let program = Program::decode(&env, &ops, assets_len, amounts_len);
        (
            program.len(),
            program.token_in,
            program.token_out,
            program.min_out,
            program.referral_id,
        )
    }
}

fn probe(env: &Env) -> Address {
    env.register(DecodeProbe, ())
}

/// Decode `ops`, requiring success.
fn decode(env: &Env, ops: &Bytes, assets_len: u32, amounts_len: u32) -> (u32, u32, u32, u32, u64) {
    DecodeProbeClient::new(env, &probe(env)).header(ops, &assets_len, &amounts_len)
}

/// Decode `ops`, requiring failure, and return the contract error raised.
fn decode_error(env: &Env, ops: &Bytes, assets_len: u32, amounts_len: u32) -> soroban_sdk::Error {
    DecodeProbeClient::new(env, &probe(env))
        .try_header(ops, &assets_len, &amounts_len)
        .unwrap_err()
        .unwrap()
}

/// One Soroswap swap instruction.
fn swap(mode: u8, idx_a: u8, idx_b: u8, idx_c: u8) -> RawOp {
    RawOp {
        opcode: OP_SWAP_SOROSWAP,
        mode,
        idx_a,
        idx_b,
        idx_c,
    }
}

/// The smallest well-formed program: one whole-balance swap over three assets.
fn one_swap(env: &Env) -> Bytes {
    encode::program(
        env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 0, 1, 2)],
        &[],
    )
}

// --- registry bounds (decode, line 1) ---------------------------------------

#[test]
fn decode_rejects_an_empty_asset_registry() {
    let env = Env::default();
    assert_eq!(
        decode_error(&env, &one_swap(&env), 0, 1),
        Error::InvalidRouteXdr.into()
    );
}

/// The registry is addressed by `u8`, so 256 entries is the whole index space
/// and anything larger is a caller mistake worth refusing outright.
#[test]
fn decode_rejects_an_asset_registry_past_the_index_space() {
    let env = Env::default();
    assert_eq!(
        decode_error(&env, &one_swap(&env), MAX_ASSETS + 44, 1),
        Error::InvalidRouteXdr.into()
    );
}

#[test]
fn decode_accepts_the_largest_addressable_asset_registry() {
    let env = Env::default();
    let (op_count, ..) = decode(&env, &one_swap(&env), MAX_ASSETS, 1);
    assert_eq!(op_count, 1);
}

/// `Mode::Fixed` can only name `MODE_PPM_BASE - MODE_FIXED_BASE` amounts, so a
/// longer amount registry has unreachable tail entries.
#[test]
fn decode_rejects_an_amount_registry_past_the_fixed_mode_span() {
    let env = Env::default();
    assert_eq!(
        decode_error(&env, &one_swap(&env), 3, MAX_AMOUNTS + 1),
        Error::InvalidRouteXdr.into()
    );
}

#[test]
fn decode_accepts_the_largest_fixed_addressable_amount_registry() {
    let env = Env::default();
    let (op_count, ..) = decode(&env, &one_swap(&env), 3, MAX_AMOUNTS);
    assert_eq!(op_count, 1);
}

// --- payload length (decode, line 2) ----------------------------------------

#[test]
fn decode_rejects_a_payload_shorter_than_the_header() {
    let env = Env::default();
    let mut bytes = Bytes::new(&env);
    for byte in [VERSION, 1, 2, 0, 0] {
        bytes.push_back(byte);
    }
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

/// One byte past the stack buffer. The counts in this payload would otherwise
/// raise `EmptyBatch`, which pins the rejection on the length gate rather than
/// on anything downstream of it.
#[test]
fn decode_rejects_a_payload_one_byte_past_the_stack_buffer() {
    let env = Env::default();
    let mut bytes = Bytes::new(&env);
    for byte in [VERSION, 1, 2, 0, 0, 0, 0, 0, 0, 0] {
        bytes.push_back(byte);
    }
    while bytes.len() <= MAX_PROGRAM_BYTES {
        bytes.push_back(0);
    }
    assert_eq!(bytes.len(), MAX_PROGRAM_BYTES + 1);
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

/// A program sitting on every cap at once: 48 instructions, 32 weights, and a
/// payload exactly the size of the decode buffer.
#[test]
fn decode_accepts_a_program_on_every_structural_cap() {
    let env = Env::default();
    let ops: alloc::vec::Vec<RawOp> = (0..MAX_OPS).map(|_| swap(encode::ALL, 0, 1, 2)).collect();
    let mut weights = alloc::vec![1u32; MAX_WEIGHTS as usize];
    // A split weight of exactly 1e6 routes the whole balance and is legal.
    weights[0] = PPM_DENOMINATOR as u32;

    let bytes = encode::program(&env, 1, 2, 0, 0x0102_0304, &ops, &weights);
    assert_eq!(
        bytes.len(),
        MAX_PROGRAM_BYTES,
        "the largest legal program must fill the decode buffer exactly"
    );

    assert_eq!(
        decode(&env, &bytes, 3, 1),
        (MAX_OPS, 1, 2, 0, 0x0102_0304),
        "header fields must survive the maximal payload byte for byte"
    );
}

// --- instruction and weight counts (decode, line 3) -------------------------

#[test]
fn decode_rejects_an_empty_instruction_stream() {
    let env = Env::default();
    let bytes = encode::program(&env, 1, 2, 0, 0, &[], &[]);
    assert_eq!(decode_error(&env, &bytes, 3, 1), Error::EmptyBatch.into());
}

#[test]
fn decode_rejects_more_instructions_than_the_cap() {
    let env = Env::default();
    let ops: alloc::vec::Vec<RawOp> = (0..MAX_OPS + 12)
        .map(|_| swap(encode::ALL, 0, 1, 2))
        .collect();
    let bytes = encode::program(&env, 1, 2, 0, 0, &ops, &[]);
    assert_eq!(decode_error(&env, &bytes, 3, 1), Error::EmptyBatch.into());
}

#[test]
fn decode_rejects_more_weights_than_the_cap() {
    let env = Env::default();
    let weights = alloc::vec![1u32; (MAX_WEIGHTS + 8) as usize];
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 0, 1, 2)],
        &weights,
    );
    assert_eq!(decode_error(&env, &bytes, 3, 1), Error::EmptyBatch.into());
}

// --- header indices (decode, line 4) ----------------------------------------

#[test]
fn decode_rejects_a_token_in_index_outside_the_asset_registry() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        5,
        1,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 0, 1, 2)],
        &[],
    );
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

#[test]
fn decode_rejects_a_token_out_index_outside_the_asset_registry() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        5,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 0, 1, 2)],
        &[],
    );
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

#[test]
fn decode_rejects_a_min_out_index_outside_the_amount_registry() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        2,
        5,
        0,
        &alloc::vec![swap(encode::ALL, 0, 1, 2)],
        &[],
    );
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

#[test]
fn decode_rejects_a_strategy_whose_input_and_output_token_coincide() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        1,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 0, 1, 2)],
        &[],
    );
    assert_eq!(decode_error(&env, &bytes, 3, 1), Error::SameToken.into());
}

/// The referral id is a big-endian `u32` at a fixed header offset; it must
/// round-trip byte for byte, with no sign extension into the `u64`.
#[test]
fn decode_preserves_a_full_width_referral_id() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0xFFFF_FFFF,
        &alloc::vec![swap(encode::ALL, 0, 1, 2)],
        &[],
    );
    let (.., referral_id) = decode(&env, &bytes, 3, 1);
    assert_eq!(referral_id, u32::MAX as u64);
}

// --- Mode::Prev chain (validate) --------------------------------------------

#[test]
fn validate_rejects_prev_on_the_first_instruction() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::PREV, 0, 1, 2)],
        &[],
    );
    assert_eq!(
        decode_error(&env, &bytes, 4, 1),
        Error::BrokenTokenChain.into()
    );
}

/// A `Prev` instruction chains off the *immediately preceding* record. The
/// third instruction produces a different token on purpose, so linking to the
/// wrong neighbour in either direction breaks the chain.
#[test]
fn validate_accepts_a_prev_link_onto_the_preceding_swap() {
    let env = Env::default();
    let ops = alloc::vec![
        swap(encode::ALL, 0, 1, 2),
        swap(encode::PREV, 0, 2, 1),
        swap(encode::ALL, 0, 1, 3),
    ];
    let bytes = encode::program(&env, 1, 2, 0, 0, &ops, &[]);
    let (op_count, ..) = decode(&env, &bytes, 4, 1);
    assert_eq!(op_count, 3);
}

#[test]
fn validate_rejects_a_prev_link_naming_a_token_the_predecessor_never_produced() {
    let env = Env::default();
    let ops = alloc::vec![
        swap(encode::ALL, 0, 1, 2),
        // The predecessor produced asset 2, not asset 3.
        swap(encode::PREV, 0, 3, 1),
    ];
    let bytes = encode::program(&env, 1, 2, 0, 0, &ops, &[]);
    assert_eq!(
        decode_error(&env, &bytes, 4, 1),
        Error::BrokenTokenChain.into()
    );
}

/// A mint produces its share token, so a following `Prev` may chain onto it.
#[test]
fn validate_accepts_a_prev_link_onto_a_preceding_mint() {
    let env = Env::default();
    let ops = alloc::vec![
        RawOp {
            opcode: OP_MINT,
            mode: encode::ALL,
            idx_a: 0,
            idx_b: 1,
            idx_c: 0,
        },
        swap(encode::PREV, 0, 1, 2),
    ];
    let bytes = encode::program(&env, 1, 2, 0, 0, &ops, &[]);
    let (op_count, ..) = decode(&env, &bytes, 3, 1);
    assert_eq!(op_count, 2);
}

/// A burn releases every constituent at once, so there is no single output to
/// chain onto.
#[test]
fn validate_rejects_a_prev_link_onto_a_preceding_burn() {
    let env = Env::default();
    let ops = alloc::vec![
        RawOp {
            opcode: OP_BURN,
            mode: encode::ALL,
            idx_a: 0,
            idx_b: 1,
            idx_c: 0,
        },
        swap(encode::PREV, 0, 1, 2),
    ];
    let bytes = encode::program(&env, 1, 2, 0, 0, &ops, &[]);
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::BrokenTokenChain.into()
    );
}

// --- Mode::Fixed / Mode::Ppm selectors (validate) ---------------------------

/// `mode` byte 2 is `Fixed(0)`, the first amount slot — the base offset is a
/// subtraction, and any other arithmetic shifts the whole selector space.
#[test]
fn validate_accepts_the_first_fixed_amount_selector() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::fixed(0), 0, 1, 2)],
        &[],
    );
    let (op_count, ..) = decode(&env, &bytes, 3, 1);
    assert_eq!(op_count, 1);
}

#[test]
fn validate_rejects_a_fixed_amount_selector_outside_the_registry() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::fixed(5), 0, 1, 2)],
        &[],
    );
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

#[test]
fn validate_accepts_a_ppm_selector_inside_the_weight_table() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::ppm(0), 0, 1, 2)],
        &[500_000],
    );
    let (op_count, ..) = decode(&env, &bytes, 3, 1);
    assert_eq!(op_count, 1);
}

#[test]
fn validate_rejects_a_ppm_selector_outside_the_weight_table() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::ppm(1), 0, 1, 2)],
        &[500_000],
    );
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

// --- instruction indices (validate) -----------------------------------------

#[test]
fn validate_rejects_a_pool_index_outside_the_asset_registry() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 5, 1, 2)],
        &[],
    );
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

#[test]
fn validate_rejects_an_input_token_index_outside_the_asset_registry() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 0, 5, 2)],
        &[],
    );
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

#[test]
fn validate_rejects_an_output_token_index_outside_the_asset_registry() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 0, 1, 5)],
        &[],
    );
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

#[test]
fn validate_rejects_a_swap_between_one_token_and_itself() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 0, 1, 1)],
        &[],
    );
    assert_eq!(decode_error(&env, &bytes, 3, 1), Error::SameToken.into());
}

#[test]
fn validate_rejects_an_unknown_opcode() {
    let env = Env::default();
    let ops = alloc::vec![RawOp {
        opcode: 7,
        mode: encode::ALL,
        idx_a: 0,
        idx_b: 1,
        idx_c: 2,
    }];
    let bytes = encode::program(&env, 1, 2, 0, 0, &ops, &[]);
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

/// Both liquidity legs consume the whole vault balance, so a sized mode would
/// be silently ignored.
#[test]
fn validate_rejects_a_sized_mode_on_a_liquidity_leg() {
    let env = Env::default();
    let ops = alloc::vec![RawOp {
        opcode: OP_MINT,
        mode: encode::fixed(0),
        idx_a: 0,
        idx_b: 1,
        idx_c: 0,
    }];
    let bytes = encode::program(&env, 1, 2, 0, 0, &ops, &[]);
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

// --- split weights (validate) -----------------------------------------------

#[test]
fn validate_rejects_a_zero_split_weight() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 0, 1, 2)],
        &[0],
    );
    assert_eq!(decode_error(&env, &bytes, 3, 1), Error::ZeroSplitPpm.into());
}

#[test]
fn validate_rejects_a_split_weight_one_part_over_the_denominator() {
    let env = Env::default();
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 0, 1, 2)],
        &[PPM_DENOMINATOR as u32 + 1],
    );
    assert_eq!(
        decode_error(&env, &bytes, 3, 1),
        Error::SplitPpmMismatch.into()
    );
}
