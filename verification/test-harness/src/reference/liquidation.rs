//! Exact-arithmetic reference implementation of the liquidation math.
//!
//! Mirrors the production chain in `controller/src/positions/liquidation.rs`
//! and `controller/src/helpers/mod.rs`, but uses `num_rational::BigRational`
//! so that rounding is never applied. Differential tests compare the
//! production output (chain of `mul_div_half_up` / `rescale_half_up` ops) to
//! the reference output (exact rationals, rounded only at final conversion
//! back to the protocol's precision).
//!
//! **Intentionally scoped narrowly:**
//! - Pre-liquidation HF
//! - Dynamic bonus (with 1.02 primary / 1.01 fallback target)
//! - Ideal repayment solver
//! - Proportional seizure across all collateral
//! - Protocol fee split
//!
//! **Not modeled here** (see plan "Scope boundary"):
//! - Bad-debt socialization (cross-pool index writes)
//! - Rate accrual / compound interest
//! - Isolation debt ceiling writes

extern crate std;

use std::vec::Vec;

use num_bigint::{BigInt, Sign};
use num_rational::BigRational;
use num_traits::{Signed, ToPrimitive, Zero};

use common::constants::{BPS, MAX_LIQUIDATION_BONUS, RAY, WAD};
use common::types::{POSITION_TYPE_BORROW, POSITION_TYPE_DEPOSIT};

use crate::context::LendingTest;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct RefCollateralPosition {
    /// Stable identifier mapping reference outputs back to asset snapshots.
    pub asset_id: u32,
    pub supply_scaled_ray: BigRational,
    pub supply_index_ray: BigRational,
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
    pub borrow_index_ray: BigRational,
    pub price_wad: BigRational,
    pub decimals: u32,
}

#[derive(Clone, Debug)]
pub struct RefLiquidationResult {
    /// Health factor before liquidation, in WAD scale (i.e. 1.0 = 10^18).
    pub health_factor_pre_wad: BigRational,
    /// Final bonus applied, in BPS scale (i.e. 5% = 500).
    pub final_bonus_bps: BigRational,
    /// Seized collateral per asset in *actual token units* (post-rescale).
    pub seized_per_collateral: Vec<(u32, BigRational)>,
    /// Actual debt repaid per asset in *actual token units*.
    pub repaid_per_debt: Vec<(u32, BigRational)>,
    /// Protocol fee slice of each seizure (token units).
    pub protocol_fee_per_collateral: Vec<(u32, BigRational)>,
    /// Total debt repaid in USD WAD.
    pub total_repaid_usd_wad: BigRational,
    /// Total collateral seized in USD WAD.
    pub total_seized_usd_wad: BigRational,
}

// ---------------------------------------------------------------------------
// Small constant helpers
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Rounding helpers
// ---------------------------------------------------------------------------

/// Half-up division of two BigInts (rounds .5 away from zero). Returns the
/// quotient.
pub fn half_up_div(num: BigInt, denom: BigInt) -> BigInt {
    assert!(!denom.is_zero(), "half_up_div: zero denominator");
    let denom_abs = denom.clone().abs();
    let half = &denom_abs / 2;
    // Sign of the final quotient.
    let neg = num.is_negative() ^ denom.is_negative();
    let num_abs = num.abs();
    let adjusted = num_abs + half;
    let mut q: BigInt = adjusted / denom_abs;
    if neg && !q.is_zero() {
        q = -q;
    }
    q
}

/// Convert a BigRational to an i128 using half-up rounding (half away from
/// zero). Saturates on overflow (never panics) so a reference overflow cannot
/// mask a production comparison failure.
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

/// Convert a BigRational WAD value (so `1.0 = 10^18`) to an i128.
pub fn bigrational_to_i128_wad(x: &BigRational) -> i128 {
    bigrational_to_i128_half_up(x)
}

/// Interpret an f64 amount at `decimals` precision as an exact BigRational
/// of token units. Uses the f64 representation as input (not exact bits) --
/// this is what the harness does when it calls `try_liquidate(..., amount_f64)`.
pub fn float_to_bigrational(x: f64, decimals: u32) -> BigRational {
    // Route through the same i128 conversion the harness uses to avoid
    // introducing artificial drift.
    let raw = (x * 10f64.powi(decimals as i32)) as i128;
    br_from_i128(raw)
}

