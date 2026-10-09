//! RV worker W1: liquidation economics. Hypotheses H1-H7 (see the W1 report).
//!
//! Every test drives the controller through the harness and asserts concrete token,
//! WAD-USD and index values. INV-ACCT-10 (pool books against account shares) and the
//! spoke usage identity are re-checked after every liquidation executed here.

use common::errors::CollateralError;
use common::types::{
    AccountPositionRaw, ControllerKey, DebtPositionRaw, HubAssetKey, LiquidationEstimate, SeizeMode,
};
use controller::constants::WAD;
use position_nft::PositionNftClient;
use proptest::prelude::*;
use soroban_sdk::testutils::{ContractEvents, Events};
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{Map, Vec as SVec};
use test_harness::{
    assert_contract_error, eth_preset, hub_asset, seed_band_usdc_eth, usd_cents, usdc_preset,
    usdt_stable_preset, wbtc_preset, xlm_preset, LendingTest, MarketPreset, ALICE, BOB, LIQUIDATOR,
};

/// H1 relative slack on the coverage ratio C/D.
const RATIO_TOL: f64 = 1e-9;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Copied from `tests/astra_audit.rs`: scaled supply and borrow shares of `key`, summed over
/// every live position NFT (credit receivers included).
fn account_totals(t: &LendingTest, key: &HubAssetKey) -> (i128, i128) {
    let nft = PositionNftClient::new(&t.env, &t.position_nft);
    let ids: std::vec::Vec<u64> = (0..nft.total_supply())
        .map(|i| u64::from(nft.get_token_id(&i)))
        .collect();
    t.env.as_contract(&t.controller, || {
        let storage = t.env.storage().persistent();
        let mut supplied = 0i128;
        let mut borrowed = 0i128;
        for id in ids {
            if let Some(book) = storage
                .get::<_, Map<HubAssetKey, AccountPositionRaw>>(&ControllerKey::SupplyPositions(id))
            {
                supplied += book.get(key.clone()).map_or(0, |p| p.scaled_amount);
            }
            if let Some(book) = storage
                .get::<_, Map<HubAssetKey, DebtPositionRaw>>(&ControllerKey::BorrowPositions(id))
            {
                borrowed += book.get(key.clone()).map_or(0, |p| p.scaled_amount);
            }
        }
        (supplied, borrowed)
    })
}

/// INV-ACCT-10 for every listed market: pool `supplied - revenue` equals the sum of account
/// supply shares, pool `borrowed` equals the sum of account debt shares. Also the spoke usage
/// identity. Panics with `step` in the message.
fn assert_books_reconcile(t: &LendingTest, step: &str) {
    t.assert_spoke_usage_matches_positions();
    let names: std::vec::Vec<String> = t.markets.keys().cloned().collect();
    for name in names {
        let key = hub_asset(t.resolve_asset(&name));
        let state = t.pool_client(&name).get_sync_data(&key).state;
        let (supplied, borrowed) = account_totals(t, &key);
        assert_eq!(
            state.supplied - state.revenue,
            supplied,
            "[{step}] {name}: pool supplied - revenue must equal the account supply shares"
        );
        assert_eq!(
            state.borrowed, borrowed,
            "[{step}] {name}: pool borrowed must equal the account debt shares"
        );
    }
}

fn acct(t: &LendingTest, user: &str) -> u64 {
    t.resolve_account_id(user)
}
fn coll(t: &LendingTest, acc: u64) -> i128 {
    t.ctrl_client().get_total_collateral_usd(&acc)
}
fn debt(t: &LendingTest, acc: u64) -> i128 {
    t.ctrl_client().get_total_borrow_usd(&acc)
}
fn equity(t: &LendingTest, acc: u64) -> i128 {
    coll(t, acc) - debt(t, acc)
}
/// Supplied and borrowed asset units from the controller views, which project accrual to the
/// current ledger exactly as a liquidation at this timestamp will. The harness readers use the
/// stored index and go stale after `advance_time`.
fn supply_of(t: &LendingTest, acc: u64, name: &str) -> i128 {
    t.ctrl_client()
        .get_collateral_amount(&acc, &hub_asset(t.resolve_asset(name)))
}
fn borrow_of(t: &LendingTest, acc: u64, name: &str) -> i128 {
    t.ctrl_client()
        .get_borrow_amount(&acc, &hub_asset(t.resolve_asset(name)))
}
fn px(t: &LendingTest, name: &str) -> i128 {
    t.resolve_market(name).price_wad
}
fn dec(t: &LendingTest, name: &str) -> u32 {
    t.resolve_market(name).decimals
}
/// Token units to WAD USD, floored.
fn usd_of(t: &LendingTest, name: &str, tokens: i128) -> i128 {
    tokens * px(t, name) / 10i128.pow(dec(t, name))
}
/// WAD USD to token units, floored.
fn tokens_for(t: &LendingTest, name: &str, usd_wad: i128) -> i128 {
    usd_wad * 10i128.pow(dec(t, name)) / px(t, name)
}
fn supply_index(t: &LendingTest, name: &str) -> i128 {
    t.pool_client(name)
        .get_sync_data(&hub_asset(t.resolve_asset(name)))
        .state
        .supply_index
}
fn hf_f64(t: &LendingTest, acc: u64) -> f64 {
    t.ctrl_client().get_health_factor(&acc) as f64 / WAD as f64
}

fn payment_vec(t: &LendingTest, pays: &[(&str, i128)]) -> SVec<(HubAssetKey, i128)> {
    let mut v = SVec::new(&t.env);
    for (name, amount) in pays {
        v.push_back((hub_asset(t.resolve_asset(name)), *amount));
    }
    v
}

fn estimate(
    t: &LendingTest,
    acc: u64,
    pays: &[(&str, i128)],
    mode: SeizeMode,
) -> LiquidationEstimate {
    t.ctrl_client()
        .get_liquidation_estimate(&acc, &payment_vec(t, pays), &mode)
}

/// The collateral-backed or curve quote: the estimate for an over-offer of twice each debt leg.
fn quote_usd(t: &LendingTest, acc: u64, debts: &[&str]) -> i128 {
    let pays: std::vec::Vec<(&str, i128)> = debts
        .iter()
        .map(|name| (*name, 2 * borrow_of(t, acc, name)))
        .collect();
    estimate(t, acc, &pays, SeizeMode::Transfer).max_payment_wad
}

/// Mints each payment to `who`, runs `liquidate`, then re-checks the books. Returns the
/// receiver id (`0` for transfer mode) and the `CleanBadDebtEvent` total, read straight after
/// the call: events are kept only for the most recent invocation, so any later read is empty.
fn liquidate_raw(
    t: &mut LendingTest,
    who: &str,
    acc: u64,
    pays: &[(&str, i128)],
    mode: SeizeMode,
) -> (u64, Option<i128>) {
    let addr = t.get_or_create_user(who);
    for (name, amount) in pays {
        t.resolve_market(name).token_admin.mint(&addr, amount);
    }
    let payments = payment_vec(t, pays);
    let receiver = t.ctrl_client().liquidate(&addr, &acc, &payments, &mode);
    let bad_debt = bad_debt_event_total(&t.env.events().all());
    assert_books_reconcile(t, "after liquidate");
    (receiver, bad_debt)
}

