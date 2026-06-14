//! Liquidation close-amount, bonus, refund, and seizure accounting.
//!
//! Price math uses USD WAD. Pool-facing seizure and repayment entries use
//! asset-native units.

use crate::constants::{BAD_DEBT_USD_THRESHOLD, BPS, WAD};
use common::errors::{CollateralError, GenericError};
use common::math::fp::{Bps, Ray, Wad};
use common::math::fp_core;
use controller_interface::types::{
    Account, AccountPositionRaw, DebtPosition, LiquidationResult, Payment, PaymentTuple,
    RepayEntry, SeizeEntry,
};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::cache::Cache;
use crate::helpers;
use crate::helpers::utils;
use crate::storage::iter_typed_positions;
use crate::validation;

/// Aggregate position metrics for liquidation helpers.
#[derive(Clone, Copy)]
pub(crate) struct LiquidationSnapshot {
    pub total_debt: Wad,
    pub total_collateral: Wad,
    pub weighted_coll: Wad,
    pub proportion_seized: Wad,
    pub hf: Wad,
}

/// Liquidation bonus interpolation bounds (base and protocol-max).
#[derive(Clone, Copy)]
pub(crate) struct BonusBounds {
    pub base: Bps,
    pub max: Bps,
}

/// Repayment legs after every close-amount, excess-refund, and dust-residue cap.
///
/// Seizure math must use `repay_usd`, not the liquidator's original payment
/// amount or the pre-dust-cap close amount.
pub(crate) struct NormalizedRepaymentPlan {
    pub repaid: Vec<RepayEntry>,
    pub refunds: Vec<PaymentTuple>,
    pub repay_usd: Wad,
    pub bonus: Bps,
    pub requires_post_socialization: bool,
}

impl NormalizedRepaymentPlan {
    fn validate(&self, env: &Env) {
        if sum_repaid_usd(env, &self.repaid) != self.repay_usd {
            panic_with_error!(env, GenericError::InternalError);
        }
    }
}

/// Fully-priced liquidation plan. This is the handoff object from pure
/// liquidation math to stateful pool execution.
pub(crate) struct LiquidationPlan {
    pub repayment: NormalizedRepaymentPlan,
    pub seized: Vec<SeizeEntry>,
}

impl LiquidationPlan {
    pub(crate) fn validate(&self, env: &Env) {
        self.repayment.validate(env);

        for entry in self.seized.iter() {
            if entry.amount <= 0 || entry.protocol_fee < 0 || entry.protocol_fee > entry.amount {
                panic_with_error!(env, GenericError::InternalError);
            }
        }
    }

    pub(crate) fn into_result(self) -> LiquidationResult {
        LiquidationResult {
            seized: self.seized,
            repaid: self.repayment.repaid,
            refunds: self.repayment.refunds,
            max_debt_usd: self.repayment.repay_usd.raw(),
            bonus_bps: self.repayment.bonus.raw(),
        }
    }

    pub(crate) fn requires_post_socialization(&self) -> bool {
        self.repayment.requires_post_socialization
    }
}

/// True when collateral is small enough for bad-debt socialization.
pub(crate) fn is_socializable_bad_debt(total_debt: Wad, total_collateral: Wad) -> bool {
    total_debt > total_collateral && total_collateral <= Wad::from(BAD_DEBT_USD_THRESHOLD)
}

pub(crate) fn calculate_seizure_proportions(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    weighted_coll: Wad,
    cache: &mut Cache,
) -> (Wad, BonusBounds) {
    let proportion_seized = if total_collateral > Wad::ZERO {
        weighted_coll.div(env, total_collateral)
    } else {
        Wad::ZERO
    };

    let bounds = get_account_bonus_params(env, cache, &account.supply_positions, proportion_seized);

    (proportion_seized, bounds)
}

