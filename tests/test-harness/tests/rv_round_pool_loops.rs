//! Rounding audit, lens R-6: pool share boundaries under loops.
//!
//! The model applies the pool's own resolvers (`calculate_scaled_supply`,
//! `resolve_withdrawal`, `calculate_scaled_borrow`, `resolve_repay`,
//! `resolve_net_settle`) exactly as `contracts/pool/src/ops/{supply,withdraw,
//! borrow,repay,net_settle}.rs` apply them, and keeps the user's exact wealth in
//! `BigInt` at RAY x 10^(27-d) resolution, where one native unit is
//! `unit = RAY * 10^(27-d)`:
//!
//! ```text
//! W = (tokens_out - tokens_in) * unit + supply_shares * SI - debt_shares * BI
//! ```
//!
//! Documented bound (formulas.md "Shares and token amounts", ADR-0003): each
//! boundary rounds against the user by strictly less than one native unit, so
//! W never rises; a share mint or partial burn costs at most `index - 1`, a full
//! close at most `unit - 1`, and a net settlement touches one boundary per side.
//! Pool side: floored claims never exceed cash plus ceiled debt (INV-ACCT-04),
//! and shares are conserved.
//!
//! Sweeps: decimals 0, 7 and 18; supply index from the 10^24 floor to the 10^36
//! ceiling, borrow index from RAY to the ceiling; one-unit and odd amounts;
//! repeated stateless cycles, 3,000 random index pairs per decimal, and
//! 2,000-step one-unit chains. A cross-check drives the deployed pool WASM at
//! injected indexes and compares books, mutations and token balances with the
//! model after every call.

use common::constants::{MAX_SUPPLY_INDEX_RAY, RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::math::fp::Ray;
use common::rates::{
    calculate_scaled_borrow, calculate_scaled_supply, resolve_net_settle, resolve_repay,
    resolve_withdrawal, unscale_borrow_ceil, unscale_supply, unscale_supply_floor,
};
use common::types::{
    HubAssetKey, InterestRateModel, PoolAction, PoolBorrowEntry, PoolKey, PoolNetSettleEntry,
    PoolStateRaw, PoolSupplyEntry, PoolWithdrawEntry, ScaledPositionRaw,
};
use num_bigint::BigInt;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env};
use test_harness::{
    hub_asset, usd, LendingTest, MarketPreset, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS,
};

const DECIMALS: [u32; 3] = [0, 7, 18];
const AMOUNTS: [i128; 9] = [1, 2, 3, 7, 13, 99, 101, 999, 12_345];
const WHALE: i128 = 1_000_000;

/// Supply indexes from the bad-debt floor (RAY / 1000) to the ceiling (10^36).
const SUPPLY_INDEXES: [i128; 12] = [
    SUPPLY_INDEX_FLOOR_RAW,
    SUPPLY_INDEX_FLOOR_RAW + 1,
    SUPPLY_INDEX_FLOOR_RAW + 123_456_789,
    RAY - 1,
    RAY,
    RAY + 1,
    RAY * 105 / 100,
    RAY * 3 / 2,
    2 * RAY - 1,
    1_000_000_000_000_000_000_000_000_000_007,
    MAX_SUPPLY_INDEX_RAY - 1,
    MAX_SUPPLY_INDEX_RAY,
];

/// Borrow indexes never fall below RAY (the index only grows, capped at 10^36).
const BORROW_INDEXES: [i128; 12] = [
    RAY,
    RAY + 1,
    RAY + 7,
    RAY * 105 / 100,
    RAY * 3 / 2,
    2 * RAY - 1,
    3 * RAY + 1,
    1_000_000_000_000_000_000_000_000_000_007,
    RAY * 123_456_789 / 100_000_000,
    MAX_SUPPLY_INDEX_RAY / 3,
    MAX_SUPPLY_INDEX_RAY - 1,
    MAX_SUPPLY_INDEX_RAY,
];

fn big(v: i128) -> BigInt {
    BigInt::from(v)
}

/// `loss / unit` as a decimal string with nine fractional digits.
fn units(loss: &BigInt, unit: &BigInt) -> String {
    let whole = loss / unit;
    let frac = (loss % unit) * BigInt::from(1_000_000_000i128) / unit;
    format!("{whole}.{:09}", frac)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Supply,
    WithdrawPartial,
    WithdrawFull,
    Borrow,
    RepayPartial,
    RepayFull,
    NetSettle,
}

const OPS: [Op; 7] = [
    Op::Supply,
    Op::WithdrawPartial,
    Op::WithdrawFull,
    Op::Borrow,
    Op::RepayPartial,
    Op::RepayFull,
    Op::NetSettle,
];

