//! Whole-unit liquidation of a collateral leg below 3 decimals, against the
//! documented rules in `docs/reference/formulas.md#bonus-and-target-repayment`.
//!
//! One case lists a 0-, 1- or 2-decimal collateral market in its own hub and
//! spoke, borrows 7-decimal USDC and optionally a 6- or 18-decimal second leg,
//! drops the collateral price into an HF band, then liquidates. A settled call
//! that leaves the account below HF 1 is followed by debt-sized liquidations
//! of the residue. Tests prefixed `wul_`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::string::{String, ToString};
use std::{format, println};

use crate::config::config;
use common::math::fp::{Bps, Ray, Wad};
use common::math::fp_core::{mul_div_ceil, mul_div_floor};
use common::rates::unscale_borrow_ceil;
use common::types::{HubAssetKey, LiquidationEstimate, SeizeMode};
use common::validation::max_cap_for_decimals;
use controller::constants::{BPS, RAY, WAD};
use governance::op::{AdminOperation, CreatePoolArgs, SpokeAssetArgs, SpokeLiquidationCurveArgs};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::ToPrimitive;
use proptest::prelude::*;
use proptest::test_runner::{TestCaseError, TestRunner};
use soroban_sdk::xdr::ScErrorType;
use soroban_sdk::{token, vec, Address, Env, Error, InvokeError, TryFromVal};
use test_harness::{
    errors, usd_cents, usdc_preset, AssetConfigPreset, LendingTest, MarketPreset,
    DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, HARNESS_HUB,
};

const LIQ: &str = "WUL";
const BORROWER: &str = "borrower";
const LIQUIDATOR: &str = "liquidator";
const MAX_RESIDUE_STEPS: u32 = 3;
const WEI_TOLERANCE: i128 = 16;
const COVERAGE_TOLERANCE: i128 = 1_000_000_000_000;

#[derive(Clone, Copy, Debug)]
enum Band {
    NearOne,
    Mid,
    Cover,
    Insolvent,
    UnitEdge,
}

#[derive(Clone, Debug)]
struct Case {
    decimals: u32,
    unit_cents: i128,
    units: i128,
    lt: u32,
    ltv: u32,
    bonus: u32,
    target_hf_bps: u32,
    knee_bps: u32,
    factor_bps: u32,
    second_leg: Option<u32>,
    second_share_pct: i128,
    borrow_pct: i128,
    accrue_days: u64,
    band: Band,
    band_pick: i128,
    offer_bps: i128,
    offer_all_legs: bool,
    credit: bool,
}

fn max_bonus_for(lt: u32) -> u32 {
    (100_000_000 / lt - 10_000).min(3_000)
}

fn case_strategy() -> impl Strategy<Value = Case> {
    let market = (
        0u32..=2,
        prop_oneof![50i128..=500, 500i128..=100_000, 100_000i128..=10_000_000],
        prop_oneof![2i128..=5, 2i128..=500, 2i128..=50_000],
        prop_oneof![2_500u32..=4_500, 4_500u32..=9_000],
        40u32..=95,
        0u32..=1_000,
    );
    let curve = (
        prop_oneof![Just(11_000u32), 10_100u32..=15_000],
        prop_oneof![Just(8_000u32), 3_000u32..=9_900],
        prop_oneof![Just(10_000u32), 1u32..=10_000],
    );
    let debt = (
        prop_oneof![Just(None), Just(Some(6u32)), Just(Some(18u32))],
        10i128..=60,
        50i128..=99,
        prop_oneof![Just(0u64), 1u64..=90],
    );
    let drop = (
        prop_oneof![
            Just(Band::NearOne),
            Just(Band::Mid),
            Just(Band::Cover),
            Just(Band::Insolvent),
            Just(Band::UnitEdge)
        ],
        0i128..=10_000,
        prop_oneof![100i128..=9_999, Just(10_000i128), 10_001i128..=30_000],
        any::<bool>(),
        any::<bool>(),
    );
    (market, curve, debt, drop).prop_map(
        |(
            (decimals, unit_cents, units, lt, ltv_pct, bonus_permille),
            (target_hf_bps, knee_bps, factor_bps),
            (second_leg, second_share_pct, borrow_pct, accrue_days),
            (band, band_pick, offer_bps, offer_all_legs, credit),
        )| {
            let edge = matches!(band, Band::UnitEdge);
            let two_unit_edge = edge && band_pick % 2 == 0;
            let lt = if two_unit_edge {
                2_500 + lt % 1_000
            } else {
                lt
            };
            Case {
                decimals,
                unit_cents,
                units,
                lt,
                ltv: (lt * ltv_pct / 100).clamp(1, lt - 1),
                bonus: max_bonus_for(lt) * bonus_permille
                    / if two_unit_edge { 4_000 } else { 1_000 },
                target_hf_bps,
                knee_bps,
                factor_bps: if two_unit_edge {
                    1 + factor_bps % 3_000
                } else {
                    factor_bps
                },
                second_leg,
                second_share_pct,
                borrow_pct,
                accrue_days,
                band,
                band_pick,
                offer_bps: if edge {
                    BPS + offer_bps % 20_001
                } else {
                    offer_bps
                },
                offer_all_legs: offer_all_legs || edge,
                credit,
            }
        },
    )
}

#[derive(Clone)]
struct Leg {
    name: &'static str,
    key: HubAssetKey,
    decimals: u32,
    price: i128,
}

struct Curve {
    target: i128,
    knee: i128,
    factor: i128,
}

