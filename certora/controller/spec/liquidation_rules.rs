use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

use crate::constants::{
    BAD_DEBT_USD_THRESHOLD, BPS, DEFAULT_HF_FOR_MAX_BONUS_WAD, DEFAULT_LIQUIDATION_TARGET_HF_WAD,
    WAD,
};
use crate::spec::fixture;
use crate::types::{AccountPositionType, HubAssetKey};
use common::math::fp::{Bps, Wad};
use common::math::fp_core::{mul_div_floor, mul_div_half_up};

const MAX_DEBT_AMOUNT_RAW: i128 = 1_000_000_000_000;

fn default_curve() -> crate::positions::liquidation::curve::LiquidationCurve {
    crate::positions::liquidation::curve::LiquidationCurve::from_config(
        &common::types::SpokeConfig {
            is_deprecated: false,
            liquidation_target_hf_wad: crate::constants::DEFAULT_LIQUIDATION_TARGET_HF_WAD,
            hf_for_max_bonus_wad: crate::constants::DEFAULT_HF_FOR_MAX_BONUS_WAD,
            liquidation_bonus_factor_bps: crate::constants::DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
        },
    )
}

#[rule]
fn liquidation_does_not_increase_repaid_debt(
    e: Env,
    liquidator: Address,
    owner: Address,
    debt_asset: Address,
    collateral_asset: Address,
    debt_amount: i128,
    scaled_debt_before: i128,
) {
    let account_id: u64 = 1;

    cvlr_assume!(debt_amount > 0);
    cvlr_assume!(debt_amount <= MAX_DEBT_AMOUNT_RAW);
    cvlr_assume!(owner != liquidator);
    cvlr_assume!(scaled_debt_before > 0 && scaled_debt_before <= 20 * common::constants::RAY);
    crate::spec::fixture::seed_live_account(&e, account_id, &owner, &debt_asset);
    crate::spec::fixture::seed_market(&e, &collateral_asset);
    fixture::seed_empty_books(&e, account_id);
    crate::spec::fixture::seed_debt_position(&e, account_id, &debt_asset, scaled_debt_before);
    // A collateralized debt book is required for liquidation to be able to
    // execute (the seize path), keeping the transition meaningful.
    crate::spec::fixture::seed_supply_position(
        &e,
        account_id,
        &collateral_asset,
        10 * common::constants::RAY,
    );

    let mut payments: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    payments.push_back((
        HubAssetKey {
            hub_id: crate::spec::fixture::HUB_ID,
            asset: debt_asset.clone(),
        },
        debt_amount,
    ));

    crate::positions::liquidation::process_liquidation(
        &e,
        &liquidator,
        account_id,
        &payments,
        crate::types::SeizeMode::Transfer,
    );

    let borrow_post =
        crate::storage::get_position(&e, account_id, AccountPositionType::Borrow, &debt_asset);
    match borrow_post {
        Some(pos) => cvlr_assert!(pos.scaled_amount <= scaled_debt_before),
        None => cvlr_assert!(true),
    }
}

#[rule]
fn liquidation_does_not_increase_seized_collateral(
    e: Env,
    liquidator: Address,
    owner: Address,
    collateral_asset: Address,
    debt_asset: Address,
    debt_amount: i128,
    scaled_col_before: i128,
    scaled_debt_before: i128,
) {
    let account_id: u64 = 1;

    cvlr_assume!(debt_amount > 0);
    cvlr_assume!(debt_amount <= MAX_DEBT_AMOUNT_RAW);
    cvlr_assume!(owner != liquidator);
    cvlr_assume!(scaled_col_before > 0 && scaled_col_before <= 20 * common::constants::RAY);
    cvlr_assume!(scaled_debt_before > 0 && scaled_debt_before <= 20 * common::constants::RAY);
    crate::spec::fixture::seed_live_account(&e, account_id, &owner, &collateral_asset);
    crate::spec::fixture::seed_market(&e, &debt_asset);
    fixture::seed_empty_books(&e, account_id);
    crate::spec::fixture::seed_supply_position(
        &e,
        account_id,
        &collateral_asset,
        scaled_col_before,
    );
    crate::spec::fixture::seed_debt_position(&e, account_id, &debt_asset, scaled_debt_before);

    let mut payments: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    payments.push_back((
        HubAssetKey {
            hub_id: crate::spec::fixture::HUB_ID,
            asset: debt_asset,
        },
        debt_amount,
    ));

    crate::positions::liquidation::process_liquidation(
        &e,
        &liquidator,
        account_id,
        &payments,
        crate::types::SeizeMode::Transfer,
    );

    let supply_post = crate::storage::get_position(
        &e,
        account_id,
        AccountPositionType::Deposit,
        &collateral_asset,
    );
    match supply_post {
        Some(pos) => cvlr_assert!(pos.scaled_amount <= scaled_col_before),
        None => cvlr_assert!(true),
    }
}

