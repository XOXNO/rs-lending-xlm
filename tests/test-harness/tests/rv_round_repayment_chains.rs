//! Rounding audit, lens R-3 / R-4, round 1 sweeps: debt repayment across
//! several debt legs of mixed decimals, a one-base-unit 18-decimal dust leg,
//! chosen leg order, and chains of partial liquidations over three collateral
//! legs. Every assertion is the documented bound (`docs/reference/formulas.md`,
//! "Liquidation sizing and fees" and "Seizure and fees"; review note MC-3 in
//! `docs/explanation/rv-defensive-review-2026-10.md`):
//!
//! * solvent partial over-offer: the kept amount rounds up by less than one
//!   native unit of the split (last) leg, whatever the leg order;
//! * insolvent over-offer: the kept amount rounds down by less than one native
//!   unit of the split leg and never exceeds the collateral-backed quote `Q`;
//! * insolvent `seize_all`: an untrimmed under-offer within one native unit of
//!   every kept repayment leg takes all collateral, so the liquidator's gain
//!   over paying `Q` is at most that sum, once per account;
//! * full close: each leg is charged at its ceiling, at most one native unit
//!   per leg above the WAD-valued debt;
//! * chains of partials never take more than one close beyond the BPS floor
//!   of the HF-preserving cap plus one unit per collateral leg per step.
//!
//! Fixture: ALICE holds USDC (7 dec), ETH (18 dec) and WBTC (7 dec) and owes
//! USDT (7 dec, $1), GOLD (3 dec, $6000: one base unit is $6), XLM (7 dec,
//! $0.10) and one base unit of DUST (18 dec, $1).

use common::math::fp::{Ray, Wad};
use common::math::fp_core::mul_div_floor;
use common::rates::unscale_borrow_ceil;
use common::types::{HubAssetKey, LiquidationEstimate, PaymentTuple, SeizeMode};
use controller::constants::{BPS, WAD};
use soroban_sdk::Vec as SVec;
use test_harness::{
    eth_preset, hub_asset, usd, usdc_preset, usdt_stable_preset, wbtc_preset, xlm_preset,
    LendingTest, MarketPreset, ALICE, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, LIQUIDATOR,
};

const COLL: [&str; 3] = ["USDC", "ETH", "WBTC"];
const DEBT: [&str; 4] = ["USDT", "GOLD", "XLM", "DUST"];

/// Raw WAD slack for the half-up / ceil / floor valuation steps (1e-15 USD).
const ULP: i128 = 1_000;

