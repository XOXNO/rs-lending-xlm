//! Solvency and cross-contract consistency rules.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Map, Vec};

use crate::constants::{MILLISECONDS_PER_YEAR, RAY, WAD};
use common::math::fp::{Ray, Wad};

// Rules that read pool quantity views (`reserves`, `capital_utilisation`,
// `supplied_amount`, `borrowed_amount`) or `get_sync_data` and asserted a relation
// over them were removed: under the certora harness those resolve to independent
// nondet summaries (shared/summaries/pool.rs), so the asserts either re-stated a
// summary's own assume (tautology) or compared two unrelated nondet draws (not
// entailed). The real invariants belong on the pool side where the math runs:
//   * utilization==0 when supplied==0 — common/spec/rates_rules.rs.
//   * supply-index floor / monotonicity — pool/spec/integrity_rules.rs and
//     common/spec/rates_rules.rs.
//   * supply/borrow caps, claim<=reserves, borrow<=reserves — proved against the
//     real ops in pool/spec/summary_contract_rules.rs (supply_respects_supply_cap,
//     borrow_respects_borrow_cap, claim_revenue_satisfies_*, borrow_within_reserves).

/// Isolated debt stays non-negative after repay.
#[rule]
fn isolation_debt_never_negative_after_repay(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!(crate::storage::get_isolated_debt(&e, &asset) >= 0);

    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset.clone(), amount);

    cvlr_assert!(crate::storage::get_isolated_debt(&e, &asset) >= 0);
}

/// Post-borrow total debt does not exceed LTV-weighted collateral.
#[rule]
fn ltv_borrow_bound_enforced(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let post_account = crate::storage::get_account(&e, account_id);

    let ltv_collateral = crate::helpers::calculate_ltv_collateral_wad(
        &e,
        &mut cache,
        &post_account.supply_positions,
    );

    let mut total_debt = Wad::ZERO;
    for asset in post_account.borrow_positions.keys() {
        let position = post_account.borrow_positions.get(asset.clone()).unwrap();
        let feed = cache.cached_price(&asset);
        let market_index = cache.cached_market_index(&asset);
        let value = crate::helpers::position_value(
            &e,
            Ray::from(position.scaled_amount_ray),
            market_index.borrow_index,
            feed.price,
        );
        total_debt += value;
    }

    cvlr_assert!(total_debt.raw() <= ltv_collateral.raw());
}

/// Supply with amount zero reverts.
#[rule]
fn supply_rejects_zero_amount(e: Env, caller: Address, e_mode_category: u32) {
    let account_id: u64 = 1;
    let asset = e.current_contract_address();
    let zero_amount: i128 = 0;

    let mut assets = Vec::new(&e);
    assets.push_back((asset, zero_amount));

    crate::Controller::supply(e.clone(), caller, account_id, e_mode_category, assets);

    cvlr_satisfy!(false);
}

/// Borrow with amount zero reverts.
#[rule]
fn borrow_rejects_zero_amount(e: Env, caller: Address) {
    let account_id: u64 = 1;
    let asset = e.current_contract_address();
    let zero_amount: i128 = 0;

    let mut borrows = Vec::new(&e);
    borrows.push_back((asset, zero_amount));

    crate::Controller::borrow(e.clone(), caller, account_id, borrows);

    cvlr_satisfy!(false);
}

/// Repay with amount zero reverts.
#[rule]
fn repay_rejects_zero_amount(e: Env, caller: Address) {
    let account_id: u64 = 1;
    let asset = e.current_contract_address();
    let zero_amount: i128 = 0;

    let mut payments = Vec::new(&e);
    payments.push_back((asset, zero_amount));

    crate::Controller::repay(e.clone(), caller, account_id, payments);

    cvlr_satisfy!(false);
}

/// Supply reverts when adding a new asset at max_supply_positions.
#[rule]
fn supply_position_limit_enforced(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    new_asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let limits = crate::storage::get_position_limits(&e);
    let current_list = crate::storage::get_position_list(
        &e,
        account_id,
        crate::types::AccountPositionType::Deposit,
    );
    cvlr_assume!(current_list.len() == limits.max_supply_positions as u32);
    cvlr_assume!(limits.max_supply_positions as u32 <= 10);

    let mut already_exists = false;
    for i in 0..current_list.len() {
        let existing = current_list.get(i).unwrap();
        if existing == new_asset {
            already_exists = true;
        }
    }
    cvlr_assume!(!already_exists);

    let mut assets = Vec::new(&e);
    assets.push_back((new_asset, amount));

    crate::Controller::supply(e.clone(), caller, account_id, e_mode_category, assets);

    cvlr_satisfy!(false);
}

