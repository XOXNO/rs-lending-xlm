use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

use crate::constants::WAD;
use crate::spec::fixture;
use crate::storage;
use crate::types::HubAssetKey;
use controller_interface::ControllerInterface;

fn hub0(asset: Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
        asset,
    }
}

/// INV-RISK-01: a borrow that completes leaves the account's LTV-gated
/// collateral at or above its total debt.
///
/// This is the exact inequality `require_post_pool_risk_gates` enforces
/// (`totals.ltv_collateral >= totals.total_debt`), re-evaluated on the book
/// the transaction *persisted* rather than on the in-memory account the gate
/// held. It therefore proves both halves at once: the gate's arithmetic, and
/// that nothing after the gate changed the book it admitted.
///
/// Three modelling facts make it a real statement rather than a comparison of
/// unrelated draws, and all three are needed:
///
/// - under `certora-solvency-rules` `calculate_account_risk_totals` compiles
///   its real body (`contracts/controller/src/risk/totals.rs`), so this is the
///   gate's own arithmetic and not a havoc summary;
/// - `spec::ghost_prices` memoises the price and market-index draws per rule,
///   so the valuation here uses the numbers the gate used;
/// - the pool mutation summaries report the same market snapshot index, so the
///   index the controller caches during the borrow is the one read here.
///
/// The assertion is guarded on a non-empty debt book because the gate returns
/// early for a debt-free account and claims nothing about it.
#[rule]
fn ltv_borrow_bound_enforced(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
    collateral_scaled: i128,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!(collateral_scaled > 0 && collateral_scaled <= 20 * common::constants::RAY);
    fixture::seed_live_account(&e, account_id, &caller, &asset);
    // One collateral position carrying the listing's risk tuple. Excludes
    // second assets and pre-book tuples no listing could stamp; the borrow can
    // otherwise complete, so the rule is not an empty implication.
    fixture::seed_empty_books(&e, account_id);
    fixture::seed_supply_position(&e, account_id, &asset, collateral_scaled);

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset, amount);

    let account = storage::get_account(&e, account_id);
    let mut cache = crate::context::Context::new(&e);
    let totals = crate::risk::calculate_account_risk_totals(
        &e,
        &mut cache,
        &account.supply_positions,
        &account.borrow_positions,
    );

    cvlr_assert!(
        account.borrow_positions.is_empty()
            || totals.ltv_collateral.raw() >= totals.total_debt.raw()
    );
}

/// Witness for `ltv_borrow_bound_enforced`: the fixture really does reach a
/// completed borrow with a debt record, so the implication above is not empty.
#[rule]
fn ltv_borrow_bound_enforced_fixture_completes(e: Env, caller: Address, asset: Address) {
    let account_id: u64 = 1;
    fixture::seed_live_account(&e, account_id, &caller, &asset);
    fixture::seed_empty_books(&e, account_id);
    fixture::seed_supply_position(&e, account_id, &asset, 20 * common::constants::RAY);

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset.clone(), WAD);

    cvlr_satisfy!(storage::get_debt_positions(&e, account_id)
        .get(fixture::hub_asset(&asset))
        .is_some());
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
#[allow(clippy::too_many_arguments)]
fn supply_position_limit_enforced(
    e: Env,
    caller: Address,
    new_asset: Address,
    amount: i128,
    a1: Address,
    a2: Address,
    a3: Address,
    a4: Address,
    a5: Address,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    fixture::seed_live_account(&e, account_id, &caller, &new_asset);
    fixture::seed_empty_books(&e, account_id);

    let limits = storage::get_position_limits(&e);
    cvlr_assume!(limits.max_supply_positions == common::constants::POSITION_LIMIT_MAX);

    // Concrete pre-existing position book exactly at the configured limit. The
    // assets are symbolic; pairwise distinctness is what makes the map hold
    // `POSITION_LIMIT` entries and the new asset a fresh key, and the array
    // type is what keeps the count tied to the constant (see the guard in
    // `fixture.rs`).
    let assets: [Address; fixture::POSITION_LIMIT] = [a1, a2, a3, a4, a5];
    fixture::assume_pairwise_distinct(&assets, &new_asset);
    let seeded = fixture::seed_supply_positions(&e, account_id, &assets);
    cvlr_assume!(seeded == limits.max_supply_positions);

    let mut assets_vec = Vec::new(&e);
    assets_vec.push_back((hub0(new_asset), amount));

    // One more distinct asset opens slot `POSITION_LIMIT + 1`.
    crate::Controller::supply(
        e.clone(),
        caller,
        account_id,
        crate::spec::fixture::SPOKE_ID,
        assets_vec,
    );

    cvlr_assert!(false);
}