/// Moves each listed collateral price by `factor`.
fn shock(t: &mut LendingTest, names: &[&str], factor: f64) {
    for name in names {
        let price = px(t, name);
        t.set_price(name, (price as f64 * factor).round() as i128);
    }
}

// ---------------------------------------------------------------------------
// H1: partial liquidations never lower C/D on a solvent multi-leg account
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Leg {
    ltv: u32,
    lt: u32,
    bonus: u32,
    fees: u32,
}

#[derive(Clone, Copy, Debug)]
struct Shape {
    /// USDC, ETH, WBTC.
    legs: [Leg; 3],
    dec: [u32; 3],
    debt_dec: u32,
    xlm: bool,
}

const DEFAULT_SHAPE: Shape = Shape {
    legs: [
        Leg {
            ltv: 7500,
            lt: 8500,
            bonus: 400,
            fees: 1000,
        },
        Leg {
            ltv: 7000,
            lt: 7800,
            bonus: 800,
            fees: 1500,
        },
        Leg {
            ltv: 6500,
            lt: 7500,
            bonus: 1000,
            fees: 1200,
        },
    ],
    dec: [7, 7, 7],
    debt_dec: 7,
    xlm: false,
};

fn build_shape(s: &Shape) -> LendingTest {
    let mut b = LendingTest::new()
        .with_market(MarketPreset {
            decimals: s.dec[0],
            ..usdc_preset()
        })
        .with_market(MarketPreset {
            decimals: s.dec[1],
            ..eth_preset()
        })
        .with_market(MarketPreset {
            decimals: s.dec[2],
            ..wbtc_preset()
        })
        .with_market(MarketPreset {
            decimals: s.debt_dec,
            ..usdt_stable_preset()
        });
    if s.xlm {
        b = b.with_market(xlm_preset());
    }
    for (name, leg) in ["USDC", "ETH", "WBTC"].into_iter().zip(s.legs) {
        b = b.with_market_config(name, move |c| {
            c.loan_to_value = leg.ltv;
            c.liquidation_threshold = leg.lt;
            c.liquidation_bonus = leg.bonus;
            c.liquidation_fees = leg.fees;
        });
    }
    b.build()
}

/// ALICE supplies 10 000 USDC, 2 ETH and 0.1 WBTC, borrows `debt_usd` of USDT and
/// `xlm_usd` of XLM, then the three collateral prices move by one common factor so that
/// HF = `target_hf` (the debt tokens are not shocked).
fn stress_book(s: &Shape, debt_usd: f64, xlm_usd: f64, target_hf: f64) -> LendingTest {
    let mut t = build_shape(s);
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 2.0);
    t.supply(ALICE, "WBTC", 0.1);
    t.borrow(ALICE, "USDT", debt_usd);
    if xlm_usd > 0.0 {
        t.borrow(ALICE, "XLM", xlm_usd * 10.0); // XLM is $0.10
    }
    let hf0 = hf_f64(&t, acct(&t, ALICE));
    shock(&mut t, &["USDC", "ETH", "WBTC"], target_hf / hf0);
    t.get_or_create_user(LIQUIDATOR);
    t.assert_liquidatable(ALICE);
    t
}

struct Coverage {
    ratio_before: f64,
    /// `None` when the account is debt-free after the call.
    ratio_after: Option<f64>,
    quote_usd: i128,
    bonus_bps: i128,
    repaid_usd: i128,
    seized_usd: i128,
}

/// Repays `frac` of the quote, split across the debt legs by `shares` (USD fractions), then
/// checks that C/D did not fall and that the account stayed solvent.
fn coverage_step(t: &mut LendingTest, shares: &[(&str, f64)], frac: f64) -> Coverage {
    let acc = acct(t, ALICE);
    let (c0, d0) = (coll(t, acc), debt(t, acc));
    assert!(c0 >= d0, "fixture must be solvent: C={c0} D={d0}");
    let names: std::vec::Vec<&str> = shares.iter().map(|(n, _)| *n).collect();
    let quote = quote_usd(t, acc, &names);
    let offers: std::vec::Vec<(&str, i128)> = shares
        .iter()
        .map(|(name, share)| {
            (
                *name,
                tokens_for(t, name, (quote as f64 * frac * share) as i128),
            )
        })
        .collect();
    let est = estimate(t, acc, &offers, SeizeMode::Transfer);
    let debt_before: std::vec::Vec<i128> =
        shares.iter().map(|(n, _)| borrow_of(t, acc, n)).collect();
    liquidate_raw(t, LIQUIDATOR, acc, &offers, SeizeMode::Transfer);

    let (c1, d1) = (coll(t, acc), debt(t, acc));
    let repaid_usd: i128 = shares
        .iter()
        .zip(&debt_before)
        .map(|((n, _), before)| usd_of(t, n, before - borrow_of(t, acc, n)))
        .sum();
    let ratio_before = c0 as f64 / d0 as f64;
    let ratio_after = if d1 == 0 {
        None
    } else {
        assert!(
            c1 >= d1,
            "partial left the account insolvent: C={c1} D={d1} (was C={c0} D={d0})"
        );
        assert!(
            t.find_account_id(ALICE).is_some(),
            "partial with debt left must keep the account"
        );
        let ratio = c1 as f64 / d1 as f64;
        assert!(
            ratio >= ratio_before * (1.0 - RATIO_TOL),
            "C/D fell: before={ratio_before:.12} after={ratio:.12} frac={frac} \
             quote={quote} bonus={} C0={c0} D0={d0} C1={c1} D1={d1}",
            est.bonus_rate_bps
        );
        Some(ratio)
    };
    Coverage {
        ratio_before,
        ratio_after,
        quote_usd: quote,
        bonus_bps: est.bonus_rate_bps,
        repaid_usd,
        seized_usd: c0 - c1,
    }
}

fn run_h1(
    shape: Shape,
    target_hf: f64,
    usdt_usd: f64,
    xlm_usd: f64,
    shares: &[(&str, f64)],
    fracs: &[f64],
    label: &str,
) {
    for &frac in fracs {
        let mut t = stress_book(&shape, usdt_usd, xlm_usd, target_hf);
        let cov = coverage_step(&mut t, shares, frac);
        std::println!(
            "RV-H1 {label} hf={target_hf} frac={frac:.2}: quote_usd={} bonus_bps={} repaid_usd={} \
             seized_usd={} C/D before={:.9} after={}",
            cov.quote_usd,
            cov.bonus_bps,
            cov.repaid_usd,
            cov.seized_usd,
            cov.ratio_before,
            cov.ratio_after
                .map_or("debt-free".to_string(), |r| format!("{r:.9}"))
        );
    }
}

/// Band regime (cap < base bonus): the quote is the full debt at the cap.
#[test]
fn rv_liq_partial_three_legs_band_regime_never_lowers_coverage() {
    run_h1(
        DEFAULT_SHAPE,
        0.85,
        13_000.0,
        0.0,
        &[("USDT", 1.0)],
        &[0.10, 0.50, 1.00, 1.50],
        "band",
    );
}

