//! Builds and validates a liquidation's repayment and seizure plan: sizing the debt
//! to repay, the collateral to seize, and the applied bonus, given the liquidator's
//! payments and the account's liquidation snapshot.

use common::errors::{CollateralError, GenericError};
use common::math::fp::{Bps, Ray, Wad};
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
use crate::context::Cache;
use crate::payments;
use crate::risk;
use crate::storage::iter_typed_positions;
use common::validation::expect_invariant;

impl LiquidationCurve {
    /// Builds a `LiquidationCurve` from the spoke configuration for `spoke_id`,
    /// loaded through `cache`.
    pub(crate) fn resolve(cache: &mut Cache, spoke_id: u32) -> Self {
        Self::from_config(&cache.spoke_config(spoke_id))
    }
}

/// A resolved liquidation repayment: the token amounts collected from the
/// liquidator, any refunds for excess payment, the total USD value repaid, and the
/// applied bonus.
pub(crate) struct NormalizedRepaymentPlan {
    pub repaid: Vec<RepayEntry>,
    pub refunds: Vec<PaymentTuple>,
    pub repay_usd: Wad,
    pub bonus: Bps,
}

impl NormalizedRepaymentPlan {
    /// Panics if the summed USD value of `repaid` does not equal `repay_usd`.
    fn validate(&self, env: &Env) {
        if sum_repaid_usd(env, &self.repaid) != self.repay_usd {
            panic_with_error!(env, GenericError::InternalError);
        }
    }
}

/// A liquidation's resolved repayment and seizure plan.
pub(crate) struct LiquidationPlan {
    pub repayment: NormalizedRepaymentPlan,
    pub seized: Vec<SeizeEntry>,
}

impl LiquidationPlan {
    /// Validates the repayment plan and panics if any seized entry has a
    /// non-positive amount or a protocol fee outside `[0, amount]`.
    pub(crate) fn validate(&self, env: &Env) {
        self.repayment.validate(env);

        for entry in self.seized.iter() {
            if entry.amount <= 0 || entry.protocol_fee < 0 || entry.protocol_fee > entry.amount {
                panic_with_error!(env, GenericError::InternalError);
            }
        }
    }

    /// Converts the plan into a `LiquidationResult`, using `repay_usd` as
    /// `max_debt_usd` and `bonus` as `bonus_bps`.
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
/// Computes the proportion of weighted collateral to seize — a unitless ratio
/// `weighted_coll / total_collateral` — and the account's liquidation bonus
/// bounds, both derived from its supply positions.
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

    let bounds = get_account_bonus_params(
        env,
        cache,
        &account.supply_positions,
        total_collateral,
        proportion_seized,
    );

    (proportion_seized, bounds)
}

/// Aggregates `raw_payments` per asset and matches each against the account's
/// outstanding debt, capping each leg at that position's full outstanding debt
/// (ceil-unscaled; not a protocol close-factor fraction) and pushing any excess
/// to `refunds`. Returns the total USD value applied and the resulting repay
/// entries. Panics if a payment references an asset the account has no debt
/// position for.
pub(crate) fn calculate_repayment_amounts(
    env: &Env,
    raw_payments: &Vec<HubPayment>,
    account: &Account,
    refunds: &mut Vec<PaymentTuple>,
    cache: &mut Cache,
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

/// Builds a validated repayment plan from `raw_payments`, sized to at most the ideal
/// repayment from `estimate_liquidation_amount`, trimming any excess into refunds.
/// Panics with `CollateralError::FullCloseRequired` if the payment falls short of
/// the ideal amount while only a full liquidation would keep the bonus curve
/// solvable.
pub(crate) fn normalize_repayment_plan(
    env: &Env,
    account: &Account,
    raw_payments: &Vec<HubPayment>,
    snap: &LiquidationSnapshot,
    bonus_bounds: BonusBounds,
    curve: &LiquidationCurve,
    cache: &mut Cache,
) -> NormalizedRepaymentPlan {
    let mut refunds = Vec::new(env);
    let (total_debt_payment_usd, repaid_tokens) =
        calculate_repayment_amounts(env, raw_payments, account, &mut refunds, cache);

    let (ideal_repayment_usd, bonus) = estimate_liquidation_amount(env, snap, bonus_bounds, curve);

    if total_debt_payment_usd < ideal_repayment_usd {
        if let Some(cap) = max_hf_preserving_bonus_bps(snap) {
            if cap >= 0
                && cap < bonus_bounds.base.raw()
                && sum_repaid_usd_ceil(env, &repaid_tokens) < ideal_repayment_usd
            {
                panic_with_error!(env, CollateralError::FullCloseRequired);
            }
        }
    }

    let max_debt_to_repay_usd = total_debt_payment_usd.min(ideal_repayment_usd);

    let mut final_repayment_tokens = repaid_tokens;
    if total_debt_payment_usd > max_debt_to_repay_usd {
        let excess_usd = total_debt_payment_usd.checked_sub(env, max_debt_to_repay_usd);
        process_excess_payment(env, &mut final_repayment_tokens, &mut refunds, excess_usd);
    }

    let repayment = NormalizedRepaymentPlan {
        repay_usd: sum_repaid_usd(env, &final_repayment_tokens),
        repaid: final_repayment_tokens,
        refunds,
        bonus,
    };
    repayment.validate(env);
    repayment
}

/// Returns the token amount required to close `position`'s debt, rounded up, given
/// the current borrow index.
fn debt_close_amount(
    env: &Env,
    position: &DebtPosition,
    borrow_index: Ray,
    asset_decimals: u32,
) -> i128 {
    unscale_borrow_ceil(env, position.scaled_amount, borrow_index, asset_decimals)
}

/// Sums the USD value of `repaid_tokens`.
pub(crate) fn sum_repaid_usd(env: &Env, repaid_tokens: &Vec<RepayEntry>) -> Wad {
    let mut total = Wad::ZERO;
    for entry in repaid_tokens.iter() {
        total = total.checked_add(env, Wad::from(entry.usd_wad));
    }
    total
}

/// Sums the USD value of `repaid_tokens`, recomputed from each entry's token amount
/// and price with ceiling rounding.
fn sum_repaid_usd_ceil(env: &Env, repaid_tokens: &Vec<RepayEntry>) -> Wad {
    let mut total = Wad::ZERO;
    for entry in repaid_tokens.iter() {
        let value = Wad::from_token(entry.amount, entry.feed.asset_decimals)
            .mul_ceil(env, Wad::from(entry.feed.price_wad));
        total = total.checked_add(env, value);
    }
    total
}

/// Scales each entry's `amount` and `protocol_fee` in `seized` by
/// `received_usd / planned_usd`, using floor rounding. Returns `seized` unchanged
/// when `planned_usd` is not positive or `received_usd` is at or above
/// `planned_usd`.
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
            amount: common::math::fp_core::mul_div_floor(env, entry.amount, num, den),
            protocol_fee: common::math::fp_core::mul_div_floor(env, entry.protocol_fee, num, den),
            hub_asset: entry.hub_asset,
            feed: entry.feed,
            market_index: entry.market_index,
        });
    }
    scaled
}