#[rule]
fn bonus_bounded(
    e: Env,
    hf_wad: i128,
    base_bonus_bps: i128,
    max_bonus_bps: i128,
    target_wad: i128,
) {
    cvlr_assume!(base_bonus_bps >= 0);
    cvlr_assume!(max_bonus_bps >= base_bonus_bps);
    cvlr_assume!(max_bonus_bps <= BPS);
    cvlr_assume!(hf_wad >= 0);
    cvlr_assume!(hf_wad < WAD);
    cvlr_assume!(target_wad > 0 && target_wad <= 2 * WAD);

    let curve = default_curve();
    let bonus = crate::positions::liquidation::curve::calculate_linear_bonus_with_target(
        &e,
        Wad::from(hf_wad),
        Bps::from(base_bonus_bps),
        Bps::from(max_bonus_bps),
        &curve,
        Wad::from(target_wad),
    );

    cvlr_assert!(bonus.raw() <= max_bonus_bps);
    cvlr_assert!(bonus.raw() >= base_bonus_bps);
}

#[rule]
fn derived_bonus_respects_threshold(e: Env, proportion_seized_wad: i128) {
    cvlr_assume!(proportion_seized_wad > 0);
    cvlr_assume!(proportion_seized_wad <= WAD);

    let max = crate::positions::liquidation::curve::max_bonus_for_threshold(
        &e,
        Wad::from(proportion_seized_wad),
    );

    let eff_thr_bps = ((proportion_seized_wad * BPS + (WAD - 1)) / WAD).clamp(1, BPS);

    cvlr_assert!(eff_thr_bps * (BPS + max.raw()) <= BPS * BPS);
}

#[rule]
fn ideal_repayment_targets_curve_hf(
    e: Env,
    total_debt_wad: i128,
    weighted_collateral_wad: i128,
    base_bonus_bps: i128,
    max_bonus_bps: i128,
) {
    cvlr_assume!(total_debt_wad > 0);
    cvlr_assume!(total_debt_wad <= 1_000_000 * WAD);
    cvlr_assume!(weighted_collateral_wad > 0);
    cvlr_assume!(weighted_collateral_wad < total_debt_wad);
    cvlr_assume!(base_bonus_bps > 0);
    cvlr_assume!(base_bonus_bps <= 500);
    cvlr_assume!(max_bonus_bps >= base_bonus_bps);
    cvlr_assume!(max_bonus_bps <= BPS);

    let proportion_seized_wad = mul_div_half_up(&e, weighted_collateral_wad, WAD, total_debt_wad);
    let hf_wad = Wad::from(weighted_collateral_wad)
        .div_floor(&e, Wad::from(total_debt_wad))
        .raw();
    cvlr_assume!(hf_wad > 0);
    cvlr_assume!(hf_wad < WAD);
    let total_collateral_wad = total_debt_wad;

    let snap = crate::positions::liquidation::curve::LiquidationSnapshot {
        total_debt: Wad::from(total_debt_wad),
        total_collateral: Wad::from(total_collateral_wad),
        weighted_collateral: Wad::from(weighted_collateral_wad),
        proportion_seized: Wad::from(proportion_seized_wad),
        hf: Wad::from(hf_wad),
    };
    let bounds = crate::positions::liquidation::curve::BonusBounds {
        base: Bps::from(base_bonus_bps),
        max: Bps::from(max_bonus_bps),
    };
    let curve = default_curve();
    let (ideal, bonus) = crate::positions::liquidation::curve::estimate_liquidation_amount(
        &e, &snap, bounds, &curve,
    );

    cvlr_assert!(ideal.raw() > 0);
    cvlr_assert!(ideal.raw() <= total_debt_wad);

    let bonus_wad = bonus.to_wad(&e);
    let one_plus_bonus = Wad::ONE.checked_add(&e, bonus_wad);
    let max_repayable = Wad::from(total_collateral_wad).div(&e, one_plus_bonus);
    cvlr_assert!(ideal.raw() <= max_repayable.raw() + 1);
}