struct World {
    t: LendingTest,
    liq_key: HubAssetKey,
    decimals: u32,
    liq_price: i128,
    legs: std::vec::Vec<Leg>,
    account: u64,
    receivers: std::vec::Vec<u64>,
    curve: Curve,
    listed_bonus: i128,
    lt: i128,
}

#[derive(Default)]
struct Report {
    counts: BTreeMap<String, u64>,
    maxima: BTreeMap<String, f64>,
}

type Stats = RefCell<Report>;

fn bump(stats: &Stats, key: &str) {
    *stats
        .borrow_mut()
        .counts
        .entry(key.to_string())
        .or_insert(0) += 1;
}

fn record_max(stats: &Stats, key: &str, value: &BigRational) {
    let value = value.to_f64().unwrap_or(f64::NAN);
    let mut report = stats.borrow_mut();
    let slot = report
        .maxima
        .entry(key.to_string())
        .or_insert(f64::NEG_INFINITY);
    if value > *slot {
        *slot = value;
    }
}

/// Returns `(units, unit_cents)`: at least the units whose LTV value clears the
/// $5 borrow floor with margin; a unit-edge case opens 2 units.
fn sizing(case: &Case) -> (i128, i128) {
    let ltv = i128::from(case.ltv);
    if matches!(case.band, Band::UnitEdge) {
        let floor_cents = (700 * BPS + 2 * ltv - 1) / (2 * ltv);
        return (2, case.unit_cents.max(floor_cents));
    }
    let per_unit_ltv_cents = case.unit_cents * ltv;
    let floor_units = (700 * BPS + per_unit_ltv_cents - 1) / per_unit_ltv_cents;
    (case.units.max(floor_units), case.unit_cents)
}

fn second_leg_preset(decimals: u32) -> (&'static str, i128, f64) {
    if decimals == 18 {
        ("D18", usd_cents(200_000), 10_000_000.0)
    } else {
        ("D6", usd_cents(108), 10_000_000_000.0)
    }
}

fn spoke_listing(
    hub_id: u32,
    asset: Address,
    spoke_id: u32,
    decimals: u32,
    borrowable: bool,
) -> SpokeAssetArgs {
    SpokeAssetArgs {
        hub_id,
        asset,
        spoke_id,
        can_collateral: !borrowable,
        can_borrow: borrowable,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: 7_500,
        threshold: 8_000,
        bonus: 500,
        liquidation_fees: 0,
        supply_cap: max_cap_for_decimals(decimals),
        borrow_cap: if borrowable {
            max_cap_for_decimals(decimals)
        } else {
            0
        },
    }
}

/// Lists the market, opens the account and prices the collateral into the
/// case's HF band or onto the unit edge. Returns `None` when the priced
/// account is not liquidatable.
fn open_world(case: &Case) -> Result<Option<World>, TestCaseError> {
    let (units, unit_cents) = sizing(case);
    let token_price = usd_cents(unit_cents) * 10i128.pow(case.decimals);
    let mut builder = LendingTest::new()
        .with_market(MarketPreset {
            initial_liquidity: 10_000_000_000.0,
            ..usdc_preset()
        })
        .with_market(MarketPreset {
            name: LIQ,
            decimals: case.decimals,
            price_wad: token_price,
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
        });
    if let Some(decimals) = case.second_leg {
        let (name, price_wad, liquidity) = second_leg_preset(decimals);
        builder = builder.with_market(MarketPreset {
            name,
            decimals,
            price_wad,
            initial_liquidity: liquidity,
            config: DEFAULT_ASSET_CONFIG,
            params: DEFAULT_MARKET_PARAMS,
        });
    }
    let mut t = builder.build();

    let admin = t.admin();
    let liq = t.resolve_asset(LIQ);
    let hub = t.create_hub();
    let spoke = {
        let gov = t.gov_client();
        gov.execute_immediate(
            &admin,
            &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
                hub_id: hub,
                asset: liq.clone(),
                params: DEFAULT_MARKET_PARAMS.to_market_params(&liq, case.decimals),
            }),
        );
        let spoke_val = gov.execute_immediate(&admin, &AdminOperation::AddSpoke);
        u32::try_from_val(&t.env, &spoke_val).unwrap()
    };
    let mut collateral = spoke_listing(hub, liq.clone(), spoke, case.decimals, false);
    collateral.ltv = case.ltv;
    collateral.threshold = case.lt;
    collateral.bonus = case.bonus;
    let mut legs = std::vec![Leg {
        name: "USDC",
        key: HubAssetKey {
            hub_id: HARNESS_HUB,
            asset: t.resolve_asset("USDC"),
        },
        decimals: 7,
        price: usd_cents(100),
    }];
    if let Some(decimals) = case.second_leg {
        let (name, price, _) = second_leg_preset(decimals);
        legs.push(Leg {
            name,
            key: HubAssetKey {
                hub_id: HARNESS_HUB,
                asset: t.resolve_asset(name),
            },
            decimals,
            price,
        });
    }
    {
        let gov = t.gov_client();
        gov.execute_immediate(&admin, &AdminOperation::AddAssetToSpoke(collateral));
        for leg in &legs {
            gov.execute_immediate(
                &admin,
                &AdminOperation::AddAssetToSpoke(spoke_listing(
                    HARNESS_HUB,
                    leg.key.asset.clone(),
                    spoke,
                    leg.decimals,
                    true,
                )),
            );
        }
        gov.execute_immediate(
            &admin,
            &AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs {
                spoke_id: spoke,
                target_hf_wad: i128::from(case.target_hf_bps) * WAD / BPS,
                hf_for_max_bonus_wad: i128::from(case.knee_bps) * WAD / BPS,
                liquidation_bonus_factor_bps: case.factor_bps,
            }),
        );
    }

    let ltv = i128::from(case.ltv);
    let borrower = t.get_or_create_user(BORROWER);
    t.get_or_create_user(LIQUIDATOR);
    t.resolve_market(LIQ).token_admin.mint(&borrower, &units);
    let liq_key = HubAssetKey {
        hub_id: hub,
        asset: liq,
    };
    let account = t.ctrl_client().supply(
        &borrower,
        &0,
        &spoke,
        &vec![&t.env, (liq_key.clone(), units)],
    );

    let borrow_usdc_raw = units * unit_cents * 100_000 * ltv * case.borrow_pct / (BPS * 100);
    let mut borrows = vec![&t.env];
    let (usdc_raw, second_raw) = match case.second_leg {
        None => (borrow_usdc_raw, 0),
        Some(decimals) => {
            let share = borrow_usdc_raw * case.second_share_pct / 100;
            let raw = if decimals == 18 {
                share * 100_000_000_000 / 2_000
            } else {
                share * 100 / 1_080
            };
            (borrow_usdc_raw - share, raw)
        }
    };
    borrows.push_back((legs[0].key.clone(), usdc_raw.max(1)));
    if legs.len() > 1 {
        borrows.push_back((legs[1].key.clone(), second_raw.max(1)));
    }
    let borrowed = t
        .ctrl_client()
        .try_borrow(&borrower, &account, &borrows, &None);
    prop_assert!(
        matches!(borrowed, Ok(Ok(_))),
        "generated in-LTV borrow failed: units={units} borrows={:?} error={:?}",
        borrows,
        borrowed.err()
    );

    if case.accrue_days > 0 {
        t.advance_and_sync(case.accrue_days * 86_400);
    }

    let mut w = World {
        t,
        liq_key,
        decimals: case.decimals,
        liq_price: token_price,
        legs,
        account,
        receivers: std::vec::Vec::new(),
        curve: Curve {
            target: i128::from(case.target_hf_bps) * WAD / BPS,
            knee: i128::from(case.knee_bps) * WAD / BPS,
            factor: i128::from(case.factor_bps),
        },
        listed_bonus: i128::from(case.bonus),
        lt: i128::from(case.lt),
    };
    let liquidatable = match case.band {
        Band::UnitEdge => w.price_at_unit_edge(case),
        band => w.price_into_band(band, case, units),
    };
    Ok(liquidatable.then_some(w))
}

