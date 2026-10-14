//! Structural decoding of the packed program.
//!
//! `Program::decode` validates attacker-supplied bytes before any venue call.
//! Each registry bound, length cap, and instruction range check has a rejecting
//! case and, where the guard is an inequality, an accepted boundary case.
//!
//! The tests call `Program::decode` through a probe contract, not
//! `execute_strategy`, so each payload is malformed in one way only and the
//! exact contract error is observed.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, Address, Bytes, Env, Vec};

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
        assets: Vec<Address>,
        amounts_len: u32,
    ) -> (u32, u32, u32, u32, u64) {
        let program = Program::decode(&env, &ops, &assets, amounts_len);
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

/// An address registry of `len` distinct addresses.
fn registry(env: &Env, len: u32) -> Vec<Address> {
    let mut assets = Vec::new(env);
    for _ in 0..len {
        assets.push_back(Address::generate(env));
    }
    assets
}

/// Decode `ops` over `assets_len` distinct addresses, requiring success.
fn decode(env: &Env, ops: &Bytes, assets_len: u32, amounts_len: u32) -> (u32, u32, u32, u32, u64) {
    DecodeProbeClient::new(env, &probe(env)).header(ops, &registry(env, assets_len), &amounts_len)
}

/// Decode `ops` over `assets_len` distinct addresses, requiring failure, and
/// return the contract error raised.
fn decode_error(env: &Env, ops: &Bytes, assets_len: u32, amounts_len: u32) -> soroban_sdk::Error {
    decode_error_over(env, ops, &registry(env, assets_len), amounts_len)
}

/// Decode `ops` over the given registry, requiring failure, and return the
/// contract error raised.
fn decode_error_over(
    env: &Env,
    ops: &Bytes,
    assets: &Vec<Address>,
    amounts_len: u32,
) -> soroban_sdk::Error {
    DecodeProbeClient::new(env, &probe(env))
        .try_header(ops, assets, &amounts_len)
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

// --- registry bounds (decode) -----------------------------------------------

#[test]
fn decode_rejects_an_empty_asset_registry() {
    let env = Env::default();
    assert_eq!(
        decode_error(&env, &one_swap(&env), 0, 1),
        Error::InvalidRouteXdr.into()
    );
}

/// A `u8` index addresses at most 256 assets, so a larger registry is rejected.
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

// --- payload length (decode) ------------------------------------------------

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

// --- instruction and weight counts (decode) ---------------------------------

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

// --- header indices (decode) ------------------------------------------------

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

/// R-B fix (router_adapter): the header check compares addresses. A registry
/// that lists the input token twice cannot declare the input as the output
/// through a second index; before the fix this header decoded.
#[test]
fn decode_rejects_distinct_header_indices_naming_the_same_token() {
    let env = Env::default();
    let assets = registry(&env, 3);
    let mut duplicated = assets.clone();
    duplicated.push_back(assets.get_unchecked(1)); // index 3 repeats index 1
    let bytes = encode::program(
        &env,
        1,
        3,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 0, 1, 2)],
        &[],
    );
    assert_eq!(
        decode_error_over(&env, &bytes, &duplicated, 1),
        Error::SameToken.into()
    );
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

/// `mode` byte 2 decodes to `Fixed(0)`, the first amount slot.
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

/// R-B fix (router_adapter): the per-swap check compares addresses too, so a
/// swap from a token into a second registry entry for the same token is
/// refused at decode, before any earlier instruction runs.
#[test]
fn validate_rejects_a_swap_whose_distinct_indices_name_the_same_token() {
    let env = Env::default();
    let assets = registry(&env, 3);
    let mut duplicated = assets.clone();
    duplicated.push_back(assets.get_unchecked(1)); // index 3 repeats index 1
    let bytes = encode::program(
        &env,
        1,
        2,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 0, 1, 3)],
        &[],
    );
    assert_eq!(
        decode_error_over(&env, &bytes, &duplicated, 1),
        Error::SameToken.into()
    );
}

/// Skeptic (router_adapter, R-B), over-refusal guard: the address comparison
/// applies to each swap's own endpoints and to the strategy endpoints only. A
/// route that revisits the input token mid-route through a duplicate entry
/// (A -> B -> A' -> C, A' a second entry for A) has distinct endpoints at every
/// check and must still decode.
#[test]
fn skeptic_rb_decode_accepts_a_route_that_revisits_the_input_through_a_duplicate_index() {
    let env = Env::default();
    let base = registry(&env, 4); // [A, B, C, pool]
    let mut assets = base.clone();
    assets.push_back(base.get_unchecked(0)); // index 4 repeats A
    let bytes = encode::program(
        &env,
        0,
        2,
        0,
        0,
        &alloc::vec![
            swap(encode::ALL, 3, 0, 1),
            swap(encode::PREV, 3, 1, 4),
            swap(encode::PREV, 3, 4, 2),
        ],
        &[],
    );
    let (len, token_in, token_out, _, _) =
        DecodeProbeClient::new(&env, &probe(&env)).header(&bytes, &assets, &1);
    assert_eq!((len, token_in, token_out), (3, 0, 2));
}