/// Cap-binding curve regime: the quote is the full debt, with a sub-$5 residual promoted.
#[test]
fn rv_liq_partial_three_legs_cap_regime_never_lowers_coverage() {
    run_h1(
        DEFAULT_SHAPE,
        0.92,
        13_000.0,
        0.0,
        &[("USDT", 1.0)],
        &[0.10, 0.50, 1.00, 1.50],
        "cap",
    );
}

/// Curve regime near HF = 1: the quote is a partial amount below the debt.
#[test]
fn rv_liq_partial_three_legs_curve_regime_never_lowers_coverage() {
    run_h1(
        DEFAULT_SHAPE,
        0.999,
        13_000.0,
        0.0,
        &[("USDT", 1.0)],
        &[0.10, 0.50, 1.00, 1.50],
        "curve",
    );
}

/// Two debt legs (USDT and XLM): the offer is split across both, and the over-offer trims
/// from the last leg backward.
#[test]
fn rv_liq_partial_two_debt_legs_never_lowers_coverage() {
    run_h1(
        Shape {
            xlm: true,
            ..DEFAULT_SHAPE
        },
        0.97,
        6_000.0,
        3_000.0,
        &[("USDT", 0.5), ("XLM", 0.5)],
        &[0.50, 1.50],
        "two-debt",
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Random three-leg books: risk parameters, decimals, debt fraction, target HF and
    /// repayment size. C/D must not fall and the account must stay solvent.
    #[test]
    fn rv_liq_partial_random_three_legs_never_lowers_coverage(
        lt_u in 7_000u32..=8_800, bonus_u in 200u32..=800, fee_u in 500u32..=2_000,
        lt_e in 6_500u32..=8_300, bonus_e in 400u32..=1_200, fee_e in 500u32..=2_000,
        lt_w in 6_000u32..=8_000, bonus_w in 500u32..=1_500, fee_w in 500u32..=2_000,
        dec_u in prop_oneof![Just(6u32), Just(7u32)],
        dec_w in prop_oneof![Just(7u32), Just(8u32)],
        dec_d in prop_oneof![Just(6u32), Just(7u32)],
        debt_frac in 0.80f64..0.97,
        target_hf in 0.85f64..0.999,
        frac in prop_oneof![Just(0.10f64), Just(0.50f64), Just(1.00f64), Just(1.50f64)],
    ) {
        let shape = Shape {
            legs: [
                Leg { ltv: lt_u - 500, lt: lt_u, bonus: bonus_u, fees: fee_u },
                Leg { ltv: lt_e - 500, lt: lt_e, bonus: bonus_e, fees: fee_e },
                Leg { ltv: lt_w - 500, lt: lt_w, bonus: bonus_w, fees: fee_w },
            ],
            dec: [dec_u, 7, dec_w],
            debt_dec: dec_d,
            xlm: false,
        };
        // LTV-weighted capacity in USD: 10 000 USDC, 2 ETH at $2 000, 0.1 WBTC at $60 000.
        let capacity = 10_000.0 * f64::from(lt_u - 500) / 1e4
            + 4_000.0 * f64::from(lt_e - 500) / 1e4
            + 6_000.0 * f64::from(lt_w - 500) / 1e4;
        let mut t = stress_book(&shape, debt_frac * capacity, 0.0, target_hf);
        let acc = acct(&t, ALICE);
        prop_assume!(coll(&t, acc) >= debt(&t, acc));
        let cov = coverage_step(&mut t, &[("USDT", 1.0)], frac);
        std::println!(
            "RV-H1 prop frac={frac:.2} hf={target_hf:.4} quote={} bonus={} C/D before={:.9}",
            cov.quote_usd, cov.bonus_bps, cov.ratio_before
        );
    }
}

fn est_amount(est: &LiquidationEstimate, t: &LendingTest, name: &str) -> i128 {
    let asset = t.resolve_asset(name);
    est.seized_collaterals
        .iter()
        .find(|entry| entry.asset == asset)
        .map_or(0, |entry| entry.amount)
}

// ---------------------------------------------------------------------------
// H2: self-liquidation through a credit receiver gives no edge over an external liquidator
// ---------------------------------------------------------------------------

/// Alice: 10 000 USDC at $0.70 against 3 ETH. HF = 0.933, solvent.
fn h2_book() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(70));
    t.assert_liquidatable(ALICE);
    t.get_or_create_user(LIQUIDATOR);
    t
}

#[test]
fn rv_liq_self_credit_liquidation_gives_no_edge_over_external_transfer() {
    const OFFER: i128 = 1_0000000; // 1 ETH, 7 decimals
    let tol_usd = 3 * usd_of_unit();

    // Self path: ALICE repays her own account and credits the seized shares to a receiver she owns.
    let mut s = h2_book();
    let acc = acct(&s, ALICE);
    let eq0 = equity(&s, acc);
    let (debt0, usdc0, rev0) = (
        borrow_of(&s, acc, "ETH"),
        supply_of(&s, acc, "USDC"),
        s.snapshot_revenue("USDC"),
    );
    let alice_eth0 = s.token_balance_raw(ALICE, "ETH");
    let (receiver, _) = liquidate_raw(&mut s, ALICE, acc, &[("ETH", OFFER)], SeizeMode::Credit(0));
    let retired_s = debt0 - borrow_of(&s, acc, "ETH");
    let seized_s = usdc0 - supply_of(&s, acc, "USDC");
    let fee_s = s.snapshot_revenue("USDC") - rev0;
    let recv_usd = s.ctrl_client().get_total_collateral_usd(&receiver);
    let recv_usdc = supply_of(&s, receiver, "USDC");
    // Alice's wallet: the minted OFFER is hers, so her net ETH change is minus the debt retired.
    let wallet_s = usd_of(
        &s,
        "ETH",
        s.token_balance_raw(ALICE, "ETH") - alice_eth0 - OFFER,
    );
    let delta_self = equity(&s, acc) + recv_usd - eq0 + wallet_s;

    // External path: an identical book, LIQUIDATOR runs the transfer-mode liquidation.
    let mut p = h2_book();
    let acc_p = acct(&p, ALICE);
    let eq0_p = equity(&p, acc_p);
    let (debt0_p, usdc0_p, rev0_p) = (
        borrow_of(&p, acc_p, "ETH"),
        supply_of(&p, acc_p, "USDC"),
        p.snapshot_revenue("USDC"),
    );
    let liq_usdc0 = p.token_balance_raw(LIQUIDATOR, "USDC");
    let liq_eth0 = p.token_balance_raw(LIQUIDATOR, "ETH");
    liquidate_raw(
        &mut p,
        LIQUIDATOR,
        acc_p,
        &[("ETH", OFFER)],
        SeizeMode::Transfer,
    );
    let retired_p = debt0_p - borrow_of(&p, acc_p, "ETH");
    let seized_p = usdc0_p - supply_of(&p, acc_p, "USDC");
    let fee_p = p.snapshot_revenue("USDC") - rev0_p;
    let got_usdc = p.token_balance_raw(LIQUIDATOR, "USDC") - liq_usdc0;
    let spent_eth = liq_eth0 + OFFER - p.token_balance_raw(LIQUIDATOR, "ETH");
    let delta_alice_p = equity(&p, acc_p) - eq0_p;
    let liquidator_profit = usd_of(&p, "USDC", got_usdc) - usd_of(&p, "ETH", spent_eth);

    std::println!(
        "RV-H2 self: retired_eth={retired_s} seized_usdc={seized_s} fee_usdc={fee_s} \
         receiver_usdc={recv_usdc} receiver_usd={recv_usd} alice_total_delta_usd={delta_self}"
    );
    std::println!(
        "RV-H2 external: retired_eth={retired_p} seized_usdc={seized_p} fee_usdc={fee_p} \
         liquidator_usdc={got_usdc} liquidator_profit_usd={liquidator_profit} \
         alice_total_delta_usd={delta_alice_p}"
    );

    assert!(
        (retired_s - retired_p).abs() <= 1,
        "same repayment must retire the same debt: self={retired_s} external={retired_p}"
    );
    assert!(
        (seized_s - seized_p).abs() <= 2,
        "same plan must seize the same collateral: self={seized_s} external={seized_p}"
    );
    assert!(
        fee_s + 1 >= fee_p,
        "self-liquidation must book at least the external protocol fee: self={fee_s} external={fee_p}"
    );
    // Alice's total in the self path must not exceed what the external path leaves for Alice
    // plus what the external liquidator keeps: the external path's total is `-fee`.
    assert!(
        delta_self <= delta_alice_p + liquidator_profit + tol_usd,
        "self-liquidation extracts more than the external pie: self={delta_self} \
         external_alice={delta_alice_p} external_liquidator={liquidator_profit}"
    );
}

