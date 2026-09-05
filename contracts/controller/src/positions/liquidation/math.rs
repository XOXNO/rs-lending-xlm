use common::constants::BPS;
use common::errors::{CollateralError, GenericError};
use common::math::fp::{Bps, Ray, Wad};
use common::math::fp_core::{mul_div_ceil, mul_div_floor};
use common::rates::{resolve_withdrawal, unscale_borrow_ceil};
use common::types::{
    Account, AccountPositionRaw, DebtPosition, HubAssetKey, HubPayment, LiquidationResult,
    PaymentTuple, RepayEntry, SeizeEntry,
};
use soroban_sdk::{panic_with_error, Env, Map, Vec};

use super::curve::{
    estimate_liquidation_amount, max_bonus_for_threshold, max_hf_preserving_bonus_bps, BonusBounds,
    LiquidationCurve, LiquidationSnapshot,
};
use crate::context::Context;
use crate::payments;
use crate::risk;
use crate::storage::iter_typed_positions;
use common::validation::expect_invariant;

pub(crate) struct NormalizedRepaymentPlan {
    pub repaid: Vec<RepayEntry>,
    pub refunds: Vec<PaymentTuple>,
    pub repay_usd: Wad,
    pub bonus: Bps,
}

impl NormalizedRepaymentPlan {
    /// Requires recorded repayment values to sum to `repay_usd`.
    fn validate(&self, env: &Env) {
        if sum_repaid_usd(env, &self.repaid) != self.repay_usd {
            panic_with_error!(env, GenericError::InternalError);
        }
    }
}

pub(crate) struct LiquidationPlan {
    pub repayment: NormalizedRepaymentPlan,
    pub seized: Vec<SeizeEntry>,
}