pub(crate) fn calculate_repayment_amounts(
    env: &Env,
    raw_payments: &Vec<Payment>,
    account: &Account,
    refunds: &mut Vec<PaymentTuple>,
    cache: &mut Cache,
) -> (Wad, Vec<RepayEntry>, bool) {
    let mut total_repaid_usd = Wad::ZERO;
    let mut repaid_tokens: Vec<RepayEntry> = Vec::new(env);

    let merged = utils::aggregate_positive_payments(env, raw_payments);

    // Full coverage is decided in token terms — the only quantization the
    // pool settles in: the deduped plan reaches every debt position (plan
    // assets must exist as positions, so length equality is set equality)
    // and every payment settles its position's full token debt.
    let mut covers_full_debt = merged.len() == account.borrow_positions.len();

    for i in 0..merged.len() {
        let (asset, amount) = validation::expect_invariant(env, merged.get(i));
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let position: DebtPosition = (&account
            .borrow_positions
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound)))
            .into();

        let actual_debt = debt_close_amount(
            env,
            &position,
            market_index.borrow_index,
            feed.asset_decimals,
        );

        let mut payment_amount = amount;
        if payment_amount > actual_debt {
            let excess = payment_amount - actual_debt;
            refunds.push_back(PaymentTuple {
                asset: asset.clone(),
                amount: excess,
            });
            payment_amount = actual_debt;
        } else if payment_amount < actual_debt {
            covers_full_debt = false;
        }

        let payment_usd = feed.usd_value_wad(env, payment_amount);

        total_repaid_usd += payment_usd;
        repaid_tokens.push_back(RepayEntry {
            asset,
            amount: payment_amount,
            usd_wad: payment_usd.raw(),
            feed: (&feed).into(),
            market_index: (&market_index).into(),
        });
    }

    (total_repaid_usd, repaid_tokens, covers_full_debt)
}

pub(crate) fn normalize_repayment_plan(
    env: &Env,
    account: &Account,
    raw_payments: &Vec<Payment>,
    snap: &LiquidationSnapshot,
    bonus_bounds: BonusBounds,
    cache: &mut Cache,
) -> NormalizedRepaymentPlan {
    let mut refunds = Vec::new(env);
    let (total_debt_payment_usd, repaid_tokens, _) =
        calculate_repayment_amounts(env, raw_payments, account, &mut refunds, cache);

    let (max_debt_to_repay_usd, bonus) =
        calculate_liquidation_amounts(env, snap, bonus_bounds, total_debt_payment_usd);

    let protected_minimums = protected_full_close_minimums(env, account, cache, &repaid_tokens);

    let mut final_repayment_tokens = repaid_tokens;
    if total_debt_payment_usd > max_debt_to_repay_usd {
        let excess_usd = total_debt_payment_usd - max_debt_to_repay_usd;
        process_excess_payment(
            env,
            &mut final_repayment_tokens,
            &mut refunds,
            excess_usd,
            &protected_minimums,
        );
    }

    let mut repayment = NormalizedRepaymentPlan {
        repay_usd: sum_repaid_usd(env, &final_repayment_tokens),
        repaid: final_repayment_tokens,
        refunds,
        bonus,
        requires_post_socialization: false,
    };
    repayment.validate(env);

    if repayment_has_wholly_subfloor_underpayment(env, account, cache, &repayment.repaid)
        && repayment_leaves_socializable_bad_debt(env, account, snap, &repayment, cache)
    {
        repayment.requires_post_socialization = true;
        return repayment;
    }

    cap_repaid_tokens_to_debt_floors(
        env,
        account,
        cache,
        &mut repayment.repaid,
        &mut repayment.refunds,
    );

    repayment.repay_usd = sum_repaid_usd(env, &repayment.repaid);
    repayment.validate(env);
    repayment
}

fn repayment_has_wholly_subfloor_underpayment(
    env: &Env,
    account: &Account,
    cache: &mut Cache,
    repaid_tokens: &Vec<RepayEntry>,
) -> bool {
    for i in 0..repaid_tokens.len() {
        let entry = validation::expect_invariant(env, repaid_tokens.get(i));
        let debt_position: DebtPosition = (&account
            .borrow_positions
            .get(entry.asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound)))
            .into();
        let market_index: common::types::pool::MarketIndex = (&entry.market_index).into();
        let actual_debt = debt_close_amount(
            env,
            &debt_position,
            market_index.borrow_index,
            entry.feed.asset_decimals,
        );
        if entry.amount >= actual_debt {
            continue;
        }

        let min_debt_floor = cache
            .cached_asset_config(&entry.asset)
            .min_debt_floor_usd
            .raw();
        if min_debt_floor <= 0 {
            continue;
        }

        let min_residual = min_residual_token_amount(env, min_debt_floor, &entry.feed);
        if actual_debt <= min_residual {
            return true;
        }
    }

    false
}