/// One unit of the 7-decimal USDC market at $1, in WAD.
fn usd_of_unit() -> i128 {
    WAD / 10_000_000
}

// ---------------------------------------------------------------------------
// H3: solvent accounts are never socialized; insolvent quotes seize every unit
// ---------------------------------------------------------------------------

fn i128_field(data: &ScVal, name: &str) -> Option<i128> {
    let ScVal::Map(Some(map)) = data else {
        return None;
    };
    map.iter().find_map(|entry| match (&entry.key, &entry.val) {
        (ScVal::Symbol(key), ScVal::I128(value)) if key.0.to_string() == name => {
            Some(i128::from(value))
        }
        _ => None,
    })
}

/// `total_borrow_usd_wad` of the last `CleanBadDebtEvent` (topic `debt`, `bad_debt`).
fn bad_debt_event_total(events: &ContractEvents) -> Option<i128> {
    events.events().iter().find_map(|event| {
        let ContractEventBody::V0(body) = &event.body;
        let is_bad_debt = matches!(
            (body.topics.first(), body.topics.get(1)),
            (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b)))
                if a.0.to_string() == "debt" && b.0.to_string() == "bad_debt"
        );
        if is_bad_debt {
            i128_field(&body.data, "total_borrow_usd_wad")
        } else {
            None
        }
    })
}

#[test]
fn rv_liq_insolvent_quote_seizes_every_unit_and_socializes_the_residual() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
    t.get_or_create_user(LIQUIDATOR);
    let acc = acct(&t, ALICE);
    let (c0, d0) = (coll(&t, acc), debt(&t, acc));
    assert!(c0 < d0, "fixture must be insolvent: C={c0} D={d0}");

    t.update_indexes_for(&["USDC", "ETH"]);
    let (usdc_idx0, eth_idx0) = (supply_index(&t, "USDC"), supply_index(&t, "ETH"));
    let bob_eth0 = t.supply_balance_raw(BOB, "ETH");

    // The collateral-backed quote at the base bonus: floor(C / 1.05).
    let quote = quote_usd(&t, acc, &["ETH"]);
    let backed = c0 * 10_000 / 10_500; // floor(C / (1 + base))
    assert!(
        quote <= backed && backed - quote <= usd_of(&t, "ETH", 1),
        "quote keeps whole ETH units below floor(C/1.05): quote={quote} backed={backed}"
    );
    let offer = tokens_for(&t, "ETH", quote);
    let est = estimate(&t, acc, &[("ETH", offer)], SeizeMode::Transfer);
    assert!(est.refunds.is_empty(), "a quote-sized offer is not trimmed");
    let usdc_planned = est_amount(&est, &t, "USDC");
    assert_eq!(
        usdc_planned,
        10_000 * 10_000_000,
        "the quote seizes every USDC unit"
    );
    let base_bonus = est.bonus_rate_bps;

    let rev0 = t.snapshot_revenue("USDC");
    let liq_usdc0 = t.token_balance_raw(LIQUIDATOR, "USDC");
    let debt_before = borrow_of(&t, acc, "ETH");
    let (_, event_total) = liquidate_raw(
        &mut t,
        LIQUIDATOR,
        acc,
        &[("ETH", offer)],
        SeizeMode::Transfer,
    );
    let got_usdc = t.token_balance_raw(LIQUIDATOR, "USDC") - liq_usdc0;
    let fee = t.snapshot_revenue("USDC") - rev0;
    assert!(
        (got_usdc + fee - usdc_planned).abs() <= 1,
        "payout plus reclassified fee must equal every seized unit: got={got_usdc} fee={fee} \
         planned={usdc_planned}"
    );

    let residual_tokens = debt_before - offer;
    let residual_usd = usd_of(&t, "ETH", residual_tokens);
    assert_eq!(
        event_total,
        Some(residual_usd),
        "the socialized residual is the remaining 0.619 ETH at $2 000"
    );
    assert!(
        !t.account_exists(acc),
        "the account is removed after socialization"
    );
    assert!(
        !t.try_nft_owner_of(acc),
        "the position NFT is burned with the account"
    );
    assert!(
        supply_index(&t, "ETH") < eth_idx0,
        "the debt market supply index must write down: before={eth_idx0} after={}",
        supply_index(&t, "ETH")
    );
    assert_eq!(
        supply_index(&t, "USDC"),
        usdc_idx0,
        "the collateral market index must not move"
    );
    assert!(
        t.supply_balance_raw(BOB, "ETH") < bob_eth0,
        "the ETH suppliers absorb the residual"
    );

    let got_usd = usd_of(&t, "USDC", got_usdc);
    let repaid_usd = usd_of(&t, "ETH", offer);
    let bound = repaid_usd * (10_000 + base_bonus) / 10_000 + 2 * usd_of_unit();
    std::println!(
        "RV-H3 insolvent: quote_usd={quote} repaid_usd={repaid_usd} got_usdc={got_usdc} \
         got_usd={got_usd} bound_usd={bound} fee_usdc={fee} residual_usd={residual_usd} \
         eth_index {eth_idx0}->{}",
        supply_index(&t, "ETH")
    );
    assert!(
        got_usd <= bound,
        "liquidator collateral value must not exceed repaid*(1+base)+rounding: \
         got={got_usd} bound={bound}"
    );
}