impl LiquidationPlan {
    /// Validates repayment totals and both seizure representations: positive
    /// amounts, fees bounded by gross seizure, bonus shares bounded by seized
    /// shares, and fee rates below 100%. Credit splits are checked at each use.
    pub(crate) fn validate(&self, env: &Env) {
        self.repayment.validate(env);

        for entry in self.seized.iter() {
            if entry.amount <= 0 || entry.protocol_fee < 0 || entry.protocol_fee > entry.amount {
                panic_with_error!(env, GenericError::InternalError);
            }
            if entry.scaled_amount <= 0
                || entry.bonus_scaled < 0
                || entry.bonus_scaled > entry.scaled_amount
                || i128::from(entry.liquidation_fees) >= BPS
            {
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
}
/// Returns `weighted_collateral / total_collateral` and the account's bonus
/// bounds. The proportion is zero without collateral.
pub(crate) fn calculate_seizure_proportions(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    weighted_collateral: Wad,
    cache: &mut Context,
) -> (Wad, BonusBounds) {
    let proportion_seized = if total_collateral > Wad::ZERO {
        weighted_collateral.div(env, total_collateral)
    } else {
        Wad::ZERO
    };

    let bounds = get_account_bonus_params(
        env,
        cache,
        &account.supply_positions,
        total_collateral,
        proportion_seized,
    );

    (proportion_seized, bounds)
}

/// Merges positive payments and caps each at its ceiling-rounded debt balance.
/// Requires an existing debt position. Returns planned WAD USD repayments and
/// records unused inputs in `refunds`; this planning step moves no tokens.
pub(crate) fn calculate_repayment_amounts(
    env: &Env,
    raw_payments: &Vec<HubPayment>,
    account: &Account,
    refunds: &mut Vec<PaymentTuple>,
    cache: &mut Context,
) -> (Wad, Vec<RepayEntry>) {
    let mut total_repaid_usd = Wad::ZERO;
    let mut repaid_tokens: Vec<RepayEntry> = Vec::new(env);

    let merged = payments::aggregate_positive_payments(env, raw_payments);

    for (hub_asset, amount) in merged {
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);

        let position: DebtPosition = (&account
            .borrow_positions
            .get(hub_asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound)))
            .into();

        let actual_debt = unscale_borrow_ceil(
            env,
            position.scaled_amount,
            market_index.borrow_index,
            feed.asset_decimals,
        );

        let mut payment_amount = amount;
        if payment_amount > actual_debt {
            let excess = payment_amount - actual_debt;
            refunds.push_back(PaymentTuple {
                asset: hub_asset.asset.clone(),
                amount: excess,
            });
            payment_amount = actual_debt;
        }

        let payment_usd = feed.usd_value_wad(env, payment_amount);

        total_repaid_usd = total_repaid_usd.checked_add(env, payment_usd);
        repaid_tokens.push_back(RepayEntry {
            hub_asset,
            amount: payment_amount,
            usd_wad: payment_usd.raw(),
            feed: (&feed).into(),
            market_index: (&market_index).into(),
        });
    }

    (total_repaid_usd, repaid_tokens)
}

/// Caps planned repayments at the ideal WAD USD amount and records unused inputs.
/// A nonnegative HF-preserving bonus cap below the base requires full repayment;
/// ceiling-rounded valuation tolerates a shortfall caused solely by rounding.
pub(crate) fn normalize_repayment_plan(
    env: &Env,
    account: &Account,
    raw_payments: &Vec<HubPayment>,
    snap: &LiquidationSnapshot,
    bonus_bounds: BonusBounds,
    curve: &LiquidationCurve,
    cache: &mut Context,
) -> NormalizedRepaymentPlan {
    let mut refunds = Vec::new(env);
    let (total_debt_payment_usd, repaid_tokens) =
        calculate_repayment_amounts(env, raw_payments, account, &mut refunds, cache);

    let (ideal_repayment_usd, bonus) = estimate_liquidation_amount(env, snap, bonus_bounds, curve);

    // Revalue upward only when the full-close gate would otherwise reject a
    // rounding-only shortfall.
    let cap_forces_full_close = max_hf_preserving_bonus_bps(snap)
        .is_some_and(|cap| (0..bonus_bounds.base.raw()).contains(&cap));
    if total_debt_payment_usd < ideal_repayment_usd
        && cap_forces_full_close
        && sum_repaid_usd_ceil(env, &repaid_tokens) < ideal_repayment_usd
    {
        panic_with_error!(env, CollateralError::FullCloseRequired);
    }

    let max_debt_to_repay_usd = total_debt_payment_usd.min(ideal_repayment_usd);

    let mut final_repayment_tokens = repaid_tokens;
    if total_debt_payment_usd > max_debt_to_repay_usd {
        let excess_usd = total_debt_payment_usd.checked_sub(env, max_debt_to_repay_usd);
        process_excess_payment(env, &mut final_repayment_tokens, &mut refunds, excess_usd);
    }

    // Sum the final entries before moving them; plan validation checks equality.
    NormalizedRepaymentPlan {
        repay_usd: sum_repaid_usd(env, &final_repayment_tokens),
        repaid: final_repayment_tokens,
        refunds,
        bonus,
    }
}

/// Sums recorded repayment values in WAD USD.
pub(crate) fn sum_repaid_usd(env: &Env, repaid_tokens: &Vec<RepayEntry>) -> Wad {
    let mut total = Wad::ZERO;
    for entry in repaid_tokens.iter() {
        total = total.checked_add(env, Wad::from(entry.usd_wad));
    }
    total
}

/// Revalues token amounts at their prices with upward rounding, in WAD USD.
fn sum_repaid_usd_ceil(env: &Env, repaid_tokens: &Vec<RepayEntry>) -> Wad {
    let mut total = Wad::ZERO;
    for entry in repaid_tokens.iter() {
        let value = Wad::from_token(env, entry.amount, entry.feed.asset_decimals)
            .mul_ceil(env, Wad::from(entry.feed.price_wad));
        total = total.checked_add(env, value);
    }
    total
}

/// Allocates repayment plus bonus pro-rata by collateral USD value, capped at
/// each held balance. Principal comes from the uncapped seizure; fees apply
/// only to bonus remaining after the cap.
///
/// Transfer amounts round down for partial closes and half-up for full closes.
/// Positive fees floor to asset units with a one-unit minimum, capped by the
/// pool's gross withdrawal. Credit mode retains a separate exact share representation.
pub(crate) fn calculate_seized_collateral(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    repayment: &NormalizedRepaymentPlan,
    cache: &mut Context,
) -> Vec<SeizeEntry> {
    let mut seized: Vec<SeizeEntry> = Vec::new(env);
    if total_collateral <= Wad::ZERO {
        return seized;
    }

    let one_plus_bonus = Wad::ONE.checked_add(env, repayment.bonus.to_wad(env));

    let total_seizure_usd = repayment.repay_usd.mul(env, one_plus_bonus);

    // Units: *_ray = RAY asset value (shares * index); *_scaled = RAY shares;
    // *_amount, fee_asset, and protocol_fee = token units at the feed's decimals.
    for (hub_asset, position) in iter_typed_positions(&account.supply_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);

        let actual_ray = position.scaled_amount.mul(env, market_index.supply_index);
        let asset_value = risk::position_value(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        let share = asset_value.div(env, total_collateral);
        let seizure_for_asset_usd = total_seizure_usd.mul(env, share);

        let seizure_amount_wad = seizure_for_asset_usd.div(env, feed.price);
        let seizure_ray = seizure_amount_wad.to_ray(env);

        if seizure_ray <= Ray::ZERO {
            continue;
        }

        let capped_ray = seizure_ray.min(actual_ray);
        if capped_ray <= Ray::ZERO {
            continue;
        }
        // Share and token conversions must agree on whether the close is full.
        let is_full_close = capped_ray == actual_ray;

        // Derive principal before the cap; dividing the capped seizure would
        // invent a fee-bearing bonus where collateral no longer covers repayment.
        let base_ray = seizure_ray.div_floor(env, one_plus_bonus.to_ray(env));
        // A cap below principal leaves no bonus; avoid negative subtraction.
        let bonus_ray = if capped_ray > base_ray {
            capped_ray.checked_sub(env, base_ray)
        } else {
            Ray::ZERO
        };
        let protocol_fee_ray = position.liquidation_fees.apply_to_ray(env, bonus_ray);

        // Full credit closes take exact held shares; an asset-unit round trip
        // could strand or invent shares. Partial share conversions round down.
        let seized_scaled = if is_full_close {
            position.scaled_amount
        } else {
            capped_ray.div_floor(env, market_index.supply_index)
        };
        // Independent conversions must still preserve `bonus <= seized`.
        let bonus_scaled = bonus_ray
            .div_floor(env, market_index.supply_index)
            .min(seized_scaled);
        if seized_scaled <= Ray::ZERO {
            continue;
        }

        let capped_amount = if is_full_close {
            capped_ray.to_asset(env, feed.asset_decimals)
        } else {
            capped_ray.to_asset_floor(env, feed.asset_decimals)
        };
        if capped_amount <= 0 {
            continue;
        }

        let (_, pool_gross) = resolve_withdrawal(
            env,
            capped_amount,
            position.scaled_amount,
            market_index.supply_index,
            feed.asset_decimals,
        );
        let fee_asset = protocol_fee_ray.to_asset_floor(env, feed.asset_decimals);
        let bumped_fee = if protocol_fee_ray > Ray::ZERO && fee_asset == 0 {
            1
        } else {
            fee_asset
        };
        let protocol_fee = bumped_fee.min(pool_gross);

        seized.push_back(SeizeEntry {
            hub_asset,
            amount: capped_amount,
            protocol_fee,
            scaled_amount: seized_scaled.raw(),
            bonus_scaled: bonus_scaled.raw(),
            liquidation_fees: position.liquidation_fees.raw() as u32,
            feed: (&feed).into(),
            market_index: (&market_index).into(),
        });
    }

    seized
}

/// Splits credit-mode shares with `fee = ceil(fee_rate * bonus_scaled)`.
/// The liquidator gets the exact remainder. Enforces bounds and
/// `seized_scaled == fee + liquidator` to prevent unbacked share creation.
pub(crate) fn split_seized_shares(
    env: &Env,
    seized_scaled: Ray,
    bonus_scaled: Ray,
    liquidation_fees_bps: u32,
) -> (Ray, Ray) {
    let fees = i128::from(liquidation_fees_bps);
    if seized_scaled < Ray::ZERO
        || bonus_scaled < Ray::ZERO
        || bonus_scaled > seized_scaled
        || fees >= BPS
    {
        panic_with_error!(env, GenericError::InternalError);
    }

    let fee_scaled = Ray::from(mul_div_ceil(env, bonus_scaled.raw(), fees, BPS));
    // Recheck the computed fee against the seizure; rates come from position stamps.
    if fee_scaled > seized_scaled {
        panic_with_error!(env, GenericError::InternalError);
    }

    let liquidator_scaled = seized_scaled.checked_sub(env, fee_scaled);
    if fee_scaled.checked_add(env, liquidator_scaled) != seized_scaled {
        panic_with_error!(env, GenericError::InternalError);
    }
    (fee_scaled, liquidator_scaled)
}

/// Returns the threshold-derived maximum and USD-weighted stamped base bonus,
/// capped at that maximum. Without collateral, the base is zero.
pub(crate) fn get_account_bonus_params(
    env: &Env,
    cache: &mut Context,
    supply_positions: &Map<HubAssetKey, AccountPositionRaw>,
    total_collateral: Wad,
    proportion_seized: Wad,
) -> BonusBounds {
    let max = max_bonus_for_threshold(env, proportion_seized);

    if total_collateral == Wad::ZERO {
        return BonusBounds {
            base: Bps::from(0),
            max,
        };
    }

    let mut weighted_bonus_sum: i128 = 0;
    for (hub_asset, position) in iter_typed_positions(supply_positions) {
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);

        let value = risk::position_value(
            env,
            position.scaled_amount,
            market_index.supply_index,
            feed.price,
        );

        let weight = value.div(env, total_collateral);
        weighted_bonus_sum = weighted_bonus_sum
            .checked_add(
                weight
                    .mul(env, Wad::from(position.liquidation_bonus.raw()))
                    .raw(),
            )
            .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow));
    }

    let base = Bps::from(weighted_bonus_sum.min(max.raw()));
    BonusBounds { base, max }
}

