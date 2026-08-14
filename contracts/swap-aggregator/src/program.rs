//! Compact instruction stream: byte layout, decoding, and structural checks.
//!
//! A strategy is a linear program over an address registry and an amount
//! registry. Every instruction is a fixed 5-byte record holding one opcode, one
//! amount selector, and three `u8` registry indices, so a hop costs 5 bytes on
//! the wire instead of a full XDR struct.
//!
//! ```text
//! header (10 bytes)
//!   [0]      version, must be VERSION
//!   [1]      token_in   -> assets[..]
//!   [2]      token_out  -> assets[..]
//!   [3]      min_out    -> amounts[..]
//!   [4..8]   referral id, u32 big-endian (0 = none)
//!   [8]      op_count
//!   [9]      weight_count
//! instructions (5 * op_count bytes)
//!   [0]      opcode      -> Opcode
//!   [1]      mode        -> Mode
//!   [2]      idx_a       pool
//!   [3]      idx_b       token_in  | lp share token
//!   [4]      idx_c       token_out | amounts index
//! weights (3 * weight_count bytes)
//!   u24 big-endian parts-per-million, each in 1..=PPM_DENOMINATOR
//! ```
//!
//! The whole blob is copied into a stack buffer with a single host call and
//! parsed in Wasm; no host object is allocated per instruction.

use soroban_sdk::{panic_with_error, Bytes, Env};

use crate::constants::PPM_DENOMINATOR;
use crate::errors::Error;
use crate::types::SwapVenue;

/// Wire version of the packed program. Bump on any layout change.
pub(crate) const VERSION: u8 = 1;

/// Header length in bytes.
const HEADER_LEN: u32 = 10;
/// Instruction record length in bytes.
const OP_LEN: u32 = 5;
/// Split-weight record length in bytes (u24 big-endian ppm).
const WEIGHT_LEN: u32 = 3;

/// Byte offsets within the header.
mod head {
    pub(super) const VERSION: usize = 0;
    pub(super) const TOKEN_IN: usize = 1;
    pub(super) const TOKEN_OUT: usize = 2;
    pub(super) const MIN_OUT: usize = 3;
    /// Start of the big-endian `u32` referral id.
    pub(super) const REFERRAL: usize = 4;
    pub(super) const OP_COUNT: usize = 8;
    pub(super) const WEIGHT_COUNT: usize = 9;
}

/// Byte offsets within one instruction record.
mod field {
    pub(super) const OPCODE: usize = 0;
    pub(super) const MODE: usize = 1;
    /// Pool address index.
    pub(super) const POOL: usize = 2;
    /// Input token index, or the LP share token for a liquidity leg.
    pub(super) const TOKEN_IN: usize = 3;
    /// Output token index, or an `amounts` index for a liquidity leg.
    pub(super) const TOKEN_OUT: usize = 4;
}

/// Upper bound on instructions per strategy. Well above what the Soroban CPU
/// budget can execute, but low enough to bound the decode buffer.
const MAX_OPS: u32 = 48;
/// Upper bound on split weights per strategy.
const MAX_WEIGHTS: u32 = 32;
/// Stack buffer sized for the largest legal program.
const MAX_PROGRAM_BYTES: usize =
    (HEADER_LEN + OP_LEN * MAX_OPS + WEIGHT_LEN * MAX_WEIGHTS) as usize;

/// Largest address registry the `u8` index space can address.
pub(crate) const MAX_ASSETS: u32 = 256;
/// Largest amount registry addressable by [`Mode::Fixed`].
pub(crate) const MAX_AMOUNTS: u32 = (MODE_PPM_BASE - MODE_FIXED_BASE) as u32;

/// `mode` byte: consume the whole vault balance of the input token.
const MODE_ALL: u8 = 0;
/// `mode` byte: consume the previous instruction's output.
const MODE_PREV: u8 = 1;
/// First `mode` byte that selects `amounts[mode - MODE_FIXED_BASE]`.
const MODE_FIXED_BASE: u8 = 2;
/// First `mode` byte that selects `weights[mode - MODE_PPM_BASE]`.
const MODE_PPM_BASE: u8 = 128;