/// Solvent band account (C/D = 1.033): partial and full closes keep every market index and
/// keep the account until its debt is gone, and then only while collateral is positive.
#[test]
fn rv_liq_solvent_band_never_moves_indexes_or_removes_account_with_debt() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(BOB, "ETH", 100.0);
    seed_band_usdc_eth(&mut t);
    t.get_or_create_user(LIQUIDATOR);
    let acc = acct(&t, ALICE);
    let (c0, d0) = (coll(&t, acc), debt(&t, acc));
    assert!(
        c0 >= d0 && c0 < d0 * 105 / 100,
        "fixture must sit in the band"
    );
    t.update_indexes_for(&["USDC", "ETH"]);
    let idx0 = (supply_index(&t, "USDC"), supply_index(&t, "ETH"));

    let (_, partial_event) = liquidate_raw(
        &mut t,
        LIQUIDATOR,
        acc,
        &[("ETH", 1_0000000)],
        SeizeMode::Transfer,
    );
    assert_eq!(partial_event, None, "a partial emits no bad-debt event");
    assert_eq!(
        (supply_index(&t, "USDC"), supply_index(&t, "ETH")),
        idx0,
        "a partial band liquidation writes nothing down"
    );
    assert!(
        t.find_account_id(ALICE).is_some(),
        "partial keeps the account"
    );
    assert_eq!(
        borrow_of(&t, acc, "ETH"),
        2_0000000,
        "2 ETH of debt remains"
    );

    let (_, close_event) = liquidate_raw(
        &mut t,
        LIQUIDATOR,
        acc,
        &[("ETH", 4_0000000)],
        SeizeMode::Transfer,
    );
    assert_eq!(
        borrow_of(&t, acc, "ETH"),
        0,
        "the over-offer closes the debt"
    );
    assert_eq!(
        (supply_index(&t, "USDC"), supply_index(&t, "ETH")),
        idx0,
        "a solvent full close writes nothing down"
    );
    assert!(
        coll(&t, acc) > 0 && t.account_exists(acc),
        "a debt-free account with collateral is kept: C={}",
        coll(&t, acc)
    );
    assert_eq!(
        close_event, None,
        "no CleanBadDebtEvent on a solvent account"
    );
    std::println!(
        "RV-H3 solvent band: C0={c0} D0={d0} partial_event={partial_event:?} close_event={close_event:?} \
         collateral_after_close={} indices_before={idx0:?} indices_after={:?}",
        coll(&t, acc),
        (supply_index(&t, "USDC"), supply_index(&t, "ETH"))
    );
}

// ---------------------------------------------------------------------------
// H4: dust promotion. The full-debt quote is not trimmed for smaller offers.
// ---------------------------------------------------------------------------

/// The target-HF repayment before dust promotion, in USD, re-derived in f64 from the on-chain
/// state for one 500 bps leg, LT 0.80, target 1.10, floor 0.80, factor 1.0, max bonus 2 500 bps.
/// Mirrors `formulas.md` "Bonus and target repayment". Returns (ideal, bonus fraction).
fn unpromoted_curve_ideal(c_usd: f64, d_usd: f64, hf: f64) -> (f64, f64) {
    let (p, base, max, h, k) = (0.80, 0.05, 0.25, 1.10, 0.80);
    let cap = (hf / p * 1e4).floor() / 1e4 - 1.0;
    let s = ((h - hf) / (h - k)).clamp(0.0, 1.0);
    let bonus = (base + (max - base) * s).min(cap);
    let ideal = (h * d_usd - p * c_usd) / (h - p * (1.0 + bonus));
    (ideal.min(d_usd), bonus)
}

fn dust_book() -> LendingTest {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", 630_630_000_000_000_000); // $0.63063
    t.assert_liquidatable(ALICE);
    t.get_or_create_user(LIQUIDATOR);
    t
}

#[test]
fn rv_liq_dust_promoted_quote_keeps_partial_offers_untrimmed_and_account_alive() {
    let mut t = dust_book();
    let acc = acct(&t, ALICE);
    let (c0, d0) = (coll(&t, acc), debt(&t, acc));
    let (ideal, bonus0) = unpromoted_curve_ideal(
        c0 as f64 / WAD as f64,
        d0 as f64 / WAD as f64,
        hf_f64(&t, acc),
    );
    let residual = d0 as f64 / WAD as f64 - ideal;
    assert!(
        residual > 0.0 && residual < 5.0,
        "the un-promoted residual must sit in (0, $5): {residual:.6}"
    );

    let quote = quote_usd(&t, acc, &["ETH"]);
    assert!(
        (quote - d0).abs() <= usd_of(&t, "ETH", 1),
        "promoted quote equals the whole debt within one ETH unit: quote={quote} debt={d0}"
    );

    // An offer of 2 ETH, below the quote: accepted as offered.
    let pays = [("ETH", 2_0000000i128)];
    let est = estimate(&t, acc, &pays, SeizeMode::Transfer);
    assert!(
        est.refunds.is_empty(),
        "no refund for an offer below the quote"
    );
    assert_eq!(
        est.max_payment_wad,
        usd_of(&t, "ETH", 2_0000000),
        "the partial repays exactly what was offered"
    );
    let liq_before = t.token_balance_raw(LIQUIDATOR, "ETH");
    liquidate_raw(&mut t, LIQUIDATOR, acc, &pays, SeizeMode::Transfer);
    let spent = liq_before + 2_0000000 - t.token_balance_raw(LIQUIDATOR, "ETH");
    assert_eq!(spent, 2_0000000, "no trim: every offered unit is pulled");
    assert_eq!(
        borrow_of(&t, acc, "ETH"),
        1_0000000,
        "one ETH of debt remains"
    );
    assert!(
        t.find_account_id(ALICE).is_some(),
        "a partial that leaves debt and collateral keeps the account"
    );
    let c1 = coll(&t, acc);
    let expected_seized = usd_of(&t, "ETH", 2_0000000) * (10_000 + est.bonus_rate_bps) / 10_000;
    assert!(
        (c0 - c1 - expected_seized).abs() <= 1_000_000_000_000,
        "seizure is repaid*(1+bonus): seized={} expected={expected_seized}",
        c0 - c1
    );
    std::println!(
        "RV-H4 partial: residual_unpromoted_usd={residual:.6} bonus_bps_f64={:.1} \
         est_bonus_bps={} quote_usd={quote} debt_usd={d0} collateral_after_usd={c1}",
        bonus0 * 1e4,
        est.bonus_rate_bps
    );

    // One unit short of the promoted quote: still not trimmed; one debt unit remains.
    let mut u = dust_book();
    let acc_u = acct(&u, ALICE);
    let short = [("ETH", 3_0000000i128 - 1)];
    let est_u = estimate(&u, acc_u, &short, SeizeMode::Transfer);
    assert!(est_u.refunds.is_empty(), "one unit short is not trimmed");
    liquidate_raw(&mut u, LIQUIDATOR, acc_u, &short, SeizeMode::Transfer);
    assert_eq!(borrow_of(&u, acc_u, "ETH"), 1, "one debt unit remains");
    assert!(
        u.find_account_id(ALICE).is_some(),
        "collateral above $0 and debt below collateral: the account is kept"
    );
    let c_u = coll(&u, acc_u);
    assert!(
        c_u <= 5 * WAD && debt(&u, acc_u) < c_u,
        "the kept account is not bad debt: C={c_u} D={}",
        debt(&u, acc_u)
    );
    std::println!(
        "RV-H4 one-unit-short: debt_units_left=1 collateral_usd={c_u} debt_usd={}",
        debt(&u, acc_u)
    );
}