impl World {
    fn set_liq_price(&mut self, price: &BigInt) {
        let price: i128 = price.try_into().expect("price fits i128");
        assert!(price > 0, "generated price must be positive");
        self.t.set_price(LIQ, price);
        self.liq_price = price;
    }

    /// Prices the collateral so HF lands at the band's target. Returns whether
    /// HF is below 1.
    fn price_into_band(&mut self, band: Band, case: &Case, units: i128) -> bool {
        let lt = i128::from(case.lt);
        let pick = case.band_pick;
        let hf_ppm = match band {
            Band::NearOne => 990_000 + pick * 9_500 / 10_000,
            Band::Mid => 900_000 + pick * 90_000 / 10_000,
            Band::Cover => lt * (1_000_000 + pick * i128::from(case.bonus) * 100 / 10_000) / BPS,
            Band::Insolvent | Band::UnitEdge => lt * (300_000 + pick * 699_999 / 10_000) / BPS,
        }
        .min(999_500);
        self.price_at_hf(hf_ppm, lt, units)
    }

    fn price_at_hf(&mut self, hf_ppm: i128, lt: i128, units: i128) -> bool {
        let (_, debt, _, _) = self.totals();
        let price = BigInt::from(debt) * hf_ppm * BPS * BigInt::from(10i128.pow(self.decimals))
            / (BigInt::from(1_000_000) * lt * units);
        self.set_liq_price(&price);
        self.totals().3 < WAD
    }