#[rule]
#[allow(clippy::too_many_arguments)]
fn borrow_position_limit_enforced(
    e: Env,
    caller: Address,
    new_asset: Address,
    amount: i128,
    a1: Address,
    a2: Address,
    a3: Address,
    a4: Address,
    a5: Address,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    fixture::seed_live_account(&e, account_id, &caller, &new_asset);
    fixture::seed_empty_books(&e, account_id);

    let limits = storage::get_position_limits(&e);
    cvlr_assume!(limits.max_borrow_positions == common::constants::POSITION_LIMIT_MAX);

    // Concrete pre-existing debt book at the configured limit (see
    // supply_position_limit_enforced for the distinctness rationale).
    let assets: [Address; fixture::POSITION_LIMIT] = [a1, a2, a3, a4, a5];
    fixture::assume_pairwise_distinct(&assets, &new_asset);
    let seeded = fixture::seed_debt_positions(&e, account_id, &assets);
    cvlr_assume!(seeded == limits.max_borrow_positions);

    let mut borrows = Vec::new(&e);
    borrows.push_back((hub0(new_asset), amount));

    crate::Controller::borrow(e.clone(), caller, account_id, borrows, None);

    cvlr_assert!(false);
}

/// Satisfy twin of `supply_position_limit_enforced`: the same fixture one slot
/// below the cap admits the new asset, so the revert rule is a statement about
/// the slot past the cap and not about an unreachable supply.
#[rule]
#[allow(clippy::too_many_arguments)]
fn supply_position_limit_enforced_fixture_completes(
    e: Env,
    caller: Address,
    new_asset: Address,
    a1: Address,
    a2: Address,
    a3: Address,
    a4: Address,
) {
    let account_id: u64 = 1;
    fixture::seed_live_account(&e, account_id, &caller, &new_asset);
    fixture::seed_empty_books(&e, account_id);

    let limits = storage::get_position_limits(&e);
    cvlr_assume!(limits.max_supply_positions == common::constants::POSITION_LIMIT_MAX);

    let assets: [Address; fixture::POSITION_LIMIT - 1] = [a1, a2, a3, a4];
    fixture::assume_pairwise_distinct(&assets, &new_asset);
    let seeded = fixture::seed_supply_positions(&e, account_id, &assets);
    cvlr_assume!(seeded + 1 == limits.max_supply_positions);

    let mut assets_vec = Vec::new(&e);
    assets_vec.push_back((hub0(new_asset), WAD));

    crate::Controller::supply(e.clone(), caller, account_id, fixture::SPOKE_ID, assets_vec);

    cvlr_satisfy!(
        storage::get_supply_positions(&e, account_id).len() == limits.max_supply_positions
    );
}

