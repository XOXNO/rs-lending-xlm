//! Liquidation fixes on PR #196: permanent regression tests. Each test is
//! named after the verification check it ports (`l<N>` = liquidation-N) and
//! asserts the safe outcome:
//!
//! * MC-3 (`l2`-`l4`): the PR's `seize_all` rule, its zero-payment phantom
//!   closure, its slack bound and the coarse-leg-last residual, which
//!   permissionless cleanup now admits once no offer can repay a debt unit
//!   (review LIQ-1, `l4a`, `l4c`);
//! * MC-1 (`l5a`, `l5b`): a capped leg is credited at most the debt it clears;
//! * MC-2 (`l6`, `l8b`, `l8c`, `mc2_*`): a partial seizure whose floored amount
//!   reaches the pool's half-up balance is recorded as a full close, so the
//!   two seize modes agree, the liquidator gets what it paid for, and a
//!   fractional whole-unit holding stays liquidatable;
//! * MC-4 (`l9a`-`l9c`): an empty receiver skips the governed limit, a
//!   non-empty one keeps it, and nothing exceeds `POSITION_LIMIT_MAX`;
//! * CR-25 (`l10`, `cr25_*`): the Credit estimate applies every caller-free
//!   receiver gate execution applies;
//! * F1 (`l11`, `f1_*`): pool and oracle decimals must agree on every leg a
//!   liquidation moves, and only on those;
//! * base bonus (`l12`): the weighted average rounds once.
//!
//! Tests that measure several properties collect every violation and fail
//! once at the end, so one run prints the whole picture.

use common::constants::{RAY, WAD};
use common::math::fp::{Ray, Wad};
use common::math::fp_core::mul_div_floor;
use common::rates::{
    position_value_ceil, position_value_floor, unscale_borrow_ceil, unscale_supply,
    unscale_supply_floor,
};
use common::types::{HubAssetKey, LiquidationEstimate, PoolStateRaw, PositionMode, SeizeMode};
use controller::constants::BPS;
use controller::types::PriceKey;
use governance::op::{AdminOperation, CreatePoolArgs, SpokeAssetArgs};
use soroban_sdk::{vec, Env, TryFromVal, Vec as SVec, U256};
use test_harness::errors::{self, OracleError};
use test_harness::presets::{AssetConfigPreset, MarketPreset, DEFAULT_ASSET_CONFIG};
use test_harness::{
    asset_payment_vec, eth_preset, f64_to_i128, hub_asset, seed_liquidatable_usdc_eth, usd,
    usd_cents, usdc_preset, usdt_stable_preset, wbtc_preset, LendingTest, ALICE, BOB, CAROL, DAVE,
    DEFAULT_MARKET_PARAMS, HARNESS_HUB, HARNESS_SPOKE, LIQUIDATOR, STABLECOIN_SPOKE,
};

const USDC_UNIT: i128 = 10_000_000;
/// Raw WAD slack for the half-up versus floor valuation steps.
const ULP: i128 = 1_000;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Collects safe-outcome violations and fails once, after printing all.
struct Checks {
    id: &'static str,
    fails: std::vec::Vec<String>,
}

impl Checks {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            fails: std::vec::Vec::new(),
        }
    }

    fn check(&mut self, ok: bool, msg: String) {
        std::println!("[{}] {} {msg}", self.id, if ok { "PASS" } else { "FAIL" });
        if !ok {
            self.fails.push(msg);
        }
    }

    fn finish(self) {
        assert!(
            self.fails.is_empty(),
            "{}: {} safe-outcome violation(s):\n  - {}",
            self.id,
            self.fails.len(),
            self.fails.join("\n  - ")
        );
    }
}

fn code_of<T>(r: &Result<T, soroban_sdk::Error>) -> Option<u32> {
    r.as_ref().err().map(|e| e.get_code())
}

fn key(t: &LendingTest, name: &str) -> HubAssetKey {
    hub_asset(t.resolve_asset(name))
}

/// Collapses a generated `try_*` result into one `Result`.
fn flat<T, E: Into<soroban_sdk::Error>>(
    result: Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<T, soroban_sdk::Error> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(err.into()),
        Err(Ok(err)) => Err(err),
        Err(Err(_)) => panic!("expected a contract error, got an InvokeError"),
    }
}