    /// Finds the lowest collateral price where `U / (1 + b) >= D / (1 + e)`,
    /// with `b` the controller's bonus at that price and `e` within 3 parts
    /// per million of zero. `D` then falls on either side of the whole-unit
    /// margin band or inside it. An odd `pick` first sells one of the two
    /// units, so the edge is priced for a one-unit residue.
    fn price_at_unit_edge(&mut self, case: &Case) -> bool {
        let pick = case.band_pick;
        if pick % 2 == 1 && !self.sell_to_one_unit(case) {
            return false;
        }
        let eps_ppb = (pick / 2) * 6 / 5 - 3_000;
        let (_, debt, _, _) = self.totals();
        let scale = BigInt::from(10i128.pow(self.decimals));
        let target = |bonus: i128| {
            BigInt::from(debt) * (BPS + bonus) * &scale * 1_000_000_000
                / (BigInt::from(BPS) * (1_000_000_000 + eps_ppb))
        };
        let mut lo = target(0) / 2;
        let mut hi = target(4 * BPS) * 2;
        if self.reaches_edge(&lo, debt, eps_ppb) || !self.reaches_edge(&hi, debt, eps_ppb) {
            return false;
        }
        while &hi - &lo > &lo / 10_000_000_000i64 + 1 {
            let mid: BigInt = (&lo + &hi) / 2;
            if self.reaches_edge(&mid, debt, eps_ppb) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        self.set_liq_price(&hi);
        self.totals().3 < WAD
    }

    fn reaches_edge(&mut self, price: &BigInt, debt: i128, eps_ppb: i128) -> bool {
        self.set_liq_price(price);
        let bonus = mirror_bonus(&self.t.env, self.totals(), self.listed_bonus, &self.curve);
        BigInt::from(self.unit_usd().raw()) * BPS * (1_000_000_000 + eps_ppb)
            >= BigInt::from(debt) * (BPS + bonus) * 1_000_000_000
    }

    /// Prices the two-unit account at `C / D = 1.9`, or HF 0.99 when that is
    /// lower, and liquidates it with a debt-sized offer. Returns whether one
    /// unit and some debt remain.
    fn sell_to_one_unit(&mut self, case: &Case) -> bool {
        let lt = i128::from(case.lt);
        if !self.price_at_hf((lt * 190).min(990_000), lt, 2) {
            return false;
        }
        let liquidator = self.t.get_or_create_user(LIQUIDATOR);
        let mut payments = vec![&self.t.env];
        for leg in self.legs.clone() {
            let debt = self.debt_ceil(&leg);
            if debt > 0 {
                self.t
                    .resolve_market(leg.name)
                    .token_admin
                    .mint(&liquidator, &debt);
                payments.push_back((leg.key.clone(), debt));
            }
        }
        let sold = self.t.ctrl_client().try_liquidate(
            &liquidator,
            &self.account,
            &payments,
            &SeizeMode::Transfer,
        );
        sold.is_ok()
            && self.exists(self.account)
            && self.units_of(self.account) == 1
            && self.totals().1 > 0
    }

    fn unit_scaled(&self) -> i128 {
        RAY / 10i128.pow(self.decimals)
    }

    fn liq_scaled(&self, id: u64) -> i128 {
        let (supplies, _) = self.t.ctrl_client().get_account_positions(&id);
        supplies
            .get(self.liq_key.clone())
            .map(|p| p.scaled_amount)
            .unwrap_or(0)
    }

    fn debt_ceil(&self, leg: &Leg) -> i128 {
        let (_, debts) = self.t.ctrl_client().get_account_positions(&self.account);
        let Some(position) = debts.get(leg.key.clone()) else {
            return 0;
        };
        let index = self.t.ctrl_client().get_market_index(&leg.key).borrow_index;
        unscale_borrow_ceil(
            &self.t.env,
            Ray::from(position.scaled_amount),
            Ray::from(index),
            leg.decimals,
        )
    }

    fn unit_usd(&self) -> Wad {
        let env = &self.t.env;
        Wad::from_token(env, 1, self.decimals).mul(env, Wad::from(self.liq_price))
    }

    /// Returns `(C, D, W, HF)` in WAD.
    fn totals(&self) -> (i128, i128, i128, i128) {
        let ctrl = self.t.ctrl_client();
        let id = self.account;
        (
            ctrl.get_total_collateral_usd(&id),
            ctrl.get_total_borrow_usd(&id),
            ctrl.get_liquidation_collateral(&id),
            ctrl.get_health_factor(&id),
        )
    }

    fn exists(&self, id: u64) -> bool {
        self.t.ctrl_client().account_exists(&id)
    }

    fn units_of(&self, id: u64) -> i128 {
        if !self.exists(id) {
            return 0;
        }
        self.t
            .ctrl_client()
            .get_collateral_amount(&id, &self.liq_key)
    }
}

/// The controller's bonus selection for a single-leg account, step by step:
/// threshold ceiling, curve ramp, HF-preserving cap, insolvent and band branches.
fn mirror_bonus(env: &Env, totals: (i128, i128, i128, i128), listed: i128, curve: &Curve) -> i128 {
    let (c, d, w, hf) = totals;
    let p = if c > 0 {
        Wad::from(w).div(env, Wad::from(c)).raw()
    } else {
        0
    };
    let max = if p <= 0 {
        0
    } else {
        let eff = mul_div_ceil(env, p, BPS, WAD).clamp(1, BPS);
        BPS * (BPS - eff) / eff
    };
    let base = listed.min(max);
    let scaled = if hf >= curve.target {
        base
    } else {
        let scale = Wad::from(curve.target - hf)
            .div(env, Wad::from(curve.target - curve.knee))
            .raw()
            .min(WAD);
        let increment = Wad::from(max - base).mul(env, Wad::from(scale)).raw();
        base + Bps::from(curve.factor).apply_to(env, increment)
    };
    if p <= 0 || hf >= WAD {
        return scaled;
    }
    let cap = hf * BPS / p - BPS;
    if c < d {
        base
    } else if cap < base {
        cap.max(0)
    } else {
        scaled.min(cap)
    }
}

/// `floor(U / (1 + b)) - R < D <= ceil((U + m) / (1 + b))`: neither rule 1 nor
/// rule 2 applies.
fn in_margin_band(env: &Env, unit: Wad, bonus_bps: i128, debt: i128, per_leg_unit: i128) -> bool {
    let one_plus_bonus = Wad::ONE.checked_add(env, Bps::from(bonus_bps).to_wad(env));
    let margin = (unit.raw() / 1_000_000).max(1);
    let unit_at_bonus = mul_div_floor(env, unit.raw(), WAD, one_plus_bonus.raw());
    let unit_repayment = mul_div_ceil(env, unit.raw() + margin, WAD, one_plus_bonus.raw());
    unit_at_bonus < debt + per_leg_unit && unit_repayment >= debt
}

fn rat(v: i128) -> BigRational {
    BigRational::from_integer(BigInt::from(v))
}

fn per_unit(price: i128, decimals: u32) -> BigRational {
    rat(price) / rat(10i128.pow(decimals))
}

#[derive(Clone, Debug, PartialEq)]
enum Revert {
    Code(u32),
    Host(String),
}

fn revert_of(err: Result<Error, InvokeError>) -> Revert {
    match err {
        Ok(e) if e.is_type(ScErrorType::Contract) => Revert::Code(e.get_code()),
        Ok(e) => Revert::Host(format!("{e:?}")),
        Err(e) => Revert::Host(format!("{e:?}")),
    }
}

fn refunded(estimate: &LiquidationEstimate, asset: &Address) -> i128 {
    estimate
        .refunds
        .iter()
        .filter(|r| &r.asset == asset)
        .map(|r| r.amount)
        .sum()
}

/// Runs one liquidation, checks P1-P8 and returns whether the account is
/// still liquidatable with debt left.
fn liquidate_step(
    w: &mut World,
    offer_bps: i128,
    all_legs: bool,
    credit: bool,
    residue: bool,
    stats: &Stats,
) -> Result<bool, TestCaseError> {
    let liquidator = w.t.get_or_create_user(LIQUIDATOR);
    let env = w.t.env.clone();
    let id = w.account;
    let totals = w.totals();
    let (c, d, _, hf) = totals;
    prop_assert!(hf < WAD, "step needs HF < 1, got {hf}");
    let unit_scaled = w.unit_scaled();
    let scaled_before = w.liq_scaled(id);
    let units_before = scaled_before / unit_scaled;
    prop_assert_eq!(
        scaled_before % unit_scaled,
        0,
        "P1: held shares are whole units"
    );
    let solvent = c >= d;
    let unit = w.unit_usd();
    let bonus_m = mirror_bonus(&env, totals, w.listed_bonus, &w.curve);
    let legs = w.legs.clone();
    let debts: std::vec::Vec<i128> = legs.iter().map(|leg| w.debt_ceil(leg)).collect();
    let per_leg_unit = legs
        .iter()
        .zip(&debts)
        .filter(|(_, debt)| **debt > 0)
        .map(|(leg, _)| {
            Wad::from_token(&env, 1, leg.decimals)
                .mul(&env, Wad::from(leg.price))
                .raw()
        })
        .sum::<i128>();
    let band = solvent && in_margin_band(&env, unit, bonus_m, d, per_leg_unit);
    if band {
        bump(stats, "band: step in margin band");
    }

    let offer_every_leg = all_legs || debts[0] == 0;
    let mut offers = std::vec::Vec::new();
    let mut payments = vec![&env];
    for (i, leg) in legs.iter().enumerate() {
        let offer = if debts[i] == 0 || (i > 0 && !offer_every_leg) {
            0
        } else {
            (debts[i] * offer_bps / BPS).max(1)
        };
        offers.push(offer);
        if offer > 0 {
            payments.push_back((leg.key.clone(), offer));
        }
    }
    let debt_sized = offer_bps >= BPS && offer_every_leg;
    let offered_usd = legs
        .iter()
        .enumerate()
        .filter(|(i, _)| offers[*i] > 0)
        .map(|(i, leg)| {
            Wad::from_token(&env, offers[i].min(debts[i]), leg.decimals)
                .mul(&env, Wad::from(leg.price))
                .raw()
        })
        .sum::<i128>();
    let one_plus_bonus_m = Wad::ONE.checked_add(&env, Bps::from(bonus_m).to_wad(&env));
    let offered_backing = Wad::from(offered_usd).mul(&env, one_plus_bonus_m).raw();
    let unit_tolerance = unit.raw() / 1_000_000_000_000 + 4;
    let sub_unit_offer = offered_backing < unit.raw() + unit_tolerance;
    let ctx = format!(
        "units={units_before} C={c} D={d} HF={hf} U={} b={bonus_m} R={per_leg_unit} \
         debts={debts:?} offers={offers:?} solvent={solvent} band={band} residue={residue}",
        unit.raw()
    );

    let mode = if credit {
        SeizeMode::Credit(0)
    } else {
        SeizeMode::Transfer
    };
    let estimate =
        w.t.ctrl_client()
            .try_get_liquidation_estimate(&id, &payments, &mode);
    for (i, leg) in legs.iter().enumerate() {
        if offers[i] > 0 {
            w.t.resolve_market(leg.name)
                .token_admin
                .mint(&liquidator, &offers[i]);
        }
    }
    let balance = |asset: &Address| token::Client::new(&env, asset).balance(&liquidator);
    let paid_before: std::vec::Vec<i128> = legs.iter().map(|l| balance(&l.key.asset)).collect();
    let liq_before = balance(&w.liq_key.asset);

    if debt_sized && solvent && units_before >= 1 {
        bump(stats, "p8: debt-sized solvent offers");
    }

    let executed =
        w.t.ctrl_client()
            .try_liquidate(&liquidator, &id, &payments, &mode);
    let receiver = match executed {
        Ok(Ok(receiver)) => receiver,
        Ok(Err(e)) => return Err(TestCaseError::fail(format!("receiver decode: {e:?}"))),
        Err(err) => {
            let revert = revert_of(err);
            match estimate {
                Err(e) => prop_assert_eq!(
                    revert_of(e),
                    revert.clone(),
                    "P7: estimate and execution revert alike; {}",
                    ctx
                ),
                Ok(Ok(e)) if revert == Revert::Code(errors::INVALID_PAYMENTS) => {
                    prop_assert!(
                        e.seized_collaterals.is_empty() && e.max_payment_wad == 0,
                        "P7: an empty plan shows a zero estimate; {}",
                        ctx
                    );
                    for (i, leg) in legs.iter().enumerate() {
                        prop_assert_eq!(
                            refunded(&e, &leg.key.asset),
                            offers[i],
                            "P7: an empty plan refunds every offer; {}",
                            &ctx
                        );
                    }
                }
                Ok(_) => {
                    return Err(TestCaseError::fail(format!(
                        "P7: estimate settled but execution reverted {revert:?}; {ctx}"
                    )))
                }
            }
            if revert != Revert::Code(errors::INVALID_PAYMENTS) {
                return Err(TestCaseError::fail(format!(
                    "unexpected revert {revert:?}; {ctx}"
                )));
            }
            if debt_sized && units_before >= 1 {
                prop_assert!(
                    solvent && band,
                    "P8: a debt-sized offer reverted outside the margin band; {}",
                    ctx
                );
                bump(stats, "revert InvalidPayments: debt-sized, margin band");
                record_max(
                    stats,
                    "debt-sized band revert: k * LT * (1 + b)",
                    &(rat(units_before) * rat(w.lt) * rat(BPS + bonus_m) / rat(BPS * BPS)),
                );
                if units_before == 1 {
                    bump(
                        stats,
                        "revert InvalidPayments: debt-sized, margin band, 1 unit",
                    );
                }
            } else if sub_unit_offer {
                bump(stats, "revert InvalidPayments: offer backs < 1 unit");
                if offered_backing >= unit.raw() {
                    bump(stats, "revert InvalidPayments: offer within unit tolerance");
                }
            } else if band {
                bump(stats, "revert InvalidPayments: partial offer, margin band");
            } else {
                return Err(TestCaseError::fail(format!(
                    "P8: an offer backing {offered_backing} >= one unit reverted outside the band; {ctx}"
                )));
            }
            return Ok(false);
        }
    };
    let estimate = match estimate {
        Ok(Ok(estimate)) => estimate,
        other => {
            return Err(TestCaseError::fail(format!(
                "P7: execution settled but the estimate did not: {:?}; {ctx}",
                other.err()
            )))
        }
    };
    prop_assert_eq!(
        estimate.bonus_rate_bps,
        bonus_m,
        "mirrored bonus disagrees with the controller; {}",
        &ctx
    );

    let exists = w.exists(id);
    let scaled_after = if exists { w.liq_scaled(id) } else { 0 };
    let paid: std::vec::Vec<i128> = legs
        .iter()
        .enumerate()
        .map(|(i, l)| paid_before[i] - balance(&l.key.asset))
        .collect();
    let received_scaled = if credit {
        prop_assert!(receiver > 0, "credit mode names a receiver");
        w.receivers.push(receiver);
        w.liq_scaled(receiver)
    } else {
        (balance(&w.liq_key.asset) - liq_before) * unit_scaled
    };
    let received_units = received_scaled / unit_scaled;
    prop_assert_eq!(received_scaled % unit_scaled, 0, "P1: whole units received");
    prop_assert_eq!(scaled_after % unit_scaled, 0, "P1: whole units kept");
    if exists {
        prop_assert_eq!(
            scaled_before - scaled_after,
            received_scaled,
            "P1: the liquidator receives exactly the seized shares (fee 0)"
        );
    } else {
        prop_assert!(received_scaled <= scaled_before);
    }
    prop_assert!(received_units >= 1, "a settled call seizes; {}", ctx);

    let seized_estimate = estimate
        .seized_collaterals
        .get(0)
        .map(|s| s.amount)
        .unwrap_or(0);
    let seized_expected = if credit {
        received_scaled
    } else {
        received_units
    };
    prop_assert_eq!(seized_estimate, seized_expected, "P7: seizure; {}", &ctx);
    prop_assert!(
        estimate.protocol_fees.iter().all(|f| f.amount == 0),
        "listing below 3 decimals carries no fee"
    );
    let mut paid_usd_wad = 0i128;
    for (i, leg) in legs.iter().enumerate() {
        let planned = offers[i] - refunded(&estimate, &leg.key.asset);
        prop_assert_eq!(paid[i], planned, "P7: paid leg {}; {}", leg.name, &ctx);
        paid_usd_wad += Wad::from_token(&env, paid[i], leg.decimals)
            .mul(&env, Wad::from(leg.price))
            .raw();
    }
    prop_assert_eq!(
        estimate.max_payment_wad,
        paid_usd_wad,
        "P7: estimate repayment value; {}",
        &ctx
    );

    let bonus = rat(estimate.bonus_rate_bps);
    let one_plus_b = rat(BPS) + &bonus;
    let unit_value = per_unit(w.liq_price, w.decimals);
    let received = rat(received_units) * &unit_value;
    let paid_value = legs
        .iter()
        .enumerate()
        .map(|(i, l)| rat(paid[i]) * per_unit(l.price, l.decimals))
        .fold(rat(0), |a, b| a + b);
    let per_leg = legs
        .iter()
        .zip(&debts)
        .filter(|(_, debt)| **debt > 0)
        .map(|(l, _)| per_unit(l.price, l.decimals))
        .fold(rat(0), |a, b| a + b);
    let tol = rat(WEI_TOLERANCE);
    let full_close = debts.iter().zip(&paid).all(|(debt, p)| p >= debt);
    let at_bonus = &paid_value * &one_plus_b / rat(BPS);
    let excess = &received - &at_bonus;
    if full_close {
        prop_assert!(
            excess <= &unit_value + &tol,
            "P2: full close received {} for {} at bonus; {}",
            received,
            at_bonus,
            ctx
        );
        record_max(
            stats,
            "full close: excess over the bonus, in units",
            &(&excess / &unit_value),
        );
        if received_units == 1 && at_bonus < unit_value {
            bump(stats, "ok: full close, one unit for the whole debt");
            let ceiling = rat(BPS) / (rat(units_before) * rat(w.lt));
            prop_assert!(
                &received / &paid_value < ceiling,
                "one unit for the whole debt pays at most 1 / (k * LT); {}",
                ctx
            );
            let effective_bps = (&received / &paid_value - rat(1)) * rat(BPS);
            record_max(
                stats,
                "one-unit full close: effective bonus bps",
                &effective_bps,
            );
            record_max(
                stats,
                "one-unit full close: effective minus quoted bonus bps",
                &(effective_bps - &bonus),
            );
        }
    } else {
        prop_assert!(
            excess <= &per_leg * &one_plus_b / rat(BPS) + &tol,
            "P2: partial received {} above {} at bonus; {}",
            received,
            at_bonus,
            ctx
        );
        let undercharge = &received * rat(BPS) / &one_plus_b - &paid_value;
        prop_assert!(
            undercharge <= &per_leg + &tol,
            "P3: paid {} for {} at bonus; {}",
            paid_value,
            received,
            ctx
        );
        record_max(
            stats,
            "partial: received above paid * (1 + b), WAD",
            &excess,
        );
        record_max(stats, "partial: undercharge, WAD", &undercharge);
    }

    let (c_after, d_after) = if exists {
        let (c2, d2, _, _) = w.totals();
        (c2, d2)
    } else {
        (0, 0)
    };
    prop_assert!(d_after < d, "P5: debt falls; {} -> {}; {}", d, d_after, ctx);
    let units_after = scaled_after / unit_scaled;
    if exists && d_after > 0 {
        prop_assert!(units_after > 0, "P5: debt left without collateral; {}", ctx);
    }
    if solvent {
        prop_assert!(
            full_close || (exists && d_after > 0),
            "P5: a solvent account lost its debt without full repayment; {}",
            ctx
        );
        if d_after > 0 {
            let before = BigInt::from(c) * d_after;
            let after = BigInt::from(c_after) * d;
            prop_assert!(
                &after * COVERAGE_TOLERANCE >= &before * (COVERAGE_TOLERANCE - 1),
                "P4: C/D fell: {}/{} -> {}/{}; {}",
                c,
                d,
                c_after,
                d_after,
                ctx
            );
            if after < before {
                bump(stats, "P4: C/D fell inside the 1e-12 tolerance");
            }
        }
    }

    let pool = w.t.resolve_market(LIQ).pool.clone();
    let pool_balance = token::Client::new(&env, &w.liq_key.asset).balance(&pool);
    let revenue = w.t.pool_client(LIQ).get_revenue(&w.liq_key);
    let mut held = w.units_of(id);
    for r in &w.receivers {
        held += w.units_of(*r);
    }
    prop_assert_eq!(
        pool_balance,
        held + revenue,
        "P6: pool units == account units"
    );
    prop_assert_eq!(
        w.t.pool_state_on_hub(w.liq_key.hub_id, LIQ).cash,
        pool_balance,
        "P6: pool cash == token balance"
    );
    prop_assert_eq!(
        w.t.ctrl_client().get_market_index(&w.liq_key).supply_index,
        RAY,
        "supply index of a collateral-only market stays RAY"
    );

    let kind = if full_close {
        "ok: full close"
    } else if !exists || d_after == 0 {
        "ok: seized all, residue socialized"
    } else if units_after == 0 {
        "ok: seized all"
    } else {
        "ok: partial"
    };
    bump(stats, kind);
    if band {
        bump(stats, "band: settled, the curve quote backs a unit");
    }
    if residue {
        bump(stats, "residue: settled step");
        if units_before == 1 {
            bump(stats, "residue: settled 1-unit step");
        }
    }

    Ok(exists && d_after > 0 && w.t.ctrl_client().get_health_factor(&id) < WAD)
}

fn run_case(case: &Case, stats: &Stats) -> Result<(), TestCaseError> {
    let Some(mut w) = open_world(case)? else {
        bump(stats, "skipped: not liquidatable after pricing");
        return Ok(());
    };
    bump(stats, "cases");
    let (c, d, _, _) = w.totals();
    bump(
        stats,
        if c < d {
            "start: insolvent"
        } else {
            "start: solvent"
        },
    );
    let mut more = liquidate_step(
        &mut w,
        case.offer_bps,
        case.offer_all_legs,
        case.credit,
        false,
        stats,
    )?;
    let mut steps = 0;
    while more && steps < MAX_RESIDUE_STEPS {
        steps += 1;
        let units = w.units_of(w.account);
        bump(stats, "residue: step");
        if units == 1 {
            bump(stats, "residue: 1-unit step");
        }
        more = liquidate_step(&mut w, BPS, true, case.credit, true, stats)?;
    }
    Ok(())
}

/// Runs `f` on an unnamed thread, so the host writes no per-`Env` test snapshot.
fn unnamed_thread<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .unwrap_or_else(|e| std::panic::resume_unwind(e));
}