/// Computes the collateral to seize from each of the account's supply positions,
/// proportional to each asset's share of `total_collateral`, scaled up by the
/// repayment's bonus. Caps each seizure at the position's actual balance, splits the
/// bonus portion into a protocol fee via the position's configured fee rate, and
/// rounds a positive fee up to at least one asset unit, capped at the pool's gross
/// withdrawal amount. Returns an empty vector when `total_collateral` is not
/// positive.
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

    let one_plus_bonus = Wad::ONE.checked_add(env, repayment.bonus.to_wad(env));

    let total_seizure_usd = repayment.repay_usd.mul(env, one_plus_bonus);

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
        let seizure_ray = seizure_amount_wad.to_ray();

        if seizure_ray <= Ray::ZERO {
            continue;
        }

        let capped_ray = seizure_ray.min(actual_ray);
        if capped_ray <= Ray::ZERO {
            continue;
        }

        // The base is the leg's own repayment share, not the clamped seizure
        // divided back out: once the seizure clamps there is no bonus to take a
        // cut of, and dividing the clamp by (1 + bonus) invents one.
        let base_ray = seizure_ray.div_floor(env, one_plus_bonus.to_ray());
        // A seizure clamped below the repayment share is a bad-debt close with no
        // excess at all. Ray::checked_sub traps on a negative result, so guard.
        let bonus_ray = if capped_ray > base_ray {
            capped_ray.checked_sub(env, base_ray)
        } else {
            Ray::ZERO
        };
        let protocol_fee_ray = position.liquidation_fees.apply_to_ray(env, bonus_ray);

        let capped_amount = if capped_ray == actual_ray {
            capped_ray.to_asset(feed.asset_decimals)
        } else {
            capped_ray.to_asset_floor(feed.asset_decimals)
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
        let fee_asset = protocol_fee_ray.to_asset_floor(feed.asset_decimals);
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
            feed: (&feed).into(),
            market_index: (&market_index).into(),
        });
    }

    seized
}

/// Reduces `repaid_tokens` by `excess_usd`, working backward from the last entry:
/// partially reduces an entry to absorb the remaining excess, or removes it entirely
/// and continues to the next, pushing each removed or reduced amount to `refunds`.
pub(crate) fn process_excess_payment(
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
            let refund_amount = Wad::from_token(entry.amount, entry.feed.asset_decimals)
                .mul_floor(env, ratio)
                .to_token_floor(entry.feed.asset_decimals);

            let new_amount = entry.amount - refund_amount;

            let new_amount_wad = Wad::from_token(new_amount, entry.feed.asset_decimals);
            let new_usd = new_amount_wad.mul(env, Wad::from(entry.feed.price_wad));

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

/// Computes the account's liquidation bonus bounds: `max` from `proportion_seized`,
/// and `base` as the value-weighted average of each supply position's configured
/// liquidation bonus, capped at `max`. Returns `base = 0` when `total_collateral` is
/// zero. Panics on overflow while accumulating the weighted sum.
pub(crate) fn get_account_bonus_params(
    env: &Env,
    cache: &mut Cache,
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