/// Satisfy twin of `borrow_position_limit_enforced`. Carries collateral as
/// well as the seeded debts, so the borrow that completes is a real one and
/// not a path that stops at an earlier gate.
#[rule]
#[allow(clippy::too_many_arguments)]
fn borrow_position_limit_enforced_fixture_completes(
    e: Env,
    caller: Address,
    new_asset: Address,
    a1: Address,
    a2: Address,
    a3: Address,
    a4: Address,
) {
    let account_id: u64 = 1;
    fixture::seed_live_account(&e, account_id, &caller, &new_asset);
    fixture::seed_empty_books(&e, account_id);

    let limits = storage::get_position_limits(&e);
    cvlr_assume!(limits.max_borrow_positions == common::constants::POSITION_LIMIT_MAX);

    let assets: [Address; fixture::POSITION_LIMIT - 1] = [a1, a2, a3, a4];
    fixture::assume_pairwise_distinct(&assets, &new_asset);
    fixture::seed_supply_position(&e, account_id, &new_asset, 10 * common::constants::RAY);
    let seeded = fixture::seed_debt_positions(&e, account_id, &assets);
    cvlr_assume!(seeded + 1 == limits.max_borrow_positions);

    let mut borrows = Vec::new(&e);
    borrows.push_back((hub0(new_asset.clone()), WAD));

    crate::Controller::borrow(e.clone(), caller, account_id, borrows, None);

    cvlr_satisfy!(storage::get_debt_positions(&e, account_id)
        .get(fixture::hub_asset(&new_asset))
        .is_some());
}

/// A top-up of an asset the account already holds is admitted after governance
/// lowers the position limit below the account's current count.
///
/// A top-up opens no slot, so it cannot breach the bound; before the fix in
/// `validate_bulk_position_limits`, every supply to such an account failed and
/// the position was locked in place. The witness is the strictly larger scaled
/// record, so a path that reverted or wrote nothing does not satisfy it.
#[rule]
#[allow(clippy::too_many_arguments)]
fn supply_topup_survives_lowered_limit(
    e: Env,
    caller: Address,
    held_asset: Address,
    a2: Address,
    a3: Address,
    a4: Address,
    a5: Address,
) {
    let account_id: u64 = 1;
    fixture::seed_live_account(&e, account_id, &caller, &held_asset);
    fixture::seed_empty_books(&e, account_id);

    // A book at the old cap, holding `held_asset`.
    let assets: [Address; fixture::POSITION_LIMIT] = [held_asset.clone(), a2, a3, a4, a5];
    let others = [
        assets[1].clone(),
        assets[2].clone(),
        assets[3].clone(),
        assets[4].clone(),
    ];
    fixture::assume_pairwise_distinct(&others, &held_asset);
    let seeded = fixture::seed_supply_positions(&e, account_id, &assets);
    cvlr_assume!(seeded == common::constants::POSITION_LIMIT_MAX);

    // Governance lowers the cap below the account's current count.
    storage::set_position_limits(
        &e,
        &common::types::PositionLimits {
            max_supply_positions: 2,
            max_borrow_positions: 2,
        },
    );

    let hub = fixture::hub_asset(&held_asset);
    let before = storage::get_supply_positions(&e, account_id)
        .get(hub.clone())
        .map(|position| position.scaled_amount)
        .unwrap_or(0);

    let mut legs = Vec::new(&e);
    legs.push_back((hub.clone(), WAD));
    crate::Controller::supply(e.clone(), caller, account_id, fixture::SPOKE_ID, legs);

    let after = storage::get_supply_positions(&e, account_id)
        .get(hub)
        .map(|position| position.scaled_amount)
        .unwrap_or(0);

    cvlr_satisfy!(after > before);
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
    let mut cache = crate::context::Context::new(&e);

    let hub_asset = hub0(asset);
    let index1 = cache.cached_market_index(&hub_asset);
    let index2 = cache.cached_market_index(&hub_asset);

    cvlr_assert!(index1.supply_index.raw() == index2.supply_index.raw());
    cvlr_assert!(index1.borrow_index.raw() == index2.borrow_index.raw());
}

#[rule]
fn index_cache_snapshot_sanity(e: Env, asset: Address) {
    crate::spec::fixture::seed_protocol(&e);
    let mut cache = crate::context::Context::new(&e);
    let _index = cache.cached_market_index(&hub0(asset));
    cvlr_satisfy!(true);
}
