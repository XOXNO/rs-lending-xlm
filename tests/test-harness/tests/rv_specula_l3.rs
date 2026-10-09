//! Second opinion on Specula lending MC-3: on an insolvent account the plan
//! sets `seize_all` when the kept repayment plus one native unit of every
//! repaid debt leg reaches the collateral-backed quote
//! (`normalize_repayment_plan`, `one_unit_per_leg_usd`). The slack counts the
//! unit of each repaid leg, trimmed or not, so an offer that is never trimmed
//! can sit up to that sum below the quote and still take every collateral
//! unit. The lenders socialize the shortfall in the same call.
//!
//! The probe lists a 3-decimal borrowable market priced at $6,000 (one base
//! unit is $6; `MIN_BORROWABLE_ASSET_DECIMALS` admits it), borrows it next to
//! USDT against USDC, prices the USDC into `C < D`, and offers exactly
//! `Q - R + 2` raw WAD with the GOLD leg paid at its full ceiling. The offer
//! is below the quote, so nothing is trimmed, yet the GOLD unit widens the
//! tolerance by $6. One USDT unit less and the same call is a partial
//! seizure. Both runs pin the HEAD behaviour; the `C_after == 0` assertion of
//! the first run is the one that must flip once the tolerance is narrowed to
//! the floor shortfall of the trimmed leg.

use common::math::fp::Ray;
use common::math::fp_core::mul_div_floor;
use common::rates::unscale_borrow_ceil;
use common::types::{HubAssetKey, LiquidationEstimate, SeizeMode};
use controller::constants::{BPS, WAD};
use soroban_sdk::Vec as SVec;
use test_harness::{
    hub_asset, usd, usdc_preset, usdt_stable_preset, LendingTest, MarketPreset, ALICE,
    DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, LIQUIDATOR,
};

const GOLD: &str = "GOLD3";
const USDC: &str = "USDC";
const USDT: &str = "USDT";
const USDC_UNIT: i128 = 10_000_000;
/// Raw WAD slack for the half-up versus floor valuation steps.
const ULP: i128 = 1_000;

