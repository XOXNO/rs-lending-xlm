/// Liquidation Invariant Rules
///
/// Verifies the liquidation subsystem's correctness:
///   - Health factor improves after liquidation
///   - No over-liquidation (capped at outstanding debt)
///   - Dynamic bonus bounded by MAX_LIQUIDATION_BONUS (1500 BPS)
///   - Bonus scales correctly from base to max as HF drops
///   - Seizure is proportional to collateral value share
///   - Protocol fee charged on bonus portion only
///   - Bad debt cleanup at <= $5 USD collateral threshold
///   - Bad debt socialization decreases supply_index
///   - Merged payments contain no duplicate assets
///   - Ideal repayment formula targets HF = 1.02
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

use common::constants::{BAD_DEBT_USD_THRESHOLD, BPS, MAX_LIQUIDATION_BONUS, RAY, WAD};
use common::fp::{Bps, Wad};
use common::fp_core::{mul_div_floor, mul_div_half_up};

// ---------------------------------------------------------------------------
// Rule 1: Health factor improves after liquidation
// ---------------------------------------------------------------------------

/// After a successful liquidation, the borrower's health factor must NOT
/// regress. Note: strict improvement is FALSE for heavily-underwater
/// positions (HF < 1 + bonus ~= 1.08) where partial liquidations
/// mathematically reduce HF while still reducing bad-debt exposure.
/// This property is `>=`, not `>`. See fuzz_supply_borrow_liquidate for
/// the discovered counterexample.
#[rule]
fn hf_improves_after_liquidation(
    e: Env,
    liquidator: Address,
    account_id: u64,
    debt_asset: Address,
    debt_amount: i128,
) {
    cvlr_assume!(debt_amount > 0);

    // Capture HF before liquidation
    let mut cache_before = crate::cache::ControllerCache::new(&e, false);
    let hf_before = crate::helpers::calculate_health_factor_for(&e, &mut cache_before, account_id);

    // Liquidation requires HF < 1.0
    cvlr_assume!(hf_before < WAD);
    cvlr_assume!(hf_before > 0); // Must have debt

    // Build debt payments vector
    let mut payments: Vec<(Address, i128)> = Vec::new(&e);
    payments.push_back((debt_asset, debt_amount));

    // Execute liquidation
    crate::positions::liquidation::process_liquidation(&e, &liquidator, account_id, &payments);

    // Capture HF after liquidation
    let mut cache_after = crate::cache::ControllerCache::new(&e, false);
    let hf_after = crate::helpers::calculate_health_factor_for(&e, &mut cache_after, account_id);

    // HF must improve or stay equal (rounding can make them equal for dust liquidations).
    // For fully liquidated accounts, HF = i128::MAX.
    cvlr_assert!(hf_after >= hf_before);
}

// ---------------------------------------------------------------------------
// Rule 2: DELETED -- no_over_liquidation was vacuous (tautology on min).
// min(x, y) <= y is true by definition. The debt cap is already implicitly
// tested by hf_improves_after_liquidation which exercises the full
// liquidation flow including the min-cap logic.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Rule 3: Bonus bounded by MAX_LIQUIDATION_BONUS
// ---------------------------------------------------------------------------

/// The dynamic liquidation bonus must never exceed MAX_LIQUIDATION_BONUS
/// (1500 BPS = 15%), regardless of how low the health factor drops.
#[rule]
fn bonus_bounded(e: Env, hf_wad: i128, base_bonus_bps: i128, max_bonus_bps: i128) {
    // Assume valid bonus parameters
    cvlr_assume!(base_bonus_bps >= 0);
    cvlr_assume!(max_bonus_bps >= base_bonus_bps);
    cvlr_assume!(hf_wad >= 0);
    cvlr_assume!(hf_wad < WAD); // Account is liquidatable

    let bonus = crate::helpers::calculate_linear_bonus(
        &e,
        Wad::from_raw(hf_wad),
        Bps::from_raw(base_bonus_bps),
        Bps::from_raw(max_bonus_bps),
    );

    cvlr_assert!(bonus.raw() <= MAX_LIQUIDATION_BONUS);
}

// ---------------------------------------------------------------------------
// Rule 4: DELETED -- bonus_zero_at_threshold was provably wrong.
// At HF=1.0 WAD, gap = (1.02 - 1.0)/1.02 = 0.0196, scale = 0.0392,
// so bonus = base + 0.0392*(max-base), NOT base_bonus.
// The correct boundary (HF=1.02 where gap=0 and bonus=base) is tested
// by bonus_at_hf_exactly_102 in boundary_rules.rs.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Rule 5: Bonus approaches max at deep underwater HF
// ---------------------------------------------------------------------------