#[test]
fn prop_whole_unit_liquidation_holds_the_documented_bounds() {
    unnamed_thread(|| {
        let mut cfg = config(64);
        cfg.source_file = Some(file!());
        let mut runner = TestRunner::new(cfg);
        let stats: Stats = RefCell::new(Report::default());
        let result = runner.run(&case_strategy(), |case| run_case(&case, &stats));
        let report = stats.borrow();
        println!("whole-unit liquidation classification:");
        for (key, count) in &report.counts {
            println!("  {count:>7}  {key}");
        }
        for (key, value) in &report.maxima {
            println!("  max {key}: {value:e}");
        }
        if let Err(e) = result {
            panic!("{e}");
        }
    });
}

/// A two-unit $1,000 unit-edge case. With LT 26%, bonus 3% and curve factor
/// 15%, the controller's bonus at the edge is 45.24% and HF is about 0.755.
fn edge_case(pick: i128, lt: u32, bonus: u32, factor_bps: u32) -> Case {
    Case {
        decimals: 0,
        unit_cents: 100_000,
        units: 2,
        lt,
        ltv: lt * 3 / 4,
        bonus,
        target_hf_bps: 11_000,
        knee_bps: 8_000,
        factor_bps,
        second_leg: None,
        second_share_pct: 10,
        borrow_pct: 90,
        accrue_days: 0,
        band: Band::UnitEdge,
        band_pick: pick,
        offer_bps: BPS,
        offer_all_legs: true,
        credit: false,
    }
}