fn op_index(op: Op) -> usize {
    OPS.iter().position(|o| *o == op).expect("known op")
}

/// One market, one whale supplier (static) and one active user.
struct Model {
    env: Env,
    decimals: u32,
    unit: BigInt,
    si: Ray,
    bi: Ray,
    supplied: i128,
    borrowed: i128,
    cash: i128,
    whale: i128,
    s: i128,
    d: i128,
    tokens_in: i128,
    tokens_out: i128,
    ops: u64,
    max_loss: Vec<BigInt>,
    total_loss: BigInt,
    /// Sum of the per-operation bounds implied by the floor/ceil chain.
    total_bound: BigInt,
}

impl Model {
    fn new(env: &Env, decimals: u32, si: i128, bi: i128) -> Self {
        Self {
            env: env.clone(),
            decimals,
            unit: big(RAY) * big(10i128.pow(27 - decimals)),
            si: Ray::from(si),
            bi: Ray::from(bi),
            supplied: 0,
            borrowed: 0,
            cash: 0,
            whale: 0,
            s: 0,
            d: 0,
            tokens_in: 0,
            tokens_out: 0,
            ops: 0,
            max_loss: (0..OPS.len()).map(|_| BigInt::from(0)).collect(),
            total_loss: BigInt::from(0),
            total_bound: BigInt::from(0),
        }
    }

    fn label(&self) -> String {
        format!(
            "d={} SI={} BI={}",
            self.decimals,
            self.si.raw(),
            self.bi.raw()
        )
    }

    fn wealth(&self) -> BigInt {
        (big(self.tokens_out) - big(self.tokens_in)) * &self.unit + big(self.s) * big(self.si.raw())
            - big(self.d) * big(self.bi.raw())
    }

    fn supply_half_up(&self) -> i128 {
        unscale_supply(&self.env, Ray::from(self.s), self.si, self.decimals)
    }

    fn debt_ceil(&self) -> i128 {
        unscale_borrow_ceil(&self.env, Ray::from(self.d), self.bi, self.decimals)
    }

    fn claims(&self) -> i128 {
        unscale_supply_floor(&self.env, Ray::from(self.supplied), self.si, self.decimals)
    }

    fn debt(&self) -> i128 {
        unscale_borrow_ceil(&self.env, Ray::from(self.borrowed), self.bi, self.decimals)
    }

    /// Floored claims never exceed cash plus ceiled debt; shares are conserved.
    fn check_pool(&self, ctx: &str) {
        let claims = self.claims();
        let backing = self.cash + self.debt();
        assert!(
            claims <= backing,
            "{} {ctx}: floored claims {claims} exceed cash + ceiled debt {backing}",
            self.label()
        );
        assert_eq!(self.supplied, self.whale + self.s, "{ctx}: supply shares");
        assert_eq!(self.borrowed, self.d, "{ctx}: debt shares");
        assert!(self.cash >= 0, "{ctx}: cash went negative");
    }

    /// The bound the floor/ceil chain implies for one operation.
    fn bound(&self, op: Op) -> BigInt {
        let si = big(self.si.raw()) - 1;
        let bi = big(self.bi.raw()) - 1;
        let one = &self.unit - 1;
        match op {
            Op::Supply | Op::WithdrawPartial => si,
            Op::Borrow | Op::RepayPartial => bi,
            Op::WithdrawFull | Op::RepayFull => one,
            // Supply side: partial burn or full close; debt side likewise.
            Op::NetSettle => si.max(one.clone()) + bi.max(one),
        }
    }

    fn settle(&mut self, op: Op, before: &BigInt, ctx: &str) {
        let after = self.wealth();
        let loss = before - &after;
        assert!(
            loss >= BigInt::from(0),
            "{} {ctx} {op:?}: user gained {} units",
            self.label(),
            units(&-&loss, &self.unit)
        );
        let bound = self.bound(op);
        assert!(
            loss <= bound,
            "{} {ctx} {op:?}: loss {} units exceeds the documented boundary bound {}",
            self.label(),
            units(&loss, &self.unit),
            units(&bound, &self.unit)
        );
        let documented = if op == Op::NetSettle {
            (&self.unit - 1) * 2
        } else {
            &self.unit - 1
        };
        assert!(loss <= documented, "{ctx}: above one unit per boundary");
        let slot = op_index(op);
        if loss > self.max_loss[slot] {
            self.max_loss[slot] = loss.clone();
        }
        self.total_loss += loss;
        self.total_bound += bound;
        self.ops += 1;
        self.check_pool(ctx);
    }

    /// `ops/supply.rs::apply` for the whale: no wealth tracking.
    fn whale_supply(&mut self, amount: i128) {
        let minted = calculate_scaled_supply(&self.env, amount, self.decimals, self.si).raw();
        assert!(minted > 0, "whale supply rounds to zero shares");
        self.whale += minted;
        self.supplied += minted;
        self.cash += amount;
        self.check_pool("whale supply");
    }