fn repayment_leaves_socializable_bad_debt(
    env: &Env,
    account: &Account,
    snap: &LiquidationSnapshot,
    repayment: &NormalizedRepaymentPlan,
    cache: &mut Cache,
) -> bool {
    if repayment.repaid.is_empty() {
        return false;
    }

    let seized = calculate_seized_collateral(env, account, snap.total_collateral, repayment, cache);
    let seized_usd = sum_seized_usd(env, &seized).min(snap.total_collateral);
    let post_collateral = snap.total_collateral - seized_usd;
    let conservative_post_collateral =
        post_collateral + socialization_rounding_buffer(env, &seized);
    let post_debt = if repayment.repay_usd >= snap.total_debt {
        Wad::ZERO
    } else {
        snap.total_debt - repayment.repay_usd
    };

    is_socializable_bad_debt(post_debt, conservative_post_collateral)
}

fn sum_seized_usd(env: &Env, seized: &Vec<SeizeEntry>) -> Wad {
    let mut total = Wad::ZERO;
    for i in 0..seized.len() {
        let entry = validation::expect_invariant(env, seized.get(i));
        let feed: controller_interface::types::PriceFeed = (&entry.feed).into();
        total += feed.usd_value_wad(env, entry.amount);
    }
    total
}

fn socialization_rounding_buffer(env: &Env, seized: &Vec<SeizeEntry>) -> Wad {
    let mut buffer = Wad::from(1);
    for i in 0..seized.len() {
        let entry = validation::expect_invariant(env, seized.get(i));
        let feed: controller_interface::types::PriceFeed = (&entry.feed).into();
        let one_unit_usd = feed.usd_value_wad(env, 1);
        buffer += if one_unit_usd > Wad::ZERO {
            one_unit_usd
        } else {
            Wad::from(1)
        };
    }
    buffer
}

fn protected_full_close_minimums(
    env: &Env,
    account: &Account,
    cache: &mut Cache,
    repaid_tokens: &Vec<RepayEntry>,
) -> Vec<PaymentTuple> {
    let mut protected: Vec<PaymentTuple> = Vec::new(env);

    for i in 0..repaid_tokens.len() {
        let entry = validation::expect_invariant(env, repaid_tokens.get(i));
        let debt_position: DebtPosition = (&account
            .borrow_positions
            .get(entry.asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound)))
            .into();
        let market_index: common::types::pool::MarketIndex = (&entry.market_index).into();
        let actual_debt = debt_close_amount(
            env,
            &debt_position,
            market_index.borrow_index,
            entry.feed.asset_decimals,
        );

        if entry.amount < actual_debt {
            continue;
        }

        let min_debt_floor = cache
            .cached_asset_config(&entry.asset)
            .min_debt_floor_usd
            .raw();
        if min_debt_floor <= 0 {
            continue;
        }

        let min_residual = min_residual_token_amount(env, min_debt_floor, &entry.feed);
        if actual_debt <= min_residual {
            protected.push_back(PaymentTuple {
                asset: entry.asset,
                amount: actual_debt,
            });
        }
    }

    protected
}