fn edge_world(case: &Case) -> World {
    open_world(case)
        .expect("edge setup")
        .expect("edge account is liquidatable")
}

fn step_counts(w: &mut World, offer_bps: i128) -> BTreeMap<String, u64> {
    let stats: Stats = RefCell::new(Report::default());
    liquidate_step(w, offer_bps, true, false, false, &stats)
        .unwrap_or_else(|e| panic!("properties hold: {e}"));
    let counts = stats.borrow().counts.clone();
    counts
}

/// Inside `floor(U / (1 + b)) - R < D <= ceil((U + m) / (1 + b))` a two-unit
/// account whose curve quote backs less than one unit refuses every offer;
/// one day of debt accrual moves it out, and a debt-sized offer then sells one
/// unit.
#[test]
fn wul_two_units_inside_the_margin_band_refuse_every_offer_until_accrual() {
    unnamed_thread(|| {
        let mut w = edge_world(&edge_case(5_000, 2_600, 300, 1_500));
        for offer_bps in [100, BPS, 3 * BPS] {
            let counts = step_counts(&mut w, offer_bps);
            assert_eq!(counts.get("band: step in margin band"), Some(&1));
            assert!(
                !counts.keys().any(|k| k.starts_with("ok:")),
                "offer {offer_bps} bps settled inside the band: {counts:?}"
            );
        }
        assert_eq!(w.units_of(w.account), 2);

        w.t.advance_and_sync(86_400);
        let counts = step_counts(&mut w, BPS);
        assert!(
            !counts.contains_key("band: step in margin band"),
            "{counts:?}"
        );
        assert_eq!(counts.get("ok: partial"), Some(&1), "{counts:?}");
        assert_eq!(w.units_of(w.account), 1);
    });
}