// ---------------------------------------------------------------------------
// H5: six partial steps on a three-leg account never out-seize one close
// ---------------------------------------------------------------------------

/// Three collateral legs at HF = `target_hf` with USDT debt of 13 490 (95 % of capacity).
fn three_leg_book(target_hf: f64) -> LendingTest {
    stress_book(&DEFAULT_SHAPE, 13_490.0, 0.0, target_hf)
}

const COLLATERAL: [&str; 3] = ["USDC", "ETH", "WBTC"];

#[test]
fn rv_liq_six_partial_steps_three_legs_never_out_seize_one_close() {
    let mut chain = three_leg_book(0.97);
    let acc = acct(&chain, ALICE);
    let (c0, d0) = (coll(&chain, acc), debt(&chain, acc));
    let mut seized_chain = [0i128; 3];
    let mut repaid_chain = 0i128;
    for step in 0..6 {
        assert!(
            chain.can_be_liquidated(ALICE),
            "step {step}: the account is no longer liquidatable (HF={:.6}); each partial raises HF \
             while the bonus stays under the cap",
            hf_f64(&chain, acc)
        );
        let quote = quote_usd(&chain, acc, &["USDT"]);
        let offer = tokens_for(&chain, "USDT", quote / 10);
        let hf_now = hf_f64(&chain, acc);
        let before = COLLATERAL.map(|n| supply_of(&chain, acc, n));
        let debt_before = borrow_of(&chain, acc, "USDT");
        liquidate_raw(
            &mut chain,
            LIQUIDATOR,
            acc,
            &[("USDT", offer)],
            SeizeMode::Transfer,
        );
        for (i, name) in COLLATERAL.iter().enumerate() {
            seized_chain[i] += before[i] - supply_of(&chain, acc, name);
        }
        let retired = debt_before - borrow_of(&chain, acc, "USDT");
        repaid_chain += retired;
        std::println!(
            "RV-H5 step {step}: hf_before={hf_now:.6} quote_usd={quote} offer={offer} retired={retired}"
        );
    }
    let (c1, d1) = (coll(&chain, acc), debt(&chain, acc));
    std::println!("RV-H5 final: hf={:.6}", hf_f64(&chain, acc));
    let ratio0 = c0 as f64 / d0 as f64;
    let ratio1 = c1 as f64 / d1 as f64;
    assert!(
        ratio1 >= ratio0 * (1.0 - RATIO_TOL),
        "C/D fell over the chain: before={ratio0:.12} after={ratio1:.12}"
    );

    let mut single = three_leg_book(0.97);
    let acc_s = acct(&single, ALICE);
    let before_s = COLLATERAL.map(|n| supply_of(&single, acc_s, n));
    let debt_s0 = borrow_of(&single, acc_s, "USDT");
    liquidate_raw(
        &mut single,
        LIQUIDATOR,
        acc_s,
        &[("USDT", repaid_chain)],
        SeizeMode::Transfer,
    );
    let retired_single = debt_s0 - borrow_of(&single, acc_s, "USDT");
    let seized_single: [i128; 3] =
        std::array::from_fn(|i| before_s[i] - supply_of(&single, acc_s, COLLATERAL[i]));
    std::println!(
        "RV-H5 chain: seized={seized_chain:?} repaid={repaid_chain} C/D {ratio0:.9}->{ratio1:.9} | \
         single: seized={seized_single:?} repaid={retired_single}"
    );

    assert!(
        (retired_single - repaid_chain).abs() <= 6,
        "both paths must retire the same debt: chain={repaid_chain} single={retired_single}"
    );
    for i in 0..3 {
        assert!(
            seized_chain[i] <= seized_single[i] + 6,
            "leg {} out-seized one close: chain={} single={} (steps*1 unit allowed)",
            COLLATERAL[i],
            seized_chain[i],
            seized_single[i]
        );
    }
}

/// Why H5 uses 10% steps: each partial raises HF while the bonus stays under the cap, so at
/// 15% the account turns healthy after five steps and the sixth estimate reverts.
#[test]
fn rv_liq_fifteen_percent_chain_turns_healthy_after_five_steps() {
    let mut chain = three_leg_book(0.97);
    let acc = acct(&chain, ALICE);
    let mut executed = 0;
    while chain.can_be_liquidated(ALICE) && executed < 6 {
        let quote = quote_usd(&chain, acc, &["USDT"]);
        let offer = tokens_for(&chain, "USDT", quote * 15 / 100);
        liquidate_raw(
            &mut chain,
            LIQUIDATOR,
            acc,
            &[("USDT", offer)],
            SeizeMode::Transfer,
        );
        executed += 1;
    }
    let hf_end = hf_f64(&chain, acc);
    std::println!("RV-H5 15pct: executed={executed} hf_end={hf_end:.6}");
    assert_eq!(
        executed, 5,
        "15% steps leave the liquidatable range after five steps"
    );
    assert!(hf_end >= 1.0, "HF must have reached 1 WAD: {hf_end}");
    let pays = payment_vec(&chain, &[("USDT", 1_0000000)]);
    let refused: Result<LiquidationEstimate, soroban_sdk::Error> = match chain
        .ctrl_client()
        .try_get_liquidation_estimate(&acc, &pays, &SeizeMode::Transfer)
    {
        Ok(Ok(estimate)) => Ok(estimate),
        Ok(Err(conversion)) => panic!("unexpected conversion error: {conversion:?}"),
        Err(Ok(err)) => Err(err),
        Err(Err(invoke)) => panic!("unexpected invoke error: {invoke:?}"),
    };
    assert_contract_error(refused, CollateralError::HealthFactorTooHigh as u32);
}

// ---------------------------------------------------------------------------
// H6: estimate taken before one 5-second ledger of accrual
// ---------------------------------------------------------------------------

/// Measurements of one H6 run: an estimate at T0, a second estimate on the execution ledger,
/// and the liquidation itself. Projected balances and wallet deltas, so no stale-index reads.
struct H6 {
    quote0: i128,
    quote1: i128,
    offer: i128,
    repaid: i128,
    refund0: i128,
    refund1: i128,
    refund_exec: i128,
    gross0: [i128; 3],
    gross1: [i128; 3],
    gross_exec: [i128; 3],
    net_exec: [i128; 3],
    bonus0: i128,
    bonus1: i128,
}