#[rule]
fn liquidation_bonus_sanity(e: Env) {
    let hf = WAD / 2;
    let base = 500;
    let max = 1_000;
    let target = DEFAULT_LIQUIDATION_TARGET_HF_WAD;

    let curve = default_curve();
    let bonus = crate::positions::liquidation::curve::calculate_linear_bonus_with_target(
        &e,
        Wad::from(hf),
        Bps::from(base),
        Bps::from(max),
        &curve,
        Wad::from(target),
    );
    let _bonus = bonus;
    cvlr_satisfy!(true);
}

#[rule]
fn bonus_monotone_in_hf(e: Env, hf_lo: i128, hf_hi: i128, base_bps: i128, max_bps: i128) {
    cvlr_assume!(hf_lo >= 0);
    cvlr_assume!(hf_lo <= hf_hi);
    cvlr_assume!(base_bps >= 0);
    cvlr_assume!(max_bps >= base_bps);
    cvlr_assume!(max_bps <= BPS);

    let curve = default_curve();
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);
    let bonus_lo = crate::positions::liquidation::curve::calculate_linear_bonus_with_target(
        &e,
        Wad::from(hf_lo),
        Bps::from(base_bps),
        Bps::from(max_bps),
        &curve,
        target,
    );
    let bonus_hi = crate::positions::liquidation::curve::calculate_linear_bonus_with_target(
        &e,
        Wad::from(hf_hi),
        Bps::from(base_bps),
        Bps::from(max_bps),
        &curve,
        target,
    );

    cvlr_assert!(bonus_lo.raw() >= bonus_hi.raw());
}

#[rule]
fn bonus_is_max_below_curve_floor(e: Env, hf: i128, base_bps: i128, max_bps: i128) {
    cvlr_assume!(hf >= 0);
    cvlr_assume!(hf <= DEFAULT_HF_FOR_MAX_BONUS_WAD);
    cvlr_assume!(base_bps >= 0);
    cvlr_assume!(max_bps >= base_bps);
    cvlr_assume!(max_bps <= BPS);

    let curve = default_curve();
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);
    let bonus = crate::positions::liquidation::curve::calculate_linear_bonus_with_target(
        &e,
        Wad::from(hf),
        Bps::from(base_bps),
        Bps::from(max_bps),
        &curve,
        target,
    );
    cvlr_assert!(bonus.raw() == max_bps);
}

#[rule]
fn bonus_is_base_at_or_above_target(e: Env, hf: i128, base_bps: i128, max_bps: i128) {
    cvlr_assume!(hf >= DEFAULT_LIQUIDATION_TARGET_HF_WAD);
    cvlr_assume!(base_bps >= 0);
    cvlr_assume!(max_bps >= base_bps);
    cvlr_assume!(max_bps <= BPS);

    let curve = default_curve();
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);
    let bonus = crate::positions::liquidation::curve::calculate_linear_bonus_with_target(
        &e,
        Wad::from(hf),
        Bps::from(base_bps),
        Bps::from(max_bps),
        &curve,
        target,
    );
    cvlr_assert!(bonus.raw() == base_bps);
}