pub(crate) fn cap_repaid_tokens_to_debt_floors(
    env: &Env,
    account: &Account,
    cache: &mut Cache,
    repaid_tokens: &mut Vec<RepayEntry>,
    refunds: &mut Vec<PaymentTuple>,
) {
    let mut i = 0;
    while i < repaid_tokens.len() {
        let entry = validation::expect_invariant(env, repaid_tokens.get(i));
        let debt_position: DebtPosition = (&account
            .borrow_positions
            .get(entry.asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound)))
            .into();
        let market_index: common::types::pool::MarketIndex = (&entry.market_index).into();
        let actual_debt = debt_close_amount(
            env,
            &debt_position,
            market_index.borrow_index,
            entry.feed.asset_decimals,
        );

        if entry.amount >= actual_debt {
            i += 1;
            continue;
        }

        let min_debt_floor = cache
            .cached_asset_config(&entry.asset)
            .min_debt_floor_usd
            .raw();
        if min_debt_floor <= 0 {
            i += 1;
            continue;
        }

        let feed: controller_interface::types::PriceFeed = (&entry.feed).into();
        let residual = actual_debt - entry.amount;
        let residual_usd = feed.usd_value_wad(env, residual).raw();
        if residual_usd >= min_debt_floor {
            i += 1;
            continue;
        }

        let min_residual = min_residual_token_amount(env, min_debt_floor, &entry.feed);
        let capped_amount = actual_debt.saturating_sub(min_residual);
        if capped_amount <= 0 {
            // A fully covered wholly-sub-floor position is protected before the
            // excess-refund pass and reaches the full-close branch above. If we
            // get here, the liquidator did not cover enough to close the debt,
            // so the only dust-safe mutation for this leg is no mutation.
            refunds.push_back(PaymentTuple {
                asset: entry.asset,
                amount: entry.amount,
            });
            repaid_tokens.remove(i);
            continue;
        }

        if capped_amount < entry.amount {
            let refund_amount = entry.amount - capped_amount;
            let capped_usd = feed.usd_value_wad(env, capped_amount);
            refunds.push_back(PaymentTuple {
                asset: entry.asset.clone(),
                amount: refund_amount,
            });
            repaid_tokens.set(
                i,
                RepayEntry {
                    asset: entry.asset,
                    amount: capped_amount,
                    usd_wad: capped_usd.raw(),
                    feed: entry.feed,
                    market_index: entry.market_index,
                },
            );
        }

        i += 1;
    }
}

fn debt_close_amount(
    env: &Env,
    position: &DebtPosition,
    borrow_index: Ray,
    asset_decimals: u32,
) -> i128 {
    position
        .scaled_amount
        .mul(env, borrow_index)
        .to_asset_ceil(asset_decimals)
}

pub(crate) fn sum_repaid_usd(env: &Env, repaid_tokens: &Vec<RepayEntry>) -> Wad {
    let mut total = Wad::ZERO;
    for i in 0..repaid_tokens.len() {
        let entry = validation::expect_invariant(env, repaid_tokens.get(i));
        total += Wad::from(entry.usd_wad);
    }
    total
}

fn min_residual_token_amount(
    env: &Env,
    min_debt_floor_usd_wad: i128,
    feed: &controller_interface::types::PriceFeedRaw,
) -> i128 {
    if feed.price_wad <= 0 {
        panic_with_error!(env, GenericError::InternalError);
    }
    let min_residual_wad = fp_core::mul_div_ceil(env, min_debt_floor_usd_wad, WAD, feed.price_wad);
    fp_core::rescale_ceil(
        min_residual_wad,
        common::constants::WAD_DECIMALS,
        feed.asset_decimals,
    )
}

pub(crate) fn calculate_liquidation_amounts(
    env: &Env,
    snap: &LiquidationSnapshot,
    bonus_bounds: BonusBounds,
    total_payment_usd: Wad,
) -> (Wad, Bps) {
    let (ideal_repayment_usd, bonus) = estimate_liquidation_amount(env, snap, bonus_bounds);

    let final_repayment_usd = total_payment_usd.min(ideal_repayment_usd);

    (final_repayment_usd, bonus)
}