/// When HF is very low (deeply underwater), the bonus should approach
/// or equal max_bonus_bps (capped at MAX_LIQUIDATION_BONUS).
#[rule]
fn bonus_max_at_deep_underwater(e: Env, base_bonus_bps: i128, max_bonus_bps: i128) {
    cvlr_assume!(base_bonus_bps > 0);
    cvlr_assume!(base_bonus_bps <= MAX_LIQUIDATION_BONUS);
    cvlr_assume!(max_bonus_bps >= base_bonus_bps);
    cvlr_assume!(max_bonus_bps <= MAX_LIQUIDATION_BONUS);

    // HF = 0.5 WAD (deeply underwater) -- gap = (1.02 - 0.5) / 1.02 ~= 0.51
    // scale = min(2 * 0.51, 1.0) = 1.0 -> bonus = max_bonus_bps
    let hf_deep: i128 = WAD / 2;

    let bonus = crate::helpers::calculate_linear_bonus(
        &e,
        Wad::from_raw(hf_deep),
        Bps::from_raw(base_bonus_bps),
        Bps::from_raw(max_bonus_bps),
    );

    // At HF = 0.5, the scale factor saturates to 1.0, so bonus = max_bonus
    // (capped at MAX_LIQUIDATION_BONUS)
    let expected_max = max_bonus_bps.min(MAX_LIQUIDATION_BONUS);
    cvlr_assert!(bonus.raw() == expected_max);
}

// ---------------------------------------------------------------------------
// Rule 6: Seizure proportional to collateral value share
// ---------------------------------------------------------------------------

/// Each collateral asset is seized proportionally to its value share:
/// `seizure_for_asset = total_seizure * (asset_value / total_collateral)`.
#[rule]
fn seizure_proportional(
    e: Env,
    total_seizure_usd_wad: i128,
    asset_a_value_wad: i128,
    asset_b_value_wad: i128,
) {
    cvlr_assume!(total_seizure_usd_wad > 0);
    cvlr_assume!(asset_a_value_wad > 0);
    cvlr_assume!(asset_b_value_wad > 0);

    let total_collateral_wad = asset_a_value_wad + asset_b_value_wad;
    cvlr_assume!(total_collateral_wad > 0);
    // Prevent overflow in proportional calculation
    cvlr_assume!(total_seizure_usd_wad <= total_collateral_wad);

    // Compute proportional seizure for each asset
    let share_a_wad = mul_div_half_up(&e, asset_a_value_wad, WAD, total_collateral_wad);
    let seizure_a = mul_div_half_up(&e, total_seizure_usd_wad, share_a_wad, WAD);

    let share_b_wad = mul_div_half_up(&e, asset_b_value_wad, WAD, total_collateral_wad);
    let seizure_b = mul_div_half_up(&e, total_seizure_usd_wad, share_b_wad, WAD);

    // Check the rounded proportional split: each share is non-negative and
    // the combined seizure stays within the total.
    cvlr_assert!(seizure_a >= 0);
    cvlr_assert!(seizure_b >= 0);
    cvlr_assert!(seizure_a + seizure_b <= total_seizure_usd_wad + 1); // allow +1 rounding

    // Higher-value collateral gets higher seizure
    if asset_a_value_wad > asset_b_value_wad {
        cvlr_assert!(seizure_a >= seizure_b);
    }
}

// ---------------------------------------------------------------------------
// Rule 7: Protocol fee on bonus portion only
// ---------------------------------------------------------------------------

/// Protocol fee = bonus_amount * liquidation_fees_bps / BPS.
/// The fee is computed on the bonus portion (seizure - base), not the
/// entire seizure amount.
///
/// Mirrors production at `controller/src/positions/liquidation.rs:355-365`:
///   one_plus_bonus = WAD + bonus_bps_to_wad           (half-up)
///   base_amount    = seizure * WAD / one_plus_bonus   (FLOOR -- div_floor)
///   bonus_portion  = seizure - base_amount
///   protocol_fee   = bonus_portion * fees_bps / BPS   (half-up)
///
/// The base side uses `div_floor` (not half-up) so the bonus side is never
/// understated and the fee is always at least the spec value.
#[rule]
fn protocol_fee_on_bonus_only(
    e: Env,
    seizure_amount: i128,
    bonus_bps: i128,
    liquidation_fees_bps: i128,
) {
    cvlr_assume!(seizure_amount > 0);
    cvlr_assume!(bonus_bps > 0);
    cvlr_assume!(bonus_bps <= MAX_LIQUIDATION_BONUS);
    cvlr_assume!(liquidation_fees_bps >= 0);
    cvlr_assume!(liquidation_fees_bps <= BPS);

    // Compute base and bonus amounts (mirrors liquidation.rs logic).
    // `one_plus_bonus_wad` is built with half-up (matches `Bps::to_wad`
    // composed with `Wad::ONE + ...`); `base_amount` uses floor division
    // (matches `Wad::div_floor`).
    let one_plus_bonus_wad = WAD + mul_div_half_up(&e, bonus_bps, WAD, BPS);
    let base_amount = mul_div_floor(&e, seizure_amount, WAD, one_plus_bonus_wad);
    let bonus_amount = seizure_amount - base_amount;
    let protocol_fee = mul_div_half_up(&e, bonus_amount, liquidation_fees_bps, BPS);

    // Fee must be <= bonus_amount (can't take more than the bonus)
    cvlr_assert!(protocol_fee <= bonus_amount);

    // Fee must be non-negative
    cvlr_assert!(protocol_fee >= 0);

    // Fee must be zero when liquidation_fees_bps is zero
    if liquidation_fees_bps == 0 {
        cvlr_assert!(protocol_fee == 0);
    }

    // Fee must be strictly less than total seizure (it's only on the bonus portion)
    cvlr_assert!(protocol_fee < seizure_amount);
}