// ---------------------------------------------------------------------------
// Internal math helpers (exact rationals, no rounding)
// ---------------------------------------------------------------------------

/// `position_value` in exact rationals, output scale = WAD.
/// Matches `helpers::position_value`: actual = scaled * index / RAY, then
/// multiplied by price (WAD).
fn position_value_wad(
    scaled_ray: &BigRational,
    index_ray: &BigRational,
    price_wad: &BigRational,
) -> BigRational {
    // actual_ray = scaled_ray * index_ray / RAY
    let actual_ray = scaled_ray * index_ray / ray_scale();
    // actual_wad = actual_ray / 10^9 (RAY 27 decimals -> WAD 18 decimals)
    let actual_wad = &actual_ray / br_ten_pow(9);
    // value_wad = actual_wad * price_wad / WAD
    actual_wad * price_wad / wad_scale()
}

fn compute_hf_wad(supplies: &[RefCollateralPosition], debts: &[RefDebtPosition]) -> BigRational {
    if debts.is_empty() {
        // Sentinel for non-liquidatable debt-free accounts.
        return BigRational::from_integer(BigInt::from(i128::MAX));
    }

    let mut weighted = br_zero();
    for c in supplies {
        let value = position_value_wad(&c.supply_scaled_ray, &c.supply_index_ray, &c.price_wad);
        // weighted = value * threshold_bps / BPS
        let w = &value * br_from_i128(c.liq_threshold_bps) / bps_scale();
        weighted += w;
    }

    let mut total_debt = br_zero();
    for d in debts {
        let v = position_value_wad(&d.borrow_scaled_ray, &d.borrow_index_ray, &d.price_wad);
        total_debt += v;
    }

    if total_debt.is_zero() {
        return BigRational::from_integer(BigInt::from(i128::MAX));
    }

    // HF = weighted / total_debt, scaled to WAD (so 1.0 -> WAD).
    weighted * wad_scale() / total_debt
}

fn weighted_collateral_total(supplies: &[RefCollateralPosition]) -> BigRational {
    let mut w = br_zero();
    for c in supplies {
        let value = position_value_wad(&c.supply_scaled_ray, &c.supply_index_ray, &c.price_wad);
        w += &value * br_from_i128(c.liq_threshold_bps) / bps_scale();
    }
    w
}

fn total_collateral_wad(supplies: &[RefCollateralPosition]) -> BigRational {
    let mut t = br_zero();
    for c in supplies {
        t += position_value_wad(&c.supply_scaled_ray, &c.supply_index_ray, &c.price_wad);
    }
    t
}

fn total_debt_wad(debts: &[RefDebtPosition]) -> BigRational {
    let mut t = br_zero();
    for d in debts {
        t += position_value_wad(&d.borrow_scaled_ray, &d.borrow_index_ray, &d.price_wad);
    }
    t
}

/// Average bonus params: weighted avg of per-asset bonus_bps by value share.
/// Returns (base_bonus_bps, max_bonus_bps), both as rationals in BPS scale.
fn get_account_bonus_params(supplies: &[RefCollateralPosition]) -> (BigRational, BigRational) {
    let total = total_collateral_wad(supplies);
    if total.is_zero() {
        return (br_zero(), br_from_i128(MAX_LIQUIDATION_BONUS));
    }

    let mut weighted_bonus = br_zero();
    for c in supplies {
        let value = position_value_wad(&c.supply_scaled_ray, &c.supply_index_ray, &c.price_wad);
        let share = &value / &total;
        weighted_bonus += share * br_from_i128(c.liq_bonus_bps);
    }

    (weighted_bonus, br_from_i128(MAX_LIQUIDATION_BONUS))
}