pub(crate) fn calculate_seized_collateral(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    repayment: &NormalizedRepaymentPlan,
    cache: &mut Cache,
) -> Vec<SeizeEntry> {
    let mut seized: Vec<SeizeEntry> = Vec::new(env);
    if total_collateral <= Wad::ZERO {
        return seized;
    }

    let one_plus_bonus = Wad::ONE + repayment.bonus.to_wad(env);
    let total_seizure_usd = repayment.repay_usd.mul(env, one_plus_bonus);

    for (asset, position) in iter_typed_positions(&account.supply_positions) {
        let feed = cache.cached_price(&asset);
        if feed.price.raw() == 0 {
            continue;
        }

        let asset_config = cache.cached_asset_config(&asset);
        let market_index = cache.cached_market_index(&asset);

        let actual_ray = position.scaled_amount.mul(env, market_index.supply_index);
        let actual_amount_wad = actual_ray.to_wad();
        let asset_value = actual_amount_wad.mul(env, feed.price);

        let share = asset_value.div(env, total_collateral);
        let seizure_for_asset_usd = total_seizure_usd.mul(env, share);

        let seizure_amount_wad = seizure_for_asset_usd.div(env, feed.price);
        let seizure_ray = seizure_amount_wad.to_ray();

        if seizure_ray <= Ray::ZERO {
            continue;
        }

        let capped_ray = seizure_ray.min(actual_ray);
        if capped_ray <= Ray::ZERO {
            continue;
        }

        // Floor-divide so the base side is the lower bound; the bonus side
        // absorbs the rounding remainder before the protocol fee applies.
        let base_ray = capped_ray.div_floor(env, one_plus_bonus.to_ray());
        let bonus_ray = capped_ray - base_ray;
        let protocol_fee = asset_config.liquidation_fees.apply_to_ray(env, bonus_ray);
        // Full seizure must use the pool's half-up conversion: the pool
        // full-closes only when `amount >= unscale_supply(pos_scaled)`
        // (half-up), and a floored amount one unit short would leave a
        // sub-unit residue that trips the dust gate. Partial seizures floor
        // so they never exceed the computed RAY amount; fee <= amount holds
        // in both branches.
        let capped_amount = if capped_ray == actual_ray {
            capped_ray.to_asset(feed.asset_decimals)
        } else {
            capped_ray.to_asset_floor(feed.asset_decimals)
        };
        if capped_amount <= 0 {
            continue;
        }

        seized.push_back(SeizeEntry {
            asset,
            amount: capped_amount,
            protocol_fee: protocol_fee.to_asset_floor(feed.asset_decimals),
            feed: (&feed).into(),
            market_index: (&market_index).into(),
        });
    }

    seized
}

pub(crate) fn process_excess_payment(
    env: &Env,
    repaid_tokens: &mut Vec<RepayEntry>,
    refunds: &mut Vec<PaymentTuple>,
    excess_usd: Wad,
    protected_minimums: &Vec<PaymentTuple>,
) {
    let mut remaining_excess_usd = excess_usd;

    let mut current_index = repaid_tokens.len();
    while remaining_excess_usd > Wad::ZERO && current_index > 0 {
        current_index -= 1;
        let entry = validation::expect_invariant(env, repaid_tokens.get(current_index));
        let minimum_amount = protected_minimum_amount(env, protected_minimums, &entry.asset);
        if entry.amount <= minimum_amount {
            continue;
        }

        let usd = Wad::from(entry.usd_wad);
        let minimum_usd = if minimum_amount > 0 {
            let feed: controller_interface::types::PriceFeed = (&entry.feed).into();
            feed.usd_value_wad(env, minimum_amount)
        } else {
            Wad::ZERO
        };
        if usd <= minimum_usd {
            continue;
        }
        let refundable_usd = usd - minimum_usd;
        let refundable_amount = entry.amount - minimum_amount;

        if refundable_usd > remaining_excess_usd {
            // Floor every step: the refund returned to the payer never exceeds
            // the exact pro-rata share; sub-ulp remainder stays as repayment.
            let ratio = remaining_excess_usd.div_floor(env, refundable_usd);
            let refund_amount = Wad::from_token(refundable_amount, entry.feed.asset_decimals)
                .mul_floor(env, ratio)
                .to_token_floor(entry.feed.asset_decimals);

            let new_amount = entry.amount - refund_amount;
            // Recompute new_usd from new_amount * price; subtracting the excess
            // directly drifts the two precision paths and desyncs the RepayEntry pair.
            let new_amount_wad = Wad::from_token(new_amount, entry.feed.asset_decimals);
            let new_usd = new_amount_wad.mul(env, Wad::from(entry.feed.price_wad));

            refunds.push_back(PaymentTuple {
                asset: entry.asset.clone(),
                amount: refund_amount,
            });
            repaid_tokens.set(
                current_index,
                RepayEntry {
                    asset: entry.asset,
                    amount: new_amount,
                    usd_wad: new_usd.raw(),
                    feed: entry.feed,
                    market_index: entry.market_index,
                },
            );
            remaining_excess_usd = Wad::ZERO;
        } else {
            refunds.push_back(PaymentTuple {
                asset: entry.asset.clone(),
                amount: refundable_amount,
            });
            if minimum_amount > 0 {
                let new_amount_wad = Wad::from_token(minimum_amount, entry.feed.asset_decimals);
                let new_usd = new_amount_wad.mul(env, Wad::from(entry.feed.price_wad));
                repaid_tokens.set(
                    current_index,
                    RepayEntry {
                        asset: entry.asset,
                        amount: minimum_amount,
                        usd_wad: new_usd.raw(),
                        feed: entry.feed,
                        market_index: entry.market_index,
                    },
                );
            } else {
                repaid_tokens.remove(current_index);
            }
            remaining_excess_usd -= refundable_usd;
        }
    }
}