#[rule]
fn estimate_leaves_no_sub_threshold_dust(
    e: Env,
    total_debt_wad: i128,
    weighted_collateral_wad: i128,
    base_bonus_bps: i128,
    max_bonus_bps: i128,
) {
    cvlr_assume!(total_debt_wad > 0);
    cvlr_assume!(total_debt_wad <= 1_000_000 * WAD);
    cvlr_assume!(weighted_collateral_wad > 0);
    cvlr_assume!(weighted_collateral_wad < total_debt_wad);
    cvlr_assume!(base_bonus_bps > 0);
    cvlr_assume!(base_bonus_bps <= 500);
    cvlr_assume!(max_bonus_bps >= base_bonus_bps);
    cvlr_assume!(max_bonus_bps <= BPS);

    let proportion_seized_wad = mul_div_half_up(&e, weighted_collateral_wad, WAD, total_debt_wad);
    let hf_wad = Wad::from(weighted_collateral_wad)
        .div_floor(&e, Wad::from(total_debt_wad))
        .raw();
    cvlr_assume!(hf_wad > 0);
    cvlr_assume!(hf_wad < WAD);
    let snap = crate::positions::liquidation::curve::LiquidationSnapshot {
        total_debt: Wad::from(total_debt_wad),
        total_collateral: Wad::from(total_debt_wad),
        weighted_collateral: Wad::from(weighted_collateral_wad),
        proportion_seized: Wad::from(proportion_seized_wad),
        hf: Wad::from(hf_wad),
    };
    let bounds = crate::positions::liquidation::curve::BonusBounds {
        base: Bps::from(base_bonus_bps),
        max: Bps::from(max_bonus_bps),
    };
    let curve = default_curve();
    let (ideal, _bonus) = crate::positions::liquidation::curve::estimate_liquidation_amount(
        &e, &snap, bounds, &curve,
    );

    let remaining = total_debt_wad - ideal.raw();
    cvlr_assert!(remaining == 0 || remaining >= BAD_DEBT_USD_THRESHOLD);
}

#[rule]
fn estimate_liquidation_sanity(e: Env) {
    let total_debt = 2 * WAD;
    let weighted_col = WAD;
    let hf = WAD / 2;

    let snap = crate::positions::liquidation::curve::LiquidationSnapshot {
        total_debt: Wad::from(total_debt),
        total_collateral: Wad::from(total_debt),
        weighted_collateral: Wad::from(weighted_col),
        proportion_seized: Wad::from(WAD / 2),
        hf: Wad::from(hf),
    };
    let bounds = crate::positions::liquidation::curve::BonusBounds {
        base: Bps::from(500),
        max: Bps::from(1000),
    };
    let curve = default_curve();
    let (_ideal, _bonus) = crate::positions::liquidation::curve::estimate_liquidation_amount(
        &e, &snap, bounds, &curve,
    );
    cvlr_satisfy!(true);
}

#[rule]
fn liquidation_transition_sanity(
    e: Env,
    liquidator: Address,
    owner: Address,
    debt_asset: Address,
    collateral_asset: Address,
) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    cvlr_assume!(owner != liquidator);
    crate::spec::fixture::seed_live_account(&e, account_id, &owner, &debt_asset);
    crate::spec::fixture::seed_market(&e, &collateral_asset);
    crate::spec::fixture::seed_debt_position(
        &e,
        account_id,
        &debt_asset,
        10 * common::constants::RAY,
    );
    crate::spec::fixture::seed_supply_position(
        &e,
        account_id,
        &collateral_asset,
        10 * common::constants::RAY,
    );

    let mut payments: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    payments.push_back((
        HubAssetKey {
            hub_id: crate::spec::fixture::HUB_ID,
            asset: debt_asset,
        },
        WAD,
    ));
    crate::positions::liquidation::process_liquidation(
        &e,
        &liquidator,
        account_id,
        &payments,
        crate::types::SeizeMode::Transfer,
    );
    cvlr_satisfy!(true);
}

// --- V-6: splitting a close into partials is never more profitable ---------
//
// CS-AAVE4-009 against Aave: when `proportion_seized * (1 + bonus) > HF`, every
// partial liquidation lowers the health factor, the bonus curve pays more at
// the lower health factor, and N slices extract more collateral than one close
// of the summed repayment. Aave forbade the configuration off-chain. We clamp
// at runtime: `max_hf_preserving_bonus_bps` caps the bonus at
// `HF / proportion_seized - 1`, the exact rate that leaves the health factor
// unchanged, so the next slice can never be priced better than this one.
//
// These rules work at the plan-math level, on the same
// `estimate_liquidation_amount` the plan calls, with prices and indexes held
// fixed. Two steps suffice: `split_liq_bonus_never_ratchets_up_across_a_partial`
// is the induction step, and with it the N-step chain telescopes onto the
// first step's rate.

/// An account's liquidation-relevant totals, all WAD except the bonus.
#[derive(Clone, Copy)]
struct SplitBook {
    debt: i128,
    collateral: i128,
    weighted: i128,
    /// The USD-weighted average of the collateral legs' configured bonuses,
    /// before `get_account_bonus_params` clamps it to the derived max.
    base_bps: i128,
}

/// What one plan-math step quotes for `book`.
#[derive(Clone, Copy)]
struct SplitQuote {
    ideal: i128,
    bonus_bps: i128,
    hf_wad: i128,
}