fn gold_preset() -> MarketPreset {
    MarketPreset {
        name: "GOLD",
        decimals: 3,
        price_wad: usd(6_000),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn dust_preset() -> MarketPreset {
    MarketPreset {
        name: "DUST",
        decimals: 18,
        price_wad: usd(1),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn build() -> LendingTest {
    build_with(7500, 8500)
}

/// Same markets, with the USDC collateral leg at the given LTV / threshold.
fn build_with(usdc_ltv: u32, usdc_lt: u32) -> LendingTest {
    let legs = [
        (usdc_ltv, usdc_lt, 400u32, 1000u32),
        (7000, 7800, 800, 1500),
        (6500, 7500, 1000, 1200),
    ];
    let mut b = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(MarketPreset {
            decimals: 18,
            ..eth_preset()
        })
        .with_market(wbtc_preset())
        .with_market(usdt_stable_preset())
        .with_market(gold_preset())
        .with_market(xlm_preset())
        .with_market(dust_preset());
    for (name, (ltv, lt, bonus, fees)) in COLL.into_iter().zip(legs) {
        b = b.with_market_config(name, move |c| {
            c.loan_to_value = ltv;
            c.liquidation_threshold = lt;
            c.liquidation_bonus = bonus;
            c.liquidation_fees = fees;
        });
    }
    b.build()
}

fn acct(t: &LendingTest) -> u64 {
    t.resolve_account_id(ALICE)
}
fn coll(t: &LendingTest, acc: u64) -> i128 {
    t.ctrl_client().get_total_collateral_usd(&acc)
}
fn debt(t: &LendingTest, acc: u64) -> i128 {
    t.ctrl_client().get_total_borrow_usd(&acc)
}
fn hf(t: &LendingTest, acc: u64) -> i128 {
    t.ctrl_client().get_health_factor(&acc)
}
fn px(t: &LendingTest, name: &str) -> i128 {
    t.resolve_market(name).price_wad
}
fn dec(t: &LendingTest, name: &str) -> u32 {
    t.resolve_market(name).decimals
}
/// The contract's own valuation of `tokens`: `Wad::from_token(..).mul(price)`, half-up.
fn usd_of(t: &LendingTest, name: &str, tokens: i128) -> i128 {
    Wad::from_token(&t.env, tokens, dec(t, name))
        .mul(&t.env, Wad::from(px(t, name)))
        .raw()
}
fn unit_usd(t: &LendingTest, name: &str) -> i128 {
    usd_of(t, name, 1)
}
/// Raw WAD USD to whole token units, floored.
fn tokens_of(t: &LendingTest, name: &str, usd_wad: i128) -> i128 {
    mul_div_floor(&t.env, usd_wad, 10i128.pow(dec(t, name)), px(t, name))
}
fn key(t: &LendingTest, name: &str) -> HubAssetKey {
    hub_asset(t.resolve_asset(name))
}
fn borrow_of(t: &LendingTest, acc: u64, name: &str) -> i128 {
    t.ctrl_client().get_borrow_amount(&acc, &key(t, name))
}
fn debt_scaled(t: &LendingTest, acc: u64, name: &str) -> i128 {
    let (_, borrows) = t.ctrl_client().get_account_positions(&acc);
    borrows.get(key(t, name)).map_or(0, |p| p.scaled_amount)
}
/// The leg's full-close amount: ceil at both conversion steps, as the plan caps it.
fn debt_ceil(t: &LendingTest, acc: u64, name: &str) -> i128 {
    let scaled = debt_scaled(t, acc, name);
    if scaled == 0 {
        return 0;
    }
    let index = t.ctrl_client().get_market_index(&key(t, name)).borrow_index;
    unscale_borrow_ceil(&t.env, Ray::from(scaled), Ray::from(index), dec(t, name))
}
fn pay_vec(t: &LendingTest, pays: &[(&str, i128)]) -> SVec<(HubAssetKey, i128)> {
    let mut v = SVec::new(&t.env);
    for (n, a) in pays {
        v.push_back((key(t, n), *a));
    }
    v
}
fn estimate(t: &LendingTest, acc: u64, pays: &[(&str, i128)]) -> LiquidationEstimate {
    t.ctrl_client()
        .get_liquidation_estimate(&acc, &pay_vec(t, pays), &SeizeMode::Transfer)
}
fn by_collateral(t: &LendingTest, tuples: &SVec<PaymentTuple>) -> [i128; 3] {
    let mut out = [0i128; 3];
    for tuple in tuples.iter() {
        if let Some(i) = COLL.iter().position(|n| t.resolve_asset(n) == tuple.asset) {
            out[i] += tuple.amount;
        }
    }
    out
}
fn coll_usd(t: &LendingTest, tokens: [i128; 3]) -> i128 {
    COLL.iter().zip(tokens).map(|(n, a)| usd_of(t, n, a)).sum()
}
fn collateral_units(t: &LendingTest) -> i128 {
    COLL.iter().map(|n| unit_usd(t, n)).sum()
}
/// Open debt legs of the account, in `DEBT` order, each offered at twice its ceiling.
fn open_over_offers(t: &LendingTest, acc: u64) -> Vec<(&'static str, i128)> {
    DEBT.iter()
        .filter_map(|n| {
            let c = debt_ceil(t, acc, n);
            (c > 0).then_some((*n, 2 * c))
        })
        .collect()
}
/// `floor(C / (1 + base))`: the insolvent collateral-backed quote.
fn backed_quote(t: &LendingTest, c: i128, base_bps: i128) -> i128 {
    mul_div_floor(&t.env, c, WAD, WAD + base_bps * WAD / BPS)
}
fn with_bonus(x: i128, bonus_bps: i128) -> i128 {
    x + x * bonus_bps / BPS
}

/// Shocks the three collateral prices by one factor so `C / D` lands near
/// `coverage_bps / BPS`. Debt: USDT $6000, GOLD 1000 units ($6000), XLM $1000.
fn book(coverage_bps: i128) -> LendingTest {
    book_with(coverage_bps, 6_000 * 10_000_000, 1_000, 10_000 * 10_000_000)
}

fn book_with(coverage_bps: i128, usdt: i128, gold: i128, xlm: i128) -> LendingTest {
    let mut t = build();
    t.supply_raw(ALICE, "USDC", 10_000 * 10_000_000);
    t.supply_raw(ALICE, "ETH", 2 * 10i128.pow(18));
    t.supply_raw(ALICE, "WBTC", 1_000_000);
    t.borrow_raw(ALICE, "USDT", usdt);
    t.borrow_raw(ALICE, "GOLD", gold);
    t.borrow_raw(ALICE, "XLM", xlm);
    t.borrow_raw(ALICE, "DUST", 10i128.pow(18));
    t.repay_raw(ALICE, "DUST", 10i128.pow(18) - 1);
    let acc = acct(&t);
    assert_eq!(
        borrow_of(&t, acc, "DUST"),
        1,
        "dust leg must be one base unit"
    );
    let (c0, d0) = (coll(&t, acc), debt(&t, acc));
    let factor = coverage_bps as f64 / BPS as f64 * d0 as f64 / c0 as f64;
    for name in COLL {
        let p = px(&t, name);
        t.set_price(name, (p as f64 * factor).round() as i128);
    }
    t.get_or_create_user(LIQUIDATOR);
    t.assert_liquidatable(ALICE);
    t
}

struct Outcome {
    /// Net tokens the liquidator spent per payment leg, in `pays` order.
    spent: Vec<i128>,
    /// Net collateral tokens the liquidator received per `COLL` leg.
    received: [i128; 3],
    fees: [i128; 3],
    c_after: i128,
    d_after: i128,
    alive: bool,
    est: LiquidationEstimate,
}

impl Outcome {
    fn charged_usd(&self, t: &LendingTest, pays: &[(&str, i128)]) -> i128 {
        pays.iter()
            .zip(&self.spent)
            .map(|((n, _), s)| usd_of(t, n, *s))
            .sum()
    }
    fn taken_usd(&self, t: &LendingTest) -> i128 {
        coll_usd(t, self.received) + coll_usd(t, self.fees)
    }
}

/// Mints the offers, liquidates in transfer mode and measures wallets and books.
fn run(t: &mut LendingTest, acc: u64, pays: &[(&str, i128)]) -> Outcome {
    let est = estimate(t, acc, pays);
    let fees = by_collateral(t, &est.protocol_fees);
    let addr = t.get_or_create_user(LIQUIDATOR);
    let wallet_before: Vec<i128> = pays
        .iter()
        .map(|(n, _)| t.token_balance_raw(LIQUIDATOR, n))
        .collect();
    for (n, a) in pays {
        t.resolve_market(n).token_admin.mint(&addr, a);
    }
    let coll_wallet_before = COLL.map(|n| t.token_balance_raw(LIQUIDATOR, n));
    t.ctrl_client()
        .liquidate(&addr, &acc, &pay_vec(t, pays), &SeizeMode::Transfer);
    t.assert_spoke_usage_matches_positions();
    let alive = t.account_exists(acc);
    let spent: Vec<i128> = pays
        .iter()
        .zip(wallet_before)
        .map(|((n, a), b)| b + a - t.token_balance_raw(LIQUIDATOR, n))
        .collect();
    let received: [i128; 3] =
        std::array::from_fn(|i| t.token_balance_raw(LIQUIDATOR, COLL[i]) - coll_wallet_before[i]);
    Outcome {
        spent,
        received,
        fees,
        c_after: if alive { coll(t, acc) } else { 0 },
        d_after: if alive { debt(t, acc) } else { 0 },
        alive,
        est,
    }
}

// ---------------------------------------------------------------------------
// R-3: insolvent book, the seize_all tolerance, sweep across the edge
// ---------------------------------------------------------------------------

/// Insolvent book: `Q = floor(C / (1 + base))`. An untrimmed offer whose kept
/// value is within one native unit of every kept leg of `Q` seizes every unit
/// (MC-3). Below that edge the seizure is proportional at the base bonus.
/// Bound: the liquidator's gain over paying `Q` is at most the sum of those
/// units, once, and what it takes is never above `C`.
#[test]
fn rv_round_r3_insolvent_seize_all_edge_sweep_gain_is_at_most_one_unit_per_kept_leg() {
    let t = book(8_500);
    let acc = acct(&t);
    let (c, d) = (coll(&t, acc), debt(&t, acc));
    assert!(c < d, "fixture must be insolvent: C={c} D={d}");
    let base = estimate(&t, acc, &open_over_offers(&t, acc)).bonus_rate_bps;
    let q = backed_quote(&t, c, base);
    let units: i128 = DEBT.iter().map(|n| unit_usd(&t, n)).sum();
    let gold = debt_ceil(&t, acc, "GOLD");
    let xlm = debt_ceil(&t, acc, "XLM");
    let dust = debt_ceil(&t, acc, "DUST");
    let fixed_usd = usd_of(&t, "GOLD", gold) + usd_of(&t, "XLM", xlm) + usd_of(&t, "DUST", dust);
    let usdt_unit = unit_usd(&t, "USDT");
    std::println!("R3 edge sweep: C={c} D={d} base={base} Q={q} units={units}");

    let steps = 24i128;
    let lo = q - units - 2 * usdt_unit;
    let span = units + 4 * usdt_unit;
    let mut max_gain_all = 0i128;
    let mut fired = 0;
    for k in 0..=steps {
        let target = lo + span * k / steps;
        let usdt = tokens_of(&t, "USDT", target - fixed_usd);
        let pays = [("USDT", usdt), ("GOLD", gold), ("XLM", xlm), ("DUST", dust)];
        let est = estimate(&t, acc, &pays);
        let kept = est.max_payment_wad;
        let planned: i128 = pays.iter().map(|(n, a)| usd_of(&t, n, *a)).sum();
        if planned <= q {
            assert_eq!(kept, planned, "an offer at or below Q is never trimmed");
        } else {
            // Trimmed from the last leg backward: DUST is dropped, XLM is split
            // and floored, so the kept value sits below Q by under one XLM unit.
            assert!(kept <= q, "kept {kept} above Q {q}");
            assert!(
                q - kept <= unit_usd(&t, "XLM") + unit_usd(&t, "DUST") + ULP,
                "over-Q offer trimmed by {} raw WAD, more than one unit of the split leg",
                q - kept
            );
        }
        let seized_usd = coll_usd(&t, by_collateral(&t, &est.seized_collaterals));
        let all = kept + units >= q;
        if all {
            fired += 1;
            // Every unit: the planned full-close amounts are the half-up balances.
            assert!(
                seized_usd + collateral_units(&t) + ULP >= c && seized_usd <= c + ULP,
                "seize_all at kept={kept}: seized {seized_usd} is not the whole collateral {c}"
            );
            max_gain_all = max_gain_all.max(q - kept);
        } else {
            let fair = with_bonus(kept, base);
            assert!(
                seized_usd <= fair + ULP,
                "below the edge at kept={kept}: seized {seized_usd} above repaid*(1+base) {fair}"
            );
            assert!(
                seized_usd + collateral_units(&t) + ULP >= fair,
                "below the edge at kept={kept}: seized {seized_usd} more than one unit per \
                 collateral leg below {fair}"
            );
        }
    }
    assert!(
        fired > 0 && fired < steps as usize + 1,
        "sweep must straddle the edge"
    );
    std::println!(
        "R3 edge sweep: seize_all fired at {fired}/{} points; max gain over Q while taking \
         everything = {max_gain_all} raw WAD (bound {units})",
        steps + 1
    );
    assert!(
        max_gain_all <= units,
        "gain {max_gain_all} above one unit per kept leg {units}"
    );

    // Execute at the edge: the largest under-payment the tolerance admits.
    let mut t = t;
    let usdt = tokens_of(&t, "USDT", q - units + 1 - fixed_usd) + 1;
    let pays = [("USDT", usdt), ("GOLD", gold), ("XLM", xlm), ("DUST", dust)];
    let out = run(&mut t, acc, &pays);
    let charged = out.charged_usd(&t, &pays);
    let gain = q - charged;
    let taken = out.taken_usd(&t);
    std::println!(
        "R3 edge exec: spent={:?} charged={charged} gain={gain} taken={taken} C={c} \
         received={:?} fees={:?} alive={} extra socialized over a Q close = {gain}",
        out.spent,
        out.received,
        out.fees,
        out.alive
    );
    assert_eq!(
        out.spent,
        pays.map(|(_, a)| a).to_vec(),
        "no trim, no refund"
    );
    assert!(
        !out.alive,
        "insolvent close with nothing left is cleaned up"
    );
    assert!(
        gain > 0 && gain <= units,
        "gain {gain} outside (0, {units}]"
    );
    // What leaves the pool is the whole collateral less the payout floors, and
    // the only excess over the base-bonus deal is the tolerance at the bonus.
    assert!(taken <= c + ULP && taken + collateral_units(&t) + ULP >= c);
    assert!(
        taken <= with_bonus(charged, base) + with_bonus(units, base) + ULP,
        "taken {taken} exceeds (charged + units) * (1 + base)"
    );
}

/// Insolvent over-offer with the coarse GOLD leg last: the trim floors GOLD's
/// kept amount, so the kept value is below `Q` by less than one GOLD unit
/// and `seize_all` still fires. When the kept GOLD amount floors to zero the
/// leg is dropped, the tolerance shrinks to the remaining legs, and the
/// seizure falls back to the proportional base-bonus deal.
#[test]
fn rv_round_r3_insolvent_over_offer_trim_floors_the_split_leg_by_less_than_one_unit() {
    let t = book(8_500);
    let acc = acct(&t);
    let c = coll(&t, acc);
    let base = estimate(&t, acc, &open_over_offers(&t, acc)).bonus_rate_bps;
    let q = backed_quote(&t, c, base);
    let gold_unit = unit_usd(&t, "GOLD");
    let xlm = debt_ceil(&t, acc, "XLM");
    let dust = debt_ceil(&t, acc, "DUST");
    let gold = debt_ceil(&t, acc, "GOLD");
    let usdt_full = debt_ceil(&t, acc, "USDT");
    let others_fixed = usd_of(&t, "XLM", xlm) + usd_of(&t, "DUST", dust);

    // Walk the USDT offer so that (Q - others) mod gold_unit sweeps one unit.
    let mut worst = (0i128, 0i128);
    let steps = 24i128;
    for k in 0..steps {
        let rem_usd = q - others_fixed - 4 * gold_unit - gold_unit * k / steps;
        let usdt = tokens_of(&t, "USDT", rem_usd).min(usdt_full);
        let pays = [
            ("USDT", usdt),
            ("XLM", xlm),
            ("DUST", dust),
            ("GOLD", 2 * gold),
        ];
        let est = estimate(&t, acc, &pays);
        let kept = est.max_payment_wad;
        let delta = q - kept;
        let seized_usd = coll_usd(&t, by_collateral(&t, &est.seized_collaterals));
        assert!(kept <= q, "kept {kept} above Q {q}");
        assert!(
            delta < gold_unit,
            "trim floored by {delta}, a whole GOLD unit or more ({gold_unit})"
        );
        assert!(
            seized_usd + collateral_units(&t) + ULP >= c,
            "seize_all must fire after the trim: seized {seized_usd} vs C {c}"
        );
        if delta > worst.0 {
            worst = (delta, usdt);
        }
    }
    std::println!(
        "R3 trim sweep: worst floor {} raw WAD (GOLD unit {gold_unit}) at usdt={}",
        worst.0,
        worst.1
    );

    // Dropped leg, on a book whose USDT debt alone covers Q: the remainder for
    // GOLD is below one unit, GOLD's kept amount floors to zero and the leg
    // leaves the plan with its whole offer refunded.
    let t = book_with(8_500, 10_000 * 10_000_000, 2, 10_000 * 10_000_000);
    let acc = acct(&t);
    let c = coll(&t, acc);
    let base = estimate(&t, acc, &open_over_offers(&t, acc)).bonus_rate_bps;
    let q = backed_quote(&t, c, base);
    let xlm = debt_ceil(&t, acc, "XLM");
    let dust = debt_ceil(&t, acc, "DUST");
    let gold = debt_ceil(&t, acc, "GOLD");
    let usdt_full = debt_ceil(&t, acc, "USDT");
    let others_fixed = usd_of(&t, "XLM", xlm) + usd_of(&t, "DUST", dust);
    let rem_usd = q - others_fixed - gold_unit * 9 / 10;
    let usdt = tokens_of(&t, "USDT", rem_usd);
    assert!(
        usdt < usdt_full,
        "USDT must cover the remainder without capping"
    );
    let pays = [
        ("USDT", usdt),
        ("XLM", xlm),
        ("DUST", dust),
        ("GOLD", 2 * gold),
    ];
    let est = estimate(&t, acc, &pays);
    let kept = est.max_payment_wad;
    let delta = q - kept;
    let seized_usd = coll_usd(&t, by_collateral(&t, &est.seized_collaterals));
    let gold_refund: i128 = est
        .refunds
        .iter()
        .filter(|r| r.asset == t.resolve_asset("GOLD"))
        .map(|r| r.amount)
        .sum();
    let rest_units = unit_usd(&t, "USDT") + unit_usd(&t, "XLM") + unit_usd(&t, "DUST");
    std::println!(
        "R3 dropped leg: kept={kept} delta={delta} gold_refund={gold_refund} seized={seized_usd} \
         C={c} tolerance without GOLD={rest_units}"
    );
    assert_eq!(gold_refund, 2 * gold, "the whole GOLD offer is refunded");
    assert!(delta < gold_unit && delta > rest_units);
    let fair = with_bonus(kept, base);
    assert!(
        seized_usd <= fair + ULP && seized_usd + collateral_units(&t) + ULP >= fair,
        "without the tolerance the seizure is proportional: {seized_usd} vs {fair}"
    );
    // Execute it: the liquidator gets the base-bonus deal, the residue stays
    // with the (still insolvent) account for the next liquidation or cleanup.
    let mut t = t;
    let out = run(&mut t, acc, &pays);
    let charged = out.charged_usd(&t, &pays);
    let taken = out.taken_usd(&t);
    std::println!(
        "R3 dropped leg exec: charged={charged} taken={taken} fair={} C_after={} D_after={} alive={}",
        with_bonus(charged, base),
        out.c_after,
        out.d_after,
        out.alive
    );
    assert!(charged <= q && taken <= with_bonus(charged, base) + ULP);
    assert!(out.alive && out.c_after + taken + collateral_units(&t) + ULP >= c);
}

// ---------------------------------------------------------------------------
// R-3: solvent partial quote, every leg order, and a quote below one coarse unit
// ---------------------------------------------------------------------------

/// All 24 leg orders of a 2x over-offer on a solvent partial quote: the kept
/// value exceeds the ideal by less than one native unit of the split leg (the
/// last leg the trim reaches), plus one USDT unit of quote slack. The dust leg
/// steers nothing.
#[test]
fn rv_round_r3_solvent_over_offer_every_leg_order_rounds_up_below_one_unit_of_the_split_leg() {
    let t = book(12_035);
    let acc = acct(&t);
    let d = debt(&t, acc);
    let ceils: Vec<(&str, i128)> = DEBT.iter().map(|n| (*n, debt_ceil(&t, acc, n))).collect();
    let fine = estimate(
        &t,
        acc,
        &[
            ("GOLD", 2_000),
            ("XLM", 2 * ceils[2].1),
            ("DUST", 2),
            ("USDT", 2 * ceils[0].1),
        ],
    );
    let ideal_hi = fine.max_payment_wad;
    let ideal_lo = ideal_hi - unit_usd(&t, "USDT");
    assert!(ideal_hi < d, "fixture must quote a partial repayment");
    std::println!(
        "R3 orders: D={d} ideal in [{ideal_lo}, {ideal_hi}] bonus={}",
        fine.bonus_rate_bps
    );

    let mut worst = (0i128, "");
    let mut perms: Vec<[usize; 4]> = Vec::new();
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for e in 0..4 {
                    if a != b && a != c && a != e && b != c && b != e && c != e {
                        perms.push([a, b, c, e]);
                    }
                }
            }
        }
    }
    assert_eq!(perms.len(), 24);
    for perm in perms {
        let pays: Vec<(&str, i128)> = perm.iter().map(|&i| (ceils[i].0, 2 * ceils[i].1)).collect();
        let est = estimate(&t, acc, &pays);
        let kept = est.max_payment_wad;
        // Every 2x offer refunds its half above the ceiling; the split leg is
        // the one whose refund is strictly between that cap and the whole offer.
        let mut split = "";
        for (n, offered) in &pays {
            let refunded: i128 = est
                .refunds
                .iter()
                .filter(|r| r.asset == t.resolve_asset(n))
                .map(|r| r.amount)
                .sum();
            if refunded > offered / 2 && refunded < *offered {
                assert!(split.is_empty(), "at most one leg is split");
                split = n;
            }
        }
        let over = kept - ideal_lo;
        let bound = if split.is_empty() {
            0
        } else {
            unit_usd(&t, split)
        };
        assert!(
            kept + ULP >= ideal_lo,
            "kept {kept} below the ideal {ideal_lo}"
        );
        assert!(
            over <= bound + unit_usd(&t, "USDT") + 4 * ULP,
            "order {pays:?}: kept {over} above the ideal, more than one unit of {split:?} ({bound})"
        );
        if over > worst.0 {
            worst = (over, split);
        }
    }
    std::println!(
        "R3 orders: worst over-ideal {} raw WAD, split leg {}",
        worst.0,
        worst.1
    );
    assert!(worst.0 < unit_usd(&t, "GOLD") + unit_usd(&t, "USDT"));
}

/// A solvent book whose GOLD debt leg is larger than the curve quote: an
/// over-offer of GOLD alone keeps whole units, so the kept value rounds up by
/// less than one GOLD unit above the quote (measured with a fine XLM-only
/// offer). The borrower pays the bonus on that extra once; the account then
/// leaves the liquidatable range. The 60% threshold keeps the quote below half
/// the debt so both measurements are possible on one book.
#[test]
fn rv_round_r3_solvent_quote_below_the_coarse_leg_rounds_up_below_one_unit_once() {
    let mut t = build_with(5_000, 6_000);
    t.supply_raw(ALICE, "USDC", 420 * 10_000_000);
    t.borrow_raw(ALICE, "GOLD", 17);
    t.borrow_raw(ALICE, "XLM", 900 * 10_000_000);
    t.borrow_raw(ALICE, "USDT", 8 * 10_000_000);
    t.get_or_create_user(LIQUIDATOR);
    let acc = acct(&t);
    let gold_unit = unit_usd(&t, "GOLD");
    let xlm_unit = unit_usd(&t, "XLM");
    let gold_cap = usd_of(&t, "GOLD", 17);
    let xlm_cap = usd_of(&t, "XLM", 900 * 10_000_000);
    let usdc0 = px(&t, "USDC");
    let mut worst = (0i128, 0i128, 0i128);
    let mut points = 0;
    // Walk the USDC price so the quote sweeps across several GOLD units.
    for k in 0..90i128 {
        let p = usdc0 * (7_930 - k) / 10_000;
        t.set_price("USDC", p);
        if hf(&t, acc) >= WAD {
            continue;
        }
        let d = debt(&t, acc);
        // Fine quote: XLM alone, split and rounded up by at most one XLM unit.
        let fine = estimate(&t, acc, &[("XLM", 2 * 900 * 10_000_000)]).max_payment_wad;
        if fine >= d || fine >= xlm_cap || fine >= gold_cap {
            continue;
        }
        let ideal_lo = fine - xlm_unit;
        let kept = estimate(&t, acc, &[("GOLD", 34)]).max_payment_wad;
        points += 1;
        let over = kept - ideal_lo;
        assert!(
            kept + ULP >= ideal_lo,
            "kept {kept} below the ideal {ideal_lo}"
        );
        assert!(
            over < gold_unit + xlm_unit + ULP,
            "price {p}: kept {kept} is a whole GOLD unit or more above the ideal {ideal_lo}"
        );
        assert_eq!(kept % gold_unit, 0, "GOLD alone keeps whole units");
        if over > worst.0 {
            worst = (over, p, ideal_lo);
        }
    }
    let (over, p, ideal_lo) = worst;
    std::println!(
        "R3 coarse sweep: {points} points, worst round-up {over} raw WAD (unit {gold_unit})"
    );
    assert!(
        points >= 20 && over > gold_unit * 3 / 4,
        "sweep must reach a sizeable round-up"
    );
    t.set_price("USDC", p);
    let pays = [("GOLD", 34)];
    let out = run(&mut t, acc, &pays);
    let charged = out.charged_usd(&t, &pays);
    let b = out.est.bonus_rate_bps;
    std::println!(
        "R3 coarse round-up: ideal_lo={ideal_lo} charged={charged} over={} ({:.4}x the quote) \
         bonus={b} extra bonus on the round-up={} taken={} hf_after={} liquidatable={}",
        charged - ideal_lo,
        charged as f64 / ideal_lo as f64,
        (charged - ideal_lo) * b / BPS,
        out.taken_usd(&t),
        hf(&t, acc),
        t.can_be_liquidated(ALICE)
    );
    assert!(charged - ideal_lo < gold_unit + xlm_unit + ULP);
    assert!(out.taken_usd(&t) <= with_bonus(charged, b) + ULP);
    assert!(
        !t.can_be_liquidated(ALICE),
        "one round-up and the account is healthy"
    );
}

// ---------------------------------------------------------------------------
// R-3: full close after accrual, each leg charged at its ceiling
// ---------------------------------------------------------------------------

/// Thirty days of accrual leave every debt leg fractional. A band quote closes
/// in full at each leg's ceiling: the liquidator is charged at most one native
/// unit per leg above the ceil-valued debt, the pool refunds exactly the
/// offered excess, and what leaves the pool never exceeds the collateral.
#[test]
fn rv_round_r3_band_full_close_after_accrual_charges_at_most_one_unit_per_leg_over_debt() {
    let mut t = book(10_300);
    t.advance_time(30 * 86_400);
    let acc = acct(&t);
    let (c, d) = (coll(&t, acc), debt(&t, acc));
    assert!(c >= d && hf(&t, acc) < WAD, "band fixture: C={c} D={d}");
    let order = ["XLM", "DUST", "GOLD", "USDT"];
    let pays: Vec<(&str, i128)> = order
        .iter()
        .map(|n| (*n, 2 * debt_ceil(&t, acc, n)))
        .collect();
    let ceils: Vec<i128> = order.iter().map(|n| debt_ceil(&t, acc, n)).collect();
    // Exact (unrounded) debt per leg, in raw WAD, floored at 1e-18 USD.
    let exact: i128 = order
        .iter()
        .map(|n| {
            let scaled = debt_scaled(&t, acc, n);
            let index = t.ctrl_client().get_market_index(&key(&t, n)).borrow_index;
            let ray = Ray::from(scaled).mul_floor(&t.env, Ray::from(index));
            ray.to_wad(&t.env)
                .mul_floor(&t.env, Wad::from(px(&t, n)))
                .raw()
        })
        .sum();
    let out = run(&mut t, acc, &pays);
    let charged = out.charged_usd(&t, &pays);
    let units: i128 = DEBT.iter().map(|n| unit_usd(&t, n)).sum();
    let taken = out.taken_usd(&t);
    std::println!(
        "R3 accrued close: C={c} D={d} exact={exact} bonus={} ceils={ceils:?} spent={:?} \
         charged={charged} over_D={} over_exact={} units={units} taken={taken} alive={}",
        out.est.bonus_rate_bps,
        out.spent,
        charged - d,
        charged - exact,
        out.alive
    );
    assert_eq!(
        out.spent, ceils,
        "a full close pulls exactly each ceiled debt"
    );
    assert!(
        charged + ULP >= d,
        "charged {charged} below the ceil-valued debt {d}"
    );
    assert!(
        charged - d <= units + ULP,
        "charged {} above D, more than one unit per leg",
        charged - d
    );
    assert!(charged - exact > 0 && charged - exact <= units + ULP);
    assert_eq!(out.d_after, 0, "the debt closes");
    assert!(
        taken <= c + ULP,
        "the pool never pays more than the held collateral"
    );
    assert!(taken <= with_bonus(charged, out.est.bonus_rate_bps) + ULP);
}

// ---------------------------------------------------------------------------
// R-4: chains of partials across three collateral legs
// ---------------------------------------------------------------------------

struct Chain {
    repaid_tokens: [i128; 4],
    received: [i128; 3],
    fees: [i128; 3],
    steps: usize,
    bonuses: Vec<i128>,
}

/// Runs up to `steps` partial liquidations, each paying `num/den` of the
/// current quote in one open debt leg, cycling USDT, GOLD, XLM.
fn chain(t: &mut LendingTest, acc: u64, steps: usize, num: i128, den: i128) -> Chain {
    let mut repaid = [0i128; 4];
    let mut received = [0i128; 3];
    let mut fees = [0i128; 3];
    let mut bonuses = Vec::new();
    let cycle = ["USDT", "GOLD", "XLM"];
    let mut done = 0;
    for step in 0..steps {
        if !t.account_exists(acc) || !t.can_be_liquidated(ALICE) {
            break;
        }
        let Some(leg) = (0..cycle.len())
            .map(|k| cycle[(step + k) % cycle.len()])
            .find(|leg| debt_ceil(t, acc, leg) > 0)
        else {
            break;
        };
        let quote = estimate(t, acc, &open_over_offers(t, acc)).max_payment_wad;
        let want = tokens_of(t, leg, quote * num / den);
        let amount = want.min(debt_ceil(t, acc, leg)).max(1);
        let pays = [(leg, amount)];
        let out = run(t, acc, &pays);
        let i = DEBT.iter().position(|n| *n == leg).unwrap();
        repaid[i] += out.spent[0];
        for k in 0..3 {
            received[k] += out.received[k];
            fees[k] += out.fees[k];
        }
        bonuses.push(out.est.bonus_rate_bps);
        done += 1;
    }
    Chain {
        repaid_tokens: repaid,
        received,
        fees,
        steps: done,
        bonuses,
    }
}

/// One liquidation on a fresh book paying the chain's per-leg totals.
fn one_close(
    coverage_bps: i128,
    repaid: [i128; 4],
) -> (LendingTest, Outcome, Vec<(&'static str, i128)>) {
    let mut t = book(coverage_bps);
    let acc = acct(&t);
    let pays: Vec<(&str, i128)> = DEBT
        .iter()
        .zip(repaid)
        .filter(|(_, a)| *a > 0)
        .map(|(n, a)| (*n, a))
        .collect();
    let out = run(&mut t, acc, &pays);
    (t, out, pays)
}

fn sum_usd(t: &LendingTest, names: &[&str], tokens: &[i128]) -> i128 {
    names
        .iter()
        .zip(tokens)
        .map(|(n, a)| usd_of(t, n, *a))
        .sum()
}

/// Compares a chain against one close of the same per-leg totals. Returns the
/// chain's excess over one close in raw WAD (negative when the chain took less).
fn compare(label: &str, coverage_bps: i128, steps: usize, num: i128, den: i128) -> i128 {
    let mut c = book(coverage_bps);
    let acc = acct(&c);
    let (c0, d0) = (coll(&c, acc), debt(&c, acc));
    let ch = chain(&mut c, acc, steps, num, den);
    let (c1, d1) = if c.account_exists(acc) {
        (coll(&c, acc), debt(&c, acc))
    } else {
        (0, 0)
    };
    let chain_repaid = sum_usd(&c, &DEBT, &ch.repaid_tokens);
    let chain_taken = sum_usd(&c, &COLL, &ch.received) + sum_usd(&c, &COLL, &ch.fees);
    let chain_received = sum_usd(&c, &COLL, &ch.received);

    let (s, out, pays) = one_close(coverage_bps, ch.repaid_tokens);
    let single_charged = out.charged_usd(&s, &pays);
    let single_taken = out.taken_usd(&s);
    let single_received = coll_usd(&s, out.received);
    assert_eq!(
        out.spent,
        pays.iter().map(|(_, a)| *a).collect::<Vec<_>>(),
        "[{label}] the one-close offer must fit inside its own quote (no trim)"
    );
    let unit_slack = collateral_units(&c) * ch.steps as i128;
    let bps_slack = 2 * chain_repaid / BPS;
    let diff = chain_taken - single_taken;
    let first = ch.bonuses[0];
    let max_bonus = *ch.bonuses.iter().max().unwrap();
    std::println!(
        "R4 {label}: steps={} bonuses {first}..{max_bonus} (creep {}) repaid chain={chain_repaid} \
         single={single_charged} | taken chain={chain_taken} single={single_taken} diff={diff} \
         ({:.3} bps of repaid; bound {}) | received chain={chain_received} single={single_received} \
         | C/D {:.9}->{:.9}, single after {:.9}",
        ch.steps,
        max_bonus - first,
        diff as f64 * BPS as f64 / chain_repaid as f64,
        bps_slack + unit_slack,
        c0 as f64 / d0 as f64,
        c1 as f64 / d1.max(1) as f64,
        out.c_after as f64 / out.d_after.max(1) as f64
    );
    assert!(
        (chain_repaid - single_charged).abs() <= 8 * ULP,
        "[{label}] both paths must retire the same value"
    );
    assert!(
        chain_taken <= single_taken + bps_slack + unit_slack + 8 * ULP,
        "[{label}] the chain took {diff} raw WAD more collateral than one close; bound {}",
        bps_slack + unit_slack
    );
    assert!(
        chain_received <= single_received + bps_slack + unit_slack + 8 * ULP,
        "[{label}] the chain out-earned one close by {} raw WAD",
        chain_received - single_received
    );
    if c0 >= d0 && d1 > 0 {
        assert!(
            c1 as f64 / d1 as f64 >= c0 as f64 / d0 as f64 * (1.0 - 1e-9),
            "[{label}] C/D fell over the chain"
        );
    }
    diff
}

/// Band book: thirty partials of 5% of the running quote repay about 78% of the
/// debt; the BPS floor of the HF-preserving cap creeps up by at most a couple
/// of BPS over the chain, so the chain beats one close by well under 2 BPS.
#[test]
fn rv_round_r4_band_chain_of_thirty_partials_beats_one_close_by_less_than_the_bps_floor() {
    let diff = compare("band 30x5%", 10_300, 30, 5, 100);
    std::println!("R4 band 30x5% excess over one close: {diff} raw WAD");
}

#[test]
fn rv_round_r4_band_chain_of_twelve_partials_alternating_debt_legs() {
    compare("band 12x8%", 10_300, 12, 8, 100);
}

/// Curve book: the bonus falls as HF rises, so a chain earns strictly less than
/// one close at the opening HF.
#[test]
fn rv_round_r4_curve_chain_of_twelve_partials_earns_less_than_one_close() {
    let diff = compare("curve 12x1/12", 12_035, 12, 1, 12);
    assert!(
        diff <= 0,
        "a curve chain must not out-earn one close: {diff}"
    );
}

/// Insolvent book: four partials at the base bonus, then an edge close that
/// fires `seize_all`. The chain's total charge is within the tolerance of a
/// single edge close, and both take all the collateral: the tolerance is paid
/// once per account, not once per step.
#[test]
fn rv_round_r4_insolvent_chain_pays_the_seize_all_tolerance_once() {
    let mut t = book(8_500);
    let acc = acct(&t);
    let c0 = coll(&t, acc);
    let base = estimate(&t, acc, &open_over_offers(&t, acc)).bonus_rate_bps;
    let q0 = backed_quote(&t, c0, base);
    let ch = chain(&mut t, acc, 4, 1, 5);
    assert_eq!(ch.steps, 4);
    assert!(
        ch.bonuses.iter().all(|b| *b == base),
        "insolvent slices pay the base bonus"
    );
    let partial_charged = sum_usd(&t, &DEBT, &ch.repaid_tokens);
    let partial_taken = sum_usd(&t, &COLL, &ch.received) + sum_usd(&t, &COLL, &ch.fees);
    assert!(partial_taken <= with_bonus(partial_charged, base) + 4 * ULP);

    // Edge close on what is left.
    let c1 = coll(&t, acc);
    let q1 = backed_quote(&t, c1, base);
    let open: Vec<&str> = DEBT
        .iter()
        .copied()
        .filter(|n| debt_ceil(&t, acc, n) > 0)
        .collect();
    let units: i128 = open.iter().map(|n| unit_usd(&t, n)).sum();
    let mut fixed_usd = 0i128;
    let mut pays: Vec<(&str, i128)> = Vec::new();
    for n in &open {
        if *n != "USDT" {
            let a = debt_ceil(&t, acc, n);
            fixed_usd += usd_of(&t, n, a);
            pays.push((n, a));
        }
    }
    let usdt = tokens_of(&t, "USDT", q1 - units + 1 - fixed_usd) + 1;
    pays.insert(0, ("USDT", usdt));
    let out = run(&mut t, acc, &pays);
    let edge_charged = out.charged_usd(&t, &pays);
    let edge_taken = out.taken_usd(&t);
    let total_charged = partial_charged + edge_charged;
    let total_taken = partial_taken + edge_taken;
    std::println!(
        "R4 insolvent chain: Q0={q0} partial charged={partial_charged} taken={partial_taken} | \
         Q1={q1} edge charged={edge_charged} gain={} units={units} taken={edge_taken} alive={} | \
         total charged={total_charged} (Q0 - total = {}) taken={total_taken} C0={c0}",
        q1 - edge_charged,
        out.alive,
        q0 - total_charged
    );
    assert!(
        !out.alive,
        "the edge close takes everything and the account is cleaned up"
    );
    assert!(q1 - edge_charged > 0 && q1 - edge_charged <= units);
    // Everything the book held left the pool, less the payout floors per step.
    let slack = collateral_units(&t) * 5;
    assert!(total_taken <= c0 + ULP && total_taken + slack + ULP >= c0);
    // The whole chain under-pays the opening quote by at most the tolerance plus
    // the per-step quote floors.
    assert!(q0 - total_charged <= units + 5 * ULP + slack);
    assert!(total_charged <= q0 + ULP);
}