/// The contract's own valuation of `tokens`: `Wad::from_token(..).mul(price)`.
fn usd_of(t: &LendingTest, name: &str, tokens: i128) -> i128 {
    let m = t.resolve_market(name);
    Wad::from_token(&t.env, tokens, m.decimals)
        .mul(&t.env, Wad::from(m.price_wad))
        .raw()
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

/// The WAD value the risk totals remove when the leg closes (ceil valuation).
fn leg_cleared_usd(t: &LendingTest, acc: u64, name: &str) -> i128 {
    let (_, borrows) = t.ctrl_client().get_account_positions(&acc);
    let scaled = borrows.get(key(t, name)).map_or(0, |p| p.scaled_amount);
    let index = t.ctrl_client().get_market_index(&key(t, name)).borrow_index;
    position_value_ceil(
        &t.env,
        Ray::from(scaled),
        Ray::from(index),
        Wad::from(t.resolve_market(name).price_wad),
    )
    .raw()
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

fn coll(t: &LendingTest, acc: u64) -> i128 {
    t.ctrl_client().get_total_collateral_usd(&acc)
}

fn debt(t: &LendingTest, acc: u64) -> i128 {
    t.ctrl_client().get_total_borrow_usd(&acc)
}

fn amounts(v: &SVec<common::types::PaymentTuple>) -> std::vec::Vec<i128> {
    v.iter().map(|p| p.amount).collect()
}

/// Mints `pays` to the liquidator and liquidates in `mode`.
fn try_liq(
    t: &mut LendingTest,
    acc: u64,
    pays: &[(&str, i128)],
    mode: SeizeMode,
) -> Result<u64, soroban_sdk::Error> {
    let addr = t.get_or_create_user(LIQUIDATOR);
    for (n, a) in pays {
        t.resolve_market(n).token_admin.mint(&addr, a);
    }
    flat(
        t.ctrl_client()
            .try_liquidate(&addr, &acc, &pay_vec(t, pays), &mode),
    )
}

// ---------------------------------------------------------------------------
// MC-3 book: USDC collateral, GOLD3 ($6 per unit) and USDT debt
// ---------------------------------------------------------------------------

const GOLD: &str = "GOLD3";
const USDT: &str = "USDT";

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

/// ALICE: 10,000 USDC against 0.1 GOLD3 (100 units, $600) and 6,800 USDT,
/// then USDC at $0.70: C = $7,000 < D = $7,400.
fn phantom_book() -> (LendingTest, u64) {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(gold_preset())
        .with_market(usdt_stable_preset())
        .build();
    t.supply_raw(ALICE, "USDC", 10_000 * USDC_UNIT);
    t.borrow_raw(ALICE, GOLD, 100);
    t.borrow_raw(ALICE, USDT, 6_800 * USDC_UNIT);
    t.set_price("USDC", usd(7) / 10);
    t.get_or_create_user(LIQUIDATOR);
    t.assert_liquidatable(ALICE);
    let acc = t.resolve_account_id(ALICE);
    (t, acc)
}

/// `(quote, base bonus)`: `Q = floor(C / (1 + base))`.
fn quote(t: &LendingTest, acc: u64) -> (i128, i128) {
    let c = coll(t, acc);
    let d = debt(t, acc);
    assert!(c < d, "fixture must be insolvent: C={c} D={d}");
    let base = estimate(t, acc, &[(GOLD, 1), (USDT, 1)]).bonus_rate_bps;
    let q = mul_div_floor(&t.env, c, WAD, WAD + base * WAD / BPS);
    (q, base)
}

/// liquidation-2 (MC-3 phantom path). The liquidator holds no GOLD3 and offers
/// USDT short of the quote by 0.9 GOLD3 unit, with one GOLD3 unit last. The
/// trim drops the GOLD3 leg whole. Safe: a partial seizure at the bonus on what
/// was charged; the account survives with collateral.
#[test]
fn l2_mc3_phantom_coarse_leg_is_a_partial_seizure() {
    let mut c = Checks::new("liquidation-2");
    let (mut t, acc) = phantom_book();
    let (q, base) = quote(&t, acc);
    let gold_unit = unit_usd(&t, GOLD);
    let usdt_px = t.resolve_market(USDT).price_wad;
    let usdt = ((q - gold_unit * 9 / 10) * USDC_UNIT + usdt_px - 1) / usdt_px;
    let pays = [(USDT, usdt), (GOLD, 1)];

    let est = estimate(&t, acc, &pays);
    let gold = t.resolve_asset(GOLD);
    std::println!(
        "l2 estimate: Q={q} base={base} kept={} refunds={:?} seized={:?} bonus={}",
        est.max_payment_wad,
        amounts(&est.refunds),
        amounts(&est.seized_collaterals),
        est.bonus_rate_bps
    );
    c.check(
        est.refunds.iter().any(|r| r.asset == gold && r.amount == 1),
        "the trim refunds the whole GOLD3 offer".into(),
    );

    let addr = t.get_or_create_user(LIQUIDATOR);
    assert_eq!(t.token_balance_raw(LIQUIDATOR, GOLD), 0, "no GOLD3 held");
    t.resolve_market(USDT).token_admin.mint(&addr, &usdt);
    let usdc_before = t.token_balance_raw(LIQUIDATOR, "USDC");
    let c_before = coll(&t, acc);
    t.ctrl_client()
        .liquidate(&addr, &acc, &pay_vec(&t, &pays), &SeizeMode::Transfer);
    let spent = usdt - t.token_balance_raw(LIQUIDATOR, USDT);
    let charged = usd_of(&t, USDT, spent);
    let received = t.token_balance_raw(LIQUIDATOR, "USDC") - usdc_before;
    let usdc = t.resolve_asset("USDC");
    let fee: i128 = est
        .protocol_fees
        .iter()
        .filter(|p| p.asset == usdc)
        .map(|p| p.amount)
        .sum();
    let taken = usd_of(&t, "USDC", received + fee);
    let fair = charged + charged * est.bonus_rate_bps / BPS;
    let alive = t.account_exists(acc);
    let c_after = if alive { coll(&t, acc) } else { 0 };
    std::println!(
        "l2 exec: Q={q} C_before={c_before} charged={charged} shortfall={} taken={taken} \
         fair={fair} alive={alive} C_after={c_after} gold_spent={}",
        q - charged,
        -t.token_balance_raw(LIQUIDATOR, GOLD)
    );
    c.check(alive, format!("the account survives (alive={alive})"));
    c.check(
        c_after > 0,
        format!("collateral is left: C_after={c_after}"),
    );
    c.check(
        taken <= fair + unit_usd(&t, "USDC") + 4 * ULP,
        format!("taken {taken} <= charged*(1+b) {fair} + one USDC unit"),
    );
    // Liveness of the residue: the fine USDT leg can still close it.
    if alive {
        let usdt_debt = debt_ceil(&t, acc, USDT);
        let r = try_liq(&mut t, acc, &[(USDT, usdt_debt)], SeizeMode::Transfer);
        std::println!(
            "l2 follow-up USDT offer: {r:?} alive_after={}",
            t.account_exists(acc)
        );
        c.check(
            r.is_ok() && !t.account_exists(acc),
            "the residue is closed by a follow-up fine-leg offer".into(),
        );
    }
    c.finish();
}

/// liquidation-3 (MC-3 residual magnitude). The liquidator holds two GOLD3
/// units and offers [USDT for Q - 2 units + e, GOLD3 2]. The trim keeps one
/// GOLD3 unit and the kept-leg conjunct of `seize_all` holds, so every unit is
/// seized while the charge is about one GOLD3 unit below the quote. Asserts
/// the documented bound: shortfall below one unit of the floored, kept leg.
#[test]
fn l3_mc3_kept_coarse_unit_bounds_the_seize_all_shortfall() {
    let mut c = Checks::new("liquidation-3");
    let (mut t, acc) = phantom_book();
    let (q, base) = quote(&t, acc);
    let c_before = coll(&t, acc);
    let gold_unit = unit_usd(&t, GOLD);
    let usdt_px = t.resolve_market(USDT).price_wad;
    let u = ((q - 2 * gold_unit) * USDC_UNIT + usdt_px - 1) / usdt_px + 1;
    let e = usd_of(&t, USDT, u) + 2 * gold_unit - q;
    assert!(e > 0 && e <= 2 * 10i128.pow(12), "pre-trim excess e={e}");
    let pays = [(USDT, u), (GOLD, 2)];
    let est = estimate(&t, acc, &pays);
    std::println!(
        "l3 estimate: Q={q} base={base} e={e} kept={} refunds={:?} seized={:?}",
        est.max_payment_wad,
        amounts(&est.refunds),
        amounts(&est.seized_collaterals)
    );

    let addr = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market(GOLD).token_admin.mint(&addr, &2);
    t.resolve_market(USDT).token_admin.mint(&addr, &u);
    let usdc_before = t.token_balance_raw(LIQUIDATOR, "USDC");
    t.ctrl_client()
        .liquidate(&addr, &acc, &pay_vec(&t, &pays), &SeizeMode::Transfer);
    let gold_spent = 2 - t.token_balance_raw(LIQUIDATOR, GOLD);
    let usdt_spent = u - t.token_balance_raw(LIQUIDATOR, USDT);
    let charged = usd_of(&t, USDT, usdt_spent) + usd_of(&t, GOLD, gold_spent);
    let shortfall = q - charged;
    let received = t.token_balance_raw(LIQUIDATOR, "USDC") - usdc_before;
    let alive = t.account_exists(acc);
    std::println!(
        "l3 exec: Q={q} C_before={c_before} charged={charged} shortfall={shortfall} \
         gold_unit={gold_unit} gold_spent={gold_spent} usdt_spent={usdt_spent} \
         received_usdc={received} taken_usd={} alive={alive}",
        usd_of(&t, "USDC", received)
    );
    c.check(!alive, "seize_all: the account is deleted".into());
    c.check(
        gold_spent == 1,
        format!("exactly one GOLD3 unit pulled ({gold_spent})"),
    );
    c.check(
        shortfall > gold_unit - 2 * 10i128.pow(12) && shortfall < gold_unit,
        format!("gold_unit - 2e-6 USD < shortfall {shortfall} < gold_unit {gold_unit}"),
    );
    c.finish();
}

// ---------------------------------------------------------------------------
// liquidation-4: liveness of the dropped-coarse-leg partial
// ---------------------------------------------------------------------------

fn asset(
    name: &'static str,
    decimals: u32,
    price_wad: i128,
    ltv: u32,
    threshold: u32,
    bonus: u32,
    liquidity: f64,
) -> MarketPreset {
    MarketPreset {
        name,
        decimals,
        price_wad,
        initial_liquidity: liquidity,
        config: AssetConfigPreset {
            loan_to_value: ltv,
            liquidation_threshold: threshold,
            liquidation_bonus: bonus,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// 1 000 COL ($0.10 each, $100) against a $90 CHEAP debt and two EXP units at
/// $10 each (PR fixture `insolvent_book_with_a_ten_dollar_unit(usd(1) / 10)`).
fn ten_dollar_unit_book() -> (LendingTest, u64) {
    let mut t = LendingTest::new()
        .with_market(asset("COL", 7, usd(1), 7500, 8000, 500, 1_000_000.0))
        .with_market(asset("CHEAP", 7, usd(1), 7500, 8000, 500, 1_000_000.0))
        .with_market(asset("EXP", 3, usd(10_000), 7500, 8000, 500, 100.0))
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply(ALICE, "COL", 1_000.0);
    t.borrow(ALICE, "CHEAP", 90.0);
    t.borrow(ALICE, "EXP", 0.002);
    t.set_price("COL", usd(1) / 10);
    t.get_or_create_user(LIQUIDATOR);
    let acc = t.resolve_account_id(ALICE);
    assert!(t.total_collateral_raw(ALICE) < t.total_debt_raw(ALICE));
    (t, acc)
}

/// liquidation-4, review LIQ-1. (a) A coarse-leg-last payment order (EXP
/// last) is a proportional partial that leaves a $5.5 / $20 insolvent residue
/// whose collateral-backed quote ($5.5 / 1.05) repays no EXP unit ($10): the
/// liquidation's own cleanup now socializes it in the same call. Fix LIQ-1
/// (was `l4_mc3_dropped_coarse_leg_residual_liveness`: the residue stayed,
/// every follow-up offer reverted, permissionless cleanup refused it and only
/// governed force-socialize closed it). (b) The fine leg last seizes every
/// unit in one call.
#[test]
fn l4_mc3_dropped_coarse_leg_residual_is_cleaned_in_the_same_call() {
    let mut c = Checks::new("liquidation-4");

    // (a) coarse leg last.
    let (mut t, acc) = ten_dollar_unit_book();
    let pays = [("CHEAP", 90_0000000i128), ("EXP", 2)];
    let r = try_liq(&mut t, acc, &pays, SeizeMode::Transfer);
    let alive = t.account_exists(acc);
    let cheap_spent = 90_0000000 - t.token_balance_raw(LIQUIDATOR, "CHEAP");
    let exp_left = t.token_balance_raw(LIQUIDATOR, "EXP");
    std::println!("l4a: {r:?} alive={alive} cheap_spent={cheap_spent} exp_left={exp_left}");
    c.check(
        r.is_ok() && !alive,
        "(a) the residue the quote cannot reach is cleaned in the same call".into(),
    );
    c.check(
        cheap_spent == 90_0000000 && exp_left == 2,
        format!("(a) the trim kept CHEAP and refunded EXP ({cheap_spent}, {exp_left})"),
    );

    // (b) fine leg last.
    let (mut t, acc) = ten_dollar_unit_book();
    let c0 = coll(&t, acc);
    let pays = [("EXP", 2i128), ("CHEAP", 90_0000000)];
    let est = estimate(&t, acc, &pays);
    let r = try_liq(&mut t, acc, &pays, SeizeMode::Transfer);
    let alive = t.account_exists(acc);
    std::println!(
        "l4b: {r:?} kept={} refunds={:?} alive={alive} C_before={c0} C_after={} \
         exp_spent={} cheap_spent={}",
        est.max_payment_wad,
        amounts(&est.refunds),
        if alive { coll(&t, acc) } else { 0 },
        2 - t.token_balance_raw(LIQUIDATOR, "EXP"),
        90_0000000 - t.token_balance_raw(LIQUIDATOR, "CHEAP")
    );
    c.check(
        r.is_ok() && !alive,
        "(b) the fine leg last seizes every unit in one call".into(),
    );
    c.finish();
}

/// 1 000 COL against two EXP units ($10 each, 3 decimals, $20 of debt) with
/// COL then priced at `col_price`: a lone coarse debt leg drifted insolvent.
fn coarse_debt_book(col_price: i128) -> (LendingTest, u64) {
    let mut t = LendingTest::new()
        .with_market(asset("COL", 7, usd(1), 7500, 8000, 500, 1_000_000.0))
        .with_market(asset("EXP", 3, usd(10_000), 7500, 8000, 500, 100.0))
        .with_min_borrow_collateral_disabled()
        .build();
    t.supply(ALICE, "COL", 1_000.0);
    t.borrow(ALICE, "EXP", 0.002);
    t.set_price("COL", col_price);
    t.get_or_create_user(LIQUIDATOR);
    let acc = t.resolve_account_id(ALICE);
    assert!(t.total_collateral_raw(ALICE) < t.total_debt_raw(ALICE));
    (t, acc)
}

/// Review LIQ-1: $7 of collateral against $20 of EXP debt. The quote, $7 /
/// 1.05, repays no EXP unit, so every offer, Transfer or Credit, one unit or
/// two, is trimmed to nothing and reverts INVALID_PAYMENTS. Permissionless
/// cleanup admits the account although its collateral is above the $5 dust
/// cap (was CANNOT_CLEAN_BAD_DEBT, leaving only governed force-socialize).
#[test]
fn l4c_unliquidatable_coarse_residue_is_cleaned_permissionlessly() {
    let (mut t, acc) = coarse_debt_book(usd(1) * 7 / 1_000);
    let c0 = coll(&t, acc);
    assert!(c0 > 5 * WAD && c0 < 8 * WAD, "collateral {c0}");
    for (pays, mode) in [
        (&[("EXP", 1i128)][..], SeizeMode::Transfer),
        (&[("EXP", 2i128)][..], SeizeMode::Transfer),
        (&[("EXP", 2i128)][..], SeizeMode::Credit(0)),
    ] {
        assert_eq!(
            code_of(&try_liq(&mut t, acc, pays, mode)),
            Some(errors::INVALID_PAYMENTS),
            "no offer can repay a unit"
        );
    }

    t.try_clean_bad_debt_by_id(acc)
        .expect("permissionless cleanup admits the unliquidatable residue");
    assert!(!t.account_exists(acc));
}

/// Review LIQ-1 control: $15 of collateral against the same $20. The quote,
/// $15 / 1.05, repays one EXP unit, so the account stays liquidatable and
/// permissionless cleanup above the dust cap still refuses it.
#[test]
fn l4c_control_liquidatable_coarse_residue_is_not_cleaned() {
    let (mut t, acc) = coarse_debt_book(usd(1) * 15 / 1_000);
    assert_eq!(
        code_of(&t.try_clean_bad_debt_by_id(acc)),
        Some(errors::CANNOT_CLEAN_BAD_DEBT)
    );
    try_liq(&mut t, acc, &[("EXP", 1)], SeizeMode::Transfer).expect("one EXP unit fits the quote");
}

// ---------------------------------------------------------------------------
// liquidation-5: MC-1 capped-leg credit
// ---------------------------------------------------------------------------

const LOW3: &str = "LOW3";
const LOW3_DECIMALS: u32 = 3;
const LOW3_UNIT_USD: i128 = WAD / 1_000;

fn low3() -> MarketPreset {
    MarketPreset {
        name: LOW3,
        decimals: LOW3_DECIMALS,
        price_wad: WAD,
        initial_liquidity: 1_000_000.0,
        config: DEFAULT_ASSET_CONFIG,
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// `(ceil risk value, floor value, ceiling credit, ceiling units)` of LOW3.
fn low3_leg(t: &LendingTest, account_id: u64) -> (i128, i128, i128, i128) {
    let asset = t.resolve_asset(LOW3);
    let (_, borrows) = t.ctrl_client().get_account_positions(&account_id);
    let scaled = Ray::from(
        borrows
            .get(hub_asset(asset.clone()))
            .expect("LOW3 debt leg")
            .scaled_amount,
    );
    let index = Ray::from(
        t.pool_client(LOW3)
            .get_sync_data(&hub_asset(asset))
            .state
            .borrow_index,
    );
    let price = Wad::from(t.resolve_market(LOW3).price_wad);
    let risk = position_value_ceil(&t.env, scaled, index, price).raw();
    let floor = position_value_floor(&t.env, scaled, index, price).raw();
    let ceil_units = unscale_borrow_ceil(&t.env, scaled, index, LOW3_DECIMALS);
    let credited = Wad::from_token(&t.env, ceil_units, LOW3_DECIMALS)
        .mul(&t.env, price)
        .raw();
    (risk, floor, credited, ceil_units)
}

/// liquidation-5a (MC-1 direct path). Closing a coarse leg at its ceiling is
/// credited at most the debt it clears, and a covered account stays covered.
#[test]
fn l5a_mc1_capped_leg_credited_at_most_the_cleared_debt() {
    let mut c = Checks::new("liquidation-5a");
    let mut t = LendingTest::new()
        .with_market(low3())
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_min_borrow_collateral_disabled()
        .with_max_utilization_disabled_all_markets()
        .build();
    t.supply(BOB, LOW3, 10_000.0);
    t.supply(BOB, "USDC", 10_000.0);
    t.supply(ALICE, "ETH", 1.0);
    t.borrow(ALICE, LOW3, 700.0);
    t.borrow(ALICE, "USDC", 100.0);
    let alice = t.resolve_account_id(ALICE);

    let gap_floor = LOW3_UNIT_USD * 4 / 10;
    let mut gap = 0;
    for _ in 0..120 {
        t.advance_and_sync(86_400);
        let (risk, _, credited, _) = low3_leg(&t, alice);
        gap = credited - risk;
        if gap >= gap_floor {
            break;
        }
    }
    assert!(
        gap >= gap_floor && gap < LOW3_UNIT_USD,
        "ceiling gap {gap} outside [0.4, 1) LOW3 units"
    );
    let (risk, floor, credited, ceil_units) = low3_leg(&t, alice);
    let debt_quote = debt(&t, alice);
    t.set_price("ETH", debt_quote + gap / 4);
    let coll_pre = coll(&t, alice);
    let debt_pre = debt(&t, alice);
    assert!(
        coll_pre >= debt_pre && coll_pre - debt_pre <= gap / 2,
        "pre: C {coll_pre} must sit within half a gap above D {debt_pre}"
    );
    assert!(t.can_be_liquidated(ALICE));

    let carol = t.get_or_create_user(CAROL);
    let low3_market = t.resolve_market(LOW3);
    low3_market.token_admin.mint(&carol, &ceil_units);
    let payments = vec![&t.env, (hub_asset(low3_market.asset.clone()), ceil_units)];
    let est = t
        .ctrl_client()
        .get_liquidation_estimate(&alice, &payments, &SeizeMode::Transfer);
    std::println!(
        "l5a: gap={gap} risk_ceil={risk} floor={floor} ceiling_credit={credited} \
         kept={} bonus={}",
        est.max_payment_wad,
        est.bonus_rate_bps
    );
    c.check(
        est.max_payment_wad <= risk,
        format!("kept {} <= position_value_ceil {risk}", est.max_payment_wad),
    );
    c.check(
        est.max_payment_wad == floor,
        format!(
            "a capped leg is credited at position_value_floor {floor} (kept {})",
            est.max_payment_wad
        ),
    );

    t.ctrl_client()
        .liquidate(&carol, &alice, &payments, &SeizeMode::Transfer);
    let closed = t.borrow_balance_raw(ALICE, LOW3) == 0;
    let coll_post = coll(&t, alice);
    let debt_post = debt(&t, alice);
    std::println!(
        "l5a exec: closed={closed} taken={} cleared={} C'={coll_post} D'={debt_post}",
        coll_pre - coll_post,
        debt_pre - debt_post
    );
    c.check(closed, "the coarse leg closed".into());
    c.check(
        coll_post >= debt_post,
        format!("C' {coll_post} >= D' {debt_post}"),
    );
    c.finish();
}

/// liquidation-5b (MC-1 solvent-trim sibling). GOLD3 at its ceiling last, USDC
/// sized so the pre-trim excess is $1, below one GOLD3 unit: the trim removes
/// no unit and must not re-price the leg above the debt it clears.
#[test]
fn l5b_mc1_solvent_trim_keeps_the_cleared_credit() {
    let mut c = Checks::new("liquidation-5b");
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(gold_preset())
        .with_market(eth_preset())
        .build();
    t.supply(ALICE, "ETH", 10.0);
    t.borrow_raw(ALICE, GOLD, 100);
    t.borrow(ALICE, "USDC", 12_000.0);
    let acc = t.resolve_account_id(ALICE);
    t.advance_and_sync(86_400);
    let gold_ceil = debt_ceil(&t, acc, GOLD);
    let gold_cleared = leg_cleared_usd(&t, acc, GOLD);
    let gap = usd_of(&t, GOLD, gold_ceil) - gold_cleared;
    assert!(
        gap > unit_usd(&t, GOLD) / 4 && gap < unit_usd(&t, GOLD),
        "ceiling gap {gap} must be a large fraction of one GOLD3 unit"
    );
    let d = debt(&t, acc);
    t.set_price("ETH", d * 11_875 / 100_000);
    assert!(coll(&t, acc) > debt(&t, acc), "fixture must be solvent");
    assert!(t.can_be_liquidated(ALICE));

    let usdc_ceil = debt_ceil(&t, acc, "USDC");
    let ideal = estimate(&t, acc, &[("USDC", usdc_ceil)]).max_payment_wad;
    assert!(ideal > gold_cleared && ideal < debt(&t, acc));
    let usdc_px = t.resolve_market("USDC").price_wad;
    let usdc_amt = ((ideal - gold_cleared + WAD) * USDC_UNIT + usdc_px - 1) / usdc_px;
    let pays = [("USDC", usdc_amt), (GOLD, gold_ceil)];
    let usdc_credit = usd_of(&t, "USDC", usdc_amt);
    let est = estimate(&t, acc, &pays);

    let addr = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market("USDC").token_admin.mint(&addr, &usdc_amt);
    t.resolve_market(GOLD).token_admin.mint(&addr, &gold_ceil);
    let (c0, d0) = (coll(&t, acc), debt(&t, acc));
    t.ctrl_client()
        .liquidate(&addr, &acc, &pay_vec(&t, &pays), &SeizeMode::Transfer);
    let (c1, d1) = (coll(&t, acc), debt(&t, acc));
    let taken = c0 - c1;
    let cleared = d0 - d1;
    let fair = cleared + cleared * est.bonus_rate_bps / BPS;
    std::println!(
        "l5b: ideal={ideal} gold_cleared={gold_cleared} gap={gap} usdc_credit={usdc_credit} \
         kept={} refunds={:?} bonus={} taken={taken} cleared={cleared} fair={fair}",
        est.max_payment_wad,
        amounts(&est.refunds),
        est.bonus_rate_bps
    );
    c.check(
        est.max_payment_wad <= usdc_credit + gold_cleared + 1,
        format!(
            "kept {} <= usdc_credit + gold_cleared + 1 raw = {}",
            est.max_payment_wad,
            usdc_credit + gold_cleared + 1
        ),
    );
    c.check(
        taken <= fair + unit_usd(&t, "ETH") * 2 + 4 * ULP,
        format!("taken {taken} <= (1 + b) * cleared {fair} (+2 ETH units)"),
    );
    c.check(
        est.refunds.iter().all(|r| r.amount > 0),
        "no zero-amount refund".into(),
    );
    c.finish();
}

// ---------------------------------------------------------------------------
// liquidation-6: MC-2 mode parity in the half-up window
// ---------------------------------------------------------------------------

const DAY_SECS: u64 = 86_400;
const SEVEN: u32 = 7;
const UNIT_RAY: i128 = 100_000_000_000_000_000_000;
const ONE_ETH_RAW: i128 = 10_000_000;

fn scaled_supply(t: &LendingTest, account_id: u64, asset_name: &str) -> i128 {
    t.ctrl_client()
        .get_account_positions(&account_id)
        .0
        .get(key(t, asset_name))
        .map_or(0, |p| p.scaled_amount)
}

fn pool_state(t: &LendingTest, asset_name: &str) -> PoolStateRaw {
    t.pool_client(asset_name)
        .get_sync_data(&key(t, asset_name))
        .state
}

/// Exact form of `delta < units` asset units of shares at `supply_index`
/// (`share_gap_under_units`, tests/controller/liquidation_seize_modes.rs).
fn share_gap_under_units(
    env: &Env,
    delta: i128,
    supply_index: i128,
    decimals: u32,
    units: u32,
) -> bool {
    let wide = |v: i128| U256::from_u128(env, u128::try_from(v).expect("non-negative"));
    let one_unit_times_index = U256::from_u128(env, 10).pow(54 - decimals);
    wide(delta).mul(&wide(supply_index))
        < one_unit_times_index.mul(&U256::from_u128(env, units.into()))
}

/// ETH with a zero protocol fee, so pool `supplied` measures the Transfer
/// share burn exactly (a fee would re-mint revenue shares in the same call).
fn eth_no_fee() -> MarketPreset {
    MarketPreset {
        config: AssetConfigPreset {
            liquidation_fees: 0,
            ..DEFAULT_ASSET_CONFIG
        },
        ..eth_preset()
    }
}

/// Insolvent borrower whose single ETH leg carries a sub-half-unit remainder.
fn window_fixture() -> (LendingTest, u64, i128) {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_no_fee())
        .build();
    t.supply(DAVE, "ETH", 100.0);
    t.supply(BOB, "USDC", 200_000.0);
    t.supply_raw(ALICE, "ETH", ONE_ETH_RAW);
    t.borrow(ALICE, "USDC", 1_400.0);
    t.borrow(BOB, "ETH", 50.0);
    let alice = t.resolve_account_id(ALICE);
    let shares = scaled_supply(&t, alice, "ETH");
    let mut found = false;
    for _ in 0..120 {
        t.advance_and_sync(DAY_SECS);
        let index = Ray::from(pool_state(&t, "ETH").supply_index);
        let actual_ray = Ray::from(shares).mul(&t.env, index).raw();
        let half_up = unscale_supply(&t.env, Ray::from(shares), index, SEVEN);
        let floor = unscale_supply_floor(&t.env, Ray::from(shares), index, SEVEN);
        let remainder = actual_ray - floor * UNIT_RAY;
        if half_up == floor && remainder >= UNIT_RAY / 100 {
            found = true;
            break;
        }
    }
    assert!(found, "no sub-half-unit remainder found within 120 days");
    t.set_price("ETH", usd(1_300));
    assert!(
        coll(&t, alice) < debt(&t, alice),
        "fixture must be insolvent"
    );
    t.get_or_create_user(LIQUIDATOR);
    (t, alice, shares)
}

fn window_offer(t: &LendingTest, alice: u64) -> i128 {
    let probe = asset_payment_vec(&t.env, t.resolve_asset("USDC"), 10_000 * ONE_ETH_RAW);
    let quote = t
        .ctrl_client()
        .get_liquidation_estimate(&alice, &probe, &SeizeMode::Transfer)
        .max_payment_wad;
    quote / (WAD / USDC_UNIT) - 3
}

/// liquidation-6. Two identical window fixtures, one liquidated in Transfer,
/// one in Credit(0), with the same partial USDC offer. Safe: the share debits
/// differ by less than one asset unit (the harness's documented parity bound)
/// and the Transfer liquidator receives the planned floored amount (or the
/// dropped unit is refunded).
#[test]
fn l6_mc2_window_mode_parity_and_payout() {
    let mut c = Checks::new("liquidation-6");

    // Fixture A: Transfer.
    let (mut ta, alice_a, shares_a) = window_fixture();
    let offer_a = window_offer(&ta, alice_a);
    let usdc = ta.resolve_asset("USDC");
    let pays = asset_payment_vec(&ta.env, usdc.clone(), offer_a);
    let index_a = pool_state(&ta, "ETH").supply_index;
    let half_up = unscale_supply(&ta.env, Ray::from(shares_a), Ray::from(index_a), SEVEN);
    let floor = unscale_supply_floor(&ta.env, Ray::from(shares_a), Ray::from(index_a), SEVEN);
    let est_t = ta
        .ctrl_client()
        .get_liquidation_estimate(&alice_a, &pays, &SeizeMode::Transfer);
    let est_c = ta
        .ctrl_client()
        .get_liquidation_estimate(&alice_a, &pays, &SeizeMode::Credit(0));
    let planned_t = est_t.seized_collaterals.get(0).map_or(0, |s| s.amount);
    let planned_c = est_c.seized_collaterals.get(0).map_or(0, |s| s.amount);
    let before_a = pool_state(&ta, "ETH");
    let liq_a = ta.get_or_create_user(LIQUIDATOR);
    ta.resolve_market("USDC").token_admin.mint(&liq_a, &offer_a);
    let eth_before = ta.token_balance_raw(LIQUIDATOR, "ETH");
    ta.ctrl_client()
        .liquidate(&liq_a, &alice_a, &pays, &SeizeMode::Transfer);
    let received = ta.token_balance_raw(LIQUIDATOR, "ETH") - eth_before;
    let usdc_spent_a = offer_a - ta.token_balance_raw(LIQUIDATOR, "USDC");
    let after_a = pool_state(&ta, "ETH");
    let transfer_removed = before_a.supplied - after_a.supplied;
    let revenue_a = after_a.revenue - before_a.revenue;
    let alive_a = ta.account_exists(alice_a);

    // Fixture B: Credit(0).
    let (mut tb, alice_b, shares_b) = window_fixture();
    let offer_b = window_offer(&tb, alice_b);
    assert_eq!(
        (shares_a, offer_a),
        (shares_b, offer_b),
        "fixtures must match"
    );
    assert_eq!(index_a, pool_state(&tb, "ETH").supply_index);
    let pays_b = asset_payment_vec(&tb.env, tb.resolve_asset("USDC"), offer_b);
    let before_b = pool_state(&tb, "ETH");
    let liq_b = tb.get_or_create_user(LIQUIDATOR);
    tb.resolve_market("USDC").token_admin.mint(&liq_b, &offer_b);
    let receiver = tb
        .ctrl_client()
        .liquidate(&liq_b, &alice_b, &pays_b, &SeizeMode::Credit(0));
    let usdc_spent_b = offer_b - tb.token_balance_raw(LIQUIDATOR, "USDC");
    let credit_removed = scaled_supply(&tb, receiver, "ETH");
    let after_b = pool_state(&tb, "ETH");
    let revenue_b = after_b.revenue - before_b.revenue;
    let alive_b = tb.account_exists(alice_b);

    let gap = credit_removed - transfer_removed;
    std::println!(
        "l6: shares={shares_a} index={index_a} half_up={half_up} floor={floor} offer={offer_a} \
         planned_transfer_units={planned_t} planned_credit_shares={planned_c} \
         refunds_t={:?} received={received} usdc_spent=({usdc_spent_a},{usdc_spent_b}) \
         transfer_removed={transfer_removed} credit_removed={credit_removed} gap={gap} \
         gap_units={:.4} residue_to_revenue=({revenue_a},{revenue_b}) alive=({alive_a},{alive_b})",
        amounts(&est_t.refunds),
        gap as f64 * index_a as f64 / (UNIT_RAY as f64 * RAY as f64),
    );
    c.check(
        usdc_spent_a == usdc_spent_b,
        format!("both modes charge the same USDC ({usdc_spent_a} vs {usdc_spent_b})"),
    );
    c.check(
        share_gap_under_units(&ta.env, gap.abs(), index_a, SEVEN, 1),
        format!(
            "Credit/Transfer share gap {gap} is below one asset unit of shares \
             (liquidation_seize_modes.rs:24-31 bound)"
        ),
    );
    c.check(
        received >= floor || !est_t.refunds.is_empty(),
        format!(
            "the Transfer liquidator receives the planned floored amount {floor} \
             (got {received}) or the dropped unit is refunded (refunds {:?})",
            amounts(&est_t.refunds)
        ),
    );
    c.finish();
}

// ---------------------------------------------------------------------------
// liquidation-8: whole-unit rounded-down guard, reachability
// ---------------------------------------------------------------------------

const LIQ: &str = "LIQVID";

struct Zdc {
    t: LendingTest,
    spoke: u32,
    liq_key: HubAssetKey,
    usdc_key: HubAssetKey,
}

/// A 0-decimal collateral-only market in its own hub and spoke (LT 9000),
/// borrowing USDC from the base hub. `usdc_fot_bps` makes USDC a fee-on-
/// transfer token (0 = standard).
fn zdc_setup(nav_wad: i128, usdc_fot_bps: i128) -> Zdc {
    let usdc = MarketPreset {
        initial_liquidity: 50_000_000.0,
        ..usdc_preset()
    };
    let liq = MarketPreset {
        name: LIQ,
        decimals: 0,
        price_wad: nav_wad,
        initial_liquidity: 0.0,
        config: AssetConfigPreset {
            is_collateralizable: false,
            is_borrowable: false,
            is_flashloanable: false,
            flashloan_fee: 0,
            liquidation_fees: 0,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    };
    let b = LendingTest::new();
    let b = if usdc_fot_bps > 0 {
        b.with_fee_on_transfer_market(usdc, usdc_fot_bps)
    } else {
        b.with_market(usdc)
    };
    let mut t = b.with_market(liq).build();
    let admin = t.admin();
    let liq_addr = t.resolve_asset(LIQ);
    let usdc_addr = t.resolve_asset("USDC");
    let hub = t.create_hub();
    let gov = t.gov_client();
    gov.execute_immediate(
        &admin,
        &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
            hub_id: hub,
            asset: liq_addr.clone(),
            params: DEFAULT_MARKET_PARAMS.to_market_params(&liq_addr, 0),
        }),
    );
    let spoke_val = gov.execute_immediate(&admin, &AdminOperation::AddSpoke);
    let spoke = u32::try_from_val(&t.env, &spoke_val).unwrap();
    gov.execute_immediate(
        &admin,
        &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
            hub_id: hub,
            asset: liq_addr.clone(),
            spoke_id: spoke,
            can_collateral: true,
            can_borrow: false,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: 8_000,
            threshold: 9_000,
            bonus: 500,
            liquidation_fees: 0,
            supply_cap: 10_000_000,
            borrow_cap: 0,
        }),
    );
    gov.execute_immediate(
        &admin,
        &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
            hub_id: HARNESS_HUB,
            asset: usdc_addr.clone(),
            spoke_id: spoke,
            can_collateral: false,
            can_borrow: true,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: 7_500,
            threshold: 8_000,
            bonus: 500,
            liquidation_fees: 0,
            supply_cap: 1_000_000_000 * USDC_UNIT,
            borrow_cap: 1_000_000_000 * USDC_UNIT,
        }),
    );
    t.get_or_create_user(LIQUIDATOR);
    Zdc {
        t,
        spoke,
        liq_key: HubAssetKey {
            hub_id: hub,
            asset: liq_addr,
        },
        usdc_key: HubAssetKey {
            hub_id: HARNESS_HUB,
            asset: usdc_addr,
        },
    }
}

impl Zdc {
    fn open(&mut self, name: &str, units: i128, usdc_raw: i128) -> u64 {
        let who = self.t.get_or_create_user(name);
        self.t.resolve_market(LIQ).token_admin.mint(&who, &units);
        let id = self.t.ctrl_client().supply(
            &who,
            &0,
            &self.spoke,
            &vec![&self.t.env, (self.liq_key.clone(), units)],
        );
        self.t.ctrl_client().borrow(
            &who,
            &id,
            &vec![&self.t.env, (self.usdc_key.clone(), usdc_raw)],
            &None,
        );
        id
    }

    fn liq_scaled(&self, id: u64) -> i128 {
        self.t
            .ctrl_client()
            .get_account_positions(&id)
            .0
            .get(self.liq_key.clone())
            .map_or(0, |p| p.scaled_amount)
    }

    fn liq_index(&self) -> i128 {
        self.t
            .ctrl_client()
            .get_market_indexes_detailed(&SVec::from_array(&self.t.env, [self.liq_key.clone()]))
            .get(0)
            .unwrap()
            .supply_index
    }

    fn estimate(&self, id: u64, usdc_raw: i128, mode: SeizeMode) -> LiquidationEstimate {
        self.t.ctrl_client().get_liquidation_estimate(
            &id,
            &vec![&self.t.env, (self.usdc_key.clone(), usdc_raw)],
            &mode,
        )
    }

    fn try_liquidate(
        &mut self,
        id: u64,
        usdc_raw: i128,
        mode: SeizeMode,
    ) -> Result<u64, soroban_sdk::Error> {
        let who = self.t.get_or_create_user(LIQUIDATOR);
        self.t
            .resolve_market("USDC")
            .token_admin
            .mint(&who, &usdc_raw);
        flat(self.t.ctrl_client().try_liquidate(
            &who,
            &id,
            &vec![&self.t.env, (self.usdc_key.clone(), usdc_raw)],
            &mode,
        ))
    }
}

/// liquidation-8b (reachability, part 1). A 0-decimal collateral-only market
/// cannot be borrowed or flash-loaned, so its supply index stays at RAY and
/// holdings stay whole without under-delivery.
#[test]
fn l8b_zero_decimal_index_stays_ray() {
    let mut z = zdc_setup(usd(1_000), 0);
    let id = z.open("alice", 100, 50_000 * USDC_UNIT);
    z.t.advance_and_sync(365 * 86_400);
    let index = z.liq_index();
    std::println!(
        "l8b: index={index} scaled={} usdc_debt={}",
        z.liq_scaled(id),
        z.t.ctrl_client().get_borrow_amount(&id, &z.usdc_key)
    );
    assert_eq!(index, RAY, "supply index stays RAY");
    assert_eq!(z.liq_scaled(id), 100 * RAY, "holding stays whole");
}

/// liquidation-8c (reachability, part 2). A 5% fee-on-transfer USDC debt
/// token under-delivers a Credit(0) liquidation planned at 2 whole units, so
/// the victim keeps 1.1 units. Priced solvent with HF < 1 and D above one unit
/// at the bonus, a one-unit step-back would turn every one-unit sale into
/// zero units. Safe: some offer liquidates the account (MC-2 promotion: the
/// one-unit sale is the pool's full close and seizes the whole holding).
#[test]
fn l8c_fractional_whole_unit_holding_stays_liquidatable() {
    let mut c = Checks::new("liquidation-8c");
    let mut z = zdc_setup(usd(1_000), 500);
    // 3 units at $1,000 against 2,000 USDC.
    let id = z.open("alice", 3, 2_000 * USDC_UNIT);
    assert_eq!(z.liq_scaled(id), 3 * RAY);

    // Insolvent at $600: an offer below the quote seizes floor(2.1) = 2 units.
    z.t.set_price(LIQ, usd(600));
    let offer = 1_200 * USDC_UNIT;
    let est = z.estimate(id, offer, SeizeMode::Credit(0));
    std::println!(
        "l8c step 2 estimate: kept={} seized={:?} bonus={}",
        est.max_payment_wad,
        est.seized_collaterals
            .iter()
            .map(|s| s.amount)
            .collect::<std::vec::Vec<_>>(),
        est.bonus_rate_bps
    );
    let receiver = z
        .try_liquidate(id, offer, SeizeMode::Credit(0))
        .expect("Credit(0) liquidation at 2 units");
    let kept_scaled = z.liq_scaled(id);
    let debt_raw = z.t.ctrl_client().get_borrow_amount(&id, &z.usdc_key);
    std::println!(
        "l8c after under-delivery: victim_scaled={kept_scaled} ({:.6} units) \
         receiver_scaled={} victim_usdc_debt={debt_raw}",
        kept_scaled as f64 / RAY as f64,
        z.liq_scaled(receiver)
    );
    let f = kept_scaled - RAY;
    assert!(
        f > RAY / 1_000_000 && f < RAY / 2,
        "victim must hold 1 + f units with f in (1e-6, 0.5): {kept_scaled}"
    );

    // Price so that the account is solvent, HF < 1, D > (unit + m) / (1 + b),
    // and the quote is partial (the curve does not ask for a full close): a
    // twice-the-debt offer is trimmed below D. Design-neutral selection: the
    // trimmed quote is below D on any design (zero on a step-back dead end).
    let mut chosen = None;
    for nav in (800..=1_000).step_by(2) {
        z.t.set_price(LIQ, usd(nav));
        let cv = z.t.ctrl_client().get_total_collateral_usd(&id);
        let dv = z.t.ctrl_client().get_total_borrow_usd(&id);
        let hf = z.t.ctrl_client().get_health_factor(&id);
        let unit = usd(nav);
        if !(cv > dv && hf < WAD && dv * 105 > unit * 100 + unit / 10_000) {
            continue;
        }
        let big = 2 * dv / (WAD / USDC_UNIT);
        let e = z.estimate(id, big, SeizeMode::Transfer);
        std::println!(
            "l8c scan nav={nav} C={cv} D={dv} HF={hf} est(2D): kept={} seized={} bonus={}",
            e.max_payment_wad,
            e.seized_collaterals.len(),
            e.bonus_rate_bps
        );
        if e.max_payment_wad < dv {
            chosen = Some((nav, cv, dv, hf));
            break;
        }
    }
    let (nav, cv, dv, hf) =
        chosen.expect("a solvent HF < 1 price with D > U / (1 + b) and a partial quote");
    std::println!("l8c state: nav={nav} C={cv} D={dv} HF={hf}");

    let full = 2 * dv / (WAD / USDC_UNIT);
    let unit_rep = usd(nav) * 100 / 105 / (WAD / USDC_UNIT) + 1;
    let mut any_ok = false;
    'offers: for (label, amount) in [
        ("one unit at the bonus", unit_rep),
        ("half a unit", unit_rep / 2),
        ("the whole debt", dv / (WAD / USDC_UNIT) + 1),
        ("twice the debt", full),
    ] {
        for mode in [SeizeMode::Transfer, SeizeMode::Credit(0)] {
            let est = z.estimate(id, amount, mode);
            let r = z.try_liquidate(id, amount, mode);
            std::println!(
                "l8c offer {label} ({amount}) {mode:?}: estimate kept={} seized={} -> {r:?}",
                est.max_payment_wad,
                est.seized_collaterals.len()
            );
            if r.is_ok() {
                any_ok = true;
                break 'offers;
            }
        }
    }
    c.check(
        any_ok,
        format!(
            "some offer liquidates a solvent HF<1 account holding {:.6} units",
            kept_scaled as f64 / RAY as f64
        ),
    );
    c.finish();
}