    /// `ops/supply.rs::apply` lines 26-41.
    fn supply(&mut self, amount: i128, ctx: &str) -> i128 {
        let before = self.wealth();
        let minted = calculate_scaled_supply(&self.env, amount, self.decimals, self.si).raw();
        assert!(amount == 0 || minted > 0, "{ctx}: SupplyRoundsToZeroShares");
        self.s += minted;
        self.supplied += minted;
        self.cash += amount;
        self.tokens_in += amount;
        self.settle(Op::Supply, &before, ctx);
        minted
    }

    /// `ops/withdraw.rs::accounting` lines 107-121 (no fee). Returns
    /// `(gross paid, full close)`.
    fn withdraw(&mut self, amount: i128, ctx: &str) -> (i128, bool) {
        let before = self.wealth();
        let full = amount >= self.supply_half_up();
        let (burned, gross) =
            resolve_withdrawal(&self.env, amount, Ray::from(self.s), self.si, self.decimals);
        let burned = burned.raw();
        assert!(
            gross == 0 || burned > 0,
            "{ctx}: WithdrawRoundsToZeroShares"
        );
        assert!(
            burned <= self.s,
            "{ctx}: burn {burned} above position {}",
            self.s
        );
        assert!(self.cash >= gross, "{ctx}: InsufficientLiquidity");
        if full {
            assert_eq!(burned, self.s, "{ctx}: full close burns every share");
        }
        self.s -= burned;
        self.supplied -= burned;
        self.cash -= gross;
        self.tokens_out += gross;
        let op = if full {
            Op::WithdrawFull
        } else {
            Op::WithdrawPartial
        };
        self.settle(op, &before, ctx);
        (gross, full)
    }

    /// `ops/borrow.rs::mint_debt` + `accounting` (reserve and buffer checks
    /// are restrictions only; the whale keeps cash ample).
    fn borrow(&mut self, amount: i128, ctx: &str) -> i128 {
        let before = self.wealth();
        assert!(amount > 0);
        assert!(self.cash >= amount, "{ctx}: InsufficientLiquidity");
        let minted = calculate_scaled_borrow(&self.env, amount, self.decimals, self.bi).raw();
        assert!(minted > 0, "{ctx}: BorrowRoundsToZeroShares");
        self.d += minted;
        self.borrowed += minted;
        self.cash -= amount;
        self.tokens_out += amount;
        self.settle(Op::Borrow, &before, ctx);
        minted
    }

    /// `ops/repay.rs::accounting` lines 120-136. Returns `(net repay, full close)`.
    /// The overpayment is refunded, so only the net counts as tokens in.
    fn repay(&mut self, amount: i128, ctx: &str) -> (i128, bool) {
        let before = self.wealth();
        let full = amount >= self.debt_ceil();
        let (burned, over) =
            resolve_repay(&self.env, amount, Ray::from(self.d), self.bi, self.decimals);
        let burned = burned.raw();
        let net = amount - over;
        assert!(net == 0 || burned > 0, "{ctx}: RepayRoundsToZeroShares");
        assert!(burned <= self.d, "{ctx}: burn above debt position");
        if full {
            assert_eq!(burned, self.d, "{ctx}: full repay burns every share");
        } else {
            assert_eq!(over, 0, "{ctx}: partial repay refunds nothing");
        }
        self.d -= burned;
        self.borrowed -= burned;
        self.cash += net;
        self.tokens_in += net;
        let op = if full {
            Op::RepayFull
        } else {
            Op::RepayPartial
        };
        self.settle(op, &before, ctx);
        (net, full)
    }

    /// `ops/net_settle.rs::apply` lines 182-191.
    fn net_settle(&mut self, amount: i128, ctx: &str) -> i128 {
        let before = self.wealth();
        let (bs, bd, settled) = resolve_net_settle(
            &self.env,
            amount,
            Ray::from(self.s),
            Ray::from(self.d),
            self.si,
            self.bi,
            self.decimals,
        );
        let (bs, bd) = (bs.raw(), bd.raw());
        assert!(
            settled == 0 || (bs > 0 && bd > 0),
            "{ctx}: NetSettleRoundsToZeroShares"
        );
        assert!(bs <= self.s && bd <= self.d, "{ctx}: burn above position");
        self.s -= bs;
        self.d -= bd;
        self.supplied -= bs;
        self.borrowed -= bd;
        self.settle(Op::NetSettle, &before, ctx);
        settled
    }

