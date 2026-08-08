//! Payload construction for tests.
//!
//! Tests describe a strategy the way the off-chain router thinks about it —
//! paths with *absolute* split weights, plus optional LP legs — and this module
//! lowers that into the packed program the contract actually decodes. It is the
//! executable reference for the off-chain encoders; keep it in step with
//! `crate::program`.

use alloc::vec::Vec as AllocVec;

use soroban_sdk::{token, xdr::ToXdr, Address, Bytes, Env, Vec};

use crate::constants::PPM_DENOMINATOR;
use crate::program::encode::{self, RawOp};
use crate::types::{StrategyPayload, SwapHop, SwapVenue};

/// Opcode byte for each venue's swap instruction.
fn venue_opcode(venue: SwapVenue) -> u8 {
    match venue {
        SwapVenue::Soroswap => 0,
        SwapVenue::Aquarius => 1,
        SwapVenue::Phoenix => 2,
        SwapVenue::Sushi => 3,
        SwapVenue::CometDex => 4,
    }
}

const OP_BURN: u8 = 5;
const OP_MINT: u8 = 6;

/// A route leg with an absolute share of its input token group.
pub(crate) struct SwapPath {
    pub hops: AllocVec<SwapHop>,
    pub split_ppm: u32,
}

pub(crate) fn new_asset<'a>(
    env: &'a Env,
    admin: &Address,
) -> (Address, token::StellarAssetClient<'a>) {
    let contract = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = contract.address();
    let sac_admin = token::StellarAssetClient::new(env, &addr);
    (addr, sac_admin)
}

pub(crate) fn one_hop_path(
    _env: &Env,
    venue: SwapVenue,
    pool: Address,
    token_in: Address,
    token_out: Address,
    split_ppm: u32,
) -> SwapPath {
    SwapPath {
        split_ppm,
        hops: alloc::vec![SwapHop {
            venue,
            pool,
            token_in,
            token_out,
        }],
    }
}

/// Build a multi-hop path from an explicit hop list.
pub(crate) fn path(hops: AllocVec<SwapHop>, split_ppm: u32) -> SwapPath {
    SwapPath { hops, split_ppm }
}

pub(crate) fn strategy_xdr(
    env: &Env,
    token_in: Address,
    token_out: Address,
    total_min_out: i128,
    paths: AllocVec<SwapPath>,
) -> Bytes {
    strategy_xdr_with_referral(env, token_in, token_out, total_min_out, paths, 0)
}

pub(crate) fn strategy_xdr_with_referral(
    env: &Env,
    token_in: Address,
    token_out: Address,
    total_min_out: i128,
    paths: AllocVec<SwapPath>,
    referral_id: u64,
) -> Bytes {
    Builder::new(env, token_in, token_out, total_min_out, referral_id)
        .paths(paths)
        .build()
}

/// Full strategy with optional burn and mint legs.
///
/// `pre_swap_amount > 0` emits an ordinary Aquarius swap against the mint pool
/// just before the deposit — the contract has no dedicated pre-swap step.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lp_strategy_xdr(
    env: &Env,
    token_in: Address,
    token_out: Address,
    total_min_out: i128,
    paths: AllocVec<SwapPath>,
    burn_pool: Option<Address>,
    burn_min_amounts: AllocVec<i128>,
    mint_pool: Option<Address>,
    mint_min_shares: i128,
    pre_swap_amount: i128,
    pre_swap_from_a: bool,
) -> Bytes {
    let mut builder = Builder::new(env, token_in.clone(), token_out.clone(), total_min_out, 0);
    if let Some(pool) = burn_pool {
        builder = builder.burn(pool, token_in, burn_min_amounts);
    }
    builder = builder.paths(paths);
    if let Some(pool) = mint_pool {
        builder = builder.mint(
            pool,
            token_out,
            mint_min_shares,
            pre_swap_amount,
            pre_swap_from_a,
        );
    }
    builder.build()
}

/// Accumulates registries and instructions, then serializes the payload.
pub(crate) struct Builder<'a> {
    env: &'a Env,
    assets: AllocVec<Address>,
    amounts: AllocVec<i128>,
    ops: AllocVec<RawOp>,
    weights: AllocVec<u32>,
    token_in: u8,
    token_out: u8,
    referral_id: u32,
    /// Deferred mint leg, emitted after the path instructions.
    pending_mint: Option<(Address, Address, i128, i128, bool)>,
}

impl<'a> Builder<'a> {
    pub(crate) fn new(
        env: &'a Env,
        token_in: Address,
        token_out: Address,
        total_min_out: i128,
        referral_id: u64,
    ) -> Self {
        let mut builder = Self {
            env,
            assets: AllocVec::new(),
            // Slot 0 is the strategy-wide minimum output.
            amounts: alloc::vec![total_min_out],
            ops: AllocVec::new(),
            weights: AllocVec::new(),
            token_in: 0,
            token_out: 0,
            referral_id: referral_id as u32,
            pending_mint: None,
        };
        builder.token_in = builder.asset(&token_in);
        builder.token_out = builder.asset(&token_out);
        builder
    }

    /// Index of `address` in the registry, appending it if new.
    fn asset(&mut self, address: &Address) -> u8 {
        if let Some(idx) = self.assets.iter().position(|a| a == address) {
            return idx as u8;
        }
        self.assets.push(address.clone());
        (self.assets.len() - 1) as u8
    }

    /// Index of a newly appended amount.
    fn amount(&mut self, value: i128) -> u8 {
        self.amounts.push(value);
        (self.amounts.len() - 1) as u8
    }