/// Dynamic bonus with a given target HF (in WAD scale).
///
/// `hf_wad`, `target_wad` are BigRational in WAD scale (i.e. 1.0 -> 10^18).
/// Output is in BPS scale (0..=10000).
fn calculate_linear_bonus_with_target(
    hf_wad: &BigRational,
    base_bps: &BigRational,
    max_bps: &BigRational,
    target_wad: &BigRational,
) -> BigRational {
    if hf_wad >= target_wad {
        return base_bps.clone();
    }
    let gap_numer = target_wad - hf_wad;
    let gap = &gap_numer / target_wad;
    let double_gap = &gap * BigRational::from_integer(BigInt::from(2));
    let scale = if double_gap > br_one() {
        br_one()
    } else {
        double_gap
    };
    let bonus_range = max_bps - base_bps;
    let bonus = base_bps + &bonus_range * &scale;
    let cap = br_from_i128(MAX_LIQUIDATION_BONUS);
    if bonus > cap {
        cap
    } else {
        bonus
    }
}

/// Returns the ideal debt-to-repay (in WAD USD) for a given bonus/target,
/// or None if the target isn't reachable (denominator <= 0).
fn try_liquidation_at_target(
    total_debt_wad: &BigRational,
    weighted_coll_wad: &BigRational,
    bonus_bps: &BigRational,
    proportion_seized: &BigRational,
    total_collateral_wad: &BigRational,
    target_wad: &BigRational,
) -> Option<BigRational> {
    // bonus_wad = bonus_bps * WAD / BPS
    let bonus_wad = bonus_bps * &wad_scale() / bps_scale();
    let one_plus_bonus = &wad_scale() + &bonus_wad;

    // d_max = total_collateral / one_plus_bonus (both WAD scale; result is WAD)
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
    // min(d_ideal, d_max, total_debt)
    let mut out = d_ideal;
    if out > d_max {
        out = d_max;
    }
    if out > *total_debt_wad {
        out = total_debt_wad.clone();
    }
    Some(out)
}

fn calculate_post_liquidation_hf(
    weighted_coll: &BigRational,
    total_debt: &BigRational,
    debt_to_repay: &BigRational,
    proportion_seized: &BigRational,
    bonus_bps: &BigRational,
) -> BigRational {
    // one_plus_bonus in BPS scale (= BPS + bonus)
    let one_plus_bonus_bps = &bps_scale() + bonus_bps;
    let seized_proportion = proportion_seized * debt_to_repay / wad_scale();
    let seized_weighted = &seized_proportion * &one_plus_bonus_bps / bps_scale();
    let seized_weighted = if seized_weighted > *weighted_coll {
        weighted_coll.clone()
    } else {
        seized_weighted
    };
    let new_weighted = weighted_coll - &seized_weighted;
    let new_debt = if debt_to_repay >= total_debt {
        br_zero()
    } else {
        total_debt - debt_to_repay
    };
    if new_debt.is_zero() {
        return BigRational::from_integer(BigInt::from(i128::MAX));
    }
    new_weighted * wad_scale() / new_debt
}