/// Borrow reverts when adding a new asset at max_borrow_positions.
#[rule]
fn borrow_position_limit_enforced(e: Env, caller: Address, new_asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let limits = crate::storage::get_position_limits(&e);
    let current_list = crate::storage::get_position_list(
        &e,
        account_id,
        crate::types::AccountPositionType::Borrow,
    );
    cvlr_assume!(current_list.len() == limits.max_borrow_positions as u32);
    cvlr_assume!(limits.max_borrow_positions as u32 <= 10);

    let mut already_exists = false;
    for i in 0..current_list.len() {
        let existing = current_list.get(i).unwrap();
        if existing == new_asset {
            already_exists = true;
        }
    }
    cvlr_assume!(!already_exists);

    let mut borrows = Vec::new(&e);
    borrows.push_back((new_asset, amount));

    crate::Controller::borrow(e.clone(), caller, account_id, borrows);

    cvlr_satisfy!(false);
}

#[rule]
fn solvency_sanity_supply(
    e: Env,
    caller: Address,
    e_mode_category: u32,
    asset: Address,
    amount: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0);
    let mut assets = Vec::new(&e);
    assets.push_back((asset, amount));
    crate::Controller::supply(e, caller, account_id, e_mode_category, assets);
    cvlr_satisfy!(true);
}

#[rule]
fn solvency_sanity_borrow(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0);
    let mut borrows = Vec::new(&e);
    borrows.push_back((asset, amount));
    crate::Controller::borrow(e, caller, account_id, borrows);
    cvlr_satisfy!(true);
}

#[rule]
fn solvency_sanity_repay(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0);
    let mut payments = Vec::new(&e);
    payments.push_back((asset, amount));
    crate::Controller::repay(e, caller, account_id, payments);
    cvlr_satisfy!(true);
}

/// cached_market_index returns the same snapshot within one transaction.
#[rule]
fn index_cache_single_snapshot(e: Env, asset: Address) {
    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);

    let index1 = cache.cached_market_index(&asset);
    let index2 = cache.cached_market_index(&asset);

    cvlr_assert!(index1.supply_index.raw() == index2.supply_index.raw());
    cvlr_assert!(index1.borrow_index.raw() == index2.borrow_index.raw());
}

/// Supply-then-withdraw roundtrip recovers at most amount + 1.
#[rule]
fn supply_withdraw_roundtrip_no_profit(e: Env) {
    let amount: i128 = cvlr::nondet::nondet();
    let supply_index: i128 = cvlr::nondet::nondet();

    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!(supply_index >= RAY);

    let scaled = common::math::fp_core::mul_div_half_up(&e, amount, RAY, supply_index);
    let recovered = common::math::fp_core::mul_div_half_up(&e, scaled, supply_index, RAY);

    cvlr_assert!(recovered <= amount + 1);
}

/// Borrow-then-repay roundtrip owes at least amount - 1.
#[rule]
fn borrow_repay_roundtrip_no_profit(e: Env) {
    let amount: i128 = cvlr::nondet::nondet();
    let borrow_index: i128 = cvlr::nondet::nondet();

    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!(borrow_index >= RAY);

    let scaled_debt = common::math::fp_core::mul_div_half_up(&e, amount, RAY, borrow_index);
    let debt_owed = common::math::fp_core::mul_div_half_up(&e, scaled_debt, borrow_index, RAY);

    cvlr_assert!(debt_owed >= amount - 1);
}

/// Clearing prices_cache forces a fresh oracle fetch.
#[rule]
fn price_cache_invalidation_after_swap(e: Env, asset: Address) {
    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);

    let _feed1 = cache.cached_price(&asset);

    let cached = cache.prices_cache.get(asset.clone());
    cvlr_assert!(cached.is_some());

    cache.prices_cache = Map::new(&e);

    let cached_after = cache.prices_cache.get(asset.clone());
    cvlr_assert!(cached_after.is_none());

    let _feed2 = cache.cached_price(&asset);

    let cached_repopulated = cache.prices_cache.get(asset.clone());
    cvlr_assert!(cached_repopulated.is_some());
}