/// Runs `estimate_liquidation_amount` over `book` exactly as
/// `build_liquidation_plan` does: `proportion_seized` from the weighted share of
/// collateral, the health factor floored, the max bonus derived from the
/// proportion, and the base bonus clamped to it.
///
/// Assumes the book is liquidatable. `build_liquidation_plan` rejects a health
/// factor at or above one WAD before the curve is ever consulted, so a step on
/// such a book does not exist.
fn split_liq_quote(e: &Env, book: SplitBook) -> SplitQuote {
    cvlr_assume!(book.debt > 0 && book.debt <= 1_000_000 * WAD);
    cvlr_assume!(book.collateral > 0 && book.collateral <= 1_000_000 * WAD);
    cvlr_assume!(book.weighted > 0 && book.weighted <= book.collateral);

    let proportion_wad = mul_div_half_up(e, book.weighted, WAD, book.collateral);
    cvlr_assume!(proportion_wad > 0 && proportion_wad <= WAD);
    let hf_wad = Wad::from(book.weighted)
        .div_floor(e, Wad::from(book.debt))
        .raw();
    cvlr_assume!(hf_wad > 0 && hf_wad < WAD);

    let snap = crate::positions::liquidation::curve::LiquidationSnapshot {
        total_debt: Wad::from(book.debt),
        total_collateral: Wad::from(book.collateral),
        weighted_collateral: Wad::from(book.weighted),
        proportion_seized: Wad::from(proportion_wad),
        hf: Wad::from(hf_wad),
    };
    let max =
        crate::positions::liquidation::curve::max_bonus_for_threshold(e, Wad::from(proportion_wad));
    // `get_account_bonus_params` clamps the weighted average to the derived max.
    let base = Bps::from(if book.base_bps <= max.raw() {
        book.base_bps
    } else {
        max.raw()
    });
    let bounds = crate::positions::liquidation::curve::BonusBounds { base, max };
    let (ideal, bonus) = crate::positions::liquidation::curve::estimate_liquidation_amount(
        e,
        &snap,
        bounds,
        &default_curve(),
    );

    SplitQuote {
        ideal: ideal.raw(),
        bonus_bps: bonus.raw(),
        hf_wad,
    }
}

/// Collateral value seized for `repay` at `bonus_bps`, floored — the plan sizes
/// the seizure as `repay_usd * (1 + bonus)`.
fn split_liq_seizure(e: &Env, repay: i128, bonus_bps: i128) -> i128 {
    mul_div_floor(e, repay, BPS + bonus_bps, BPS)
}

/// The book left behind after repaying `repay` at `bonus_bps`.
///
/// Seizure is pro-rata by USD value, so it removes the same fraction of every
/// collateral leg: the weighted collateral falls by `seize * weighted /
/// collateral`, and the asset mix — hence the derived max bonus and the
/// weighted-average base — is preserved.
///
/// Assumes the step leaves both a book and something to liquidate next: a
/// seizure that consumes the collateral, or a repayment that clears the debt,
/// ends the chain rather than continuing it.
fn split_liq_apply(e: &Env, book: SplitBook, repay: i128, bonus_bps: i128) -> (SplitBook, i128) {
    let seize = split_liq_seizure(e, repay, bonus_bps);
    cvlr_assume!(seize > 0 && seize < book.collateral);
    cvlr_assume!(repay > 0 && repay < book.debt);

    let weighted_out = mul_div_floor(e, seize, book.weighted, book.collateral);
    cvlr_assume!(weighted_out < book.weighted);

    let next = SplitBook {
        debt: book.debt - repay,
        collateral: book.collateral - seize,
        weighted: book.weighted - weighted_out,
        base_bps: book.base_bps,
    };
    (next, seize)
}