/// Mirror of `estimate_liquidation_amount`.
/// Returns (ideal_repayment_wad, bonus_bps).
fn estimate_liquidation_amount(
    total_debt_wad: &BigRational,
    weighted_coll_wad: &BigRational,
    hf_wad: &BigRational,
    base_bonus_bps: &BigRational,
    max_bonus_bps: &BigRational,
    proportion_seized: &BigRational,
    total_collateral_wad: &BigRational,
) -> (BigRational, BigRational) {
    let target_primary = &wad_scale() * BigRational::from_integer(BigInt::from(102))
        / BigRational::from_integer(BigInt::from(100));

    let bonus_primary =
        calculate_linear_bonus_with_target(hf_wad, base_bonus_bps, max_bonus_bps, &target_primary);

    if let Some(d) = try_liquidation_at_target(
        total_debt_wad,
        weighted_coll_wad,
        &bonus_primary,
        proportion_seized,
        total_collateral_wad,
        &target_primary,
    ) {
        let new_hf = calculate_post_liquidation_hf(
            weighted_coll_wad,
            total_debt_wad,
            &d,
            proportion_seized,
            &bonus_primary,
        );
        if new_hf >= wad_scale() {
            return (d, bonus_primary);
        }
    }

    let target_fallback = &wad_scale() * BigRational::from_integer(BigInt::from(101))
        / BigRational::from_integer(BigInt::from(100));
    let bonus_fallback =
        calculate_linear_bonus_with_target(hf_wad, base_bonus_bps, max_bonus_bps, &target_fallback);
    let fallback_result = try_liquidation_at_target(
        total_debt_wad,
        weighted_coll_wad,
        &bonus_fallback,
        proportion_seized,
        total_collateral_wad,
        &target_fallback,
    );

    // Unrecoverable-position path.
    let base_bonus_wad = base_bonus_bps * &wad_scale() / bps_scale();
    let one_plus_base = &wad_scale() + &base_bonus_wad;
    let d_max = total_collateral_wad * &wad_scale() / &one_plus_base;
    let d_max = if d_max > *total_debt_wad {
        total_debt_wad.clone()
    } else {
        d_max
    };
    let base_new_hf = calculate_post_liquidation_hf(
        weighted_coll_wad,
        total_debt_wad,
        &d_max,
        proportion_seized,
        base_bonus_bps,
    );
    if base_new_hf < wad_scale() && base_new_hf < *hf_wad {
        return (d_max, base_bonus_bps.clone());
    }

    match fallback_result {
        Some(d) => (d, bonus_fallback),
        None => (d_max, base_bonus_bps.clone()),
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute liquidation outputs from a snapshot using exact rational math.
/// Takes the debt payments as *token units* (pre-scaling); the reference
/// reproduces production's conversion to USD and solves the auction.
///
/// The returned `seized_per_collateral` / `repaid_per_debt` are in *token
/// units* (asset decimals), matching what the protocol actually transfers.
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

    // proportion_seized = weighted / total (both WAD); in WAD scale.
    let proportion_seized = if total_coll.is_zero() {
        br_zero()
    } else {
        &weighted_coll * &wad_scale() / &total_coll
    };

    let (base_bonus_bps, max_bonus_bps) = get_account_bonus_params(collateral);

    // 1. Convert per-asset debt payments (token units) into USD WAD,
    //    clamped to each asset's actual debt (like production does).
    let mut total_payment_usd = br_zero();
    let mut per_debt_payments_usd: Vec<(u32, BigRational, u32)> = Vec::new();
    for (asset_id, amt_tokens) in debt_payments {
        let d = debt
            .iter()
            .find(|d| d.asset_id == *asset_id)
            .expect("debt payment references unknown asset_id");
        // Actual debt in token units:
        // actual_ray = scaled * index / RAY; token = actual_ray / 10^(27-dec)
        let actual_ray = &d.borrow_scaled_ray * &d.borrow_index_ray / ray_scale();
        let scale_diff = 27 - d.decimals;
        let actual_tokens = actual_ray / br_ten_pow(scale_diff);
        let payment_tokens = if amt_tokens > &actual_tokens {
            actual_tokens.clone()
        } else {
            amt_tokens.clone()
        };
        // payment_wad = payment_tokens * 10^(18-dec)
        let payment_wad = if d.decimals <= 18 {
            &payment_tokens * br_ten_pow(18 - d.decimals)
        } else {
            &payment_tokens / br_ten_pow(d.decimals - 18)
        };
        // payment_usd = payment_wad * price_wad / WAD
        let payment_usd = &payment_wad * &d.price_wad / wad_scale();
        total_payment_usd += &payment_usd;
        per_debt_payments_usd.push((*asset_id, payment_tokens, d.decimals));
    }

    // 2. Solve auction.
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

    // 3. Distribute seizure proportionally.
    let mut seized: Vec<(u32, BigRational)> = Vec::new();
    let mut fees: Vec<(u32, BigRational)> = Vec::new();
    if !total_coll.is_zero() {
        for c in collateral {
            if c.price_wad.is_zero() {
                continue;
            }
            let actual_ray = &c.supply_scaled_ray * &c.supply_index_ray / ray_scale();
            let actual_wad = &actual_ray / br_ten_pow(9);
            let asset_value = &actual_wad * &c.price_wad / wad_scale();
            let share = &asset_value / &total_coll;
            let seizure_usd_for_asset = &total_seizure_usd * &share;
            let seizure_wad = &seizure_usd_for_asset * &wad_scale() / &c.price_wad;
            // Convert WAD -> asset tokens.
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
            // base_amount = capped / one_plus_bonus_wad * WAD
            let base_amount = &capped * &wad_scale() / &one_plus_bonus_wad;
            let bonus_portion = &capped - &base_amount;
            let fee = &bonus_portion * br_from_i128(c.liq_fees_bps) / bps_scale();
            seized.push((c.asset_id, capped));
            fees.push((c.asset_id, fee));
        }
    }

    // 4. Record raw per-asset debt payments. Differential assertions compare
    //    aggregate USD debt reduction against production output.
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

// ---------------------------------------------------------------------------
// Snapshot helpers -- read from LendingTest views
// ---------------------------------------------------------------------------

fn account_id_for(t: &LendingTest, user: &str) -> Option<u64> {
    t.find_account_id(user)
}

/// Resolve collateral positions for a user into reference form.
/// Uses the controller's view plus per-pool sync data, so this matches the
/// same "current" indexes the liquidation path will see.
pub fn snapshot_collateral(t: &LendingTest, user: &str) -> Vec<RefCollateralPosition> {
    let account_id = match account_id_for(t, user) {
        Some(id) => id,
        None => return Vec::new(),
    };
    let ctrl = t.ctrl_client();
    let (supplies, _borrows) = ctrl.get_account_positions(&account_id);

    let mut out: Vec<RefCollateralPosition> = Vec::new();
    for (i, (asset, position)) in supplies.iter().enumerate() {
        let market = t.resolve_market_by_asset(&asset);
        let sync = pool::LiquidityPoolClient::new(&t.env, &market.pool).get_sync_data();
        out.push(RefCollateralPosition {
            asset_id: i as u32,
            supply_scaled_ray: br_from_i128(position.scaled_amount_ray),
            supply_index_ray: br_from_i128(sync.state.supply_index_ray),
            price_wad: br_from_i128(market.price_wad),
            liq_threshold_bps: i128::from(position.liquidation_threshold_bps),
            liq_bonus_bps: i128::from(position.liquidation_bonus_bps),
            liq_fees_bps: i128::from(position.liquidation_fees_bps),
            decimals: market.decimals,
        });
    }
    // Silence unused-constant lint when test code tightens.
    let _ = POSITION_TYPE_DEPOSIT;
    out
}

/// Same as `snapshot_collateral` but for debt positions.
pub fn snapshot_debt(t: &LendingTest, user: &str) -> Vec<RefDebtPosition> {
    let account_id = match account_id_for(t, user) {
        Some(id) => id,
        None => return Vec::new(),
    };
    let ctrl = t.ctrl_client();
    let (_supplies, borrows) = ctrl.get_account_positions(&account_id);

    let mut out: Vec<RefDebtPosition> = Vec::new();
    for (i, (asset, position)) in borrows.iter().enumerate() {
        let market = t.resolve_market_by_asset(&asset);
        let sync = pool::LiquidityPoolClient::new(&t.env, &market.pool).get_sync_data();
        out.push(RefDebtPosition {
            asset_id: i as u32,
            borrow_scaled_ray: br_from_i128(position.scaled_amount_ray),
            borrow_index_ray: br_from_i128(sync.state.borrow_index_ray),
            price_wad: br_from_i128(market.price_wad),
            decimals: market.decimals,
        });
    }
    let _ = POSITION_TYPE_BORROW;
    out
}

// ---------------------------------------------------------------------------
// Self-tests
// ---------------------------------------------------------------------------

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
        // HF = 1.0 WAD, target 1.02, base 500, max 1500
        let hf = br_from_i128(WAD);
        let target = &wad_scale() * BigRational::from_integer(BigInt::from(102))
            / BigRational::from_integer(BigInt::from(100));
        let base = br_from_i128(500);
        let max = br_from_i128(1500);
        let bonus = calculate_linear_bonus_with_target(&hf, &base, &max, &target);
        // gap = (1.02 - 1.0) / 1.02 = 0.0196...; 2*gap = 0.0392
        // bonus = 500 + 1000 * 0.0392 = 539.21...
        let expected = br_from_i128(500)
            + (br_from_i128(1000) * (br_from_i128(2) * (&target - &br_from_i128(WAD)) / &target));
        assert_eq!(bonus, expected);
    }
}