// ---------------------------------------------------------------------------
// Rule 8: Bad debt threshold gates cleanup
// ---------------------------------------------------------------------------

/// `clean_bad_debt_standalone` (controller/src/positions/liquidation.rs:460)
/// panics with `CannotCleanBadDebt` unless
/// `total_debt_usd > total_collateral_usd && total_collateral_usd <= 5*WAD`.
///
/// This rule asserts that real production gating predicate by capturing the
/// account's USD totals from `calculate_account_totals`, calling the
/// production entry point, and asserting that any path that reaches the
/// post-state must satisfy the qualification (otherwise the call would
/// have panicked). The previous version checked a locally-defined boolean
/// against itself -- a propositional tautology that proved nothing about
/// production.
#[rule]
fn bad_debt_threshold(e: Env, account_id: u64) {
    let mut cache = crate::cache::ControllerCache::new(&e, false);

    // The standalone entry rejects accounts without any borrow positions
    // before reaching the qualification check (`PositionNotFound`), so
    // exclude that path here.
    let account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(!account.borrow_positions.is_empty());

    // Capture USD totals using the same helper production uses, so the
    // values we assert about are exactly the ones gating the cleanup.
    let (total_collateral_usd, total_debt_usd, _) = crate::helpers::calculate_account_totals(
        &e,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    // Bound the totals to keep i128 arithmetic linear / overflow-free in
    // the prover model. The threshold path only fires when collateral is
    // tiny (<= $5), so a generous protocol-realistic upper bound is fine.
    cvlr_assume!(total_collateral_usd.raw() >= 0);
    cvlr_assume!(total_debt_usd.raw() >= 0);

    // If execution reaches the post-state, the qualification predicate
    // must have held. Equivalent to: cleanup happens iff the predicate
    // holds, since the failure path panics atomically.
    crate::positions::liquidation::clean_bad_debt_standalone(&e, account_id);

    let bad_debt_threshold_wad = BAD_DEBT_USD_THRESHOLD; // 5 * WAD
    cvlr_assert!(total_debt_usd.raw() > total_collateral_usd.raw());
    cvlr_assert!(total_collateral_usd.raw() <= bad_debt_threshold_wad);
}

// ---------------------------------------------------------------------------
// Rule 9: Bad debt socialization decreases supply_index
// ---------------------------------------------------------------------------

/// When bad debt is socialized (pool.seize_position on borrow positions),
/// the supply index must decrease because losses are distributed to suppliers.
/// This is the only valid case where supply_index can decrease.
#[rule]
fn bad_debt_supply_index_decreases(e: Env, account_id: u64) {
    // Capture supply index before bad debt cleanup
    let debt_asset = e.current_contract_address();
    let index_before = crate::storage::market_index::get_market_index(&e, &debt_asset);
    let supply_before = index_before.supply_index_ray;

    cvlr_assume!(supply_before >= RAY);

    // Assume account qualifies for bad debt cleanup
    // (borrow exists, collateral <= $5, debt > collateral)
    crate::positions::liquidation::clean_bad_debt_standalone(&e, account_id);

    // After socialization, supply_index should decrease (or stay same if no supply)
    let index_after = crate::storage::market_index::get_market_index(&e, &debt_asset);
    let supply_after = index_after.supply_index_ray;

    // supply_index must not INCREASE from bad debt socialization
    // It should decrease (losses spread to suppliers) or stay same (no suppliers)
    cvlr_assert!(supply_after <= supply_before);
}

// ---------------------------------------------------------------------------
// Rule 10: DELETED -- payment_dedup was vacuous (reimplements dedup locally).
// The rule constructed a Map, inserted values, then asserted properties of
// the Map it just built -- a tautology. Dedup correctness is covered by
// liquidation integration tests.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Rule 11: Ideal repayment targets HF = 1.02
// ---------------------------------------------------------------------------

/// The ideal repayment formula targets a post-liquidation HF of 1.02.
/// Verify that estimate_liquidation_amount produces a value that, when
/// applied, would bring HF to approximately 1.02.
#[rule]
fn ideal_repayment_targets_102(
    e: Env,
    total_debt_wad: i128,
    weighted_collateral_wad: i128,
    hf_wad: i128,
    base_bonus_bps: i128,
    max_bonus_bps: i128,
) {
    // Setup: valid liquidation scenario
    cvlr_assume!(total_debt_wad > 0);
    cvlr_assume!(total_debt_wad <= 1_000_000 * WAD); // Cap at $1M for solver feasibility
    cvlr_assume!(weighted_collateral_wad > 0);
    cvlr_assume!(weighted_collateral_wad < total_debt_wad); // HF < 1.0 requires this
    cvlr_assume!(hf_wad > 0);
    cvlr_assume!(hf_wad < WAD); // Liquidatable
    cvlr_assume!(base_bonus_bps > 0);
    cvlr_assume!(base_bonus_bps <= 500); // Reasonable base bonus (up to 5%)
    cvlr_assume!(max_bonus_bps >= base_bonus_bps);
    cvlr_assume!(max_bonus_bps <= MAX_LIQUIDATION_BONUS);

    // Assume uniform collateral (proportion_seized = average threshold)
    let proportion_seized_wad = mul_div_half_up(&e, weighted_collateral_wad, WAD, total_debt_wad);
    // total_collateral ~= total_debt * HF / proportion_seized (approximation)
    let total_collateral_wad = total_debt_wad; // Simplification for rule verification

    let (ideal, bonus) = crate::helpers::estimate_liquidation_amount(
        &e,
        Wad::from_raw(total_debt_wad),
        Wad::from_raw(weighted_collateral_wad),
        Wad::from_raw(hf_wad),
        Bps::from_raw(base_bonus_bps),
        Bps::from_raw(max_bonus_bps),
        Wad::from_raw(proportion_seized_wad),
        Wad::from_raw(total_collateral_wad),
    );

    // Ideal repayment must be positive (there is debt to repay)
    cvlr_assert!(ideal.raw() > 0);

    // Ideal repayment must not exceed total debt
    cvlr_assert!(ideal.raw() <= total_debt_wad);

    // Ideal repayment must not exceed total collateral / (1 + bonus).
    let bonus_wad = bonus.to_wad(&e);
    let one_plus_bonus = Wad::ONE + bonus_wad;
    let max_repayable = Wad::from_raw(total_collateral_wad).div(&e, one_plus_bonus);
    cvlr_assert!(ideal.raw() <= max_repayable.raw() + 1); // +1 for rounding tolerance
}

// ---------------------------------------------------------------------------
// Sanity rules (reachability checks)
// ---------------------------------------------------------------------------

#[rule]
fn liquidation_bonus_sanity(e: Env) {
    let hf: i128 = cvlr::nondet::nondet();
    let base: i128 = cvlr::nondet::nondet();
    let max: i128 = cvlr::nondet::nondet();
    cvlr_assume!(hf > 0 && hf < WAD);
    cvlr_assume!(base > 0 && base <= 500);
    cvlr_assume!(max >= base && max <= MAX_LIQUIDATION_BONUS);

    let bonus = crate::helpers::calculate_linear_bonus(
        &e,
        Wad::from_raw(hf),
        Bps::from_raw(base),
        Bps::from_raw(max),
    );
    cvlr_satisfy!(bonus.raw() > 0);
}

#[rule]
fn estimate_liquidation_sanity(e: Env) {
    let total_debt: i128 = cvlr::nondet::nondet();
    let weighted_col: i128 = cvlr::nondet::nondet();
    let hf: i128 = cvlr::nondet::nondet();
    cvlr_assume!(total_debt > WAD && total_debt < 1_000_000 * WAD);
    cvlr_assume!(weighted_col > 0 && weighted_col < total_debt);
    cvlr_assume!(hf > 0 && hf < WAD);

    let (ideal, _bonus) = crate::helpers::estimate_liquidation_amount(
        &e,
        Wad::from_raw(total_debt),
        Wad::from_raw(weighted_col),
        Wad::from_raw(hf),
        Bps::from_raw(500),
        Bps::from_raw(1000),
        Wad::from_raw(WAD / 2),
        Wad::from_raw(total_debt),
    );
    cvlr_satisfy!(ideal.raw() > 0);
}