/// What an instruction does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Opcode {
    /// Swap through `venue`: `idx_a` pool, `idx_b` token in, `idx_c` token out.
    Swap(SwapVenue),
    /// Aquarius withdraw: `idx_a` pool, `idx_b` share token, `idx_c` first
    /// index of the per-constituent floor run in `amounts`.
    Burn,
    /// Aquarius deposit: `idx_a` pool, `idx_b` share token, `idx_c` index of
    /// the minimum share count in `amounts`.
    Mint,
}

impl Opcode {
    /// Decodes an opcode byte into its variant, or `None` if unrecognized.
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Swap(SwapVenue::Soroswap)),
            1 => Some(Self::Swap(SwapVenue::Aquarius)),
            2 => Some(Self::Swap(SwapVenue::Phoenix)),
            3 => Some(Self::Swap(SwapVenue::Sushi)),
            4 => Some(Self::Swap(SwapVenue::CometDex)),
            5 => Some(Self::Burn),
            6 => Some(Self::Mint),
            _ => None,
        }
    }
}

/// How an instruction sizes its input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Entire vault balance of the input token.
    All,
    /// Exactly what the previous instruction produced.
    Prev,
    /// `amounts[idx]`, an absolute amount.
    Fixed(u8),
    /// `weights[idx]` parts-per-million of the vault balance of the input token.
    ///
    /// Measured against the balance *at execution time*, so a multi-way split
    /// is encoded as successive shares of the shrinking remainder and the final
    /// leg uses [`Mode::All`] to absorb rounding dust.
    Ppm(u8),
}

impl Mode {
    /// Decodes a mode byte into its selector variant, by value range against
    /// `MODE_ALL`, `MODE_PREV`, `MODE_FIXED_BASE`, and `MODE_PPM_BASE`.
    fn from_u8(value: u8) -> Self {
        match value {
            MODE_ALL => Self::All,
            MODE_PREV => Self::Prev,
            v if v >= MODE_PPM_BASE => Self::Ppm(v - MODE_PPM_BASE),
            v => Self::Fixed(v - MODE_FIXED_BASE),
        }
    }
}

/// One decoded instruction record.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Op {
    pub opcode: Opcode,
    pub mode: Mode,
    pub idx_a: u32,
    pub idx_b: u32,
    pub idx_c: u32,
}

/// A validated program held in a stack buffer.
///
/// Every index is range-checked against the registry lengths supplied to
/// [`Program::decode`].
pub(crate) struct Program {
    buf: [u8; MAX_PROGRAM_BYTES],
    op_count: u32,
    weights_at: u32,
    pub token_in: u32,
    pub token_out: u32,
    pub min_out: u32,
    pub referral_id: u64,
}