    /// Closes whatever the user still holds (withdraw-all sentinel, repay the ceiled debt).
    fn close_all(&mut self, ctx: &str) {
        if self.s > 0 {
            self.withdraw(i128::MAX, ctx);
        }
        if self.d > 0 {
            let owed = self.debt_ceil();
            self.repay(owed, ctx);
        }
        assert_eq!((self.s, self.d), (0, 0), "{ctx}: position not closed");
        assert!(
            self.tokens_out <= self.tokens_in,
            "{} {ctx}: user took out {} and paid in {}",
            self.label(),
            self.tokens_out,
            self.tokens_in
        );
    }

    fn report(&self, title: &str) {
        let mut parts = Vec::new();
        for (i, op) in OPS.iter().enumerate() {
            parts.push(format!("{op:?}={}", units(&self.max_loss[i], &self.unit)));
        }
        println!(
            "{title} {} ops={} total_loss={} total_bound={} max[{}]",
            self.label(),
            self.ops,
            units(&self.total_loss, &self.unit),
            units(&self.total_bound, &self.unit),
            parts.join(" ")
        );
    }
}

fn bare_env() -> Env {
    let env = Env::default();
    env.cost_estimate().budget().reset_unlimited();
    env
}

// ---------------------------------------------------------------------------
// Cycles. Each starts and ends with the user holding nothing.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Cycle {
    /// supply(a); withdraw(i128::MAX).
    SupplyWithdrawAll,
    /// supply(a); withdraw(a); close.
    SupplyWithdrawSame,
    /// supply(a); withdraw(a - 1); close.
    SupplyWithdrawPartial,
    /// borrow(a); repay(ceil(debt)).
    BorrowRepayAll,
    /// borrow(a); repay(a); close.
    BorrowRepaySame,
    /// borrow(a); repay(ceil(debt) + 5): refund of exactly 5.
    BorrowRepayOver,
    /// supply(a); borrow(a); net_settle(i128::MAX); close.
    NetSettleFull,
    /// supply(a); borrow(a); net_settle(1); close.
    NetSettleOne,
}

const CYCLES: [Cycle; 8] = [
    Cycle::SupplyWithdrawAll,
    Cycle::SupplyWithdrawSame,
    Cycle::SupplyWithdrawPartial,
    Cycle::BorrowRepayAll,
    Cycle::BorrowRepaySame,
    Cycle::BorrowRepayOver,
    Cycle::NetSettleFull,
    Cycle::NetSettleOne,
];

/// Runs one cycle and returns the exact loss it caused.
fn run_cycle(m: &mut Model, cycle: Cycle, a: i128) -> BigInt {
    let start = m.total_loss.clone();
    let ctx = format!("{cycle:?} a={a}");
    match cycle {
        Cycle::SupplyWithdrawAll => {
            m.supply(a, &ctx);
            m.withdraw(i128::MAX, &ctx);
        }
        Cycle::SupplyWithdrawSame => {
            m.supply(a, &ctx);
            m.withdraw(a, &ctx);
        }
        Cycle::SupplyWithdrawPartial => {
            m.supply(a, &ctx);
            if a > 1 {
                m.withdraw(a - 1, &ctx);
            }
        }
        Cycle::BorrowRepayAll => {
            m.borrow(a, &ctx);
            let owed = m.debt_ceil();
            m.repay(owed, &ctx);
        }
        Cycle::BorrowRepaySame => {
            m.borrow(a, &ctx);
            m.repay(a, &ctx);
        }
        Cycle::BorrowRepayOver => {
            m.borrow(a, &ctx);
            let owed = m.debt_ceil();
            let (net, full) = m.repay(owed + 5, &ctx);
            assert!(
                full && net == owed,
                "{ctx}: overpayment of 5 must be refunded"
            );
        }
        Cycle::NetSettleFull => {
            m.supply(a, &ctx);
            m.borrow(a, &ctx);
            m.net_settle(i128::MAX, &ctx);
        }
        Cycle::NetSettleOne => {
            m.supply(a, &ctx);
            m.borrow(a, &ctx);
            m.net_settle(1, &ctx);
        }
    }
    m.close_all(&ctx);
    &m.total_loss - start
}

// ---------------------------------------------------------------------------
// A1. Fixed indexes: every cycle loses under one unit per boundary, and a
// repeated cycle loses exactly the same amount each time (no hidden state).
// ---------------------------------------------------------------------------