fn protected_minimum_amount(
    env: &Env,
    protected_minimums: &Vec<PaymentTuple>,
    asset: &Address,
) -> i128 {
    for i in 0..protected_minimums.len() {
        let protected = validation::expect_invariant(env, protected_minimums.get(i));
        if protected.asset == *asset {
            return protected.amount;
        }
    }
    0
}

/// Interpolates liquidation bonus from base to max as HF falls below target.
pub fn calculate_linear_bonus_with_target(
    env: &Env,
    hf: Wad,
    base: Bps,
    max: Bps,
    target: Wad,
) -> Bps {
    if hf >= target {
        return base;
    }
    let gap_numerator = target - hf;
    let gap_wad = gap_numerator.div(env, target);

    let double_gap = gap_wad.mul(env, Wad::from(2 * WAD));
    let scale = double_gap.min(Wad::ONE);

    let bonus_range = max - base;
    let bonus_increment = Wad::from(bonus_range.raw()).mul(env, scale).raw();
    Bps::from(
        base.raw()
            .checked_add(bonus_increment)
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow)),
    )
}

/// Estimates repayment and bonus using a 1.02 HF target, then 1.01 fallback,
/// then max-collateral seizure at base bonus without worsening account HF.
pub(crate) fn estimate_liquidation_amount(
    env: &Env,
    snap: &LiquidationSnapshot,
    bounds: BonusBounds,
) -> (Wad, Bps) {
    let target_primary = Wad::from(1_020_000_000_000_000_000i128);
    let bonus_primary =
        calculate_linear_bonus_with_target(env, snap.hf, bounds.base, bounds.max, target_primary);
    if let Some(d) = try_liquidation_at_target(env, snap, bonus_primary, target_primary) {
        let new_hf = calculate_post_liquidation_hf(env, snap, d, bonus_primary);
        if new_hf >= Wad::ONE {
            return (d, bonus_primary);
        }
    }

    let target_fallback = Wad::from(WAD + WAD / 100);
    let bonus_fallback =
        calculate_linear_bonus_with_target(env, snap.hf, bounds.base, bounds.max, target_fallback);
    let fallback_result = try_liquidation_at_target(env, snap, bonus_fallback, target_fallback);

    let base_bonus_wad = bounds.base.to_wad(env);
    let one_plus_base = Wad::ONE + base_bonus_wad;
    let d_max = snap
        .total_collateral
        .div(env, one_plus_base)
        .min(snap.total_debt);

    let base_new_hf = calculate_post_liquidation_hf(env, snap, d_max, bounds.base);

    if base_new_hf < Wad::ONE && base_new_hf < snap.hf {
        return (d_max, bounds.base);
    }

    match fallback_result {
        Some(d) => (d, bonus_fallback),
        None => (d_max, bounds.base),
    }
}