// ---------------------------------------------------------------------------
// liquidation-9: MC-4 receivers after a limit cut
// ---------------------------------------------------------------------------

fn mc4_fixture() -> LendingTest {
    let mut t = LendingTest::new()
        .with_market(MarketPreset {
            initial_liquidity: 0.0,
            ..usdc_preset()
        })
        .with_market(eth_preset())
        .with_market(wbtc_preset())
        .with_max_utilization_disabled_all_markets()
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.supply(ALICE, "WBTC", 0.1);
    t.borrow(ALICE, "ETH", 5.0);
    t.supply(BOB, "ETH", 20.0);
    t.borrow(BOB, "USDC", 9_700.0);
    t.set_price("ETH", usd(2_600));
    assert!(t.can_be_liquidated(ALICE));
    t
}

/// liquidation-9a. Transfer stays blocked on the cash-less market (shared
/// residual), a non-empty receiver keeps the lowered limit, Credit(0) takes
/// both legs.
#[test]
fn l9a_mc4_credit_zero_fits_the_victims_legs() {
    let mut c = Checks::new("liquidation-9a");
    let mut t = mc4_fixture();
    t.set_position_limits(1, 1);
    let transfer = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    std::println!("l9a transfer: {transfer:?}");
    c.check(
        code_of(&transfer) == Some(errors::INSUFFICIENT_LIQUIDITY),
        "Transfer: INSUFFICIENT_LIQUIDITY (residual)".into(),
    );
    t.supply(LIQUIDATOR, "ETH", 1.0);
    let own = t.account_id(LIQUIDATOR);
    let own_r = t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(own));
    std::println!("l9a credit(own 1-leg): {own_r:?}");
    c.check(
        code_of(&own_r) == Some(errors::POSITION_LIMIT_EXCEEDED),
        "Credit(own 1-leg ETH account): POSITION_LIMIT_EXCEEDED".into(),
    );
    let fresh = t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));
    std::println!("l9a credit(0): {fresh:?}");
    match fresh {
        Ok(receiver) => {
            let usdc = t.supply_balance_raw_for(receiver, "USDC");
            let wbtc = t.supply_balance_raw_for(receiver, "WBTC");
            std::println!("l9a credit(0) receiver={receiver} usdc={usdc} wbtc={wbtc}");
            c.check(usdc > 0 && wbtc > 0, "Credit(0) gets both legs".into());
        }
        Err(e) => c.check(false, format!("Credit(0) refused: {e:?}")),
    }
    c.check(
        t.get_position_limits().max_supply_positions == 1,
        "the governed limit is unchanged".into(),
    );
    c.finish();
}

