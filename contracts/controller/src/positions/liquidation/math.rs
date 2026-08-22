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
use crate::context::Cache;
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
    /// Panics with `GenericError::InternalError` if the summed USD value of `repaid` entries
    /// does not match `repay_usd`.
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
    /// Validates the repayment leg totals and asserts every seizure entry is internally
    /// consistent, panicking with `GenericError::InternalError` otherwise: a positive asset
    /// amount with a protocol fee between zero and that amount (the `Transfer` representation),
    /// and a positive scaled amount whose bonus base does not exceed it (the `Credit`
    /// representation). The `Credit` split itself is re-derived and re-checked by
    /// [`split_seized_shares`] at every use site.
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

    /// Converts this plan into the `LiquidationResult` returned to callers, carrying the seized,
    /// repaid, and refund entries plus the total USD repaid and the applied bonus in basis
    /// points.
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
/// Computes the liquidation-threshold-weighted share of collateral being seized
/// (`weighted_coll / total_collateral`, zero when there is no collateral) and the account's
/// base/max bonus bounds derived from it.
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

/// Merges `raw_payments` per asset (panicking with `CollateralError::DebtPositionNotFound` if a
/// payment targets an asset without a debt position) and caps each leg at that debt's
/// outstanding, ceiling-rounded balance, refunding any excess into `refunds`. Returns the total
/// USD value actually repaid and the per-asset `RepayEntry` list.
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

/// Combines the liquidator's `raw_payments` with `estimate_liquidation_amount`'s ideal repayment
/// and bonus: caps the accepted repayment at the ideal USD amount, refunding any excess back
/// through `refunds`. Panics with `CollateralError::FullCloseRequired` when the payment falls
/// short of the ideal amount, `max_hf_preserving_bonus_bps` returns a non-negative cap below the
/// base bonus, and the shortfall persists after ceiling-rounding the payment — forcing the
/// liquidator to close the full debt instead of partially repaying.
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

    // Field order is load-bearing: `repay_usd` reads `final_repayment_tokens`
    // before `repaid` moves it. The sum/`repay_usd` agreement this establishes
    // is asserted by `LiquidationPlan::validate` at the one construction site.
    NormalizedRepaymentPlan {
        repay_usd: sum_repaid_usd(env, &final_repayment_tokens),
        repaid: final_repayment_tokens,
        refunds,
        bonus,
    }
}

/// Sums the WAD USD value recorded on each `repaid_tokens` entry.
pub(crate) fn sum_repaid_usd(env: &Env, repaid_tokens: &Vec<RepayEntry>) -> Wad {
    let mut total = Wad::ZERO;
    for entry in repaid_tokens.iter() {
        total = total.checked_add(env, Wad::from(entry.usd_wad));
    }
    total
}

/// Recomputes and sums each `repaid_tokens` entry's USD value from its token amount and price,
/// rounding up instead of trusting the entry's stored `usd_wad`.
fn sum_repaid_usd_ceil(env: &Env, repaid_tokens: &Vec<RepayEntry>) -> Wad {
    let mut total = Wad::ZERO;
    for entry in repaid_tokens.iter() {
        let value = Wad::from_token(entry.amount, entry.feed.asset_decimals)
            .mul_ceil(env, Wad::from(entry.feed.price_wad));
        total = total.checked_add(env, value);
    }
    total
}

/// Splits `repayment.repay_usd * (1 + bonus)` pro-rata by USD value across supply positions,
/// capping each leg at the position's held balance (floor-rounded for a partial position,
/// half-up for a full one). The capped amount is split into a principal portion — the pre-cap
/// seizure value at `1 / (1 + bonus)`, floor-rounded — and a bonus portion equal to the
/// remainder, which is zero once the cap has consumed the pre-cap value; the position's protocol
/// fee rate applies only to the bonus portion. The resulting protocol fee is floor-rounded to
/// asset units, bumped up to one unit when a positive fee would otherwise round to zero, and
/// capped at the pool's gross withdrawal for that leg.
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

        // Credit mode moves supply shares, so the seizure is converted to scaled units once,
        // here, with no asset-unit round trip. A full close is exactly the whole scaled
        // position: re-deriving it from the asset value would let a rounding step strand or
        // invent a share.
        let seized_scaled = if capped_ray == actual_ray {
            position.scaled_amount
        } else {
            capped_ray.div_floor(env, market_index.supply_index)
        };
        // The fee base in share terms. Clamped because the two conversions round
        // independently; `split_seized_shares` relies on `bonus <= seized`.
        let bonus_scaled = bonus_ray
            .div_floor(env, market_index.supply_index)
            .min(seized_scaled);
        if seized_scaled <= Ray::ZERO {
            continue;
        }

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
            scaled_amount: seized_scaled.raw(),
            bonus_scaled: bonus_scaled.raw(),
            liquidation_fees: position.liquidation_fees.raw() as u32,
            feed: (&feed).into(),
            market_index: (&market_index).into(),
        });
    }

    seized
}

/// Splits a scaled seizure into the protocol's share and the liquidator's share, for
/// `SeizeMode::Credit`.
///
/// `fee = ceil(liquidation_fees × bonus_scaled)` rounds **up**, in the protocol's favour, and
/// the liquidator takes the exact remainder, so `seized_scaled == fee + liquidator` holds with
/// no share created or destroyed. That conservation identity is the whole point of credit mode
/// — a share invented here is an unbacked supplier claim — so it is asserted here rather than
/// only in tests. Panics with `GenericError::InternalError` if any bound or the identity fails.
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
    // `fees < BPS` already bounds the fee by `bonus_scaled`; re-check anyway, because the rate
    // is read from a stamped position rather than from live configuration.
    if fee_scaled > seized_scaled {
        panic_with_error!(env, GenericError::InternalError);
    }

    let liquidator_scaled = seized_scaled.checked_sub(env, fee_scaled);
    if fee_scaled.checked_add(env, liquidator_scaled) != seized_scaled {
        panic_with_error!(env, GenericError::InternalError);
    }
    (fee_scaled, liquidator_scaled)
}

/// Computes the account's bonus bounds: `max` from `max_bonus_for_threshold`, and `base` as the
/// USD-value-weighted average of each supply position's configured liquidation bonus, clamped
/// to not exceed `max`. Returns a zero `base` when the account has no collateral.
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

/// Scales every seized entry down by `received_usd / planned_usd` (floor-rounded) when the
/// liquidator's repayment delivered less value than planned. Returns `seized` unchanged when
/// nothing was planned or the full amount was received.
///
/// Both representations are scaled: the asset-unit pair consumed by `SeizeMode::Transfer` and
/// the share-denominated pair consumed by `SeizeMode::Credit`. Only the seizure total and the
/// fee *base* are scaled — the credit-mode fee itself is re-derived from the scaled base at the
/// use site, so the conservation identity holds exactly after scaling instead of being carried
/// across it. Flooring both share fields by the same ratio preserves
/// `bonus_scaled <= scaled_amount`.
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

/// Refunds `excess_usd` out of `repaid_tokens` in place, working backward from the last entry:
/// fully refunds and removes entries until the remaining excess fits within one entry, then
/// partially refunds that entry using a floor-rounded ratio. Appends each refund to `refunds`.
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
            let refund_amount = Wad::from_token(entry.amount, entry.feed.asset_decimals)
                .mul_floor(env, ratio)
                .to_token_floor(entry.feed.asset_decimals);
            let new_amount = entry.amount - refund_amount;
            let new_usd = Wad::from_token(new_amount, entry.feed.asset_decimals)
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
