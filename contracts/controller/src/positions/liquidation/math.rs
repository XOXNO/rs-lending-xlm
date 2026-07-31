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
    pub(crate) fn resolve(cache: &mut Cache, spoke_id: u32) -> Self {
        Self::from_config(&cache.spoke_config(spoke_id))
    }
}

pub(crate) struct NormalizedRepaymentPlan {
    pub repaid: Vec<RepayEntry>,
    pub refunds: Vec<PaymentTuple>,
    pub repay_usd: Wad,
    pub bonus: Bps,
}

impl NormalizedRepaymentPlan {
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

    let bounds = get_account_bonus_params(
        env,
        cache,
        &account.supply_positions,
        total_collateral,
        proportion_seized,
    );

    (proportion_seized, bounds)
}

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

fn debt_close_amount(
    env: &Env,
    position: &DebtPosition,
    borrow_index: Ray,
    asset_decimals: u32,
) -> i128 {
    unscale_borrow_ceil(env, position.scaled_amount, borrow_index, asset_decimals)
}

pub(crate) fn sum_repaid_usd(env: &Env, repaid_tokens: &Vec<RepayEntry>) -> Wad {
    let mut total = Wad::ZERO;
    for entry in repaid_tokens.iter() {
        total = total.checked_add(env, Wad::from(entry.usd_wad));
    }
    total
}

fn sum_repaid_usd_ceil(env: &Env, repaid_tokens: &Vec<RepayEntry>) -> Wad {
    let mut total = Wad::ZERO;
    for entry in repaid_tokens.iter() {
        let value = Wad::from_token(entry.amount, entry.feed.asset_decimals)
            .mul_ceil(env, Wad::from(entry.feed.price_wad));
        total = total.checked_add(env, value);
    }
    total
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

        let base_ray = capped_ray.div_floor(env, one_plus_bonus.to_ray());
        let bonus_ray = capped_ray.checked_sub(env, base_ray);
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
