use common::errors::CollateralError;
use common::fp::{Bps, Ray, Wad};
use common::types::{Account, Payment, RepayEntry, SeizeEntry};
use soroban_sdk::{panic_with_error, Env, Vec};

use crate::cache::ControllerCache;
use crate::helpers;
use crate::utils;
use crate::validation;

pub(crate) fn calculate_seizure_proportions(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    weighted_coll: Wad,
    cache: &mut ControllerCache,
) -> (Wad, (Bps, Bps)) {
    let proportion_seized = if total_collateral > Wad::ZERO {
        weighted_coll.div(env, total_collateral)
    } else {
        Wad::ZERO
    };

    let bonus_params = helpers::get_account_bonus_params(env, cache, &account.supply_positions);

    (proportion_seized, bonus_params)
}
pub(crate) fn calculate_repayment_amounts(
    env: &Env,
    raw_payments: &Vec<Payment>,
    account: &Account,
    refunds: &mut Vec<Payment>,
    cache: &mut ControllerCache,
) -> (Wad, Vec<RepayEntry>) {
    let mut total_repaid_usd = Wad::ZERO;
    let mut repaid_tokens: Vec<RepayEntry> = Vec::new(env);

    let merged = utils::aggregate_positive_payments(env, raw_payments);

    for i in 0..merged.len() {
        let (asset, amount) = validation::expect_invariant(env, merged.get(i));
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);

        let position = account
            .borrow_positions
            .get(asset.clone())
            .unwrap_or_else(|| panic_with_error!(env, CollateralError::DebtPositionNotFound));

        let actual_debt = Ray::from_raw(position.scaled_amount_ray)
            .mul(env, Ray::from_raw(market_index.borrow_index_ray))
            .to_asset(feed.asset_decimals);

        let mut payment_amount = amount;
        if payment_amount > actual_debt {
            let excess = payment_amount - actual_debt;
            refunds.push_back((asset.clone(), excess));
            payment_amount = actual_debt;
        }

        let payment_wad = Wad::from_token(payment_amount, feed.asset_decimals);
        let payment_usd = payment_wad.mul(env, Wad::from_raw(feed.price_wad));

        total_repaid_usd += payment_usd;
        repaid_tokens.push_back(RepayEntry {
            asset,
            amount: payment_amount,
            usd_wad: payment_usd.raw(),
            feed,
            market_index,
        });
    }

    (total_repaid_usd, repaid_tokens)
}
pub(crate) fn account_dust_floors(cache: &mut ControllerCache, account: &Account) -> (i128, i128) {
    let mut min_collat: i128 = 0;
    for asset in account.supply_positions.keys() {
        let f = cache.cached_asset_config(&asset).min_collat_floor_usd_wad;
        if f > min_collat {
            min_collat = f;
        }
    }
    let mut min_debt: i128 = 0;
    for asset in account.borrow_positions.keys() {
        let f = cache.cached_asset_config(&asset).min_debt_floor_usd_wad;
        if f > min_debt {
            min_debt = f;
        }
    }
    (min_collat, min_debt)
}
pub(crate) fn expand_to_full_close_on_dust_residue(
    env: &Env,
    cache: &mut ControllerCache,
    account: &Account,
    total_debt: Wad,
    total_collateral: Wad,
    bonus: Bps,
    // Upper bound on what the liquidator has actually delivered. Dust
    // expansion may never price seizure beyond this amount — otherwise
    // the liquidator would get full-close collateral for a partial
    // payment.
    payment_ceiling_usd: Wad,
    repay_usd: &mut Wad,
) {
    let (min_collat_floor, min_debt_floor) = account_dust_floors(cache, account);

    let one_plus_bonus = Wad::ONE + bonus.to_wad(env);
    let seizure_usd = repay_usd.mul(env, one_plus_bonus);

    let residual_debt = if *repay_usd >= total_debt {
        Wad::ZERO
    } else {
        total_debt - *repay_usd
    };
    let residual_collateral = if seizure_usd >= total_collateral {
        Wad::ZERO
    } else {
        total_collateral - seizure_usd
    };

    let leaves_debt_dust = residual_debt > Wad::ZERO && residual_debt.raw() < min_debt_floor;
    let leaves_collat_dust =
        residual_collateral > Wad::ZERO && residual_collateral.raw() < min_collat_floor;

    if !(leaves_debt_dust || leaves_collat_dust) {
        return;
    }

    // Full close is only safe when the liquidator has covered the
    // entire debt — otherwise the post-state still leaves sub-floor
    // residue on at least one side. Two cases:
    //
    //   1. Payment covers total debt → expand to a real full close.
    //      Per-asset seizure capping clamps any overshoot.
    //   2. Payment is short → no partial expansion is dust-safe.
    //      Reject with `DustResidueNotAllowed`; the liquidator must
    //      either pay the full debt or pick an amount that doesn't
    //      trip the floor.
    if payment_ceiling_usd >= total_debt {
        *repay_usd = total_debt;
    } else {
        panic_with_error!(env, CollateralError::DustResidueNotAllowed);
    }
}
pub(crate) fn calculate_liquidation_amounts(
    env: &Env,
    total_debt: Wad,
    total_collateral: Wad,
    weighted_coll: Wad,
    proportion_seized: Wad,
    bonus_params: (Bps, Bps),
    hf: Wad,
    total_payment_usd: Wad,
) -> (Wad, Wad, Bps) {
    let (base_bonus, max_bonus) = bonus_params;
    let (ideal_repayment_usd, bonus) = helpers::estimate_liquidation_amount(
        env,
        total_debt,
        weighted_coll,
        hf,
        base_bonus,
        max_bonus,
        proportion_seized,
        total_collateral,
    );

    let final_repayment_usd = total_payment_usd.min(ideal_repayment_usd);
    let seizure_multiplier = Wad::ONE + bonus.to_wad(env);
    let total_seizure_usd = final_repayment_usd.mul(env, seizure_multiplier);

    (final_repayment_usd, total_seizure_usd, bonus)
}
pub(crate) fn calculate_seized_collateral(
    env: &Env,
    account: &Account,
    total_collateral: Wad,
    repayment_usd: Wad,
    bonus: Bps,
    cache: &mut ControllerCache,
) -> Vec<SeizeEntry> {
    let mut seized: Vec<SeizeEntry> = Vec::new(env);
    if total_collateral <= Wad::ZERO {
        return seized;
    }

    let one_plus_bonus = Wad::ONE + bonus.to_wad(env);
    let total_seizure_usd = repayment_usd.mul(env, one_plus_bonus);

    for (asset, position) in account.supply_positions.iter() {
        let feed = cache.cached_price(&asset);
        if feed.price_wad == 0 {
            continue;
        }

        let asset_config = cache.cached_asset_config(&asset);
        let market_index = cache.cached_market_index(&asset);

        let actual_ray = Ray::from_raw(position.scaled_amount_ray)
            .mul(env, Ray::from_raw(market_index.supply_index_ray));
        let actual_amount_wad = actual_ray.to_wad();
        let asset_value = actual_amount_wad.mul(env, Wad::from_raw(feed.price_wad));

        let share = asset_value.div(env, total_collateral);
        let seizure_for_asset_usd = total_seizure_usd.mul(env, share);

        let seizure_amount_wad = seizure_for_asset_usd.div(env, Wad::from_raw(feed.price_wad));
        let seizure_ray = seizure_amount_wad.to_ray();

        if seizure_ray <= Ray::ZERO {
            continue;
        }

        let capped_ray = seizure_ray.min(actual_ray);
        if capped_ray <= Ray::ZERO {
            continue;
        }

        // Split the seized RAY amount into base and bonus before computing
        // the protocol fee. Floor division keeps the base side as the lower
        // bound, so the bonus side captures any rounding remainder.
        let base_ray = capped_ray.div_floor(env, one_plus_bonus.to_ray());
        let bonus_ray = capped_ray - base_ray;
        let protocol_fee =
            Bps::from_raw(asset_config.liquidation_fees_bps).apply_to_ray(env, bonus_ray);
        let capped_amount = capped_ray.to_asset(feed.asset_decimals);
        if capped_amount <= 0 {
            continue;
        }

        seized.push_back(SeizeEntry {
            asset,
            amount: capped_amount,
            protocol_fee: protocol_fee.to_asset(feed.asset_decimals),
            feed,
            market_index,
        });
    }

    seized
}
pub(crate) fn process_excess_payment(
    env: &Env,
    repaid_tokens: &mut Vec<RepayEntry>,
    refunds: &mut Vec<Payment>,
    excess_usd: Wad,
) {
    let mut remaining_excess_usd = excess_usd;

    while remaining_excess_usd > Wad::ZERO && !repaid_tokens.is_empty() {
        let current_index = repaid_tokens.len() - 1;
        let entry = validation::expect_invariant(env, repaid_tokens.get(current_index));

        let usd = Wad::from_raw(entry.usd_wad);

        if usd > remaining_excess_usd {
            let ratio = remaining_excess_usd.div(env, usd);
            let refund_amount = Wad::from_token(entry.amount, entry.feed.asset_decimals)
                .mul(env, ratio)
                .to_token(entry.feed.asset_decimals);

            let new_amount = entry.amount - refund_amount;
            // Recompute `new_usd` from `new_amount * price`. Subtracting the
            // excess directly lets the two precision paths drift and leaves
            // the RepayEntry pair inconsistent for downstream consumers.
            let new_amount_wad = Wad::from_token(new_amount, entry.feed.asset_decimals);
            let new_usd = new_amount_wad.mul(env, Wad::from_raw(entry.feed.price_wad));

            refunds.push_back((entry.asset.clone(), refund_amount));
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
            refunds.push_back((entry.asset, entry.amount));
            repaid_tokens.remove(current_index);
            remaining_excess_usd -= usd;
        }
    }
}
