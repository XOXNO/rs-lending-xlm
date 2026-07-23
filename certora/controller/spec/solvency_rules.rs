//! Solvency and cross-contract consistency rules.

use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

use crate::constants::{MILLISECONDS_PER_YEAR, RAY, WAD};
use crate::types::HubAssetKey;
use common::math::fp::{Ray, Wad};

/// Primary-hub coordinate for `asset`.
fn hub0(asset: Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
        asset,
    }
}

// Pool quantity views are independent nondet under the harness; real invariants
// live where the math runs: rates_rules (util), integrity/rates (indexes),
// summary_contract_rules (caps/reserves).

#[rule]
fn ltv_borrow_bound_enforced(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    let mut cache = crate::context::Cache::new(&e);
    let post_account = crate::storage::get_account(&e, account_id);
    cache.load_markets(&crate::risk::portfolio_hub_keys(
        post_account.supply_positions.keys(),
        &post_account.borrow_positions.keys(),
    ));

    let ltv_collateral = crate::risk::calculate_ltv_collateral_wad(
        &e,
        &mut cache,
        post_account.spoke_id,
        &post_account.supply_positions,
    );

    let mut total_debt = Wad::ZERO;
    for hub_asset in post_account.borrow_positions.keys() {
        let position = post_account
            .borrow_positions
            .get(hub_asset.clone())
            .unwrap();
        let feed = cache.cached_price(&hub_asset.asset);
        let market_index = cache.cached_market_index(&hub_asset);
        let value = crate::risk::position_value(
            &e,
            Ray::from(position.scaled_amount),
            market_index.borrow_index,
            feed.price,
        );
        total_debt.checked_add_assign(&e, value);
    }

    cvlr_assert!(total_debt.raw() <= ltv_collateral.raw());
}

#[rule]
fn supply_rejects_zero_amount(e: Env, caller: Address) {
    let account_id: u64 = 1;
    let asset = e.current_contract_address();
    let zero_amount: i128 = 0;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let mut assets = Vec::new(&e);
    assets.push_back((hub0(asset), zero_amount));

    crate::Controller::supply(
        e.clone(),
        caller,
        account_id,
        crate::spec::fixture::SPOKE_ID,
        assets,
    );

    cvlr_assert!(false);
}

#[rule]
fn borrow_rejects_zero_amount(e: Env, caller: Address) {
    let account_id: u64 = 1;
    let asset = e.current_contract_address();
    let zero_amount: i128 = 0;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let mut borrows = Vec::new(&e);
    borrows.push_back((hub0(asset), zero_amount));

    crate::Controller::borrow(e.clone(), caller, account_id, borrows, None);

    cvlr_assert!(false);
}

#[rule]
fn repay_rejects_zero_amount(e: Env, caller: Address) {
    let account_id: u64 = 1;
    let asset = e.current_contract_address();
    let zero_amount: i128 = 0;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let mut payments = Vec::new(&e);
    payments.push_back((hub0(asset), zero_amount));

    crate::Controller::repay(e.clone(), caller, account_id, payments);

    cvlr_assert!(false);
}

#[rule]
fn supply_position_limit_enforced(e: Env, caller: Address, new_asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &new_asset);

    let limits = crate::storage::get_position_limits(&e);
    let current_list = crate::storage::get_position_list(
        &e,
        account_id,
        crate::types::AccountPositionType::Deposit,
    );
    cvlr_assume!(current_list.len() == limits.max_supply_positions);
    cvlr_assume!(limits.max_supply_positions <= 10);

    // `new_asset` is genuinely new, so the supply adds a position above the limit.
    // Expressed via `get_position` (not a scan of `current_list`): an
    // `optimistic_loop` would silently drop accounts holding more positions than
    // `loop_iter`, narrowing coverage without saying so.
    cvlr_assume!(crate::storage::get_position(
        &e,
        account_id,
        crate::types::AccountPositionType::Deposit,
        &new_asset
    )
    .is_none());

    let mut assets = Vec::new(&e);
    assets.push_back((hub0(new_asset), amount));

    crate::Controller::supply(
        e.clone(),
        caller,
        account_id,
        crate::spec::fixture::SPOKE_ID,
        assets,
    );

    cvlr_assert!(false);
}

#[rule]
fn borrow_position_limit_enforced(e: Env, caller: Address, new_asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &new_asset);

    let limits = crate::storage::get_position_limits(&e);
    let current_list = crate::storage::get_position_list(
        &e,
        account_id,
        crate::types::AccountPositionType::Borrow,
    );
    cvlr_assume!(current_list.len() == limits.max_borrow_positions);
    cvlr_assume!(limits.max_borrow_positions <= 10);

    // `new_asset` is genuinely new, so the borrow adds a position above the limit.
    // Expressed via `get_position` (not a scan): an `optimistic_loop` over the
    // list would silently drop accounts with more positions than `loop_iter`.
    cvlr_assume!(crate::storage::get_position(
        &e,
        account_id,
        crate::types::AccountPositionType::Borrow,
        &new_asset
    )
    .is_none());

    let mut borrows = Vec::new(&e);
    borrows.push_back((hub0(new_asset), amount));

    crate::Controller::borrow(e.clone(), caller, account_id, borrows, None);

    cvlr_assert!(false);
}