    /// Index of a newly appended split weight.
    fn weight(&mut self, ppm: u32) -> u8 {
        self.weights.push(ppm);
        (self.weights.len() - 1) as u8
    }

    /// Emit an LP burn that consumes the whole vault balance of `lp_token`.
    pub(crate) fn burn(
        mut self,
        pool: Address,
        lp_token: Address,
        min_amounts: AllocVec<i128>,
    ) -> Self {
        let idx_a = self.asset(&pool);
        let idx_b = self.asset(&lp_token);
        let start = self.amounts.len() as u8;
        for min in min_amounts {
            self.amount(min);
        }
        self.ops.push(RawOp {
            opcode: OP_BURN,
            mode: encode::ALL,
            idx_a,
            idx_b,
            idx_c: start,
        });
        self
    }

    /// Record the mint leg; it is emitted last, after every path instruction.
    pub(crate) fn mint(
        mut self,
        pool: Address,
        lp_token: Address,
        min_shares: i128,
        pre_swap_amount: i128,
        pre_swap_from_a: bool,
    ) -> Self {
        self.pending_mint = Some((pool, lp_token, min_shares, pre_swap_amount, pre_swap_from_a));
        self
    }

    /// Lower absolute-weight paths into instructions, one token group at a time.
    ///
    /// A path with no hops contributes nothing: it emits no instruction and
    /// claims no weight, so its share is left unrouted in the vault and the
    /// contract's residual guard rejects the strategy.
    pub(crate) fn paths(mut self, paths: AllocVec<SwapPath>) -> Self {
        let paths: AllocVec<SwapPath> = paths.into_iter().filter(|p| !p.hops.is_empty()).collect();
        let mut done = alloc::vec![false; paths.len()];
        for i in 0..paths.len() {
            if done[i] {
                continue;
            }
            let group_token = paths[i].hops[0].token_in.clone();
            let members: AllocVec<usize> = (i..paths.len())
                .filter(|&j| paths[j].hops[0].token_in == group_token)
                .collect();
            let total: u32 = members.iter().map(|&j| paths[j].split_ppm).sum();
            // A group that routes its whole balance lets the final leg sweep the
            // remainder, so ppm rounding never strands dust.
            let sweeps = total == PPM_DENOMINATOR as u32;

            let mut remaining = PPM_DENOMINATOR as u32;
            for (n, &j) in members.iter().enumerate() {
                done[j] = true;
                let path = &paths[j];
                let last = n + 1 == members.len();
                let mode = if last && sweeps {
                    encode::ALL
                } else {
                    // Weights are relative to what the earlier legs left behind.
                    let relative = if remaining == 0 {
                        0
                    } else {
                        ((path.split_ppm as u64 * PPM_DENOMINATOR as u64) / remaining as u64) as u32
                    };
                    remaining = remaining.saturating_sub(path.split_ppm);
                    let idx = self.weight(relative);
                    encode::ppm(idx)
                };
                self.emit_path(path, mode);
            }
        }
        self
    }

    /// Emit one path: the head instruction sizes the input, the rest chain.
    fn emit_path(&mut self, path: &SwapPath, head_mode: u8) {
        for (n, hop) in path.hops.iter().enumerate() {
            let idx_a = self.asset(&hop.pool);
            let idx_b = self.asset(&hop.token_in);
            let idx_c = self.asset(&hop.token_out);
            self.ops.push(RawOp {
                opcode: venue_opcode(hop.venue),
                mode: if n == 0 { head_mode } else { encode::PREV },
                idx_a,
                idx_b,
                idx_c,
            });
        }
    }

    /// Serialize registries and program into `execute_strategy` bytes.
    pub(crate) fn build(mut self) -> Bytes {
        if let Some((pool, lp_token, min_shares, pre_swap_amount, pre_swap_from_a)) =
            self.pending_mint.take()
        {
            if pre_swap_amount > 0 {
                self.emit_pre_swap(&pool, pre_swap_amount, pre_swap_from_a);
            }
            let idx_a = self.asset(&pool);
            let idx_b = self.asset(&lp_token);
            let idx_c = self.amount(min_shares);
            self.ops.push(RawOp {
                opcode: OP_MINT,
                mode: encode::ALL,
                idx_a,
                idx_b,
                idx_c,
            });
        }

        let ops = encode::program(
            self.env,
            self.token_in,
            self.token_out,
            0,
            self.referral_id,
            &self.ops,
            &self.weights,
        );

        let mut assets = Vec::new(self.env);
        for asset in &self.assets {
            assets.push_back(asset.clone());
        }
        let mut amounts = Vec::new(self.env);
        for amount in &self.amounts {
            amounts.push_back(*amount);
        }

        StrategyPayload {
            amounts,
            assets,
            ops,
        }
        .to_xdr(self.env)
    }

    /// Emit the mint-balancing swap against the pool's own book.
    fn emit_pre_swap(&mut self, pool: &Address, amount: i128, from_a: bool) {
        let tokens: Vec<Address> = self.env.invoke_contract(
            pool,
            &soroban_sdk::Symbol::new(self.env, "get_tokens"),
            Vec::new(self.env),
        );
        let (token_in, token_out) = if from_a {
            (tokens.get_unchecked(0), tokens.get_unchecked(1))
        } else {
            (tokens.get_unchecked(1), tokens.get_unchecked(0))
        };
        let idx_a = self.asset(pool);
        let idx_b = self.asset(&token_in);
        let idx_c = self.asset(&token_out);
        let amount_idx = self.amount(amount);
        self.ops.push(RawOp {
            opcode: venue_opcode(SwapVenue::Aquarius),
            mode: encode::fixed(amount_idx),
            idx_a,
            idx_b,
            idx_c,
        });
    }
}