#[test]
fn rv_round_r6_fixed_index_cycles_lose_under_one_unit_per_boundary_and_repeat_exactly() {
    let env = bare_env();
    let mut worst_cycle = (BigInt::from(0), String::new(), BigInt::from(1));
    for decimals in DECIMALS {
        for (si, bi) in SUPPLY_INDEXES.into_iter().zip(BORROW_INDEXES) {
            let mut m = Model::new(&env, decimals, si, bi);
            m.whale_supply(WHALE);
            for cycle in CYCLES {
                for a in AMOUNTS {
                    let first = run_cycle(&mut m, cycle, a);
                    for k in 1..5 {
                        let again = run_cycle(&mut m, cycle, a);
                        assert_eq!(
                            again,
                            first,
                            "{} {cycle:?} a={a}: cycle {k} lost a different amount",
                            m.label()
                        );
                    }
                    // Two boundaries per single-sided cycle, four for a net settle cycle.
                    let per_cycle_bound = match cycle {
                        Cycle::NetSettleFull | Cycle::NetSettleOne => (&m.unit - 1) * 4,
                        _ => (&m.unit - 1) * 2,
                    };
                    assert!(first <= per_cycle_bound, "{} {cycle:?} a={a}", m.label());
                    if &first * &worst_cycle.2 > &worst_cycle.0 * &m.unit {
                        worst_cycle = (
                            first.clone(),
                            format!("{} {cycle:?} a={a}", m.label()),
                            m.unit.clone(),
                        );
                    }
                }
            }
            m.report("A1");
        }
    }
    println!(
        "A1 worst cycle: {} units at {}",
        units(&worst_cycle.0, &worst_cycle.2),
        worst_cycle.1
    );
}

// ---------------------------------------------------------------------------
// A2. Three thousand random index pairs per decimal (log-uniform from the
// floor to the ceiling), one cycle of each kind at each pair.
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// Log-uniform index in `[10^min_exp, 10^36]`, with the exact extremes mixed in.
    fn index(&mut self, min_exp: u32, floor: i128) -> i128 {
        match self.below(50) {
            0 => floor,
            1 => MAX_SUPPLY_INDEX_RAY,
            2 => floor + 1,
            3 => MAX_SUPPLY_INDEX_RAY - 1,
            _ => {
                let e = min_exp + self.below(u64::from(36 - min_exp)) as u32;
                let base = 10i128.pow(e);
                let mantissa = 1 + i128::from(self.below(9));
                let offset = i128::from(self.next()) % base;
                (mantissa * base + offset).min(MAX_SUPPLY_INDEX_RAY)
            }
        }
    }
}

#[test]
fn rv_round_r6_random_index_pairs_three_thousand_per_decimal_stay_within_bounds() {
    let env = bare_env();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for decimals in DECIMALS {
        let mut worst: Vec<BigInt> = (0..OPS.len()).map(|_| BigInt::from(0)).collect();
        let mut worst_unit = BigInt::from(1);
        for i in 0..3_000u64 {
            let si = rng.index(24, SUPPLY_INDEX_FLOOR_RAW);
            let bi = rng.index(27, RAY);
            let mut m = Model::new(&env, decimals, si, bi);
            m.whale_supply(WHALE);
            let a = AMOUNTS[(i % AMOUNTS.len() as u64) as usize];
            let cycle = CYCLES[((i / AMOUNTS.len() as u64) % CYCLES.len() as u64) as usize];
            run_cycle(&mut m, cycle, a);
            // A second amount at the same pair: one unit, the brief's focus.
            run_cycle(&mut m, cycle, 1);
            for (slot, loss) in m.max_loss.iter().enumerate() {
                if loss > &worst[slot] {
                    worst[slot] = loss.clone();
                    worst_unit = m.unit.clone();
                }
            }
        }
        let parts: Vec<String> = OPS
            .iter()
            .enumerate()
            .map(|(i, op)| format!("{op:?}={}", units(&worst[i], &worst_unit)))
            .collect();
        println!(
            "A2 d={decimals} worst per-op loss over 3000 pairs: {}",
            parts.join(" ")
        );
    }
}

// ---------------------------------------------------------------------------
// A3. One-unit chains of 2,000 steps with the position carried across steps.
// The cumulative loss must stay within the sum of the per-step bounds, and the
// measured total is reported in units.
// ---------------------------------------------------------------------------

const CHAIN: i128 = 2_000;