impl Program {
    /// Copies, parses, and structurally validates `ops` against the registry sizes, returning
    /// the decoded program.
    ///
    /// Panics with [`Error::InvalidRouteXdr`] on a malformed header, version, length, opcode, or
    /// index, and with a more specific error for other structural violations (empty/oversized
    /// batch, same-token swap, broken `Prev` chain, out-of-range split weight). Touches no
    /// external contract.
    pub(crate) fn decode(env: &Env, ops: &Bytes, assets_len: u32, amounts_len: u32) -> Self {
        if assets_len == 0 || assets_len > MAX_ASSETS || amounts_len > MAX_AMOUNTS {
            panic_with_error!(env, Error::InvalidRouteXdr);
        }

        let len = ops.len();
        if len < HEADER_LEN || len as usize > MAX_PROGRAM_BYTES {
            panic_with_error!(env, Error::InvalidRouteXdr);
        }

        let mut buf = [0u8; MAX_PROGRAM_BYTES];
        ops.copy_into_slice(&mut buf[..len as usize]);

        if buf[head::VERSION] != VERSION {
            panic_with_error!(env, Error::InvalidRouteXdr);
        }

        let op_count = buf[head::OP_COUNT] as u32;
        let weight_count = buf[head::WEIGHT_COUNT] as u32;
        if op_count == 0 || op_count > MAX_OPS || weight_count > MAX_WEIGHTS {
            panic_with_error!(env, Error::EmptyBatch);
        }
        let weights_at = HEADER_LEN + OP_LEN * op_count;
        if len != weights_at + WEIGHT_LEN * weight_count {
            panic_with_error!(env, Error::InvalidRouteXdr);
        }

        let token_in = buf[head::TOKEN_IN] as u32;
        let token_out = buf[head::TOKEN_OUT] as u32;
        let min_out = buf[head::MIN_OUT] as u32;
        if token_in >= assets_len || token_out >= assets_len || min_out >= amounts_len {
            panic_with_error!(env, Error::InvalidRouteXdr);
        }
        if token_in == token_out {
            panic_with_error!(env, Error::SameToken);
        }

        let referral = &buf[head::REFERRAL..head::REFERRAL + 4];
        let referral_id =
            u32::from_be_bytes([referral[0], referral[1], referral[2], referral[3]]) as u64;

        let program = Self {
            buf,
            op_count,
            weights_at,
            token_in,
            token_out,
            min_out,
            referral_id,
        };
        program.validate(env, assets_len, amounts_len, weight_count);
        program
    }

    /// Validates every instruction's opcode, mode, and indices before execution begins,
    /// including the `Prev` chain, same-token swaps, and split-weight bounds.
    fn validate(&self, env: &Env, assets_len: u32, amounts_len: u32, weight_count: u32) {
        for i in 0..self.op_count {
            let record = self.raw(i);
            let Some(opcode) = Opcode::from_u8(record[field::OPCODE]) else {
                panic_with_error!(env, Error::InvalidRouteXdr);
            };
            let mode = Mode::from_u8(record[field::MODE]);
            let (idx_a, idx_b, idx_c) = (
                record[field::POOL] as u32,
                record[field::TOKEN_IN] as u32,
                record[field::TOKEN_OUT] as u32,
            );

            // `Prev` is a purely structural link: the predecessor must exist,
            // must have a single output, and that output must be this
            // instruction's input. Checking it here means a broken chain never
            // reaches a venue.
            if mode == Mode::Prev {
                if i == 0 {
                    panic_with_error!(env, Error::BrokenTokenChain);
                }
                let previous = self.raw(i - 1);
                let produced = match Opcode::from_u8(previous[field::OPCODE]) {
                    // A swap produces its `token_out`, a mint its share token.
                    Some(Opcode::Swap(_)) => previous[field::TOKEN_OUT],
                    Some(Opcode::Mint) => previous[field::TOKEN_IN],
                    // A burn releases every constituent at once.
                    _ => panic_with_error!(env, Error::BrokenTokenChain),
                };
                if idx_b != produced as u32 {
                    panic_with_error!(env, Error::BrokenTokenChain);
                }
            }
            match mode {
                Mode::Fixed(idx) if idx as u32 >= amounts_len => {
                    panic_with_error!(env, Error::InvalidRouteXdr)
                }
                Mode::Ppm(idx) if idx as u32 >= weight_count => {
                    panic_with_error!(env, Error::InvalidRouteXdr)
                }
                _ => {}
            }

            if idx_a >= assets_len || idx_b >= assets_len {
                panic_with_error!(env, Error::InvalidRouteXdr);
            }
            match opcode {
                Opcode::Swap(_) => {
                    if idx_c >= assets_len {
                        panic_with_error!(env, Error::InvalidRouteXdr);
                    }
                    if idx_b == idx_c {
                        panic_with_error!(env, Error::SameToken);
                    }
                }
                // Both liquidity legs consume everything the vault holds, so a
                // sized mode would be silently ignored — reject it instead.
                Opcode::Burn | Opcode::Mint => {
                    if mode != Mode::All {
                        panic_with_error!(env, Error::InvalidRouteXdr);
                    }
                    if idx_c >= amounts_len {
                        panic_with_error!(env, Error::InvalidRouteXdr);
                    }
                }
            }
        }

        for i in 0..weight_count {
            let ppm = self.weight(i);
            if ppm == 0 {
                panic_with_error!(env, Error::ZeroSplitPpm);
            }
            if ppm > PPM_DENOMINATOR as u32 {
                panic_with_error!(env, Error::SplitPpmMismatch);
            }
        }
    }