#[cfg(test)]
#[path = "../../../tests/positions/liquidation_math.rs"]
mod tests;

/// Floors both transfer and credit representations by `received_usd / planned_usd`
/// when receipts fall short; otherwise returns them unchanged.
///
/// Credit fees are re-derived from the scaled bonus base to conserve shares.
/// Flooring both share fields by the same ratio preserves `bonus_scaled <= scaled_amount`.
pub(crate) fn scale_seizures_to_received(
    env: &Env,
    seized: &Vec<SeizeEntry>,
    received_usd: Wad,
    planned_usd: Wad,
) -> Vec<SeizeEntry> {
    if planned_usd <= Wad::ZERO || received_usd >= planned_usd {
        return seized.clone();
    }

    let num = received_usd.raw();
    let den = planned_usd.raw();
    let mut scaled: Vec<SeizeEntry> = Vec::new(env);
    for entry in seized.iter() {
        scaled.push_back(SeizeEntry {
            amount: mul_div_floor(env, entry.amount, num, den),
            protocol_fee: mul_div_floor(env, entry.protocol_fee, num, den),
            scaled_amount: mul_div_floor(env, entry.scaled_amount, num, den),
            bonus_scaled: mul_div_floor(env, entry.bonus_scaled, num, den),
            liquidation_fees: entry.liquidation_fees,
            hub_asset: entry.hub_asset,
            feed: entry.feed,
            market_index: entry.market_index,
        });
    }
    scaled
}