/// liquidation-9b. A five-leg victim after a cut to (1, 1): Credit(0) gives the
/// receiver 5 legs (= POSITION_LIMIT_MAX), never more; the receiver cannot add
/// a sixth leg voluntarily or by a second credit.
#[test]
fn l9b_mc4_five_leg_victim_never_exceeds_the_max() {
    let mut c = Checks::new("liquidation-9b");
    const LEGS: [&str; 5] = ["M1", "M2", "M3", "M4", "M5"];
    let mut b = LendingTest::new();
    for name in LEGS {
        b = b.with_market(asset(name, 7, usd(1), 7500, 8000, 500, 1_000_000.0));
    }
    let mut t = b
        .with_market(asset("DEBT6", 7, usd(1), 7500, 8000, 500, 1_000_000.0))
        .with_position_limits(5, 5)
        .build();
    for name in LEGS {
        t.supply(ALICE, name, 1_000.0);
    }
    t.borrow(ALICE, "DEBT6", 3_500.0);
    t.supply(CAROL, "DEBT6", 1_000.0);
    t.borrow(CAROL, "M1", 700.0);
    t.set_position_limits(1, 1);
    for name in LEGS {
        t.set_price(name, usd_cents(85));
    }
    assert!(t.can_be_liquidated(ALICE));

    let r = t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "DEBT6", 500.0, SeizeMode::Credit(0));
    std::println!("l9b credit(0) of the 5-leg victim: {r:?}");
    let Ok(receiver) = r else {
        c.check(false, format!("Credit(0) refused: {r:?}"));
        c.finish();
        return;
    };
    let legs = t.ctrl_client().get_account_positions(&receiver).0.len();
    std::println!("l9b receiver={receiver} legs={legs}");
    c.check(
        legs == 5,
        format!("the receiver holds the victim's 5 legs (POSITION_LIMIT_MAX); got {legs}"),
    );

    // Voluntary sixth leg into the receiver.
    let addr = t.get_or_create_user(LIQUIDATOR);
    let debt6 = t.resolve_market("DEBT6");
    debt6.token_admin.mint(&addr, &USDC_UNIT);
    let voluntary = flat(t.ctrl_client().try_supply(
        &addr,
        &receiver,
        &HARNESS_SPOKE,
        &vec![&t.env, (hub_asset(debt6.asset.clone()), USDC_UNIT)],
    ));
    std::println!("l9b voluntary sixth leg: {voluntary:?}");
    c.check(
        code_of(&voluntary) == Some(errors::POSITION_LIMIT_EXCEEDED),
        "voluntary sixth leg: POSITION_LIMIT_EXCEEDED".into(),
    );

    // A second victim credits a new asset (DEBT6) into the receiver.
    t.set_price("DEBT6", usd_cents(70));
    assert!(t.can_be_liquidated(CAROL));
    let second =
        t.try_liquidate_with_mode(LIQUIDATOR, CAROL, "M1", 50.0, SeizeMode::Credit(receiver));
    std::println!("l9b second credit into the receiver: {second:?}");
    c.check(
        code_of(&second) == Some(errors::POSITION_LIMIT_EXCEEDED),
        "second credit of a new asset: POSITION_LIMIT_EXCEEDED".into(),
    );
    let legs_after = t.ctrl_client().get_account_positions(&receiver).0.len();
    c.check(
        legs_after <= 5,
        format!("receiver legs stay <= 5 ({legs_after})"),
    );
    let fresh = t.try_liquidate_with_mode(LIQUIDATOR, CAROL, "M1", 50.0, SeizeMode::Credit(0));
    std::println!("l9b second victim into Credit(0): {fresh:?}");
    c.check(
        fresh.is_ok(),
        "Credit(0) still admits the second victim".into(),
    );
    c.finish();
}