#[test]
fn rv_round_r6_one_unit_chains_of_two_thousand_steps_stay_within_summed_bounds() {
    let env = bare_env();
    for decimals in DECIMALS {
        for (si, bi) in SUPPLY_INDEXES.into_iter().zip(BORROW_INDEXES) {
            // (a) 2,000 one-unit supplies, one withdraw-all.
            let mut m = Model::new(&env, decimals, si, bi);
            m.whale_supply(WHALE);
            for _ in 0..CHAIN {
                m.supply(1, "chain supply 1");
            }
            let (paid, _) = m.withdraw(i128::MAX, "chain supply 1 close");
            m.close_all("chain supply 1");
            assert!(m.total_loss <= m.total_bound);
            println!(
                "A3a {} supply(1)x{CHAIN} then withdraw-all paid {paid}: loss {} units (bound {})",
                m.label(),
                units(&m.total_loss, &m.unit),
                units(&m.total_bound, &m.unit)
            );

            // (b) 2,000 one-unit borrows, one full repay.
            let mut m = Model::new(&env, decimals, si, bi);
            m.whale_supply(WHALE);
            for _ in 0..CHAIN {
                m.borrow(1, "chain borrow 1");
            }
            let owed = m.debt_ceil();
            m.close_all("chain borrow 1");
            assert!(m.total_loss <= m.total_bound);
            println!(
                "A3b {} borrow(1)x{CHAIN} owes {owed}: loss {} units (bound {})",
                m.label(),
                units(&m.total_loss, &m.unit),
                units(&m.total_bound, &m.unit)
            );

            // (c) one supply of 2,000, then one-unit withdrawals until empty.
            let mut m = Model::new(&env, decimals, si, bi);
            m.whale_supply(WHALE);
            m.supply(CHAIN, "chain withdraw 1");
            let mut steps = 0;
            while m.s > 0 {
                m.withdraw(1, "chain withdraw 1");
                steps += 1;
                assert!(steps <= CHAIN + 1, "withdraw chain does not terminate");
            }
            m.close_all("chain withdraw 1");
            assert!(m.total_loss <= m.total_bound);
            println!(
                "A3c {} supply({CHAIN}) then withdraw(1)x{steps} got {}: loss {} units (bound {})",
                m.label(),
                m.tokens_out,
                units(&m.total_loss, &m.unit),
                units(&m.total_bound, &m.unit)
            );

            // (d) one borrow of 2,000, then one-unit repayments until clear.
            let mut m = Model::new(&env, decimals, si, bi);
            m.whale_supply(WHALE);
            m.borrow(CHAIN, "chain repay 1");
            let mut steps = 0;
            while m.d > 0 {
                m.repay(1, "chain repay 1");
                steps += 1;
                assert!(steps <= CHAIN + 2, "repay chain does not terminate");
            }
            m.close_all("chain repay 1");
            assert!(m.total_loss <= m.total_bound);
            println!(
                "A3d {} borrow({CHAIN}) then repay(1)x{steps} paid {}: loss {} units (bound {})",
                m.label(),
                m.tokens_in,
                units(&m.total_loss, &m.unit),
                units(&m.total_bound, &m.unit)
            );

            // (e) supply and borrow 2,000, then one-unit net settlements until a side closes.
            let mut m = Model::new(&env, decimals, si, bi);
            m.whale_supply(WHALE);
            m.supply(CHAIN, "chain settle 1");
            m.borrow(CHAIN, "chain settle 1");
            let mut steps = 0;
            while m.s > 0 && m.d > 0 && m.net_settle(1, "chain settle 1") > 0 {
                steps += 1;
                assert!(steps <= CHAIN + 1, "settle chain does not terminate");
            }
            let (left_s, left_d) = (m.s, m.d);
            m.close_all("chain settle 1");
            assert!(m.total_loss <= m.total_bound);
            println!(
                "A3e {} net_settle(1)x{steps} left shares ({left_s}, {left_d}): loss {} units (bound {})",
                m.label(),
                units(&m.total_loss, &m.unit),
                units(&m.total_bound, &m.unit)
            );
        }
    }
}

// ---------------------------------------------------------------------------
// B. The deployed pool WASM at injected indexes, compared with the model after
// every call: books, returned mutations and token balances.
// ---------------------------------------------------------------------------

fn preset(name: &'static str, decimals: u32) -> MarketPreset {
    let mut config = DEFAULT_ASSET_CONFIG;
    if decimals < 3 {
        config.is_borrowable = false;
        config.liquidation_fees = 0;
        config.is_flashloanable = false;
        config.flashloan_fee = 0;
    }
    MarketPreset {
        name,
        decimals,
        price_wad: usd(1),
        initial_liquidity: 0.0,
        config,
        params: DEFAULT_MARKET_PARAMS,
    }
}

const MARKETS: [(&str, u32); 3] = [("D0", 0), ("USDC", 7), ("D18", 18)];

/// `(supply index, borrow index)` pairs injected into the pool state.
const INJECTED: [(i128, i128); 4] = [
    (RAY + 1, RAY + 1),
    (SUPPLY_INDEX_FLOOR_RAW + 1, RAY * 105 / 100),
    (RAY * 3 / 2, 2 * RAY - 1),
    (MAX_SUPPLY_INDEX_RAY - 1, MAX_SUPPLY_INDEX_RAY - 1),
];

struct Driver<'a> {
    t: &'a LendingTest,
    name: &'static str,
    key: HubAssetKey,
    user: Address,
    m: Model,
}