fn calculate_post_liquidation_hf(
    env: &Env,
    snap: &LiquidationSnapshot,
    debt_to_repay: Wad,
    bonus: Bps,
) -> Wad {
    let one_plus_bonus = Bps::ONE + bonus;

    let seized_proportion = snap.proportion_seized.mul(env, debt_to_repay);
    let seized_weighted_raw = one_plus_bonus.apply_to(env, seized_proportion.raw());
    let seized_weighted = Wad::from(seized_weighted_raw).min(snap.weighted_coll);

    let new_weighted = snap.weighted_coll - seized_weighted;
    let new_debt = if debt_to_repay >= snap.total_debt {
        Wad::ZERO
    } else {
        snap.total_debt - debt_to_repay
    };

    if new_debt == Wad::ZERO {
        return Wad::from(i128::MAX);
    }
    new_weighted.div(env, new_debt)
}

fn try_liquidation_at_target(
    env: &Env,
    snap: &LiquidationSnapshot,
    bonus: Bps,
    target_hf: Wad,
) -> Option<Wad> {
    let bonus_wad = bonus.to_wad(env);
    let one_plus_bonus = Wad::ONE + bonus_wad;

    let d_max = snap.total_collateral.div(env, one_plus_bonus);

    let denom_term = snap.proportion_seized.mul(env, one_plus_bonus);
    if target_hf <= denom_term {
        return None;
    }
    let denominator = target_hf - denom_term;

    let target_debt = target_hf.mul(env, snap.total_debt);
    if target_debt <= snap.weighted_coll {
        return Some(d_max.min(snap.total_debt));
    }
    let numerator = target_debt - snap.weighted_coll;
    let d_ideal = numerator.div(env, denominator);

    Some(d_ideal.min(d_max).min(snap.total_debt))
}

/// Largest liquidation bonus that keeps seizure below account collateral.
pub(crate) fn max_bonus_for_threshold(env: &Env, proportion_seized: Wad) -> Bps {
    if proportion_seized <= Wad::ZERO {
        return Bps::from(0);
    }
    // Ceil the threshold and floor the derived max so the realized
    // effective_threshold * (1 + bonus) stays <= 1.
    let scaled = proportion_seized
        .raw()
        .checked_mul(BPS)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    let eff_thr_bps = ((scaled + (WAD - 1)) / WAD).clamp(1, BPS);
    let numerator = BPS
        .checked_mul(BPS - eff_thr_bps)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    Bps::from(numerator / eff_thr_bps)
}

/// Returns base and max liquidation bonus for the account collateral mix.
pub(crate) fn get_account_bonus_params(
    env: &Env,
    cache: &mut Cache,
    supply_positions: &Map<Address, AccountPositionRaw>,
    proportion_seized: Wad,
) -> BonusBounds {
    let max = max_bonus_for_threshold(env, proportion_seized);

    let mut total_collateral = Wad::ZERO;
    let mut asset_values: Vec<(i128, i128)> = Vec::new(env);

    for (asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let value = helpers::position_value(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        total_collateral += value;
        asset_values.push_back((value.raw(), position.liquidation_bonus.raw()));
    }

    if total_collateral == Wad::ZERO {
        return BonusBounds {
            base: Bps::from(0),
            max,
        };
    }

    let mut weighted_bonus_sum: i128 = 0;
    for i in 0..asset_values.len() {
        let (value_raw, bonus_bps) = validation::expect_invariant(env, asset_values.get(i));
        let weight = Wad::from(value_raw).div(env, total_collateral);
        weighted_bonus_sum = weighted_bonus_sum
            .checked_add(weight.mul(env, Wad::from(bonus_bps)).raw())
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    }

    // Clamp base to the account ceiling so the bonus interpolation range
    // (max - base) stays non-negative for high-threshold accounts.
    let base = Bps::from(weighted_bonus_sum.min(max.raw()));
    BonusBounds { base, max }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::constants::RAY;

    #[test]
    fn debt_close_amount_uses_pool_full_close_ceiling() {
        let env = Env::default();
        let position = DebtPosition {
            scaled_amount: Ray::from(RAY + RAY * 4 / 10),
        };

        assert_eq!(position.scaled_amount.mul(&env, Ray::ONE).to_asset(0), 1);
        assert_eq!(debt_close_amount(&env, &position, Ray::ONE, 0), 2);
    }
}