/// liquidation-9c (policy difference). A receiver that already holds one of
/// the victim's assets keeps the governed limit (only an empty receiver is
/// exempt). Asserts that policy plus the invariants it shares with the
/// `POSITION_LIMIT_MAX` belt.
#[test]
fn l9c_mc4_non_empty_receiver_keeps_the_limit() {
    let mut c = Checks::new("liquidation-9c");
    let mut t = mc4_fixture();
    t.supply(LIQUIDATOR, "USDC", 100.0);
    let own = t.account_id(LIQUIDATOR);
    t.supply(CAROL, "ETH", 1.0);
    t.borrow(CAROL, "WBTC", 0.03);
    t.set_position_limits(1, 1);

    let step1 = t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(own));
    std::println!("l9c step 1 credit(own USDC-holding receiver): {step1:?}");
    c.check(
        code_of(&step1) == Some(errors::POSITION_LIMIT_EXCEEDED),
        "a non-empty receiver keeps the lowered limit".into(),
    );
    let entry = t.try_supply(LIQUIDATOR, "ETH", 1.0);
    c.check(
        code_of(&entry) == Some(errors::POSITION_LIMIT_EXCEEDED),
        "voluntary entry stays gated".into(),
    );
    let fresh = t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0));
    std::println!("l9c credit(0): {fresh:?}");
    c.check(fresh.is_ok(), "Credit(0) is admitted".into());
    c.finish();
}