/// Two sequential partial liquidations never seize more collateral value than
/// one liquidation repaying their sum.
///
/// Both slices are assumed to fit inside their own step's ideal amount, which
/// is what `normalize_repayment_plan` accepts whole; anything above it is capped
/// or refunded, which only lowers the seizure.
#[rule]
fn split_liq_two_partials_never_out_seize_one_close(
    e: Env,
    total_debt_wad: i128,
    total_collateral_wad: i128,
    weighted_collateral_wad: i128,
    base_bonus_bps: i128,
    repay_1: i128,
    repay_2: i128,
) {
    cvlr_assume!(base_bonus_bps > 0 && base_bonus_bps <= 500);
    let book_0 = SplitBook {
        debt: total_debt_wad,
        collateral: total_collateral_wad,
        weighted: weighted_collateral_wad,
        base_bps: base_bonus_bps,
    };
    let quote_0 = split_liq_quote(&e, book_0);

    cvlr_assume!(repay_1 > 0);
    cvlr_assume!(repay_2 > 0);
    cvlr_assume!(repay_1 + repay_2 <= quote_0.ideal);

    let (book_1, seize_1) = split_liq_apply(&e, book_0, repay_1, quote_0.bonus_bps);
    let quote_1 = split_liq_quote(&e, book_1);
    cvlr_assume!(repay_2 <= quote_1.ideal);
    let seize_2 = split_liq_seizure(&e, repay_2, quote_1.bonus_bps);

    let one_close = split_liq_seizure(&e, repay_1 + repay_2, quote_0.bonus_bps);

    cvlr_assert!(seize_1 + seize_2 <= one_close);
}

/// The induction step: a partial liquidation never leaves the account priced at
/// a better bonus than it was priced at going in.
///
/// This is what makes the two-step bound generalize to N. Without the clamp the
/// seizure outruns the health factor, the curve pays more at the lower health
/// factor, and this fails on the second step.
#[rule]
fn split_liq_bonus_never_ratchets_up_across_a_partial(
    e: Env,
    total_debt_wad: i128,
    total_collateral_wad: i128,
    weighted_collateral_wad: i128,
    base_bonus_bps: i128,
    repay_1: i128,
) {
    cvlr_assume!(base_bonus_bps > 0 && base_bonus_bps <= 500);
    let book_0 = SplitBook {
        debt: total_debt_wad,
        collateral: total_collateral_wad,
        weighted: weighted_collateral_wad,
        base_bps: base_bonus_bps,
    };
    let quote_0 = split_liq_quote(&e, book_0);
    cvlr_assume!(repay_1 > 0 && repay_1 <= quote_0.ideal);

    let (book_1, _seize_1) = split_liq_apply(&e, book_0, repay_1, quote_0.bonus_bps);
    let quote_1 = split_liq_quote(&e, book_1);

    cvlr_assert!(quote_1.bonus_bps <= quote_0.bonus_bps);
}

/// The never-recovering path: the same bound, restricted to chains whose health
/// factor is strictly worse after the first slice.
///
/// That branch is only reachable where the health-factor-preserving ceiling is
/// already negative — an insolvent book, where the plan pays the base bonus and
/// `normalize_repayment_plan` still admits a partial. The base bonus is a
/// constant of the collateral mix, which pro-rata seizure preserves, so the
/// chain stays exactly additive even while the health factor erodes.
#[rule]
fn split_liq_chain_bound_holds_when_health_never_recovers(
    e: Env,
    total_debt_wad: i128,
    total_collateral_wad: i128,
    weighted_collateral_wad: i128,
    base_bonus_bps: i128,
    repay_1: i128,
    repay_2: i128,
) {
    cvlr_assume!(base_bonus_bps > 0 && base_bonus_bps <= 500);
    let book_0 = SplitBook {
        debt: total_debt_wad,
        collateral: total_collateral_wad,
        weighted: weighted_collateral_wad,
        base_bps: base_bonus_bps,
    };
    let quote_0 = split_liq_quote(&e, book_0);

    cvlr_assume!(repay_1 > 0);
    cvlr_assume!(repay_2 > 0);
    cvlr_assume!(repay_1 + repay_2 <= quote_0.ideal);

    let (book_1, seize_1) = split_liq_apply(&e, book_0, repay_1, quote_0.bonus_bps);
    let quote_1 = split_liq_quote(&e, book_1);
    // The eroding branch: this is the shape CS-AAVE4-009 exploited.
    cvlr_assume!(quote_1.hf_wad < quote_0.hf_wad);
    cvlr_assume!(repay_2 <= quote_1.ideal);
    let seize_2 = split_liq_seizure(&e, repay_2, quote_1.bonus_bps);

    let one_close = split_liq_seizure(&e, repay_1 + repay_2, quote_0.bonus_bps);

    cvlr_assert!(seize_1 + seize_2 <= one_close);
}