/// Just above the band, one unit at `1 + b` covers the debt plus one USDC base
/// unit: a debt-sized offer repays all debt for one unit (rule 1).
#[test]
fn wul_two_units_above_the_margin_band_close_in_full_for_one_unit() {
    unnamed_thread(|| {
        let mut w = edge_world(&edge_case(4_600, 2_600, 300, 1_500));
        let counts = step_counts(&mut w, BPS);
        assert_eq!(
            counts.get("ok: full close, one unit for the whole debt"),
            Some(&1),
            "{counts:?}"
        );
        assert_eq!(w.units_of(w.account), 1);
        assert_eq!(w.totals().1, 0);
    });
}

/// Just below the band, the raised quote `ceil((U + m) / (1 + b))` stays below
/// the debt: a debt-sized offer sells one unit and keeps the other (rule 2).
#[test]
fn wul_two_units_below_the_margin_band_sell_one_unit() {
    unnamed_thread(|| {
        let mut w = edge_world(&edge_case(10_000, 2_600, 300, 1_500));
        let (c, d, _, _) = w.totals();
        let counts = step_counts(&mut w, BPS);
        assert_eq!(counts.get("ok: partial"), Some(&1), "{counts:?}");
        assert_eq!(w.units_of(w.account), 1);
        let (c_after, d_after) = (w.totals().0, w.totals().1);
        assert!(d_after > 0);
        assert!(BigInt::from(c_after) * d >= BigInt::from(c) * d_after);
    });
}