// ---------------------------------------------------------------------------
// liquidation-10: CR-25 Credit-mode estimate versus execution
// ---------------------------------------------------------------------------

/// liquidation-10. For every caller-free receiver gate, the Credit estimate
/// returns the same error as `liquidate`.
#[test]
fn l10_cr25_credit_estimate_matches_execution() {
    let mut c = Checks::new("liquidation-10");

    // Receiver without room; the liquidated account itself.
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_position_limits(1, 1)
        .build();
    t.supply(ALICE, "USDC", 1_000.0);
    t.borrow(ALICE, "ETH", 0.3);
    // The liquidator's own one-leg account: ownership passes, the limit binds.
    t.supply(LIQUIDATOR, "ETH", 1.0);
    let alice_id = t.resolve_account_id(ALICE);
    let own_id = t.resolve_account_id(LIQUIDATOR);
    t.set_price("USDC", usd_cents(70));
    let payments = vec![
        &t.env,
        (hub_asset(t.resolve_asset("ETH")), f64_to_i128(0.1, 7)),
    ];
    for (label, receiver, code) in [
        (
            "liquidator's receiver at its limit",
            own_id,
            errors::POSITION_LIMIT_EXCEEDED,
        ),
        (
            "the liquidated account",
            alice_id,
            errors::SELF_LIQUIDATION_NOT_ALLOWED,
        ),
    ] {
        let view = flat(t.ctrl_client().try_get_liquidation_estimate(
            &alice_id,
            &payments,
            &SeizeMode::Credit(receiver),
        ));
        let exec =
            t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 0.1, SeizeMode::Credit(receiver));
        std::println!(
            "l10 {label}: estimate={:?} liquidate={exec:?}",
            view.as_ref().map(|e| e.max_payment_wad)
        );
        c.check(
            code_of(&exec) == Some(code) && code_of(&view) == Some(code),
            format!(
                "{label}: estimate error {:?} == liquidate error {:?} (expected {code})",
                code_of(&view),
                code_of(&exec)
            ),
        );
    }

    // Spoke, mode and existence gates.
    let mut t = LendingTest::new()
        .standard_two_asset()
        .with_spoke(2, STABLECOIN_SPOKE)
        .with_spoke_asset(2, "USDC", true, true)
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
    let alice = t.resolve_account_id(ALICE);
    let payments = vec![&t.env, (hub_asset(t.resolve_asset("ETH")), 10_000_000i128)];
    let elsewhere = t.create_spoke_account(LIQUIDATOR, 2);
    let multiply = t.create_account_full(LIQUIDATOR, HARNESS_SPOKE, PositionMode::Multiply);
    for (label, receiver, code) in [
        ("receiver in spoke 2", elsewhere, errors::SPOKE_MISMATCH),
        ("Multiply receiver", multiply, errors::ACCOUNT_MODE_MISMATCH),
        ("missing receiver 9_999", 9_999, errors::ACCOUNT_NOT_FOUND),
    ] {
        let view = flat(t.ctrl_client().try_get_liquidation_estimate(
            &alice,
            &payments,
            &SeizeMode::Credit(receiver),
        ));
        let exec =
            t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(receiver));
        std::println!(
            "l10 {label}: estimate={:?} liquidate={exec:?}",
            view.as_ref().map(|e| e.max_payment_wad)
        );
        c.check(
            code_of(&exec) == Some(code) && code_of(&view) == Some(code),
            format!(
                "{label}: estimate error {:?} == liquidate error {:?} (expected {code})",
                code_of(&view),
                code_of(&exec)
            ),
        );
    }
    c.finish();
}