/// Three-leg book at HF 0.97. Estimates at T0, advances `advance` seconds, estimates on the
/// execution ledger and then liquidates with the same offer: 1.2 x quote when `over`, else 0.5 x.
fn h6_run(advance: u64, over: bool) -> (LendingTest, H6) {
    let mut t = three_leg_book(0.97);
    let acc = acct(&t, ALICE);
    let quote0 = quote_usd(&t, acc, &["USDT"]);
    let offer = if over {
        tokens_for(&t, "USDT", quote0 * 12 / 10)
    } else {
        tokens_for(&t, "USDT", quote0 / 2)
    };
    let est0 = estimate(&t, acc, &[("USDT", offer)], SeizeMode::Transfer);
    if advance > 0 {
        t.advance_time(advance);
    }
    let quote1 = quote_usd(&t, acc, &["USDT"]);
    let est1 = estimate(&t, acc, &[("USDT", offer)], SeizeMode::Transfer);

    let coll_before = COLLATERAL.map(|n| supply_of(&t, acc, n));
    let debt_before = borrow_of(&t, acc, "USDT");
    let wallet_before = t.token_balance_raw(LIQUIDATOR, "USDT");
    let legs_before = COLLATERAL.map(|n| t.token_balance_raw(LIQUIDATOR, n));
    liquidate_raw(
        &mut t,
        LIQUIDATOR,
        acc,
        &[("USDT", offer)],
        SeizeMode::Transfer,
    );

    let repaid = wallet_before + offer - t.token_balance_raw(LIQUIDATOR, "USDT");
    let r = H6 {
        quote0,
        quote1,
        offer,
        repaid,
        refund0: est0.refunds.get(0).map_or(0, |r| r.amount),
        refund1: est1.refunds.get(0).map_or(0, |r| r.amount),
        refund_exec: offer - repaid,
        gross0: COLLATERAL.map(|n| est_amount(&est0, &t, n)),
        gross1: COLLATERAL.map(|n| est_amount(&est1, &t, n)),
        gross_exec: std::array::from_fn(|i| coll_before[i] - supply_of(&t, acc, COLLATERAL[i])),
        net_exec: std::array::from_fn(|i| {
            t.token_balance_raw(LIQUIDATOR, COLLATERAL[i]) - legs_before[i]
        }),
        bonus0: est0.bonus_rate_bps,
        bonus1: est1.bonus_rate_bps,
    };
    assert_eq!(
        debt_before - borrow_of(&t, acc, "USDT"),
        r.repaid,
        "wallet and debt-book repayment must agree"
    );
    std::println!(
        "RV-H6 advance={advance}s over={over}: quote0={} quote1={} drift_tokens={} offer={} \
         repaid={} refund0={} refund1={} refund_exec={} gross0={:?} gross1={:?} gross_exec={:?} \
         bonus0={} bonus1={}",
        r.quote0,
        r.quote1,
        tokens_for(&t, "USDT", r.quote1 - r.quote0),
        r.offer,
        r.repaid,
        r.refund0,
        r.refund1,
        r.refund_exec,
        r.gross0,
        r.gross1,
        r.gross_exec,
        r.bonus0,
        r.bonus1
    );
    (t, r)
}

/// Economic bound at execution: the value received is at most the repayment made at execution,
/// times (1 + bonus), plus two units of each collateral leg.
fn assert_received_within_repaid_bonus(t: &LendingTest, r: &H6, label: &str) {
    let received: i128 = COLLATERAL
        .iter()
        .enumerate()
        .map(|(i, n)| usd_of(t, n, r.net_exec[i]))
        .sum();
    let repaid_usd = usd_of(t, "USDT", r.repaid);
    let rounding: i128 = COLLATERAL.iter().map(|n| 2 * usd_of(t, n, 1)).sum();
    assert!(
        received <= repaid_usd * (10_000 + r.bonus1) / 10_000 + rounding,
        "{label}: received exceeds repaid*(1+bonus): received={received} repaid={repaid_usd}"
    );
}

/// Control: the same ledger. Estimate and execution agree exactly.
#[test]
fn rv_liq_same_ledger_estimate_matches_execution_exactly() {
    let (t, r) = h6_run(0, true);
    assert_eq!(r.refund_exec, r.refund0, "no ledger drift, no refund drift");
    for (i, name) in COLLATERAL.iter().enumerate() {
        assert!(
            (r.gross_exec[i] - r.gross0[i]).abs() <= 1,
            "leg {} must match the same-ledger estimate: exec={} est={}",
            name,
            r.gross_exec[i],
            r.gross0[i]
        );
    }
    assert_received_within_repaid_bonus(&t, &r, "same-ledger");
}

/// A partial below the quote, executed five seconds after its estimate: not trimmed, so the
/// seizure stays within estimate + 1 unit per leg and there is no refund either way.
#[test]
fn rv_liq_partial_after_five_seconds_seizes_at_most_estimate_plus_one_unit() {
    let (t, r) = h6_run(5, false);
    assert!(r.repaid <= r.offer, "repaid must not exceed the offer");
    assert_eq!(
        r.refund_exec, r.refund0,
        "an untrimmed partial keeps no refund"
    );
    for (i, name) in COLLATERAL.iter().enumerate() {
        assert!(
            r.gross_exec[i] <= r.gross0[i] + 1,
            "leg {} seizure {} exceeds estimate+1 unit {}",
            name,
            r.gross_exec[i],
            r.gross0[i] + 1
        );
    }
    assert_received_within_repaid_bonus(&t, &r, "partial-after-5s");
}

/// A trimmed over-offer executed five seconds after its estimate: the execution repays the
/// quote of its own ledger, so the seizure and the refund match the execution-ledger estimate,
/// and the refund moves by exactly the quote drift.
#[test]
fn rv_liq_over_offer_after_five_seconds_matches_execution_ledger_estimate() {
    let (t, r) = h6_run(5, true);
    assert!(r.repaid <= r.offer, "repaid must not exceed the offer");
    assert!(
        r.quote1 > r.quote0,
        "accrual must move the quote so the drift is observable: {} -> {}",
        r.quote0,
        r.quote1
    );
    assert!(
        (r.refund_exec - r.refund1).abs() <= 2,
        "refund must match the execution-ledger estimate: exec={} est={}",
        r.refund_exec,
        r.refund1
    );
    for (i, name) in COLLATERAL.iter().enumerate() {
        assert!(
            (r.gross_exec[i] - r.gross1[i]).abs() <= 1,
            "leg {} must match the execution-ledger estimate: exec={} est={}",
            name,
            r.gross_exec[i],
            r.gross1[i]
        );
    }
    let drift = tokens_for(&t, "USDT", r.quote0 - r.quote1);
    assert!(
        ((r.refund_exec - r.refund0) - drift).abs() <= 2,
        "refund drift {} must equal the quote drift {drift}",
        r.refund_exec - r.refund0
    );
    assert_received_within_repaid_bonus(&t, &r, "over-offer-after-5s");
}

/// H6 literal claim, measured against the T0 estimate: refuted for a trimmed over-offer. The
/// quote rises with five seconds of debt accrual, so the execution seizes the larger quote.
#[test]
#[ignore = "RV-FINDING: trimmed over-offer executed 5 s after its estimate seizes ~920 USDC units (1.3e-4 USD quote drift) above the estimate+1 unit bound; docs list only rounding and receipt as causes"]
fn rv_finding_liq_over_offer_seizure_exceeds_t0_estimate_plus_one_unit() {
    let (_t, r) = h6_run(5, true);
    for (i, name) in COLLATERAL.iter().enumerate() {
        assert!(
            r.gross_exec[i] <= r.gross0[i] + 1,
            "leg {} seizure {} exceeds the T0 estimate+1 unit {}",
            name,
            r.gross_exec[i],
            r.gross0[i] + 1
        );
    }
    assert_eq!(
        r.refund_exec, r.refund0,
        "refund must match the T0 estimate"
    );
}

/// Insolvent account (C = $5 000, D = $6 000) with ETH debt on a 10% fee-on-transfer token. The
/// liquidator sends the collateral-backed quote in ETH; the pool receives 90% of it.
fn fot_insolvent_book() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_fee_on_transfer_market(eth_preset(), 1_000)
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
    t.get_or_create_user(LIQUIDATOR);
    t
}