/// Skeptic (router_adapter, R-B): the drafter keeps the `Prev` link on index
/// identity and calls it stricter. Pin that claim: a `Prev` that names the
/// previous output through a second entry for the same address is refused
/// with `BrokenTokenChain` at decode, never accepted by address.
#[test]
fn skeptic_rb_prev_through_a_duplicate_entry_of_the_previous_output_is_refused() {
    let env = Env::default();
    let base = registry(&env, 3); // [A, B, pool]
    let mut assets = base.clone();
    assets.push_back(base.get_unchecked(1)); // index 3 repeats B
    assets.push_back(Address::generate(&env)); // index 4: C
    let bytes = encode::program(
        &env,
        0,
        4,
        0,
        0,
        &alloc::vec![swap(encode::ALL, 2, 0, 1), swap(encode::PREV, 2, 3, 4)],
        &[],
    );
    assert_eq!(
        decode_error_over(&env, &bytes, &assets, 1),
        Error::BrokenTokenChain.into()
    );
}

/// Skeptic (router_adapter, R-B): every swap venue shares the per-swap check.
/// A duplicate-entry same-token swap is refused for each swap opcode, not
/// only the Soroswap one the drafter's test encodes, and on a later
/// instruction as well as the first.
#[test]
fn skeptic_rb_every_swap_opcode_refuses_a_duplicate_entry_same_token_swap() {
    let env = Env::default();
    let base = registry(&env, 4); // [A, B, C, pool]
    let mut assets = base.clone();
    assets.push_back(base.get_unchecked(1)); // index 4 repeats B
    for opcode in 0u8..=4 {
        let bytes = encode::program(
            &env,
            0,
            2,
            0,
            0,
            &alloc::vec![
                swap(encode::ALL, 3, 0, 1),
                RawOp {
                    opcode,
                    mode: encode::PREV,
                    idx_a: 3,
                    idx_b: 1,
                    idx_c: 4,
                },
                swap(encode::ALL, 3, 1, 2),
            ],
            &[],
        );
        assert_eq!(
            decode_error_over(&env, &bytes, &assets, 1),
            Error::SameToken.into(),
            "opcode {opcode}"
        );
    }
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

// --- header self-consistency ------------------------------------------------

/// A payload with another wire version is rejected, not decoded under the
/// current field layout.
#[test]
fn decode_rejects_a_payload_from_another_wire_version() {
    let env = Env::default();
    let good = one_swap(&env);

    let mut raw = [0u8; MAX_PROGRAM_BYTES as usize];
    let len = good.len();
    good.copy_into_slice(&mut raw[..len as usize]);
    raw[0] = VERSION.wrapping_add(1); // head::VERSION
    let bumped = Bytes::from_slice(&env, &raw[..len as usize]);

    assert_eq!(
        decode_error(&env, &bumped, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

/// The payload must end exactly where `op_count` and `weight_count` put the end
/// of the weight run; one trailing byte is rejected.
#[test]
fn decode_rejects_a_payload_with_bytes_past_its_declared_weight_run() {
    let env = Env::default();
    let mut padded = one_swap(&env);
    padded.push_back(0);

    assert_eq!(
        decode_error(&env, &padded, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

/// The counterpart: a header that claims a weight run the payload is too short
/// to contain.
#[test]
fn decode_rejects_a_header_claiming_more_weights_than_the_payload_carries() {
    let env = Env::default();
    let good = one_swap(&env);

    let mut raw = [0u8; MAX_PROGRAM_BYTES as usize];
    let len = good.len();
    good.copy_into_slice(&mut raw[..len as usize]);
    raw[9] = 2; // head::WEIGHT_COUNT, with no weight bytes appended
    let lying = Bytes::from_slice(&env, &raw[..len as usize]);

    assert_eq!(
        decode_error(&env, &lying, 3, 1),
        Error::InvalidRouteXdr.into()
    );
}

// --- liquidity legs are whole-balance only ----------------------------------

/// Burn and Mint both reject a sized mode.
#[test]
fn decode_rejects_a_sized_mode_on_a_liquidity_leg() {
    let env = Env::default();
    for opcode in [OP_BURN, OP_MINT] {
        let sized = encode::program(
            &env,
            1,
            2,
            0,
            0,
            &alloc::vec![RawOp {
                opcode,
                mode: encode::fixed(0),
                idx_a: 0,
                idx_b: 1,
                idx_c: 0,
            }],
            &[],
        );
        assert_eq!(
            decode_error(&env, &sized, 3, 1),
            Error::InvalidRouteXdr.into(),
            "opcode {opcode} must reject a sized mode"
        );
    }
}

/// A liquidity leg's `idx_c` indexes the amount registry and must be inside it.
#[test]
fn decode_rejects_a_liquidity_leg_addressing_a_missing_amount_slot() {
    let env = Env::default();
    for opcode in [OP_BURN, OP_MINT] {
        let out_of_range = encode::program(
            &env,
            1,
            2,
            0,
            0,
            &alloc::vec![RawOp {
                opcode,
                mode: encode::ALL,
                idx_a: 0,
                idx_b: 1,
                idx_c: 4,
            }],
            &[],
        );
        assert_eq!(
            decode_error(&env, &out_of_range, 3, 2),
            Error::InvalidRouteXdr.into(),
            "opcode {opcode} must reject idx_c past the amount registry"
        );
    }
}