/// Removes planned excess from the last repayment backward, recording unused
/// inputs in `refunds`. A partial removal uses a floor-rounded ratio; no tokens move.
fn process_excess_payment(
    env: &Env,
    repaid_tokens: &mut Vec<RepayEntry>,
    refunds: &mut Vec<PaymentTuple>,
    excess_usd: Wad,
) {
    let mut remaining_excess_usd = excess_usd;
    let mut current_index = repaid_tokens.len();
    while remaining_excess_usd > Wad::ZERO && current_index > 0 {
        current_index -= 1;
        let entry = expect_invariant(env, repaid_tokens.get(current_index));
        if entry.amount <= 0 {
            continue;
        }
        let usd = Wad::from(entry.usd_wad);
        if usd == Wad::ZERO {
            continue;
        }
        if usd > remaining_excess_usd {
            let ratio = remaining_excess_usd.div_floor(env, usd);
            let refund_amount = Wad::from_token(env, entry.amount, entry.feed.asset_decimals)
                .mul_floor(env, ratio)
                .to_token_floor(env, entry.feed.asset_decimals);
            let new_amount = entry.amount - refund_amount;
            let new_usd = Wad::from_token(env, new_amount, entry.feed.asset_decimals)
                .mul(env, Wad::from(entry.feed.price_wad));
            refunds.push_back(PaymentTuple {
                asset: entry.hub_asset.asset.clone(),
                amount: refund_amount,
            });
            repaid_tokens.set(
                current_index,
                RepayEntry {
                    hub_asset: entry.hub_asset,
                    amount: new_amount,
                    usd_wad: new_usd.raw(),
                    feed: entry.feed,
                    market_index: entry.market_index,
                },
            );
            remaining_excess_usd = Wad::ZERO;
        } else {
            refunds.push_back(PaymentTuple {
                asset: entry.hub_asset.asset.clone(),
                amount: entry.amount,
            });
            repaid_tokens.remove(current_index);
            remaining_excess_usd = remaining_excess_usd.checked_sub(env, usd);
        }
    }
}