/// A one-unit residue priced at its debt has `C / D` just above 1, so the
/// HF-preserving cap is 0: the band quote closes the whole debt at bonus 0
/// and the liquidator takes the last unit.
#[test]
fn wul_one_unit_residue_at_cover_closes_in_full_at_zero_bonus() {
    unnamed_thread(|| {
        for lt in [2_600u32, 6_000, 9_000] {
            let mut w = edge_world(&edge_case(5_001, lt, 500, 1_500));
            assert_eq!(w.units_of(w.account), 1, "LT {lt}");
            let totals = w.totals();
            assert_eq!(mirror_bonus(&w.t.env, totals, w.listed_bonus, &w.curve), 0);
            let counts = step_counts(&mut w, BPS);
            assert_eq!(
                counts.get("ok: full close"),
                Some(&1),
                "LT {lt}: {counts:?}"
            );
            assert_eq!(w.units_of(w.account), 0, "LT {lt}");
        }
    });
}

/// With the testnet LIQVID1039 listing (LTV 50%, LT 60%, bonus 5%, default
/// curve), two units priced at the unit edge have `HF = 2 * LT * (1 + b)`
/// above 1, so the margin band that refuses every offer cannot hold them.
#[test]
fn wul_listed_liqvid_two_units_are_healthy_at_the_unit_edge() {
    unnamed_thread(|| {
        for pick in [0i128, 2_000, 4_600, 5_000, 6_000, 8_000, 10_000] {
            let case = Case {
                unit_cents: 100,
                ltv: 5_000,
                factor_bps: BPS as u32,
                ..edge_case(pick, 6_000, 500, 1)
            };
            assert!(
                open_world(&case).expect("edge setup").is_none(),
                "pick {pick}: two listed units are liquidatable at the unit edge"
            );
        }
    });
}
