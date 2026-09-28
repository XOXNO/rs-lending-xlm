#![cfg(feature = "reference-math")]

extern crate std;

use std::vec::Vec;

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;
use num_traits::{Signed, ToPrimitive, Zero};

use common::constants::MIN_BORROWABLE_ASSET_DECIMALS;
use common::types::HubAssetKey;
use controller::constants::{
    BAD_DEBT_USD_THRESHOLD, BPS, DEFAULT_HF_FOR_MAX_BONUS_WAD,
    DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS, DEFAULT_LIQUIDATION_TARGET_HF_WAD, RAY, WAD,
};
use soroban_sdk::vec as soroban_vec;

use crate::context::LendingTest;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefCurve {
    pub target_hf_wad: i128,
    pub hf_for_max_bonus_wad: i128,
    pub bonus_factor_bps: i128,
}

impl Default for RefCurve {
    fn default() -> Self {
        Self {
            target_hf_wad: DEFAULT_LIQUIDATION_TARGET_HF_WAD,
            hf_for_max_bonus_wad: DEFAULT_HF_FOR_MAX_BONUS_WAD,
            bonus_factor_bps: i128::from(DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RefCollateralPosition {
    pub asset_id: u32,
    pub supply_scaled_ray: BigRational,
    pub supply_index: BigRational,
    pub price_wad: BigRational,
    pub liq_threshold_bps: i128,
    pub liq_bonus_bps: i128,
    pub liq_fees_bps: i128,
    pub decimals: u32,
}

#[derive(Clone, Debug)]
pub struct RefDebtPosition {
    pub asset_id: u32,
    pub borrow_scaled_ray: BigRational,
    pub borrow_index: BigRational,
    pub price_wad: BigRational,
    pub decimals: u32,
}

#[derive(Clone, Debug)]
pub struct RefLiquidationResult {
    pub health_factor_pre_wad: BigRational,

    pub final_bonus_bps: BigRational,

    pub seized_per_collateral: Vec<(u32, BigRational)>,

    pub repaid_per_debt: Vec<(u32, BigRational)>,

    pub protocol_fee_per_collateral: Vec<(u32, BigRational)>,

    pub total_repaid_usd_wad: BigRational,

    pub total_seized_usd_wad: BigRational,
}

fn bi_one() -> BigInt {
    BigInt::from(1)
}

fn br_zero() -> BigRational {
    BigRational::from_integer(BigInt::zero())
}

fn br_one() -> BigRational {
    BigRational::from_integer(bi_one())
}

fn br_ten_pow(exp: u32) -> BigRational {
    BigRational::from_integer(BigInt::from(10).pow(exp))
}

fn br_from_i128(v: i128) -> BigRational {
    BigRational::from_integer(BigInt::from(v))
}

fn ray_scale() -> BigRational {
    br_from_i128(RAY)
}

fn wad_scale() -> BigRational {
    br_from_i128(WAD)
}

fn bps_scale() -> BigRational {
    br_from_i128(BPS)
}

pub fn half_up_div(num: BigInt, denom: BigInt) -> BigInt {
    assert!(!denom.is_zero(), "half_up_div: zero denominator");
    let denom_abs = denom.clone().abs();
    let half = &denom_abs / 2;

    let neg = num.is_negative() ^ denom.is_negative();
    let num_abs = num.abs();
    let adjusted = num_abs + half;
    let mut q: BigInt = adjusted / denom_abs;
    if neg && !q.is_zero() {
        q = -q;
    }
    q
}

pub fn bigrational_to_i128_half_up(x: &BigRational) -> i128 {
    let num = x.numer().clone();
    let denom = x.denom().clone();
    let q = half_up_div(num, denom);
    q.to_i128().unwrap_or_else(|| {
        if matches!(q.sign(), Sign::Minus) {
            i128::MIN
        } else {
            i128::MAX
        }
    })
}

pub fn bigrational_to_i128_wad(x: &BigRational) -> i128 {
    bigrational_to_i128_half_up(x)
}

pub fn float_to_bigrational(x: f64, decimals: u32) -> BigRational {
    let raw = (x * 10f64.powi(decimals as i32)) as i128;
    br_from_i128(raw)
}

fn position_value_wad(
    scaled_ray: &BigRational,
    index_ray: &BigRational,
    price_wad: &BigRational,
) -> BigRational {
    let actual_ray = scaled_ray * index_ray / ray_scale();

    let actual_wad = &actual_ray / br_ten_pow(9);

    actual_wad * price_wad / wad_scale()
}

fn compute_hf_wad(supplies: &[RefCollateralPosition], debts: &[RefDebtPosition]) -> BigRational {
    if debts.is_empty() {
        return BigRational::from_integer(BigInt::from(i128::MAX));
    }

    let mut weighted = br_zero();
    for c in supplies {
        let value = position_value_wad(&c.supply_scaled_ray, &c.supply_index, &c.price_wad);

        let w = &value * br_from_i128(c.liq_threshold_bps) / bps_scale();
        weighted += w;
    }

    let mut total_debt = br_zero();
    for d in debts {
        let v = position_value_wad(&d.borrow_scaled_ray, &d.borrow_index, &d.price_wad);
        total_debt += v;
    }

    if total_debt.is_zero() {
        return BigRational::from_integer(BigInt::from(i128::MAX));
    }

    weighted * wad_scale() / total_debt
}

fn weighted_collateral_total(supplies: &[RefCollateralPosition]) -> BigRational {
    let mut w = br_zero();
    for c in supplies {
        let value = position_value_wad(&c.supply_scaled_ray, &c.supply_index, &c.price_wad);
        w += &value * br_from_i128(c.liq_threshold_bps) / bps_scale();
    }
    w
}

fn total_collateral_wad(supplies: &[RefCollateralPosition]) -> BigRational {
    let mut t = br_zero();
    for c in supplies {
        t += position_value_wad(&c.supply_scaled_ray, &c.supply_index, &c.price_wad);
    }
    t
}

fn total_debt_wad(debts: &[RefDebtPosition]) -> BigRational {
    let mut t = br_zero();
    for d in debts {
        t += position_value_wad(&d.borrow_scaled_ray, &d.borrow_index, &d.price_wad);
    }
    t
}

fn max_bonus_for_threshold(proportion_seized: &BigRational) -> BigRational {
    if !proportion_seized.is_positive() {
        return br_zero();
    }
    let bps = br_from_i128(BPS);
    let eff = (proportion_seized * &bps / wad_scale()).ceil();
    let eff = if eff < br_one() {
        br_one()
    } else if eff > bps {
        bps.clone()
    } else {
        eff
    };
    (&bps * (&bps - &eff) / &eff).floor()
}

fn get_account_bonus_params(
    supplies: &[RefCollateralPosition],
    proportion_seized: &BigRational,
) -> (BigRational, BigRational) {
    let max = max_bonus_for_threshold(proportion_seized);
    let total = total_collateral_wad(supplies);
    if total.is_zero() {
        return (br_zero(), max);
    }

    let mut weighted_bonus = br_zero();
    for c in supplies {
        let value = position_value_wad(&c.supply_scaled_ray, &c.supply_index, &c.price_wad);
        let share = &value / &total;
        weighted_bonus += share * br_from_i128(c.liq_bonus_bps);
    }

    let base = if weighted_bonus > max {
        max.clone()
    } else {
        weighted_bonus
    };
    (base, max)
}

fn calculate_linear_bonus_with_target(
    hf_wad: &BigRational,
    base_bps: &BigRational,
    max_bps: &BigRational,
    target_wad: &BigRational,
    curve: &RefCurve,
) -> BigRational {
    if hf_wad >= target_wad {
        return base_bps.clone();
    }
    let knee = br_from_i128(curve.hf_for_max_bonus_wad);
    let scale = if *target_wad <= knee {
        br_one()
    } else {
        let ratio = (target_wad - hf_wad) / (target_wad - &knee);
        if ratio > br_one() {
            br_one()
        } else {
            ratio
        }
    };
    let bonus_range = max_bps - base_bps;
    base_bps + &bonus_range * &scale * br_from_i128(curve.bonus_factor_bps) / bps_scale()
}

fn try_liquidation_at_target(
    total_debt_wad: &BigRational,
    weighted_coll_wad: &BigRational,
    bonus_bps: &BigRational,
    proportion_seized: &BigRational,
    total_collateral_wad: &BigRational,
    target_wad: &BigRational,
) -> Option<BigRational> {
    let bonus_wad = bonus_bps * &wad_scale() / bps_scale();
    let one_plus_bonus = &wad_scale() + &bonus_wad;

    let d_max = total_collateral_wad * &wad_scale() / &one_plus_bonus;

    let denom_term = proportion_seized * &one_plus_bonus / wad_scale();
    let denominator = target_wad - &denom_term;

    if !denominator.is_positive() {
        return None;
    }

    let target_debt = target_wad * total_debt_wad / wad_scale();
    if target_debt <= *weighted_coll_wad {
        let capped = if &d_max <= total_debt_wad {
            d_max
        } else {
            total_debt_wad.clone()
        };
        return Some(capped);
    }
    let numerator = target_debt - weighted_coll_wad;
    let d_ideal = &numerator * &wad_scale() / &denominator;

    let mut out = d_ideal;
    if out > d_max {
        out = d_max;
    }
    if out > *total_debt_wad {
        out = total_debt_wad.clone();
    }
    Some(out)
}

fn max_hf_preserving_bonus_bps(
    hf_wad: &BigRational,
    proportion_seized: &BigRational,
) -> Option<BigRational> {
    if proportion_seized <= &BigRational::from_integer(BigInt::from(0)) || hf_wad >= &wad_scale() {
        return None;
    }
    let floored = (hf_wad * bps_scale() / proportion_seized).floor();
    Some(floored - bps_scale())
}

fn select_liquidation_tier(
    total_debt_wad: &BigRational,
    weighted_coll_wad: &BigRational,
    hf_wad: &BigRational,
    (base_bonus_bps, max_bonus_bps): (&BigRational, &BigRational),
    proportion_seized: &BigRational,
    total_collateral_wad: &BigRational,
    curve: &RefCurve,
) -> (BigRational, BigRational) {
    let target = br_from_i128(curve.target_hf_wad);

    let scaled_bonus =
        calculate_linear_bonus_with_target(hf_wad, base_bonus_bps, max_bonus_bps, &target, curve);

    let bonus = match max_hf_preserving_bonus_bps(hf_wad, proportion_seized) {
        Some(cap) if cap < scaled_bonus => cap,
        _ => scaled_bonus,
    };

    let ideal = match try_liquidation_at_target(
        total_debt_wad,
        weighted_coll_wad,
        &bonus,
        proportion_seized,
        total_collateral_wad,
        &target,
    ) {
        Some(d) => d,
        None => {
            let bonus_wad = &bonus * &wad_scale() / bps_scale();
            let one_plus_bonus = &wad_scale() + &bonus_wad;
            let d_max = total_collateral_wad * &wad_scale() / &one_plus_bonus;
            if d_max > *total_debt_wad {
                total_debt_wad.clone()
            } else {
                d_max
            }
        }
    };

    (ideal, bonus)
}

fn estimate_liquidation_amount(
    total_debt_wad: &BigRational,
    weighted_coll_wad: &BigRational,
    hf_wad: &BigRational,
    (base_bonus_bps, max_bonus_bps): (&BigRational, &BigRational),
    proportion_seized: &BigRational,
    total_collateral_wad: &BigRational,
    curve: &RefCurve,
) -> (BigRational, BigRational) {
    match max_hf_preserving_bonus_bps(hf_wad, proportion_seized) {
        Some(_) if total_collateral_wad < total_debt_wad => {
            let one_plus_base = &wad_scale() + base_bonus_bps * &wad_scale() / bps_scale();
            let backed = (total_collateral_wad * &wad_scale() / &one_plus_base).floor();
            let ideal = if backed < *total_debt_wad {
                backed
            } else {
                total_debt_wad.clone()
            };
            return (ideal, base_bonus_bps.clone());
        }
        Some(cap) if &cap < base_bonus_bps => {
            return (
                total_debt_wad.clone(),
                if cap.is_negative() { br_zero() } else { cap },
            );
        }
        _ => {}
    }

    let (ideal, bonus) = select_liquidation_tier(
        total_debt_wad,
        weighted_coll_wad,
        hf_wad,
        (base_bonus_bps, max_bonus_bps),
        proportion_seized,
        total_collateral_wad,
        curve,
    );

    let remaining = total_debt_wad - &ideal;
    let floor = br_from_i128(BAD_DEBT_USD_THRESHOLD);
    if remaining > br_zero() && remaining < floor {
        return (total_debt_wad.clone(), bonus);
    }

    (ideal, bonus)
}

/// Trims `excess` from the last leg backward, rounding each kept amount down to
/// whole token units, and returns the USD value kept.
fn kept_within_quote_usd(
    debt: &[RefDebtPosition],
    legs: &[(u32, BigRational, u32)],
    excess: &BigRational,
) -> BigRational {
    let mut remaining = excess.clone();
    let mut kept = br_zero();
    for (asset_id, tokens, decimals) in legs.iter().rev() {
        let price = &debt
            .iter()
            .find(|d| d.asset_id == *asset_id)
            .expect("debt payment references unknown asset_id")
            .price_wad;
        let to_usd =
            |tokens: &BigRational| tokens * br_ten_pow(18 - decimals) * price / wad_scale();
        let usd = to_usd(tokens);
        if !remaining.is_positive() {
            kept += usd;
        } else if usd <= remaining {
            remaining -= usd;
        } else {
            let kept_tokens =
                ((&usd - &remaining) * wad_scale() / price / br_ten_pow(18 - decimals)).floor();
            kept += to_usd(&kept_tokens);
            remaining = br_zero();
        }
    }
    kept
}

pub fn compute_liquidation(
    collateral: &[RefCollateralPosition],
    debt: &[RefDebtPosition],
    debt_payments: &[(u32, BigRational)],
    _target_hf_wad: BigRational,
) -> RefLiquidationResult {
    compute_liquidation_with_curve(collateral, debt, debt_payments, &RefCurve::default())
}

pub fn compute_liquidation_with_curve(
    collateral: &[RefCollateralPosition],
    debt: &[RefDebtPosition],
    debt_payments: &[(u32, BigRational)],
    curve: &RefCurve,
) -> RefLiquidationResult {
    let hf_wad = compute_hf_wad(collateral, debt);

    let total_coll = total_collateral_wad(collateral);
    let total_debt = total_debt_wad(debt);
    let weighted_coll = weighted_collateral_total(collateral);

    let proportion_seized = if total_coll.is_zero() {
        br_zero()
    } else {
        &weighted_coll * &wad_scale() / &total_coll
    };

    let (base_bonus_bps, max_bonus_bps) = get_account_bonus_params(collateral, &proportion_seized);

    let mut total_payment_usd = br_zero();
    let mut per_debt_payments_usd: Vec<(u32, BigRational, u32)> = Vec::new();
    for (asset_id, amt_tokens) in debt_payments {
        let d = debt
            .iter()
            .find(|d| d.asset_id == *asset_id)
            .expect("debt payment references unknown asset_id");

        let actual_ray = &d.borrow_scaled_ray * &d.borrow_index / ray_scale();
        let scale_diff = 27 - d.decimals;
        let actual_tokens = actual_ray / br_ten_pow(scale_diff);
        let payment_tokens = if amt_tokens > &actual_tokens {
            actual_tokens.clone()
        } else {
            amt_tokens.clone()
        };

        let payment_wad = if d.decimals <= 18 {
            &payment_tokens * br_ten_pow(18 - d.decimals)
        } else {
            &payment_tokens / br_ten_pow(d.decimals - 18)
        };

        let payment_usd = &payment_wad * &d.price_wad / wad_scale();
        total_payment_usd += &payment_usd;
        per_debt_payments_usd.push((*asset_id, payment_tokens, d.decimals));
    }

    let (ideal_repayment, bonus_bps) = estimate_liquidation_amount(
        &total_debt,
        &weighted_coll,
        &hf_wad,
        (&base_bonus_bps, &max_bonus_bps),
        &proportion_seized,
        &total_coll,
        curve,
    );

    let final_repayment_usd =
        if total_payment_usd < ideal_repayment || ideal_repayment >= total_debt {
            total_payment_usd.clone()
        } else if total_coll < total_debt {
            kept_within_quote_usd(
                debt,
                &per_debt_payments_usd,
                &(&total_payment_usd - &ideal_repayment),
            )
        } else {
            ideal_repayment
        };
    let one_plus_bonus_wad = &wad_scale() + &bonus_bps * &wad_scale() / bps_scale();
    let total_seizure_usd = &final_repayment_usd * &one_plus_bonus_wad / wad_scale();

    let mut seized: Vec<(u32, BigRational)> = Vec::new();
    let mut fees: Vec<(u32, BigRational)> = Vec::new();
    if !total_coll.is_zero() {
        for c in collateral {
            if c.price_wad.is_zero() {
                continue;
            }
            let actual_ray = &c.supply_scaled_ray * &c.supply_index / ray_scale();
            let actual_wad = &actual_ray / br_ten_pow(9);
            let asset_value = &actual_wad * &c.price_wad / wad_scale();
            let share = &asset_value / &total_coll;
            let seizure_usd_for_asset = &total_seizure_usd * &share;
            let seizure_wad = &seizure_usd_for_asset * &wad_scale() / &c.price_wad;

            let seizure_tokens = if c.decimals <= 18 {
                &seizure_wad / br_ten_pow(18 - c.decimals)
            } else {
                &seizure_wad * br_ten_pow(c.decimals - 18)
            };
            let actual_tokens = if c.decimals <= 27 {
                &actual_ray / br_ten_pow(27 - c.decimals)
            } else {
                &actual_ray * br_ten_pow(c.decimals - 27)
            };
            // Base is the leg's repayment share, taken before the clamp; a clamp
            // below the base leaves no bonus to charge a fee on.
            let base_amount = &seizure_tokens * &wad_scale() / &one_plus_bonus_wad;
            let capped = if seizure_tokens > actual_tokens {
                actual_tokens
            } else {
                seizure_tokens
            };

            let bonus_portion = if capped > base_amount {
                &capped - &base_amount
            } else {
                BigRational::from_integer(BigInt::from(0))
            };
            let fee = &bonus_portion * br_from_i128(c.liq_fees_bps) / bps_scale();
            seized.push((c.asset_id, capped));
            fees.push((c.asset_id, fee));
        }
    }

    let repaid_per_debt: Vec<(u32, BigRational)> = per_debt_payments_usd
        .iter()
        .map(|(id, tokens, _dec)| (*id, tokens.clone()))
        .collect();

    RefLiquidationResult {
        health_factor_pre_wad: hf_wad,
        final_bonus_bps: bonus_bps,
        seized_per_collateral: seized,
        repaid_per_debt,
        protocol_fee_per_collateral: fees,
        total_repaid_usd_wad: final_repayment_usd,
        total_seized_usd_wad: total_seizure_usd,
    }
}

fn big(x: i128) -> BigInt {
    BigInt::from(x)
}

fn fit(x: BigInt) -> i128 {
    x.to_i128().expect("exact reference value exceeds i128")
}

fn nonneg(x: i128, y: i128, d: i128) {
    assert!(
        x >= 0 && y >= 0 && d > 0,
        "exact reference operands must be nonnegative"
    );
}

fn mul_div_floor(x: i128, y: i128, d: i128) -> i128 {
    nonneg(x, y, d);
    fit(big(x) * big(y) / big(d))
}

fn mul_div_ceil(x: i128, y: i128, d: i128) -> i128 {
    nonneg(x, y, d);
    fit((big(x) * big(y) + big(d) - 1) / big(d))
}

fn mul_div_half_up(x: i128, y: i128, d: i128) -> i128 {
    nonneg(x, y, d);
    fit((big(x) * big(y) + big(d / 2)) / big(d))
}

#[derive(Clone, Copy)]
enum Round {
    Floor,
    Ceil,
    HalfUp,
}

fn rescale(a: i128, from: u32, to: u32, round: Round) -> i128 {
    assert!(a >= 0, "exact reference rescales nonnegative values");
    if to >= from {
        return a * 10i128.pow(to - from);
    }
    let f = 10i128.pow(from - to);
    match round {
        Round::Floor => a / f,
        Round::Ceil => a / f + i128::from(a % f != 0),
        Round::HalfUp => (a + f / 2) / f,
    }
}

fn bps_to_wad(bps: i128) -> i128 {
    mul_div_half_up(bps, WAD, BPS)
}

fn token_usd(amount: i128, decimals: u32, price_wad: i128) -> i128 {
    mul_div_half_up(rescale(amount, decimals, 18, Round::HalfUp), price_wad, WAD)
}

#[derive(Clone, Debug)]
pub struct ExactCollateral {
    pub asset_id: u32,
    pub scaled_ray: i128,
    pub supply_index_ray: i128,
    pub price_wad: i128,
    pub decimals: u32,
    pub liq_threshold_bps: i128,
    pub liq_bonus_bps: i128,
    pub liq_fees_bps: i128,
}

impl ExactCollateral {
    fn value_half_up(&self) -> i128 {
        let actual = mul_div_half_up(self.scaled_ray, self.supply_index_ray, RAY);
        mul_div_half_up(rescale(actual, 27, 18, Round::HalfUp), self.price_wad, WAD)
    }

    fn value_floor(&self) -> i128 {
        let actual = mul_div_floor(self.scaled_ray, self.supply_index_ray, RAY);
        mul_div_floor(rescale(actual, 27, 18, Round::Floor), self.price_wad, WAD)
    }
}

#[derive(Clone, Debug)]
pub struct ExactDebt {
    pub asset_id: u32,
    pub scaled_ray: i128,
    pub borrow_index_ray: i128,
    pub price_wad: i128,
    pub decimals: u32,
}

impl ExactDebt {
    fn value_ceil(&self) -> i128 {
        let actual = mul_div_ceil(self.scaled_ray, self.borrow_index_ray, RAY);
        mul_div_ceil(rescale(actual, 27, 18, Round::Ceil), self.price_wad, WAD)
    }

    pub fn balance_ceil(&self) -> i128 {
        let actual = mul_div_ceil(self.scaled_ray, self.borrow_index_ray, RAY);
        rescale(actual, 27, self.decimals, Round::Ceil)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExactBook {
    pub keys: Vec<HubAssetKey>,
    pub collateral: Vec<ExactCollateral>,
    pub debt: Vec<ExactDebt>,
}

impl ExactBook {
    pub fn id_of(&self, key: &HubAssetKey) -> u32 {
        self.keys
            .iter()
            .position(|k| k == key)
            .expect("hub asset is not in the book") as u32
    }

    pub fn key_of(&self, asset_id: u32) -> &HubAssetKey {
        &self.keys[asset_id as usize]
    }

    fn intern(&mut self, key: HubAssetKey) -> u32 {
        match self.keys.iter().position(|k| *k == key) {
            Some(i) => i as u32,
            None => {
                self.keys.push(key);
                (self.keys.len() - 1) as u32
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExactTotals {
    pub total_collateral: i128,
    pub weighted_collateral: i128,
    pub total_debt: i128,
    pub health_factor: i128,
    pub proportion_seized: i128,
    pub base_bonus_bps: i128,
    pub max_bonus_bps: i128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactSeizure {
    pub asset_id: u32,
    pub amount: i128,
    pub protocol_fee: i128,
    pub scaled_amount: i128,
    pub bonus_scaled: i128,
    pub credit_fee_scaled: i128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactPlan {
    pub totals: ExactTotals,
    pub quote_usd: i128,
    pub bonus_bps: i128,
    pub repay_usd: i128,
    pub repaid: Vec<(u32, i128)>,
    pub refunds: Vec<(u32, i128)>,
    pub seize_all: bool,
    pub seized: Vec<ExactSeizure>,
}

pub fn max_bonus_for_threshold_exact(proportion_seized: i128) -> i128 {
    if proportion_seized <= 0 {
        return 0;
    }
    let eff = mul_div_ceil(proportion_seized, BPS, WAD).clamp(1, BPS);
    BPS * (BPS - eff) / eff
}

pub fn exact_totals(collateral: &[ExactCollateral], debt: &[ExactDebt]) -> ExactTotals {
    let mut total_collateral = 0i128;
    let mut weighted_collateral = 0i128;
    for c in collateral {
        total_collateral += c.value_half_up();
        weighted_collateral += mul_div_floor(c.value_floor(), bps_to_wad(c.liq_threshold_bps), WAD);
    }
    let total_debt: i128 = debt.iter().map(ExactDebt::value_ceil).sum();
    let health_factor = if total_debt == 0 {
        i128::MAX
    } else {
        (big(weighted_collateral) * big(WAD) / big(total_debt))
            .to_i128()
            .unwrap_or(i128::MAX)
    };
    let proportion_seized = if total_collateral > 0 {
        mul_div_half_up(weighted_collateral, WAD, total_collateral)
    } else {
        0
    };
    let max_bonus_bps = max_bonus_for_threshold_exact(proportion_seized);
    let base_bonus_bps = if total_collateral == 0 {
        0
    } else {
        collateral
            .iter()
            .map(|c| {
                let weight = mul_div_half_up(c.value_half_up(), WAD, total_collateral);
                mul_div_half_up(weight, c.liq_bonus_bps, WAD)
            })
            .sum::<i128>()
            .min(max_bonus_bps)
    };
    ExactTotals {
        total_collateral,
        weighted_collateral,
        total_debt,
        health_factor,
        proportion_seized,
        base_bonus_bps,
        max_bonus_bps,
    }
}

pub fn bonus_scale_exact(curve: &RefCurve, hf_wad: i128, target_wad: i128) -> i128 {
    if target_wad <= curve.hf_for_max_bonus_wad {
        return WAD;
    }
    mul_div_half_up(
        target_wad - hf_wad,
        WAD,
        target_wad - curve.hf_for_max_bonus_wad,
    )
    .min(WAD)
}

pub fn calculate_linear_bonus_with_target_exact(
    hf_wad: i128,
    base_bps: i128,
    max_bps: i128,
    curve: &RefCurve,
    target_wad: i128,
) -> i128 {
    if hf_wad >= target_wad {
        return base_bps;
    }
    let scale = bonus_scale_exact(curve, hf_wad, target_wad);
    let increment = mul_div_half_up(max_bps - base_bps, scale, WAD);
    base_bps + mul_div_half_up(increment, curve.bonus_factor_bps, BPS)
}

pub fn max_hf_preserving_bonus_bps_exact(hf_wad: i128, proportion_seized: i128) -> Option<i128> {
    if proportion_seized <= 0 || hf_wad >= WAD {
        return None;
    }
    Some(hf_wad * BPS / proportion_seized - BPS)
}

fn liquidation_at_target_exact(totals: &ExactTotals, bonus_bps: i128, target_wad: i128) -> i128 {
    let one_plus_bonus = WAD + bps_to_wad(bonus_bps);
    let d_max = mul_div_half_up(totals.total_collateral, WAD, one_plus_bonus);
    let denom_term = mul_div_half_up(totals.proportion_seized, one_plus_bonus, WAD);
    let target_debt = mul_div_half_up(target_wad, totals.total_debt, WAD);
    if target_wad <= denom_term || target_debt <= totals.weighted_collateral {
        return d_max.min(totals.total_debt);
    }
    mul_div_half_up(
        target_debt - totals.weighted_collateral,
        WAD,
        target_wad - denom_term,
    )
    .min(d_max)
    .min(totals.total_debt)
}

pub fn estimate_liquidation_amount_exact(totals: &ExactTotals, curve: &RefCurve) -> (i128, i128) {
    let base = totals.base_bonus_bps;
    let scaled_bonus = calculate_linear_bonus_with_target_exact(
        totals.health_factor,
        base,
        totals.max_bonus_bps,
        curve,
        curve.target_hf_wad,
    );
    let bonus =
        match max_hf_preserving_bonus_bps_exact(totals.health_factor, totals.proportion_seized) {
            None => scaled_bonus,
            Some(_) if totals.total_collateral < totals.total_debt => {
                let backed = mul_div_floor(totals.total_collateral, WAD, WAD + bps_to_wad(base));
                return (backed.min(totals.total_debt), base);
            }
            Some(cap) if cap < base => return (totals.total_debt, cap.max(0)),
            Some(cap) => scaled_bonus.min(cap),
        };
    let ideal = liquidation_at_target_exact(totals, bonus, curve.target_hf_wad);
    let remaining = totals.total_debt - ideal;
    if remaining > 0 && remaining < BAD_DEBT_USD_THRESHOLD {
        return (totals.total_debt, bonus);
    }
    (ideal, bonus)
}

struct Leg {
    asset_id: u32,
    amount: i128,
    usd: i128,
    decimals: u32,
    price_wad: i128,
}

fn trim_excess(legs: &mut Vec<Leg>, refunds: &mut Vec<(u32, i128)>, excess: i128, keep: bool) {
    let mut remaining = excess;
    let mut i = legs.len();
    while remaining > 0 && i > 0 {
        i -= 1;
        let (amount, usd, decimals, price) = (
            legs[i].amount,
            legs[i].usd,
            legs[i].decimals,
            legs[i].price_wad,
        );
        if amount <= 0 || usd == 0 {
            continue;
        }
        if usd > remaining {
            let new_amount = if keep {
                rescale(
                    mul_div_floor(usd - remaining, WAD, price),
                    18,
                    decimals,
                    Round::Floor,
                )
                .min(amount)
            } else {
                let ratio = mul_div_floor(remaining, WAD, usd);
                let removed_wad =
                    mul_div_floor(rescale(amount, decimals, 18, Round::HalfUp), ratio, WAD);
                amount - rescale(removed_wad, 18, decimals, Round::Floor)
            };
            let new_usd = token_usd(new_amount, decimals, price);
            refunds.push((legs[i].asset_id, amount - new_amount));
            if new_amount == 0 {
                legs.remove(i);
            } else {
                legs[i].amount = new_amount;
                legs[i].usd = new_usd;
            }
            remaining = if keep && usd - new_usd < remaining {
                remaining - (usd - new_usd)
            } else {
                0
            };
        } else {
            refunds.push((legs[i].asset_id, amount));
            legs.remove(i);
            remaining -= usd;
        }
    }
}

fn seize_leg(
    c: &ExactCollateral,
    total_seizure_usd: i128,
    total_collateral: i128,
    one_plus_bonus: i128,
    seize_all: bool,
) -> Option<ExactSeizure> {
    let actual_ray = mul_div_half_up(c.scaled_ray, c.supply_index_ray, RAY);
    let share = mul_div_half_up(c.value_half_up(), WAD, total_collateral);
    let seizure_usd = mul_div_half_up(total_seizure_usd, share, WAD);
    let seizure_ray = rescale(
        mul_div_half_up(seizure_usd, WAD, c.price_wad),
        18,
        27,
        Round::HalfUp,
    );
    assert!(
        seize_all || c.decimals >= MIN_BORROWABLE_ASSET_DECIMALS || seizure_ray >= actual_ray,
        "exact reference does not model whole-unit seizure"
    );
    if seizure_ray <= 0 {
        return None;
    }
    let capped_ray = if seize_all {
        actual_ray
    } else {
        seizure_ray.min(actual_ray)
    };
    if capped_ray <= 0 {
        return None;
    }
    let full = capped_ray == actual_ray;
    let base_ray = mul_div_floor(
        seizure_ray,
        RAY,
        rescale(one_plus_bonus, 18, 27, Round::HalfUp),
    );
    let bonus_ray = (capped_ray - base_ray).max(0);
    let fee_ray = mul_div_half_up(bonus_ray, c.liq_fees_bps, BPS);
    let scaled_amount = if full {
        c.scaled_ray
    } else {
        mul_div_floor(capped_ray, RAY, c.supply_index_ray)
    };
    let bonus_scaled = mul_div_floor(bonus_ray, RAY, c.supply_index_ray).min(scaled_amount);
    if scaled_amount <= 0 {
        return None;
    }
    let round = if full { Round::HalfUp } else { Round::Floor };
    let amount = rescale(capped_ray, 27, c.decimals, round);
    if amount <= 0 {
        return None;
    }
    let held_half_up = rescale(actual_ray, 27, c.decimals, Round::HalfUp);
    let pool_gross = if amount >= held_half_up {
        rescale(
            mul_div_floor(c.scaled_ray, c.supply_index_ray, RAY),
            27,
            c.decimals,
            Round::Floor,
        )
    } else {
        amount
    };
    let fee_asset = rescale(fee_ray, 27, c.decimals, Round::Floor);
    let bumped_fee = if fee_ray > 0 && fee_asset == 0 {
        1
    } else {
        fee_asset
    };
    let paid_ray = rescale(pool_gross, c.decimals, 27, Round::HalfUp);
    let realised_excess = if paid_ray > base_ray {
        rescale(paid_ray - base_ray, 27, c.decimals, Round::Floor)
    } else {
        0
    };
    Some(ExactSeizure {
        asset_id: c.asset_id,
        amount,
        protocol_fee: bumped_fee.min(realised_excess),
        scaled_amount,
        bonus_scaled,
        credit_fee_scaled: mul_div_ceil(bonus_scaled, c.liq_fees_bps, BPS),
    })
}

pub fn plan_liquidation_exact(
    book: &ExactBook,
    payments: &[(u32, i128)],
    curve: &RefCurve,
) -> ExactPlan {
    let totals = exact_totals(&book.collateral, &book.debt);
    assert!(
        !book.debt.is_empty() && totals.health_factor < WAD,
        "account is not liquidatable"
    );

    let mut merged: Vec<(u32, i128)> = Vec::new();
    for &(asset_id, amount) in payments {
        assert!(amount > 0, "payments must be positive");
        match merged.iter_mut().find(|(id, _)| *id == asset_id) {
            Some(entry) => entry.1 += amount,
            None => merged.push((asset_id, amount)),
        }
    }

    let mut refunds = Vec::new();
    let mut legs = Vec::new();
    for (asset_id, amount) in merged {
        let d = book
            .debt
            .iter()
            .find(|d| d.asset_id == asset_id)
            .expect("payment for an asset the account does not owe");
        let balance = d.balance_ceil();
        if amount > balance {
            refunds.push((asset_id, amount - balance));
        }
        let paid = amount.min(balance);
        legs.push(Leg {
            asset_id,
            amount: paid,
            usd: token_usd(paid, d.decimals, d.price_wad),
            decimals: d.decimals,
            price_wad: d.price_wad,
        });
    }
    let offered_usd: i128 = legs.iter().map(|l| l.usd).sum();

    let (quote_usd, bonus_bps) = estimate_liquidation_amount_exact(&totals, curve);
    let insolvent = totals.total_collateral < totals.total_debt;
    assert!(
        insolvent
            || quote_usd >= totals.total_debt
            || book.collateral.len() != 1
            || book.collateral[0].decimals >= MIN_BORROWABLE_ASSET_DECIMALS,
        "exact reference does not model the whole-unit repayment"
    );
    if quote_usd < totals.total_debt && offered_usd > quote_usd {
        trim_excess(&mut legs, &mut refunds, offered_usd - quote_usd, insolvent);
    }

    let repay_usd: i128 = legs.iter().map(|l| l.usd).sum();
    let one_unit_per_leg: i128 = legs
        .iter()
        .map(|l| token_usd(1, l.decimals, l.price_wad))
        .sum();
    let seize_all = insolvent && repay_usd > 0 && repay_usd + one_unit_per_leg >= quote_usd;

    let one_plus_bonus = WAD + bps_to_wad(bonus_bps);
    let total_seizure_usd = mul_div_half_up(repay_usd, one_plus_bonus, WAD);
    let seized = if totals.total_collateral <= 0 {
        Vec::new()
    } else {
        book.collateral
            .iter()
            .filter_map(|c| {
                seize_leg(
                    c,
                    total_seizure_usd,
                    totals.total_collateral,
                    one_plus_bonus,
                    seize_all,
                )
            })
            .collect()
    };

    ExactPlan {
        totals,
        quote_usd,
        bonus_bps,
        repay_usd,
        repaid: legs.iter().map(|l| (l.asset_id, l.amount)).collect(),
        refunds,
        seize_all,
        seized,
    }
}

pub fn snapshot_exact(t: &LendingTest, account_id: u64) -> ExactBook {
    let ctrl = t.ctrl_client();
    let (supplies, borrows) = ctrl.get_account_positions(&account_id);
    let view = |key: &HubAssetKey| {
        ctrl.get_market_indexes_detailed(&soroban_vec![&t.env, key.clone()])
            .get(0)
            .expect("market index view")
    };
    let mut book = ExactBook::default();
    for (key, position) in supplies.iter() {
        let v = view(&key);
        let decimals = t.resolve_market_by_asset(&key.asset).decimals;
        let asset_id = book.intern(key);
        book.collateral.push(ExactCollateral {
            asset_id,
            scaled_ray: position.scaled_amount,
            supply_index_ray: v.supply_index,
            price_wad: v.price_wad,
            decimals,
            liq_threshold_bps: i128::from(position.liquidation_threshold),
            liq_bonus_bps: i128::from(position.liquidation_bonus),
            liq_fees_bps: i128::from(position.liquidation_fees),
        });
    }
    for (key, position) in borrows.iter() {
        let v = view(&key);
        let decimals = t.resolve_market_by_asset(&key.asset).decimals;
        let asset_id = book.intern(key);
        book.debt.push(ExactDebt {
            asset_id,
            scaled_ray: position.scaled_amount,
            borrow_index_ray: v.borrow_index,
            price_wad: v.price_wad,
            decimals,
        });
    }
    book
}

fn account_id_for(t: &LendingTest, user: &str) -> Option<u64> {
    t.find_account_id(user)
}

pub fn snapshot_collateral(t: &LendingTest, user: &str) -> Vec<RefCollateralPosition> {
    let account_id = match account_id_for(t, user) {
        Some(id) => id,
        None => return Vec::new(),
    };
    let ctrl = t.ctrl_client();
    let (supplies, _borrows) = ctrl.get_account_positions(&account_id);

    let mut out: Vec<RefCollateralPosition> = Vec::new();
    for (i, (key, position)) in supplies.iter().enumerate() {
        let market = t.resolve_market_by_asset(&key.asset);
        let sync = pool::LiquidityPoolClient::new(&t.env, &market.pool).get_sync_data(&key);
        out.push(RefCollateralPosition {
            asset_id: i as u32,
            supply_scaled_ray: br_from_i128(position.scaled_amount),
            supply_index: br_from_i128(sync.state.supply_index),
            price_wad: br_from_i128(market.price_wad),
            liq_threshold_bps: i128::from(position.liquidation_threshold),
            liq_bonus_bps: i128::from(position.liquidation_bonus),
            liq_fees_bps: i128::from(position.liquidation_fees),
            decimals: market.decimals,
        });
    }
    out
}

pub fn snapshot_debt(t: &LendingTest, user: &str) -> Vec<RefDebtPosition> {
    let account_id = match account_id_for(t, user) {
        Some(id) => id,
        None => return Vec::new(),
    };
    let ctrl = t.ctrl_client();
    let (_supplies, borrows) = ctrl.get_account_positions(&account_id);

    let mut out: Vec<RefDebtPosition> = Vec::new();
    for (i, (key, position)) in borrows.iter().enumerate() {
        let market = t.resolve_market_by_asset(&key.asset);
        let sync = pool::LiquidityPoolClient::new(&t.env, &market.pool).get_sync_data(&key);
        out.push(RefDebtPosition {
            asset_id: i as u32,
            borrow_scaled_ray: br_from_i128(position.scaled_amount),
            borrow_index: br_from_i128(sync.state.borrow_index),
            price_wad: br_from_i128(market.price_wad),
            decimals: market.decimals,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_up_div_basic() {
        assert_eq!(
            half_up_div(BigInt::from(7), BigInt::from(2)),
            BigInt::from(4)
        );
        assert_eq!(
            half_up_div(BigInt::from(-7), BigInt::from(2)),
            BigInt::from(-4)
        );
        assert_eq!(
            half_up_div(BigInt::from(5), BigInt::from(10)),
            BigInt::from(1)
        );
        assert_eq!(
            half_up_div(BigInt::from(4), BigInt::from(10)),
            BigInt::from(0)
        );
    }

    #[test]
    fn bonus_formula_baseline() {
        let hf = br_from_i128(WAD);
        let curve = RefCurve::default();
        let target = br_from_i128(curve.target_hf_wad);
        let base = br_from_i128(500);
        let max = br_from_i128(1500);
        let bonus = calculate_linear_bonus_with_target(&hf, &base, &max, &target, &curve);

        let expected = br_from_i128(500)
            + (br_from_i128(1000) * (&target - &br_from_i128(WAD))
                / (&target - &br_from_i128(curve.hf_for_max_bonus_wad)));
        assert_eq!(bonus, expected);
    }
}