/// Supplying isolated asset into e-mode account with borrows reverts.
#[rule]
fn mode_transition_blocked_with_positions(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.e_mode_category_id > 0);
    cvlr_assume!(!attrs.is_isolated);

    let borrow_list = crate::storage::get_position_list(
        &e,
        account_id,
        crate::types::AccountPositionType::Borrow,
    );
    cvlr_assume!(borrow_list.len() == 1);

    let config = crate::storage::get_asset_config(&e, &asset);
    cvlr_assume!(config.is_isolated_asset);

    let mut assets = Vec::new(&e);
    assets.push_back((asset, amount));
    crate::Controller::supply(
        e.clone(),
        caller,
        account_id,
        attrs.e_mode_category_id,
        assets,
    );

    cvlr_satisfy!(false);
}

/// compound_interest output stays below 100000*RAY for bounded inputs.
#[rule]
fn compound_interest_bounded_output(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    let time: u64 = cvlr::nondet::nondet();

    let max_rate_per_ms =
        common::math::fp_core::div_by_int_half_up(10 * RAY, MILLISECONDS_PER_YEAR as i128);

    cvlr_assume!(rate >= 0 && rate <= max_rate_per_ms);
    cvlr_assume!(time > 0 && time <= MILLISECONDS_PER_YEAR);

    let factor = common::rates::compound_interest(&e, Ray::from(rate), time);

    let upper_bound = 100_000 * RAY;
    cvlr_assert!(factor.raw() < upper_bound);
}

/// compound_interest factor is at least RAY for non-negative rate and time.
#[rule]
fn compound_interest_no_wrap(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    let time: u64 = cvlr::nondet::nondet();

    let max_rate_per_ms =
        common::math::fp_core::div_by_int_half_up(10 * RAY, MILLISECONDS_PER_YEAR as i128);

    cvlr_assume!(rate >= 0 && rate <= max_rate_per_ms);
    cvlr_assume!(time <= MILLISECONDS_PER_YEAR);

    let factor = common::rates::compound_interest(&e, Ray::from(rate), time);

    cvlr_assert!(factor.raw() >= RAY);
}

#[rule]
fn index_cache_snapshot_sanity(e: Env, asset: Address) {
    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let index = cache.cached_market_index(&asset);
    cvlr_satisfy!(index.supply_index.raw() >= RAY);
}

#[rule]
fn roundtrip_supply_sanity(e: Env) {
    let amount: i128 = cvlr::nondet::nondet();
    let index: i128 = cvlr::nondet::nondet();
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!((RAY..=10 * RAY).contains(&index));

    let scaled = common::math::fp_core::mul_div_half_up(&e, amount, RAY, index);
    let recovered = common::math::fp_core::mul_div_half_up(&e, scaled, index, RAY);
    cvlr_satisfy!(recovered <= amount + 1);
}

#[rule]
fn compound_no_wrap_sanity(e: Env) {
    let rate: i128 = cvlr::nondet::nondet();
    let time: u64 = cvlr::nondet::nondet();
    let max_rate_per_ms =
        common::math::fp_core::div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128);
    cvlr_assume!(rate > 0 && rate <= max_rate_per_ms);
    cvlr_assume!(time > 0 && time <= MILLISECONDS_PER_YEAR);
    let factor = common::rates::compound_interest(&e, Ray::from(rate), time);
    cvlr_satisfy!(factor.raw() > RAY);
}

/// Scaled balances reconstruct to actual balances at the current index.
#[rule]
fn scaled_to_actual_reconstruction(e: Env) {
    let scaled: i128 = cvlr::nondet::nondet();
    let index: i128 = cvlr::nondet::nondet();
    cvlr_assume!(scaled > 0 && scaled <= WAD * 1_000_000);
    cvlr_assume!((RAY..=10 * RAY).contains(&index));

    let actual = common::math::fp_core::mul_div_half_up(&e, scaled, index, RAY);

    cvlr_assert!(actual + 1 >= scaled);
    cvlr_assert!(actual <= scaled.saturating_mul(index) / RAY + 1);
}