// ---------------------------------------------------------------------------
// liquidation-11: F1 oracle decimals versus pool decimals
// ---------------------------------------------------------------------------

fn seed_decimals(t: &LendingTest, asset_name: &str, decimals: u32) {
    let key = PriceKey::Token(t.resolve_asset(asset_name));
    let mut oracle = t
        .price_agg_client()
        .oracle(&key)
        .expect("oracle configured");
    oracle.asset_decimals = decimals;
    t.price_agg_client().seed_oracle(&key, &oracle);
}

/// liquidation-11. A seeded oracle `asset_decimals` that differs from the
/// pool's (7) on a leg the liquidation moves. Safe: the estimate and the
/// liquidation fail with InvalidOracleDecimals (221) and nothing is seized.
/// Also measures the USDC seized against a correctly configured control.
#[test]
fn l11_f1_oracle_decimals_mismatch_fails_closed() {
    let mut c = Checks::new("liquidation-11");
    // Control: correct decimals, 0.3 ETH repaid in Transfer.
    let mut ctl = LendingTest::new().standard_two_asset().build();
    seed_liquidatable_usdc_eth(&mut ctl);
    ctl.supply(BOB, "ETH", 10.0);
    let before = ctl.supply_balance_raw(ALICE, "USDC");
    ctl.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.3)
        .expect("control liquidation");
    let control_seized = before - ctl.supply_balance_raw(ALICE, "USDC");
    std::println!("l11 control: USDC seized={control_seized}");

    for (name, decimals) in [("USDC", 6u32), ("USDC", 8), ("ETH", 6)] {
        for mode in [SeizeMode::Transfer, SeizeMode::Credit(0)] {
            let mut t = LendingTest::new().standard_two_asset().build();
            seed_liquidatable_usdc_eth(&mut t);
            t.supply(BOB, "ETH", 10.0);
            seed_decimals(&t, name, decimals);
            let alice = t.account_id(ALICE);
            let payments = vec![&t.env, (hub_asset(t.resolve_asset("ETH")), 3_000_000i128)];
            let est = flat(
                t.ctrl_client()
                    .try_get_liquidation_estimate(&alice, &payments, &mode),
            );
            let before = t.supply_balance_raw(ALICE, "USDC");
            let exec = t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 0.3, mode);
            let after = if t.account_exists(alice) {
                t.supply_balance_raw(ALICE, "USDC")
            } else {
                0
            };
            let seized = before - after;
            std::println!(
                "l11 {name}@{decimals} {mode:?}: estimate={:?} liquidate={exec:?} \
                 usdc_seized={seized} ratio_to_control={:.3}",
                est.as_ref().map(|e| (
                    e.max_payment_wad,
                    e.seized_collaterals
                        .iter()
                        .map(|s| s.amount)
                        .collect::<std::vec::Vec<_>>()
                )),
                seized as f64 / control_seized as f64
            );
            let code = OracleError::InvalidOracleDecimals as u32;
            c.check(
                code_of(&est) == Some(code) && code_of(&exec) == Some(code) && seized == 0,
                format!(
                    "{name}@{decimals} {mode:?}: estimate {:?} / liquidate {:?} must be 221 and \
                     nothing seized (seized {seized})",
                    code_of(&est),
                    code_of(&exec)
                ),
            );
        }
    }
    c.finish();
}