    /// Returns the raw 5 bytes of instruction `i`.
    fn raw(&self, i: u32) -> &[u8] {
        let at = (HEADER_LEN + OP_LEN * i) as usize;
        &self.buf[at..at + OP_LEN as usize]
    }

    /// Returns the number of instructions.
    pub(crate) fn len(&self) -> u32 {
        self.op_count
    }

    /// Returns instruction `i`; callers must respect [`Program::len`].
    ///
    /// Re-decodes and re-checks the opcode byte, panicking with
    /// [`Error::InvalidRouteXdr`] if it is unrecognized, even though `validate`
    /// already checked it.
    pub(crate) fn op(&self, env: &Env, i: u32) -> Op {
        let record = self.raw(i);
        Op {
            opcode: Opcode::from_u8(record[field::OPCODE])
                .unwrap_or_else(|| panic_with_error!(env, Error::InvalidRouteXdr)),
            mode: Mode::from_u8(record[field::MODE]),
            idx_a: record[field::POOL] as u32,
            idx_b: record[field::TOKEN_IN] as u32,
            idx_c: record[field::TOKEN_OUT] as u32,
        }
    }

    /// Returns split weight `i` in parts-per-million.
    pub(crate) fn weight(&self, i: u32) -> u32 {
        let at = (self.weights_at + WEIGHT_LEN * i) as usize;
        u32::from_be_bytes([0, self.buf[at], self.buf[at + 1], self.buf[at + 2]])
    }
}

/// Serializes a program header, instructions, and weights into the packed wire
/// byte layout consumed by [`Program::decode`].
#[cfg(test)]
pub(crate) mod encode {
    use super::*;
    use soroban_sdk::Bytes;

    /// One instruction to encode.
    pub(crate) struct RawOp {
        pub opcode: u8,
        pub mode: u8,
        pub idx_a: u8,
        pub idx_b: u8,
        pub idx_c: u8,
    }

    /// Serialize header, instructions, and weights into the packed layout.
    pub(crate) fn program(
        env: &Env,
        token_in: u8,
        token_out: u8,
        min_out: u8,
        referral_id: u32,
        ops: &[RawOp],
        weights: &[u32],
    ) -> Bytes {
        let mut out = Bytes::new(env);
        out.push_back(VERSION);
        out.push_back(token_in);
        out.push_back(token_out);
        out.push_back(min_out);
        for byte in referral_id.to_be_bytes() {
            out.push_back(byte);
        }
        out.push_back(ops.len() as u8);
        out.push_back(weights.len() as u8);
        for op in ops {
            out.push_back(op.opcode);
            out.push_back(op.mode);
            out.push_back(op.idx_a);
            out.push_back(op.idx_b);
            out.push_back(op.idx_c);
        }
        for weight in weights {
            let bytes = weight.to_be_bytes();
            out.push_back(bytes[1]);
            out.push_back(bytes[2]);
            out.push_back(bytes[3]);
        }
        out
    }

    /// `mode` byte for [`Mode::All`].
    pub(crate) const ALL: u8 = MODE_ALL;
    /// `mode` byte for [`Mode::Prev`].
    pub(crate) const PREV: u8 = MODE_PREV;

    /// `mode` byte selecting `amounts[idx]`.
    pub(crate) fn fixed(idx: u8) -> u8 {
        MODE_FIXED_BASE + idx
    }

    /// `mode` byte selecting `weights[idx]`.
    pub(crate) fn ppm(idx: u8) -> u8 {
        MODE_PPM_BASE + idx
    }
}