impl Driver<'_> {
    fn pool(&self) -> pool::LiquidityPoolClient<'_> {
        self.t.pool_client(self.name)
    }

    fn token(&self) -> token::Client<'_> {
        token::Client::new(&self.t.env, &self.t.resolve_asset(self.name))
    }

    fn pay_in(&self, who: &Address, amount: i128) {
        let market = self.t.resolve_market(self.name);
        market.token_admin.mint(who, &amount);
        self.token().transfer(who, &market.pool, &amount);
    }

    fn action(&self, position: i128, amount: i128) -> PoolAction {
        PoolAction {
            position: ScaledPositionRaw {
                scaled_amount: position,
            },
            amount,
            hub_asset: self.key.clone(),
        }
    }

    fn state(&self) -> PoolStateRaw {
        self.pool().get_sync_data(&self.key).state
    }

    fn assert_books(&self, ctx: &str) {
        let s = self.state();
        assert_eq!(s.supplied, self.m.supplied, "{ctx}: supplied");
        assert_eq!(s.borrowed, self.m.borrowed, "{ctx}: borrowed");
        assert_eq!(s.cash, self.m.cash, "{ctx}: cash");
        assert_eq!(s.supply_index, self.m.si.raw(), "{ctx}: supply index moved");
        assert_eq!(s.borrow_index, self.m.bi.raw(), "{ctx}: borrow index moved");
    }

    fn supply(&mut self, a: i128, ctx: &str) {
        self.pay_in(&self.user.clone(), a);
        let mutation = self
            .pool()
            .supply(&vec![
                &self.t.env,
                PoolSupplyEntry {
                    action: self.action(self.m.s, a),
                },
            ])
            .get(0)
            .unwrap();
        self.m.supply(a, ctx);
        assert_eq!(mutation.position.scaled_amount, self.m.s, "{ctx}: minted");
        assert_eq!(mutation.actual_amount, a, "{ctx}: supply actual");
        self.assert_books(ctx);
    }

    fn withdraw(&mut self, a: i128, ctx: &str) {
        let before = self.token().balance(&self.user);
        let mutation = self
            .pool()
            .withdraw(
                &self.user,
                &false,
                &vec![
                    &self.t.env,
                    PoolWithdrawEntry {
                        action: self.action(self.m.s, a),
                        protocol_fee: 0,
                    },
                ],
            )
            .get(0)
            .unwrap();
        let (gross, _) = self.m.withdraw(a, ctx);
        assert_eq!(
            mutation.position.scaled_amount, self.m.s,
            "{ctx}: remaining"
        );
        assert_eq!(mutation.actual_amount, gross, "{ctx}: gross");
        assert_eq!(
            self.token().balance(&self.user) - before,
            gross,
            "{ctx}: tokens received"
        );
        self.assert_books(ctx);
    }

    fn borrow(&mut self, a: i128, ctx: &str) {
        let before = self.token().balance(&self.user);
        let mutation = self
            .pool()
            .borrow(
                &self.user,
                &vec![
                    &self.t.env,
                    PoolBorrowEntry {
                        action: self.action(self.m.d, a),
                    },
                ],
            )
            .get(0)
            .unwrap();
        self.m.borrow(a, ctx);
        assert_eq!(
            mutation.position.scaled_amount, self.m.d,
            "{ctx}: debt minted"
        );
        assert_eq!(mutation.actual_amount, a, "{ctx}: borrow actual");
        assert_eq!(
            self.token().balance(&self.user) - before,
            a,
            "{ctx}: received"
        );
        self.assert_books(ctx);
    }

    fn repay(&mut self, a: i128, ctx: &str) {
        self.pay_in(&self.user.clone(), a);
        let before = self.token().balance(&self.user);
        let mutation = self
            .pool()
            .repay(&self.user, &vec![&self.t.env, self.action(self.m.d, a)])
            .get(0)
            .unwrap();
        let (net, _) = self.m.repay(a, ctx);
        assert_eq!(
            mutation.position.scaled_amount, self.m.d,
            "{ctx}: debt left"
        );
        assert_eq!(mutation.actual_amount, net, "{ctx}: net repay");
        assert_eq!(
            self.token().balance(&self.user) - before,
            a - net,
            "{ctx}: refund"
        );
        self.assert_books(ctx);
    }

    fn net_settle(&mut self, a: i128, ctx: &str) {
        let result = self.pool().net_settle(&PoolNetSettleEntry {
            hub_asset: self.key.clone(),
            amount: a,
            supply_position: ScaledPositionRaw {
                scaled_amount: self.m.s,
            },
            debt_position: ScaledPositionRaw {
                scaled_amount: self.m.d,
            },
        });
        let settled = self.m.net_settle(a, ctx);
        assert_eq!(
            result.supply_position.scaled_amount, self.m.s,
            "{ctx}: supply left"
        );
        assert_eq!(
            result.debt_position.scaled_amount, self.m.d,
            "{ctx}: debt left"
        );
        assert_eq!(result.settled_amount, settled, "{ctx}: settled");
        self.assert_books(ctx);
    }

    fn close_all(&mut self, ctx: &str) {
        if self.m.s > 0 {
            self.withdraw(i128::MAX, ctx);
        }
        if self.m.d > 0 {
            let owed = self.m.debt_ceil();
            self.repay(owed, ctx);
        }
        assert_eq!((self.m.s, self.m.d), (0, 0), "{ctx}: not closed");
        assert!(self.m.tokens_out <= self.m.tokens_in, "{ctx}: user ahead");
    }

    fn cycle(&mut self, cycle: Cycle, a: i128) {
        let ctx = format!("{} {} {cycle:?} a={a}", self.name, self.m.label());
        match cycle {
            Cycle::SupplyWithdrawAll => {
                self.supply(a, &ctx);
                self.withdraw(i128::MAX, &ctx);
            }
            Cycle::SupplyWithdrawSame => {
                self.supply(a, &ctx);
                self.withdraw(a, &ctx);
            }
            Cycle::SupplyWithdrawPartial => {
                self.supply(a, &ctx);
                if a > 1 {
                    self.withdraw(a - 1, &ctx);
                }
            }
            Cycle::BorrowRepayAll => {
                self.borrow(a, &ctx);
                let owed = self.m.debt_ceil();
                self.repay(owed, &ctx);
            }
            Cycle::BorrowRepaySame => {
                self.borrow(a, &ctx);
                self.repay(a, &ctx);
            }
            Cycle::BorrowRepayOver => {
                self.borrow(a, &ctx);
                let owed = self.m.debt_ceil();
                self.repay(owed + 5, &ctx);
            }
            Cycle::NetSettleFull => {
                self.supply(a, &ctx);
                self.borrow(a, &ctx);
                self.net_settle(i128::MAX, &ctx);
            }
            Cycle::NetSettleOne => {
                self.supply(a, &ctx);
                self.borrow(a, &ctx);
                self.net_settle(1, &ctx);
            }
        }
        self.close_all(&ctx);
    }
}