// ---------------------------------------------------------------------------
// liquidation-12: base bonus rounding at the executed level
// ---------------------------------------------------------------------------

fn leg_preset(name: &'static str, bonus: u32) -> MarketPreset {
    MarketPreset {
        name,
        decimals: 7,
        price_wad: usd(1),
        initial_liquidity: 1_000.0,
        config: AssetConfigPreset {
            loan_to_value: 7500,
            liquidation_threshold: 8500,
            liquidation_bonus: bonus,
            liquidation_fees: 1000,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    }
}

/// liquidation-12 (harness). Four equal-value legs at 250 BPS on an insolvent
/// account. Safe: the estimate's bonus is 250 (one rounding).
#[test]
fn l12_base_bonus_four_equal_legs() {
    const LEGS: [&str; 4] = ["A7", "B7", "C7", "D7"];
    let mut b = LendingTest::new();
    for name in LEGS {
        b = b.with_market(leg_preset(name, 250));
    }
    b = b.with_market(MarketPreset {
        name: "DEBT",
        decimals: 7,
        price_wad: usd(1),
        initial_liquidity: 10_000_000.0,
        config: AssetConfigPreset {
            loan_to_value: 9000,
            liquidation_threshold: 9500,
            liquidation_bonus: 200,
            ..DEFAULT_ASSET_CONFIG
        },
        params: DEFAULT_MARKET_PARAMS,
    });
    let mut t = b.build();
    for name in LEGS {
        t.supply_raw(ALICE, name, 2_500 * USDC_UNIT);
    }
    t.borrow(ALICE, "DEBT", 7_000.0);
    for name in LEGS {
        t.set_price(name, usd(69) / 100);
    }
    t.get_or_create_user(LIQUIDATOR);
    let acc = t.resolve_account_id(ALICE);
    assert!(coll(&t, acc) < debt(&t, acc), "insolvent fixture");
    let debt_raw = t.ctrl_client().get_borrow_amount(&acc, &key(&t, "DEBT"));
    let e = estimate(&t, acc, &[("DEBT", 2 * debt_raw)]);
    std::println!("l12 harness: bonus_rate_bps={}", e.bonus_rate_bps);
    assert_eq!(
        e.bonus_rate_bps, 250,
        "four equal legs at 250 average to 250 with one rounding"
    );
}

// ---------------------------------------------------------------------------
// Ported from our fix series: MC-2 share count, CR-25 ownership, F1 liveness
// ---------------------------------------------------------------------------

/// MC-2. A partial USDC offer three units below the collateral-backed quote
/// plans a token amount equal to the leg's half-up balance. The pool closes
/// such a leg, so the plan reports the whole share position in Credit units
/// too, and the receiver is credited every share less the fee.
#[test]
fn mc2_partial_plan_at_the_half_up_balance_takes_every_share() {
    let (mut t, alice, shares) = window_fixture();
    let offer_raw = window_offer(&t, alice);
    let payments = asset_payment_vec(&t.env, t.resolve_asset("USDC"), offer_raw);

    let transfer =
        t.ctrl_client()
            .get_liquidation_estimate(&alice, &payments, &SeizeMode::Transfer);
    assert!(transfer.refunds.is_empty(), "the offer is kept as made");
    let index = Ray::from(pool_state(&t, "ETH").supply_index);
    let half_up = unscale_supply(&t.env, Ray::from(shares), index, SEVEN);
    let planned = transfer.seized_collaterals.get(0).expect("ETH leg").amount;
    assert_eq!(
        planned, half_up,
        "the planned amount reaches the half-up balance"
    );

    let credit = t
        .ctrl_client()
        .get_liquidation_estimate(&alice, &payments, &SeizeMode::Credit(0));
    let planned_shares = credit.seized_collaterals.get(0).expect("ETH leg").amount;
    assert_eq!(
        planned_shares, shares,
        "a plan the pool closes takes the exact held shares"
    );

    let liquidator = t.get_or_create_user(LIQUIDATOR);
    t.resolve_market("USDC")
        .token_admin
        .mint(&liquidator, &offer_raw);
    let receiver = t
        .ctrl_client()
        .liquidate(&liquidator, &alice, &payments, &SeizeMode::Credit(0));
    let fee_shares = credit.protocol_fees.get(0).map_or(0, |f| f.amount);
    assert_eq!(scaled_supply(&t, receiver, "ETH"), shares - fee_shares);
    assert!(
        !t.account_exists(alice),
        "the insolvent residue under the dust cap is cleaned up"
    );
}

/// CR-25, the one documented difference: the estimate has no caller, so it
/// cannot apply the owner-or-delegate gate. For a receiver the liquidator does
/// not control, the estimate succeeds and execution refuses with
/// `NotAuthorized`; `Credit(0)` agrees with `Transfer`.
#[test]
fn cr25_the_estimate_cannot_check_receiver_ownership() {
    let mut t = LendingTest::new().standard_two_asset().build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.set_price("USDC", usd_cents(50));
    t.assert_liquidatable(ALICE);
    let alice = t.resolve_account_id(ALICE);
    let payments = vec![&t.env, (hub_asset(t.resolve_asset("ETH")), 10_000_000i128)];

    let bobs = t.create_account(BOB);
    t.ctrl_client()
        .get_liquidation_estimate(&alice, &payments, &SeizeMode::Credit(bobs));
    let exec = t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(bobs));
    assert_eq!(code_of(&exec), Some(errors::NOT_AUTHORIZED));

    let fresh = t
        .ctrl_client()
        .get_liquidation_estimate(&alice, &payments, &SeizeMode::Credit(0));
    let transfer =
        t.ctrl_client()
            .get_liquidation_estimate(&alice, &payments, &SeizeMode::Transfer);
    assert_eq!(fresh.max_payment_wad, transfer.max_payment_wad);
    assert_eq!(fresh.bonus_rate_bps, transfer.bonus_rate_bps);
}

/// F1 liveness: the decimals check binds only the legs a plan moves. A debt
/// leg whose oracle decimals are wrong does not block a liquidation that
/// repays another leg, in either mode, and paying the mismatched leg itself is
/// refused.
#[test]
fn f1_a_mismatch_on_an_untouched_leg_does_not_block_liquidation() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .with_market(usdt_stable_preset())
        .build();
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 2.0);
    t.borrow(ALICE, "USDT", 1_000.0);
    t.set_price("USDC", usd_cents(55));
    t.assert_liquidatable(ALICE);
    seed_decimals(&t, "USDT", 6);

    let refused = t.try_liquidate(LIQUIDATOR, ALICE, "USDT", 100.0);
    assert_eq!(
        code_of(&refused),
        Some(OracleError::InvalidOracleDecimals as u32)
    );
    let usdc_before = t.supply_balance_raw(ALICE, "USDC");
    t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.1)
        .expect("Transfer on the ETH leg is unaffected");
    t.try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 0.1, SeizeMode::Credit(0))
        .expect("Credit on the ETH leg is unaffected");
    assert!(t.supply_balance_raw(ALICE, "USDC") < usdc_before);
}