/// The coarsest borrowable listing the controller admits, priced so that one
/// base unit is worth $6.
fn gold_preset() -> MarketPreset {
    MarketPreset {
        name: GOLD,
        decimals: 3,
        price_wad: usd(6_000),
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

fn key(t: &LendingTest, name: &str) -> HubAssetKey {
    hub_asset(t.resolve_asset(name))
}

/// Token units to raw WAD USD, floored, with an exact wide intermediate.
fn usd_of(t: &LendingTest, name: &str, tokens: i128) -> i128 {
    let m = t.resolve_market(name);
    mul_div_floor(&t.env, tokens, m.price_wad, 10i128.pow(m.decimals))
}

fn unit_usd(t: &LendingTest, name: &str) -> i128 {
    usd_of(t, name, 1)
}

/// The leg's full-close amount: ceil at both conversion steps, as the plan caps it.
fn debt_ceil(t: &LendingTest, acc: u64, name: &str) -> i128 {
    let (_, borrows) = t.ctrl_client().get_account_positions(&acc);
    let scaled = borrows.get(key(t, name)).map_or(0, |p| p.scaled_amount);
    let index = t.ctrl_client().get_market_index(&key(t, name)).borrow_index;
    unscale_borrow_ceil(
        &t.env,
        Ray::from(scaled),
        Ray::from(index),
        t.resolve_market(name).decimals,
    )
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

/// ALICE: 10,000 USDC against 1.000 GOLD3 ($6,000) and 1,400 USDT, then USDC
/// at $0.70 so that `C = 7,000 < D`.
fn insolvent_book() -> (LendingTest, u64) {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(gold_preset())
        .with_market(usdt_stable_preset())
        .build();
    t.supply_raw(ALICE, USDC, 10_000 * USDC_UNIT);
    t.borrow_raw(ALICE, GOLD, 1_000);
    t.borrow_raw(ALICE, USDT, 1_400 * USDC_UNIT);
    t.set_price(USDC, usd(7) / 10);
    t.get_or_create_user(LIQUIDATOR);
    t.assert_liquidatable(ALICE);
    let acc = t.resolve_account_id(ALICE);
    (t, acc)
}

struct Outcome {
    /// Net tokens the liquidator spent per payment leg, in `pays` order.
    spent: Vec<i128>,
    /// Net USDC the liquidator received.
    received: i128,
    /// Planned USDC protocol fee.
    fee: i128,
    c_after: i128,
    d_after: i128,
    alive: bool,
}

/// Mints the offers, liquidates in transfer mode and measures wallets and books.
fn run(t: &mut LendingTest, acc: u64, pays: &[(&str, i128)]) -> Outcome {
    let est = estimate(t, acc, pays);
    let usdc = t.resolve_asset(USDC);
    let fee = est
        .protocol_fees
        .iter()
        .filter(|p| p.asset == usdc)
        .map(|p| p.amount)
        .sum();
    let addr = t.get_or_create_user(LIQUIDATOR);
    let wallet_before: Vec<i128> = pays
        .iter()
        .map(|(n, _)| t.token_balance_raw(LIQUIDATOR, n))
        .collect();
    for (n, a) in pays {
        t.resolve_market(n).token_admin.mint(&addr, a);
    }
    let usdc_before = t.token_balance_raw(LIQUIDATOR, USDC);
    t.ctrl_client()
        .liquidate(&addr, &acc, &pay_vec(t, pays), &SeizeMode::Transfer);
    t.assert_spoke_usage_matches_positions();
    let spent = pays
        .iter()
        .zip(wallet_before)
        .map(|((n, a), b)| b + a - t.token_balance_raw(LIQUIDATOR, n))
        .collect();
    let alive = t.account_exists(acc);
    let (c_after, d_after) = if alive {
        (
            t.ctrl_client().get_total_collateral_usd(&acc),
            t.ctrl_client().get_total_borrow_usd(&acc),
        )
    } else {
        (0, 0)
    };
    Outcome {
        spent,
        received: t.token_balance_raw(LIQUIDATOR, USDC) - usdc_before,
        fee,
        c_after,
        d_after,
        alive,
    }
}

/// `(quote, base bonus, one unit of each debt leg)` for the insolvent book:
/// `Q = floor(C / (1 + base))`.
fn quote(t: &LendingTest, acc: u64) -> (i128, i128, i128) {
    let c = t.ctrl_client().get_total_collateral_usd(&acc);
    let d = t.ctrl_client().get_total_borrow_usd(&acc);
    assert!(c < d, "fixture must be insolvent: C={c} D={d}");
    let probe = estimate(t, acc, &[(GOLD, 1), (USDT, 1)]);
    let base = probe.bonus_rate_bps;
    let q = mul_div_floor(&t.env, c, WAD, WAD + base * WAD / BPS);
    let units = unit_usd(t, GOLD) + unit_usd(t, USDT);
    std::println!("L3 book: C={c} D={d} base={base} Q={q} R={units}");
    (q, base, units)
}

/// The largest under-payment the tolerance admits: GOLD paid in full at its
/// ceiling, USDT sized so that the total lands at `Q - R + 2` raw WAD.
fn edge_offer(t: &LendingTest, acc: u64, q: i128, units: i128) -> [(&'static str, i128); 2] {
    let gold = debt_ceil(t, acc, GOLD);
    let target_usdt_usd = q - units + 2 - usd_of(t, GOLD, gold);
    assert!(target_usdt_usd > 0, "GOLD alone must not reach the edge");
    let px = t.resolve_market(USDT).price_wad;
    let usdt = (target_usdt_usd * USDC_UNIT + px - 1) / px;
    assert!(usdt < debt_ceil(t, acc, USDT), "USDT leg must stay partial");
    [(GOLD, gold), (USDT, usdt)]
}

/// An offer one native unit of every repaid leg below the insolvent quote,
/// with no leg trimmed, still seizes every collateral unit. The GOLD leg is
/// paid at its ceiling and is never trimmed, yet its $6 unit widens the
/// tolerance. The liquidator takes `C` for `Q - R`; the lenders socialize the
/// residual debt, which is `R` larger than at the quote.
#[test]
fn rv_specula_l3_untrimmed_offer_one_unit_per_leg_below_quote_seizes_all() {
    let (mut t, acc) = insolvent_book();
    let (q, base, units) = quote(&t, acc);
    let c = t.ctrl_client().get_total_collateral_usd(&acc);
    let d = t.ctrl_client().get_total_borrow_usd(&acc);
    let pays = edge_offer(&t, acc, q, units);
    let planned: i128 = pays.iter().map(|(n, a)| usd_of(&t, n, *a)).sum();
    assert!(
        planned < q && planned + units >= q,
        "offer must sit inside the tolerance: planned={planned} Q={q} R={units}"
    );
    assert!(
        q - planned > unit_usd(&t, GOLD),
        "the under-payment must exceed the untrimmed GOLD unit"
    );

    let est = estimate(&t, acc, &pays);
    assert_eq!(
        est.max_payment_wad, planned,
        "below the quote nothing is trimmed"
    );
    assert!(est.refunds.is_empty(), "no refund below the quote");

    let out = run(&mut t, acc, &pays);
    let charged: i128 = pays
        .iter()
        .zip(&out.spent)
        .map(|((n, _), s)| usd_of(&t, n, *s))
        .sum();
    let taken = usd_of(&t, USDC, out.received + out.fee);
    let underpaid = q - charged;
    std::println!(
        "L3 edge: spent={:?} charged={charged} underpaid={underpaid} taken={taken} C={c} \
         received={} fee={} C_after={} D_after={} alive={}",
        out.spent,
        out.received,
        out.fee,
        out.c_after,
        out.d_after,
        out.alive
    );
    assert_eq!(
        out.spent,
        pays.iter().map(|(_, a)| *a).collect::<Vec<_>>(),
        "the plan pulls the offer as made"
    );
    assert_eq!(
        out.received + out.fee,
        10_000 * USDC_UNIT,
        "every USDC unit leaves the account"
    );
    assert_eq!(out.c_after, 0, "seize_all takes every collateral unit");
    assert!(
        !out.alive,
        "the residual debt is socialized in the same call"
    );
    assert!(
        underpaid <= units && underpaid > units - unit_usd(&t, GOLD),
        "under-payment {underpaid} outside (R - GOLD unit, R] = ({}, {units}]",
        units - unit_usd(&t, GOLD)
    );
    // The liquidator's extra collateral is the under-payment at the bonus.
    let fair = charged + charged * base / BPS;
    let extra = taken - fair;
    assert!(
        extra >= underpaid && extra <= underpaid + underpaid * base / BPS + 4 * ULP,
        "extra collateral {extra} must be underpaid * (1 + base) = {}",
        underpaid + underpaid * base / BPS
    );
    assert!(
        taken >= c - 4 * ULP && taken <= c + 4 * ULP,
        "taken {taken} must be C {c}"
    );
    // The lenders' loss is `D - charged`, which exceeds the loss at the quote by `R`.
    let socialized = d - charged;
    assert!(socialized - (d - q) == underpaid);
}

/// One USDT unit ($1e-7) below the edge the same call is a partial seizure:
/// collateral worth the under-payment at the bonus stays with the residual
/// debt, and the account stays open above the $5 cleanup gate.
#[test]
fn rv_specula_l3_one_unit_below_the_tolerance_is_a_partial_seizure() {
    let (mut t, acc) = insolvent_book();
    let (q, base, units) = quote(&t, acc);
    let mut pays = edge_offer(&t, acc, q, units);
    pays[1].1 -= 1;
    let planned: i128 = pays.iter().map(|(n, a)| usd_of(&t, n, *a)).sum();
    assert!(
        planned + units < q,
        "offer must sit just outside the tolerance"
    );

    let out = run(&mut t, acc, &pays);
    let charged: i128 = pays
        .iter()
        .zip(&out.spent)
        .map(|((n, _), s)| usd_of(&t, n, *s))
        .sum();
    let taken = usd_of(&t, USDC, out.received + out.fee);
    let fair = charged + charged * base / BPS;
    std::println!(
        "L3 partial: spent={:?} charged={charged} taken={taken} fair={fair} C_after={} \
         D_after={} alive={}",
        out.spent,
        out.c_after,
        out.d_after,
        out.alive
    );
    assert!(out.alive, "the account stays open");
    assert!(out.c_after > 0, "a partial seizure leaves collateral");
    assert!(out.d_after > 0, "a partial seizure leaves debt");
    // Proportional seizure at the base bonus, within one USDC unit.
    assert!(
        taken <= fair + unit_usd(&t, USDC) + 4 * ULP && taken + unit_usd(&t, USDC) >= fair,
        "taken {taken} must be charged * (1 + base) = {fair}"
    );
    // What stays is the under-payment at the bonus: `(Q - charged) * (1 + base)`,
    // which is `R` plus one USDT unit, within one USDC unit of seizure flooring.
    let underpaid = q - charged;
    let left = underpaid + underpaid * base / BPS;
    assert!(
        underpaid > units && underpaid <= units + unit_usd(&t, USDT),
        "under-payment {underpaid} must be R plus one USDT unit"
    );
    assert!(
        out.c_after >= left - 4 * ULP && out.c_after <= left + unit_usd(&t, USDC) + 4 * ULP,
        "collateral left {} must be (Q - charged) * (1 + base) = {left}",
        out.c_after
    );
}