/// Under-delivery on the insolvent quote: `formulas.md` says the collateral-backed call "seizes
/// every unit", but `seize_all` is decided on the planned repayment and the seizure is then
/// scaled by the received share. The liquidator's value stays within repaid*(1+base).
#[test]
fn rv_liq_insolvent_fee_on_transfer_quote_pays_received_value_only() {
    let mut t = fot_insolvent_book();
    let acc = acct(&t, ALICE);
    let quote = quote_usd(&t, acc, &["ETH"]);
    let offer = tokens_for(&t, "ETH", quote);
    let base = estimate(&t, acc, &[("ETH", offer)], SeizeMode::Transfer).bonus_rate_bps;
    let usdc0 = supply_of(&t, acc, "USDC");
    let liq_usdc0 = t.token_balance_raw(LIQUIDATOR, "USDC");
    let debt0 = borrow_of(&t, acc, "ETH");
    liquidate_raw(
        &mut t,
        LIQUIDATOR,
        acc,
        &[("ETH", offer)],
        SeizeMode::Transfer,
    );
    let retired = debt0 - borrow_of(&t, acc, "ETH");
    let kept_usdc = supply_of(&t, acc, "USDC");
    let got_usd = usd_of(
        &t,
        "USDC",
        t.token_balance_raw(LIQUIDATOR, "USDC") - liq_usdc0,
    );
    let received_repay_usd = usd_of(&t, "ETH", retired);
    std::println!(
        "RV-H7-insolvent: offer={offer} retired={retired} usdc_before={usdc0} kept_usdc={kept_usdc} \
         kept_fraction={:.4} got_usd={got_usd} bound_usd={}",
        kept_usdc as f64 / usdc0 as f64,
        received_repay_usd * (10_000 + base) / 10_000 + 2 * usd_of(&t, "USDC", 1)
    );
    assert!(
        (retired - 9 * offer / 10).abs() <= 1,
        "the pool receives 90% of the offer: retired={retired}"
    );
    assert!(
        got_usd <= received_repay_usd * (10_000 + base) / 10_000 + 2 * usd_of(&t, "USDC", 1),
        "liquidator collateral value {got_usd} exceeds received repayment*(1+base)"
    );
    assert!(
        t.find_account_id(ALICE).is_some(),
        "residual collateral keeps the account"
    );
}

/// H7 vs `formulas.md`: an insolvent quote on a fee-on-transfer debt token leaves about 10% of
/// the collateral in the account, not zero. The doc says the call seizes every unit.
#[test]
#[ignore = "RV-FINDING: insolvent quote on a 10% fee-on-transfer debt token keeps ~10% of the collateral (seize_all is decided on the planned repayment); formulas.md says every unit is seized"]
fn rv_finding_liq_insolvent_fee_on_transfer_quote_seizes_every_unit() {
    let mut t = fot_insolvent_book();
    let acc = acct(&t, ALICE);
    let quote = quote_usd(&t, acc, &["ETH"]);
    let offer = tokens_for(&t, "ETH", quote);
    liquidate_raw(
        &mut t,
        LIQUIDATOR,
        acc,
        &[("ETH", offer)],
        SeizeMode::Transfer,
    );
    assert_eq!(
        supply_of(&t, acc, "USDC"),
        0,
        "the documented collateral-backed call seizes every USDC unit"
    );
}

// ---------------------------------------------------------------------------
// H7: fee-on-transfer debt. The collateral value follows the received repayment.
// ---------------------------------------------------------------------------

/// USDC and WBTC collateral (HF 0.96, curve regime), ETH debt on a 10% fee-on-transfer token.
fn fot_book() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(wbtc_preset())
        .with_fee_on_transfer_market(eth_preset(), 1_000)
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "WBTC", 0.1);
    t.borrow(ALICE, "ETH", 5.0);
    let hf0 = hf_f64(&t, acct(&t, ALICE));
    shock(&mut t, &["USDC", "WBTC"], 0.96 / hf0);
    t.get_or_create_user(LIQUIDATOR);
    t.assert_liquidatable(ALICE);
    t
}

fn fot_case(mode: SeizeMode, label: &str) {
    let mut t = fot_book();
    let acc = acct(&t, ALICE);
    let offer = 1_0000000i128; // 1 ETH sent; the pool receives 90 %
    let plan = estimate(&t, acc, &[("ETH", offer)], SeizeMode::Transfer);
    let bonus = plan.bonus_rate_bps;
    let planned_gross_usd: i128 = ["USDC", "WBTC"]
        .iter()
        .map(|n| usd_of(&t, n, est_amount(&plan, &t, n)))
        .sum();

    let debt0 = borrow_of(&t, acc, "ETH");
    let liq_usdc0 = t.token_balance_raw(LIQUIDATOR, "USDC");
    let liq_wbtc0 = t.token_balance_raw(LIQUIDATOR, "WBTC");
    let (receiver, _) = liquidate_raw(&mut t, LIQUIDATOR, acc, &[("ETH", offer)], mode);
    let retired = debt0 - borrow_of(&t, acc, "ETH");
    let received_repay_usd = usd_of(&t, "ETH", retired);
    let got_usd = match mode {
        SeizeMode::Transfer => {
            usd_of(
                &t,
                "USDC",
                t.token_balance_raw(LIQUIDATOR, "USDC") - liq_usdc0,
            ) + usd_of(
                &t,
                "WBTC",
                t.token_balance_raw(LIQUIDATOR, "WBTC") - liq_wbtc0,
            )
        }
        SeizeMode::Credit(_) => coll(&t, receiver),
    };
    let rounding = 2 * (usd_of(&t, "USDC", 1) + usd_of(&t, "WBTC", 1));
    std::println!(
        "RV-H7 {label}: retired_eth_units={retired} received_repay_usd={received_repay_usd} \
         bonus_bps={bonus} got_usd={got_usd} planned_gross_usd={planned_gross_usd} \
         bound_usd={} ",
        received_repay_usd * (10_000 + bonus) / 10_000 + rounding
    );

    assert!(
        (retired - 9_000_000).abs() <= 1,
        "{label}: the pool received 90% of the offer and the debt must fall by that: {retired}"
    );
    assert!(
        got_usd <= received_repay_usd * (10_000 + bonus) / 10_000 + rounding,
        "{label}: collateral value {got_usd} exceeds received repayment*(1+bonus)+rounding"
    );
    assert!(
        got_usd <= planned_gross_usd * 9 / 10 + rounding,
        "{label}: collateral value {got_usd} must follow the received 90%, not the 100% planned \
         ({planned_gross_usd})"
    );
    assert!(
        got_usd > 0,
        "{label}: the liquidator must receive collateral"
    );
}

#[test]
fn rv_liq_fee_on_transfer_debt_two_legs_transfer_pays_received_value_only() {
    fot_case(SeizeMode::Transfer, "transfer");
}

#[test]
fn rv_liq_fee_on_transfer_debt_two_legs_credit_credits_received_value_only() {
    fot_case(SeizeMode::Credit(0), "credit");
}