fn inject(t: &LendingTest, name: &str, si: i128, bi: i128) {
    let market = t.resolve_market(name);
    let key = hub_asset(market.asset.clone());
    let state = PoolStateRaw {
        supplied: 0,
        borrowed: 0,
        revenue: 0,
        borrow_index: bi,
        supply_index: si,
        last_timestamp: t.env.ledger().timestamp() * 1_000,
        cash: 0,
    };
    t.env.as_contract(&market.pool, || {
        t.env
            .storage()
            .persistent()
            .set(&PoolKey::State(key), &state);
    });
}

fn disable_utilization_cap(t: &LendingTest, name: &str) {
    let key = hub_asset(t.resolve_asset(name));
    let pool = t.pool_client(name);
    let mut model: InterestRateModel = pool.get_sync_data(&key).params.rate_model_view();
    model.max_utilization = RAY;
    pool.update_params(&key, &model);
}

#[test]
fn rv_round_r6_pool_wasm_matches_the_model_at_injected_indexes_for_decimals_0_7_18() {
    let t = LendingTest::new()
        .with_market(preset("D0", 0))
        .with_market(preset("USDC", 7))
        .with_market(preset("D18", 18))
        .build();
    t.env.cost_estimate().budget().reset_unlimited();
    for (name, decimals) in MARKETS {
        assert_eq!(t.resolve_market(name).decimals, decimals);
        disable_utilization_cap(&t, name);
        for (si, bi) in INJECTED {
            inject(&t, name, si, bi);
            let user = Address::generate(&t.env);
            let whale = Address::generate(&t.env);
            let key = hub_asset(t.resolve_asset(name));
            let mut drv = Driver {
                t: &t,
                name,
                key: key.clone(),
                user,
                m: Model::new(&t.env, decimals, si, bi),
            };
            // Whale through the pool and the model alike.
            drv.pay_in(&whale, WHALE);
            drv.pool().supply(&vec![
                &t.env,
                PoolSupplyEntry {
                    action: drv.action(0, WHALE),
                },
            ]);
            drv.m.whale_supply(WHALE);
            drv.assert_books("whale");
            for cycle in CYCLES {
                for a in [1, 7, 999] {
                    for _ in 0..3 {
                        drv.cycle(cycle, a);
                    }
                }
            }
            drv.m.report(&format!("B {name}"));
        }
    }
}
