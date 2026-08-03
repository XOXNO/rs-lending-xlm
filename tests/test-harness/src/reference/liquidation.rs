#![cfg(feature = "reference-math")]

extern crate std;

use std::vec::Vec;

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;
use num_traits::{Signed, ToPrimitive, Zero};

use controller::constants::{BPS, RAY, WAD};

use crate::context::LendingTest;
use crate::helpers::{hub_asset, HARNESS_SPOKE};

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

    pub requires_full_close: bool,

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
) -> BigRational {
    if hf_wad >= target_wad {
        return base_bps.clone();
    }
    let knee = hf_for_max_bonus_wad();
    let gap = target_wad - hf_wad;
    let span = target_wad - &knee;
    let ratio = &gap / &span;
    let scale = if ratio > br_one() { br_one() } else { ratio };
    let bonus_range = max_bps - base_bps;
    base_bps + &bonus_range * &scale
}

fn hf_for_max_bonus_wad() -> BigRational {
    &wad_scale() * BigRational::from_integer(BigInt::from(80))
        / BigRational::from_integer(BigInt::from(100))
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
    base_bonus_bps: &BigRational,
    max_bonus_bps: &BigRational,
    proportion_seized: &BigRational,
    total_collateral_wad: &BigRational,
) -> (BigRational, BigRational) {
    let target = &wad_scale() * BigRational::from_integer(BigInt::from(110))
        / BigRational::from_integer(BigInt::from(100));

    let scaled_bonus =
        calculate_linear_bonus_with_target(hf_wad, base_bonus_bps, max_bonus_bps, &target);

    let bonus = match max_hf_preserving_bonus_bps(hf_wad, proportion_seized) {
        None => scaled_bonus,
        Some(cap) if scaled_bonus <= cap => scaled_bonus,
        Some(cap) if &cap >= base_bonus_bps => cap,
        Some(_) => return (total_debt_wad.clone(), base_bonus_bps.clone()),
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
    base_bonus_bps: &BigRational,
    max_bonus_bps: &BigRational,
    proportion_seized: &BigRational,
    total_collateral_wad: &BigRational,
) -> (BigRational, BigRational) {
    let (ideal, bonus) = select_liquidation_tier(
        total_debt_wad,
        weighted_coll_wad,
        hf_wad,
        base_bonus_bps,
        max_bonus_bps,
        proportion_seized,
        total_collateral_wad,
    );

    let remaining = total_debt_wad - &ideal;
    let floor = &wad_scale() * BigRational::from_integer(BigInt::from(5));
    if remaining > br_zero() && remaining < floor {
        return (total_debt_wad.clone(), bonus);
    }

    (ideal, bonus)
}

pub fn compute_liquidation(
    collateral: &[RefCollateralPosition],
    debt: &[RefDebtPosition],
    debt_payments: &[(u32, BigRational)],
    _target_hf_wad: BigRational,
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
        &base_bonus_bps,
        &max_bonus_bps,
        &proportion_seized,
        &total_coll,
    );

    let final_repayment_usd = if total_payment_usd < ideal_repayment {
        total_payment_usd.clone()
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
            let capped = if seizure_tokens > actual_tokens {
                actual_tokens
            } else {
                seizure_tokens
            };

            let base_amount = &capped * &wad_scale() / &one_plus_bonus_wad;
            let bonus_portion = &capped - &base_amount;
            let fee = &bonus_portion * br_from_i128(c.liq_fees_bps) / bps_scale();
            seized.push((c.asset_id, capped));
            fees.push((c.asset_id, fee));
        }
    }

    let repaid_per_debt: Vec<(u32, BigRational)> = per_debt_payments_usd
        .iter()
        .map(|(id, tokens, _dec)| (*id, tokens.clone()))
        .collect();

    let requires_full_close = match max_hf_preserving_bonus_bps(&hf_wad, &proportion_seized) {
        Some(cap) => cap >= BigRational::from_integer(BigInt::from(0)) && cap < base_bonus_bps,
        None => false,
    };

    RefLiquidationResult {
        health_factor_pre_wad: hf_wad,
        final_bonus_bps: bonus_bps,
        requires_full_close,
        seized_per_collateral: seized,
        repaid_per_debt,
        protocol_fee_per_collateral: fees,
        total_repaid_usd_wad: final_repayment_usd,
        total_seized_usd_wad: total_seizure_usd,
    }
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
        let asset = key.asset;
        let market = t.resolve_market_by_asset(&asset);
        let sync = pool::LiquidityPoolClient::new(&t.env, &market.pool)
            .get_sync_data(&hub_asset(asset.clone()));

        let liq_fees_bps = t
            .ctrl_client()
            .get_spoke_asset(&HARNESS_SPOKE, &hub_asset(asset.clone()))
            .liquidation_fees;
        out.push(RefCollateralPosition {
            asset_id: i as u32,
            supply_scaled_ray: br_from_i128(position.scaled_amount),
            supply_index: br_from_i128(sync.state.supply_index),
            price_wad: br_from_i128(market.price_wad),
            liq_threshold_bps: i128::from(position.liquidation_threshold),
            liq_bonus_bps: i128::from(position.liquidation_bonus),
            liq_fees_bps: i128::from(liq_fees_bps),
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
        let asset = key.asset;
        let market = t.resolve_market_by_asset(&asset);
        let sync =
            pool::LiquidityPoolClient::new(&t.env, &market.pool).get_sync_data(&hub_asset(asset));
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
        let target = &wad_scale() * BigRational::from_integer(BigInt::from(110))
            / BigRational::from_integer(BigInt::from(100));
        let base = br_from_i128(500);
        let max = br_from_i128(1500);
        let bonus = calculate_linear_bonus_with_target(&hf, &base, &max, &target);

        let expected = br_from_i128(500)
            + (br_from_i128(1000) * (&target - &br_from_i128(WAD))
                / (&target - &hf_for_max_bonus_wad()));
        assert_eq!(bonus, expected);
    }
}