#[rule]
fn solvency_sanity_supply(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    let amount = WAD;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    let mut assets = Vec::new(&e);
    assets.push_back((hub0(asset), amount));
    crate::Controller::supply(
        e,
        caller,
        account_id,
        crate::spec::fixture::SPOKE_ID,
        assets,
    );
    cvlr_satisfy!(true);
}

#[rule]
fn solvency_sanity_borrow(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    let amount = WAD;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::compat::supply_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        amount * 4,
    );
    let mut borrows = Vec::new(&e);
    borrows.push_back((hub0(asset), amount));
    crate::Controller::borrow(e, caller, account_id, borrows, None);
    cvlr_satisfy!(true);
}

#[rule]
fn solvency_sanity_repay(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    let amount = WAD;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::compat::supply_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        amount * 4,
    );
    crate::spec::compat::borrow_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        amount,
    );
    let mut payments = Vec::new(&e);
    payments.push_back((hub0(asset), amount));
    crate::Controller::repay(e, caller, account_id, payments);
    cvlr_satisfy!(true);
}

#[rule]
fn index_cache_single_snapshot(e: Env, asset: Address) {
    crate::spec::fixture::seed_protocol(&e);
    let mut cache = crate::context::Cache::new(&e);

    let hub_asset = hub0(asset);
    let index1 = cache.cached_market_index(&hub_asset);
    let index2 = cache.cached_market_index(&hub_asset);

    cvlr_assert!(index1.supply_index.raw() == index2.supply_index.raw());
    cvlr_assert!(index1.borrow_index.raw() == index2.borrow_index.raw());
}

#[rule]
fn supply_withdraw_roundtrip_error_bounded(e: Env) {
    let amount: i128 = cvlr::nondet::nondet();
    let supply_index: i128 = cvlr::nondet::nondet();

    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!((RAY..=10 * RAY).contains(&supply_index));

    let scaled = common::math::fp_core::mul_div_half_up(&e, amount, RAY, supply_index);
    let recovered = common::math::fp_core::mul_div_half_up(&e, scaled, supply_index, RAY);

    // Two half-up conversions at index <= 10 RAY accumulate at most six raw
    // units of reconstruction error.
    cvlr_assert!(recovered >= amount.saturating_sub(6));
    cvlr_assert!(recovered <= amount + 6);
}

#[rule]
fn borrow_repay_roundtrip_error_bounded(e: Env) {
    let amount: i128 = cvlr::nondet::nondet();
    let borrow_index: i128 = cvlr::nondet::nondet();

    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!((RAY..=10 * RAY).contains(&borrow_index));

    let scaled_debt = common::math::fp_core::mul_div_half_up(&e, amount, RAY, borrow_index);
    let debt_owed = common::math::fp_core::mul_div_half_up(&e, scaled_debt, borrow_index, RAY);

    cvlr_assert!(debt_owed >= amount.saturating_sub(6));
    cvlr_assert!(debt_owed <= amount + 6);
}

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
    crate::spec::fixture::seed_protocol(&e);
    let mut cache = crate::context::Cache::new(&e);
    let _index = cache.cached_market_index(&hub0(asset));
    cvlr_satisfy!(true);
}

#[rule]
fn roundtrip_supply_sanity(e: Env) {
    let amount = WAD;
    let index = RAY;

    let scaled = common::math::fp_core::mul_div_half_up(&e, amount, RAY, index);
    let recovered = common::math::fp_core::mul_div_half_up(&e, scaled, index, RAY);
    let _recovered = recovered;
    cvlr_satisfy!(true);
}

#[rule]
fn compound_no_wrap_sanity(e: Env) {
    let max_rate_per_ms =
        common::math::fp_core::div_by_int_half_up(RAY, MILLISECONDS_PER_YEAR as i128);
    let rate = max_rate_per_ms;
    let time = 1;
    let factor = common::rates::compound_interest(&e, Ray::from(rate), time);
    let _factor = factor;
    cvlr_satisfy!(true);
}

#[rule]
fn scaled_to_actual_matches_floor_with_rounding(e: Env) {
    let scaled: i128 = cvlr::nondet::nondet();
    let index: i128 = cvlr::nondet::nondet();
    cvlr_assume!(scaled > 0 && scaled <= WAD * 1_000_000);
    cvlr_assume!((RAY..=10 * RAY).contains(&index));

    let actual = common::math::fp_core::mul_div_half_up(&e, scaled, index, RAY);
    let floor = common::math::fp_core::mul_div_floor(&e, scaled, index, RAY);

    cvlr_assert!(actual >= scaled);
    cvlr_assert!(actual >= floor);
    cvlr_assert!(actual <= floor + 1);
}
