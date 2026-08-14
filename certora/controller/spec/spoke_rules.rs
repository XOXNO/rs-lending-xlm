use cvlr::macros::rule;
use cvlr::nondet::nondet;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

use crate::context::Cache;
use crate::events::{EventContext, PositionAction};
use crate::positions::{LegDirection, LegOutcome, WithdrawKind};
use crate::types::{
    Account, AccountPositionType, HubAssetKey, MarketIndexRaw, PoolAction, PoolPositionMutation,
    PoolWithdrawEntry, ScaledPositionRaw, SeizeMode, SpokeAssetArgs, SpokeUsageRaw,
};

fn hub0(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
        asset: asset.clone(),
    }
}

#[rule]
fn spoke_only_registered_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let hub_asset = hub0(&asset);
    cvlr_assume!(crate::storage::get_spoke_asset(&e, attrs.spoke_id, &hub_asset).is_none());

    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    assets.push_back((hub_asset, amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &assets);

    cvlr_assert!(false);
}

#[rule]
fn spoke_borrow_only_registered_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let hub_asset = hub0(&asset);
    cvlr_assume!(crate::storage::get_spoke_asset(&e, attrs.spoke_id, &hub_asset).is_none());

    let mut borrows: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    borrows.push_back((hub_asset, amount));
    crate::positions::process_borrow(&e, &caller, account_id, &borrows, None);

    cvlr_assert!(false);
}

#[rule]
fn spoke_only_borrowable_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let mut stored =
        crate::storage::get_spoke_asset(&e, crate::spec::fixture::SPOKE_ID, &hub0(&asset)).unwrap();
    stored.is_borrowable = false;
    crate::storage::set_spoke_asset(&e, crate::spec::fixture::SPOKE_ID, &hub0(&asset), &stored);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke_asset = crate::storage::get_spoke_asset(&e, attrs.spoke_id, &hub0(&asset));
    cvlr_assume!(spoke_asset.is_some());
    let cfg = spoke_asset.unwrap();
    cvlr_assume!(!cfg.is_borrowable);

    let mut borrows: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    borrows.push_back((hub0(&asset), amount));
    crate::positions::process_borrow(&e, &caller, account_id, &borrows, None);

    cvlr_assert!(false);
}

#[rule]
fn spoke_only_collateralizable_assets(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let mut stored =
        crate::storage::get_spoke_asset(&e, crate::spec::fixture::SPOKE_ID, &hub0(&asset)).unwrap();
    stored.is_collateralizable = false;
    crate::storage::set_spoke_asset(&e, crate::spec::fixture::SPOKE_ID, &hub0(&asset), &stored);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke_asset = crate::storage::get_spoke_asset(&e, attrs.spoke_id, &hub0(&asset));
    cvlr_assume!(spoke_asset.is_some());
    let cfg = spoke_asset.unwrap();
    cvlr_assume!(!cfg.is_collateralizable);

    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    assets.push_back((hub0(&asset), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &assets);

    cvlr_assert!(false);
}

#[rule]
fn deprecated_spoke_blocks_new_supply(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    let mut deprecated = crate::storage::get_spoke(&e, crate::spec::fixture::SPOKE_ID);
    deprecated.is_deprecated = true;
    crate::storage::set_spoke(&e, crate::spec::fixture::SPOKE_ID, &deprecated);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke = crate::storage::get_spoke(&e, attrs.spoke_id);
    cvlr_assume!(spoke.is_deprecated);

    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    assets.push_back((hub0(&asset), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &assets);

    cvlr_assert!(false);
}

#[rule]
fn deprecated_spoke_blocks_new_borrow(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    let mut deprecated = crate::storage::get_spoke(&e, crate::spec::fixture::SPOKE_ID);
    deprecated.is_deprecated = true;
    crate::storage::set_spoke(&e, crate::spec::fixture::SPOKE_ID, &deprecated);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke = crate::storage::get_spoke(&e, attrs.spoke_id);
    cvlr_assume!(spoke.is_deprecated);

    let mut borrows: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    borrows.push_back((hub0(&asset), amount));
    crate::positions::process_borrow(&e, &caller, account_id, &borrows, None);

    cvlr_assert!(false);
}

#[rule]
fn deprecated_spoke_withdraw_does_not_increase_supply(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    scaled_before: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(scaled_before > 0 && scaled_before <= 20 * common::constants::RAY);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_supply_position(&e, account_id, &asset, scaled_before);
    let mut deprecated = crate::storage::get_spoke(&e, crate::spec::fixture::SPOKE_ID);
    deprecated.is_deprecated = true;
    crate::storage::set_spoke(&e, crate::spec::fixture::SPOKE_ID, &deprecated);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let spoke = crate::storage::get_spoke(&e, attrs.spoke_id);
    cvlr_assume!(spoke.is_deprecated);

    let mut withdrawals: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    withdrawals.push_back((hub0(&asset), amount));
    crate::positions::process_withdraw(&e, &caller, account_id, &withdrawals, None);

    let position_after =
        crate::storage::get_position(&e, account_id, AccountPositionType::Deposit, &asset);
    match position_after {
        None => {
            cvlr_assert!(true);
        }
        Some(pos_after) => {
            cvlr_assert!(pos_after.scaled_amount <= scaled_before);
        }
    }
}

#[rule]
fn spoke_overrides_asset_params(e: Env, asset: Address) {
    let category_id = crate::spec::fixture::SPOKE_ID;
    crate::spec::fixture::seed_market(&e, &asset);

    let spoke = crate::storage::get_spoke(&e, category_id);

    cvlr_assume!(!spoke.is_deprecated);

    let hub_asset = hub0(&asset);
    let spoke_asset = crate::storage::get_spoke_asset(&e, category_id, &hub_asset);
    cvlr_assume!(spoke_asset.is_some());
    let cfg = spoke_asset.unwrap();

    let mut cache = crate::context::Cache::new(&e);
    let asset_config: common::types::AssetConfig =
        cache.require_spoke_asset(category_id, &hub_asset);

    cvlr_assert!(asset_config.loan_to_value.raw() == i128::from(cfg.loan_to_value));
    cvlr_assert!(asset_config.liquidation_threshold.raw() == i128::from(cfg.liquidation_threshold));
    cvlr_assert!(asset_config.liquidation_bonus.raw() == i128::from(cfg.liquidation_bonus));

    cvlr_assert!(asset_config.is_collateralizable == cfg.is_collateralizable);
    cvlr_assert!(asset_config.is_borrowable == cfg.is_borrowable);
}

#[rule]
fn add_asset_enforces_valid_bounds(e: Env, asset: Address, ltv: u32, threshold: u32, bonus: u32) {
    let category_id = crate::spec::fixture::SPOKE_ID;
    crate::spec::fixture::seed_protocol(&e);

    crate::config::add_asset_to_spoke(
        &e,
        &SpokeAssetArgs {
            hub_id: crate::spec::fixture::HUB_ID,
            asset: asset.clone(),
            spoke_id: category_id,
            can_collateral: true,
            can_borrow: true,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv,
            threshold,
            bonus,
            liquidation_fees: 0,
            supply_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
            borrow_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
        },
    );

    let cfg = crate::storage::get_spoke_asset(&e, category_id, &hub0(&asset)).unwrap();
    cvlr_assert!(cfg.liquidation_threshold > cfg.loan_to_value);
}

#[rule]
fn edit_asset_enforces_valid_bounds(e: Env, asset: Address, ltv: u32, threshold: u32, bonus: u32) {
    let category_id = crate::spec::fixture::SPOKE_ID;
    crate::spec::fixture::seed_market(&e, &asset);

    crate::config::edit_asset_in_spoke(
        &e,
        &SpokeAssetArgs {
            hub_id: crate::spec::fixture::HUB_ID,
            asset: asset.clone(),
            spoke_id: category_id,
            can_collateral: true,
            can_borrow: true,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv,
            threshold,
            bonus,
            liquidation_fees: 0,
            supply_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
            borrow_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
        },
    );

    let cfg = crate::storage::get_spoke_asset(&e, category_id, &hub0(&asset)).unwrap();
    cvlr_assert!(cfg.liquidation_threshold > cfg.loan_to_value);
}

#[rule]
fn spoke_remove_category(e: Env) {
    let category_id = crate::spec::fixture::SPOKE_ID;
    crate::spec::fixture::seed_protocol(&e);

    let before = crate::storage::try_get_spoke(&e, category_id);
    cvlr_assume!(matches!(&before, Some(spoke) if !spoke.is_deprecated));

    crate::config::remove_spoke(&e, category_id);

    let spoke = crate::storage::get_spoke(&e, category_id);
    cvlr_assert!(spoke.is_deprecated);
}

#[rule]
fn spoke_add_asset_to_deprecated_category(e: Env, asset: Address) {
    let category_id = crate::spec::fixture::SPOKE_ID;
    crate::spec::fixture::seed_protocol(&e);
    let mut deprecated = crate::storage::get_spoke(&e, category_id);
    deprecated.is_deprecated = true;
    crate::storage::set_spoke(&e, category_id, &deprecated);

    let spoke = crate::storage::try_get_spoke(&e, category_id);
    cvlr_assume!(spoke.is_some());
    cvlr_assume!(spoke.unwrap().is_deprecated);

    crate::config::add_asset_to_spoke(
        &e,
        &SpokeAssetArgs {
            hub_id: crate::spec::fixture::HUB_ID,
            asset,
            spoke_id: category_id,
            can_collateral: true,
            can_borrow: true,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: 9_000,
            threshold: 9_300,
            bonus: 300,
            liquidation_fees: 0,
            supply_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
            borrow_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
        },
    );

    cvlr_assert!(false);
}

/// V-8 (controller half, Certora Hub M-03 analogue): listing an asset that is
/// already listed on the spoke must revert. Without this, a second
/// `add_asset_to_spoke` would silently overwrite the risk parameters of a
/// market that already carries positions and `SpokeUsage`, re-basing the caps
/// under live exposure. Mirrors `spoke_add_asset_to_deprecated_category`.
#[rule]
fn spoke_add_asset_to_listed_asset(e: Env, asset: Address) {
    let category_id = crate::spec::fixture::SPOKE_ID;
    // `seed_market` lists `asset` on `SPOKE_ID` with an active (non-deprecated)
    // spoke, so the only gate this call can trip is the already-listed check.
    crate::spec::fixture::seed_market(&e, &asset);

    let spoke = crate::storage::try_get_spoke(&e, category_id);
    cvlr_assume!(matches!(&spoke, Some(cfg) if !cfg.is_deprecated));
    cvlr_assume!(crate::storage::get_spoke_asset(&e, category_id, &hub0(&asset)).is_some());

    crate::config::add_asset_to_spoke(
        &e,
        &SpokeAssetArgs {
            hub_id: crate::spec::fixture::HUB_ID,
            asset,
            spoke_id: category_id,
            can_collateral: true,
            can_borrow: true,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: 9_000,
            threshold: 9_300,
            bonus: 300,
            liquidation_fees: 0,
            supply_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
            borrow_cap: crate::spec::fixture::UNCONSTRAINED_CAP,
        },
    );

    cvlr_assert!(false);
}

#[rule]
fn spoke_supply_sanity(e: Env, caller: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let amount = crate::constants::WAD;
    let spoke_id = crate::spec::fixture::SPOKE_ID;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);

    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    assets.push_back((hub0(&asset), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, spoke_id, &assets);
    cvlr_satisfy!(true);
}

#[rule]
fn spoke_borrow_sanity(e: Env, caller: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let amount = crate::constants::WAD;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::compat::supply_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        amount * 4,
    );

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let mut borrows: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    borrows.push_back((hub0(&asset), amount));
    crate::positions::process_borrow(&e, &caller, account_id, &borrows, None);
    cvlr_satisfy!(true);
}

#[rule]
fn deprecated_spoke_withdraw_sanity(e: Env, caller: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let amount = crate::constants::WAD;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::compat::supply_single(
        e.clone(),
        caller.clone(),
        account_id,
        asset.clone(),
        amount * 2,
    );
    let mut deprecated = crate::storage::get_spoke(&e, crate::spec::fixture::SPOKE_ID);
    deprecated.is_deprecated = true;
    crate::storage::set_spoke(&e, crate::spec::fixture::SPOKE_ID, &deprecated);
    crate::spec::compat::withdraw_single(e, caller, account_id, asset, amount);
    cvlr_satisfy!(true);
}

// ---------------------------------------------------------------------------
// Bulk position-limit proofs.
//
// `validate_bulk_position_limits` (risk/validation.rs:50) de-duplicates
// repeated assets *within one call* (`seen` map) before comparing the new
// unique-position count against the configured limits. These rules pin that
// the duplicated-leg bulk flow at the exact boundary succeeds (supply) or
// reverts only when a *second distinct* leg would exceed the cap (supply and
// borrow), and that a fresh multi-leg supply persists both records.
// ---------------------------------------------------------------------------

#[rule]
fn bulk_supply_duplicate_asset_counted_once(
    e: Env,
    caller: Address,
    account_id: u64,
    asset_a: Address,
    s1: Address,
    s2: Address,
    s3: Address,
    s4: Address,
    s5: Address,
    s6: Address,
    s7: Address,
    s8: Address,
    s9: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);
    crate::spec::fixture::seed_market(&e, &asset_a);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let assets = [s1, s2, s3, s4, s5, s6, s7, s8, s9];
    cvlr_assume!(
        assets[0] != assets[1]
            && assets[0] != assets[2]
            && assets[0] != assets[3]
            && assets[0] != assets[4]
            && assets[0] != assets[5]
            && assets[0] != assets[6]
            && assets[0] != assets[7]
            && assets[0] != assets[8]
    );
    cvlr_assume!(
        assets[1] != assets[2]
            && assets[1] != assets[3]
            && assets[1] != assets[4]
            && assets[1] != assets[5]
            && assets[1] != assets[6]
            && assets[1] != assets[7]
            && assets[1] != assets[8]
    );
    cvlr_assume!(
        assets[2] != assets[3]
            && assets[2] != assets[4]
            && assets[2] != assets[5]
            && assets[2] != assets[6]
            && assets[2] != assets[7]
            && assets[2] != assets[8]
    );
    cvlr_assume!(
        assets[3] != assets[4]
            && assets[3] != assets[5]
            && assets[3] != assets[6]
            && assets[3] != assets[7]
            && assets[3] != assets[8]
    );
    cvlr_assume!(
        assets[4] != assets[5]
            && assets[4] != assets[6]
            && assets[4] != assets[7]
            && assets[4] != assets[8]
    );
    cvlr_assume!(assets[5] != assets[6] && assets[5] != assets[7] && assets[5] != assets[8]);
    cvlr_assume!(assets[6] != assets[7] && assets[6] != assets[8]);
    cvlr_assume!(assets[7] != assets[8]);
    cvlr_assume!(
        asset_a != assets[0]
            && asset_a != assets[1]
            && asset_a != assets[2]
            && asset_a != assets[3]
            && asset_a != assets[4]
            && asset_a != assets[5]
            && asset_a != assets[6]
            && asset_a != assets[7]
            && asset_a != assets[8]
    );

    let seeded = crate::spec::fixture::seed_supply_positions(&e, account_id, &assets);
    cvlr_assume!(seeded == 9);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &legs);

    // The duplicated leg is counted once for the limit check: at 9+1 unique
    // positions the call must reach the boundary without reverting, and the
    // new asset's record must be persisted.
    let book = crate::storage::get_supply_positions(&e, account_id);
    cvlr_assert!(book
        .get(crate::spec::fixture::hub_asset(&asset_a))
        .is_some());
    cvlr_assert!(book.len() == 10);
}

#[rule]
fn bulk_supply_distinct_legs_exceed_limit_reverts(
    e: Env,
    caller: Address,
    account_id: u64,
    asset_a: Address,
    asset_b: Address,
    s1: Address,
    s2: Address,
    s3: Address,
    s4: Address,
    s5: Address,
    s6: Address,
    s7: Address,
    s8: Address,
    s9: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(asset_a != asset_b);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);
    crate::spec::fixture::seed_market(&e, &asset_a);
    crate::spec::fixture::seed_market(&e, &asset_b);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let assets = [s1, s2, s3, s4, s5, s6, s7, s8, s9];
    cvlr_assume!(
        assets[0] != assets[1]
            && assets[0] != assets[2]
            && assets[0] != assets[3]
            && assets[0] != assets[4]
            && assets[0] != assets[5]
            && assets[0] != assets[6]
            && assets[0] != assets[7]
            && assets[0] != assets[8]
    );
    cvlr_assume!(
        assets[1] != assets[2]
            && assets[1] != assets[3]
            && assets[1] != assets[4]
            && assets[1] != assets[5]
            && assets[1] != assets[6]
            && assets[1] != assets[7]
            && assets[1] != assets[8]
    );
    cvlr_assume!(
        assets[2] != assets[3]
            && assets[2] != assets[4]
            && assets[2] != assets[5]
            && assets[2] != assets[6]
            && assets[2] != assets[7]
            && assets[2] != assets[8]
    );
    cvlr_assume!(
        assets[3] != assets[4]
            && assets[3] != assets[5]
            && assets[3] != assets[6]
            && assets[3] != assets[7]
            && assets[3] != assets[8]
    );
    cvlr_assume!(
        assets[4] != assets[5]
            && assets[4] != assets[6]
            && assets[4] != assets[7]
            && assets[4] != assets[8]
    );
    cvlr_assume!(assets[5] != assets[6] && assets[5] != assets[7] && assets[5] != assets[8]);
    cvlr_assume!(assets[6] != assets[7] && assets[6] != assets[8]);
    cvlr_assume!(assets[7] != assets[8]);
    cvlr_assume!(
        asset_a != assets[0]
            && asset_a != assets[1]
            && asset_a != assets[2]
            && asset_a != assets[3]
            && asset_a != assets[4]
            && asset_a != assets[5]
            && asset_a != assets[6]
            && asset_a != assets[7]
            && asset_a != assets[8]
    );
    cvlr_assume!(
        asset_b != assets[0]
            && asset_b != assets[1]
            && asset_b != assets[2]
            && asset_b != assets[3]
            && asset_b != assets[4]
            && asset_b != assets[5]
            && asset_b != assets[6]
            && asset_b != assets[7]
            && asset_b != assets[8]
    );

    let seeded = crate::spec::fixture::seed_supply_positions(&e, account_id, &assets);
    cvlr_assume!(seeded == 9);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    legs.push_back((crate::spec::fixture::hub_asset(&asset_b), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &legs);

    // Two distinct new assets would bring the account to 11 > POSITION_LIMIT_MAX.
    cvlr_assert!(false);
}

#[rule]
fn bulk_supply_two_assets_both_persisted(
    e: Env,
    caller: Address,
    account_id: u64,
    asset_a: Address,
    asset_b: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(asset_a != asset_b);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);
    crate::spec::fixture::seed_market(&e, &asset_a);
    crate::spec::fixture::seed_market(&e, &asset_b);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    legs.push_back((crate::spec::fixture::hub_asset(&asset_b), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &legs);

    let book = crate::storage::get_supply_positions(&e, account_id);
    cvlr_assert!(book.len() == 2);
    cvlr_assert!(book
        .get(crate::spec::fixture::hub_asset(&asset_a))
        .is_some());
    cvlr_assert!(book
        .get(crate::spec::fixture::hub_asset(&asset_b))
        .is_some());
}

#[rule]
fn bulk_borrow_duplicate_leg_not_double_counted(
    e: Env,
    caller: Address,
    account_id: u64,
    asset_a: Address,
    collateral: Address,
    s1: Address,
    s2: Address,
    s3: Address,
    s4: Address,
    s5: Address,
    s6: Address,
    s7: Address,
    s8: Address,
    s9: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(asset_a != collateral);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);
    crate::spec::fixture::seed_market(&e, &asset_a);
    crate::spec::fixture::seed_market(&e, &collateral);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let assets = [s1, s2, s3, s4, s5, s6, s7, s8, s9];
    cvlr_assume!(
        assets[0] != assets[1]
            && assets[0] != assets[2]
            && assets[0] != assets[3]
            && assets[0] != assets[4]
            && assets[0] != assets[5]
            && assets[0] != assets[6]
            && assets[0] != assets[7]
            && assets[0] != assets[8]
    );
    cvlr_assume!(
        assets[1] != assets[2]
            && assets[1] != assets[3]
            && assets[1] != assets[4]
            && assets[1] != assets[5]
            && assets[1] != assets[6]
            && assets[1] != assets[7]
            && assets[1] != assets[8]
    );
    cvlr_assume!(
        assets[2] != assets[3]
            && assets[2] != assets[4]
            && assets[2] != assets[5]
            && assets[2] != assets[6]
            && assets[2] != assets[7]
            && assets[2] != assets[8]
    );
    cvlr_assume!(
        assets[3] != assets[4]
            && assets[3] != assets[5]
            && assets[3] != assets[6]
            && assets[3] != assets[7]
            && assets[3] != assets[8]
    );
    cvlr_assume!(
        assets[4] != assets[5]
            && assets[4] != assets[6]
            && assets[4] != assets[7]
            && assets[4] != assets[8]
    );
    cvlr_assume!(assets[5] != assets[6] && assets[5] != assets[7] && assets[5] != assets[8]);
    cvlr_assume!(assets[6] != assets[7] && assets[6] != assets[8]);
    cvlr_assume!(assets[7] != assets[8]);
    cvlr_assume!(
        asset_a != assets[0]
            && asset_a != assets[1]
            && asset_a != assets[2]
            && asset_a != assets[3]
            && asset_a != assets[4]
            && asset_a != assets[5]
            && asset_a != assets[6]
            && asset_a != assets[7]
            && asset_a != assets[8]
    );
    cvlr_assume!(
        collateral != assets[0]
            && collateral != assets[1]
            && collateral != assets[2]
            && collateral != assets[3]
            && collateral != assets[4]
            && collateral != assets[5]
            && collateral != assets[6]
            && collateral != assets[7]
            && collateral != assets[8]
    );

    // Positive collateral book so a borrow path can pass the health gate.
    crate::spec::fixture::seed_supply_position(&e, account_id, &collateral, common::constants::RAY);
    let seeded = crate::spec::fixture::seed_debt_positions(&e, account_id, &assets);
    cvlr_assume!(seeded == 9);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    crate::positions::process_borrow(&e, &caller, account_id, &legs, None);

    // A duplicated leg counts once: 9 + 1 unique positions stays within the
    // cap, so a borrow of the new asset is reachable.
    cvlr_satisfy!(crate::storage::get_debt_positions(&e, account_id)
        .get(crate::spec::fixture::hub_asset(&asset_a))
        .is_some());
}

#[rule]
fn bulk_borrow_distinct_legs_exceed_limit_reverts(
    e: Env,
    caller: Address,
    account_id: u64,
    asset_a: Address,
    asset_b: Address,
    s1: Address,
    s2: Address,
    s3: Address,
    s4: Address,
    s5: Address,
    s6: Address,
    s7: Address,
    s8: Address,
    s9: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(asset_a != asset_b);
    crate::spec::fixture::seed_protocol(&e);
    crate::spec::fixture::seed_account(&e, account_id, &caller);
    crate::spec::fixture::seed_market(&e, &asset_a);
    crate::spec::fixture::seed_market(&e, &asset_b);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let assets = [s1, s2, s3, s4, s5, s6, s7, s8, s9];
    cvlr_assume!(
        assets[0] != assets[1]
            && assets[0] != assets[2]
            && assets[0] != assets[3]
            && assets[0] != assets[4]
            && assets[0] != assets[5]
            && assets[0] != assets[6]
            && assets[0] != assets[7]
            && assets[0] != assets[8]
    );
    cvlr_assume!(
        assets[1] != assets[2]
            && assets[1] != assets[3]
            && assets[1] != assets[4]
            && assets[1] != assets[5]
            && assets[1] != assets[6]
            && assets[1] != assets[7]
            && assets[1] != assets[8]
    );
    cvlr_assume!(
        assets[2] != assets[3]
            && assets[2] != assets[4]
            && assets[2] != assets[5]
            && assets[2] != assets[6]
            && assets[2] != assets[7]
            && assets[2] != assets[8]
    );
    cvlr_assume!(
        assets[3] != assets[4]
            && assets[3] != assets[5]
            && assets[3] != assets[6]
            && assets[3] != assets[7]
            && assets[3] != assets[8]
    );
    cvlr_assume!(
        assets[4] != assets[5]
            && assets[4] != assets[6]
            && assets[4] != assets[7]
            && assets[4] != assets[8]
    );
    cvlr_assume!(assets[5] != assets[6] && assets[5] != assets[7] && assets[5] != assets[8]);
    cvlr_assume!(assets[6] != assets[7] && assets[6] != assets[8]);
    cvlr_assume!(assets[7] != assets[8]);
    cvlr_assume!(
        asset_a != assets[0]
            && asset_a != assets[1]
            && asset_a != assets[2]
            && asset_a != assets[3]
            && asset_a != assets[4]
            && asset_a != assets[5]
            && asset_a != assets[6]
            && asset_a != assets[7]
            && asset_a != assets[8]
    );
    cvlr_assume!(
        asset_b != assets[0]
            && asset_b != assets[1]
            && asset_b != assets[2]
            && asset_b != assets[3]
            && asset_b != assets[4]
            && asset_b != assets[5]
            && asset_b != assets[6]
            && asset_b != assets[7]
            && asset_b != assets[8]
    );

    let seeded = crate::spec::fixture::seed_debt_positions(&e, account_id, &assets);
    cvlr_assume!(seeded == 9);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((crate::spec::fixture::hub_asset(&asset_a), amount));
    legs.push_back((crate::spec::fixture::hub_asset(&asset_b), amount));
    // The borrow position-limit gate runs inside validate_position_entry_gates
    // before any health computation: 9 + 2 distinct -> PositionLimitExceeded.
    crate::positions::process_borrow(&e, &caller, account_id, &legs, None);

    cvlr_assert!(false);
}

// ---------------------------------------------------------------------------
// V-5 — spoke usage reconciliation (stream S1a, `usage_` prefix).
//
// `SpokeUsage(spoke_id, hub_asset)` is the ONLY per-spoke record of cap
// consumption: the pool keeps no per-spoke book at all. It is a *second*
// accumulator, maintained by a separate code path (`spoke_usage.rs`) from the
// position maps it shadows. A verb that mutates a position but skips its
// `apply_leg_usage` call breaks cap enforcement in either direction —
// under-count lets a spoke exceed its cap, over-count locks out legitimate
// supply — and nothing else in the system would notice.
//
// The property proved below, for one `(spoke_id, hub_asset)` cell:
//
//     usage_after - usage_before == scaled_after - scaled_before
//     usage_after >= 0
//
// on BOTH sides at once, so "the side this verb does not touch stays put" is
// proved together with "the side it does touch moves by exactly the leg
// delta".
//
// The scaled term is a *sum over affected accounts* (`stored_scaled_totals` /
// `account_scaled_totals` take a slice), not a single account read.
// `SeizeMode::Credit` moves collateral between two accounts on the same spoke
// in one call, where the per-account deltas cancel and only the total is
// invariant; stream S1b passes both accounts in one slice rather than
// restating the property.
//
// One caveat the slice form makes precise, and which the liquidation rules
// below turn on: the summed delta is NOT zero in credit mode. The protocol
// fee shares are reclassified into pool revenue by `absorb_supply_as_revenue`
// and so leave the account system altogether, which the implementation books
// with an explicit `apply_spoke_exit` (apply.rs:197). Sum over the two
// accounts therefore falls by exactly the fee, and usage falls with it. Were
// that exit missing, usage would ratchet up on every credit-mode liquidation
// and `remove_asset_from_spoke` — which requires a zero usage row — would
// become permanently unreachable for the asset.
// ---------------------------------------------------------------------------

/// Upper bound for every seeded scaled amount and usage row in this section.
///
/// Keeps `SpokeUsage` far inside `UNCONSTRAINED_CAP` so a cap revert can never
/// silently make a rule vacuous, and keeps every difference computed below far
/// from the `i128` domain edges.
const USAGE_SEED_MAX: i128 = 20 * common::constants::RAY;

/// Scaled supply and debt recorded for one hub asset across a set of accounts.
#[derive(Clone, Copy)]
struct ScaledTotals {
    supply: i128,
    debt: i128,
}

/// Sums the scaled supply and debt held in `asset` across `accounts`, read
/// from storage.
///
/// Deliberately a sum over a slice rather than a single-account read: see the
/// shape note above. Returns `None` on overflow, so callers prove the totals
/// are representable instead of assuming it.
fn stored_scaled_totals(e: &Env, accounts: &[u64], asset: &Address) -> Option<ScaledTotals> {
    let mut totals = ScaledTotals { supply: 0, debt: 0 };
    for account_id in accounts {
        let supply = crate::storage::positions::get_scaled_amount(
            e,
            *account_id,
            AccountPositionType::Deposit,
            asset,
        );
        let debt = crate::storage::positions::get_scaled_amount(
            e,
            *account_id,
            AccountPositionType::Borrow,
            asset,
        );
        totals.supply = totals.supply.checked_add(supply)?;
        totals.debt = totals.debt.checked_add(debt)?;
    }
    Some(totals)
}

/// The same totals as `stored_scaled_totals`, read from in-memory `Account`
/// values.
///
/// The leg primitives mutate the in-memory account and buffer usage in the
/// `Cache`; only `finalize_position_flow` writes both out. Leg-level rules
/// must therefore compare in-memory positions against explicitly persisted
/// usage, or they would compare a moved position against a stale storage read.
fn account_scaled_totals(accounts: &[&Account], hub_asset: &HubAssetKey) -> Option<ScaledTotals> {
    let mut totals = ScaledTotals { supply: 0, debt: 0 };
    for account in accounts {
        let supply = account
            .supply_positions
            .get(hub_asset.clone())
            .map_or(0, |p| p.scaled_amount);
        let debt = account
            .borrow_positions
            .get(hub_asset.clone())
            .map_or(0, |p| p.scaled_amount);
        totals.supply = totals.supply.checked_add(supply)?;
        totals.debt = totals.debt.checked_add(debt)?;
    }
    Some(totals)
}

/// Reads the stored usage row for `asset` on the fixture spoke.
fn usage_row(e: &Env, asset: &Address) -> SpokeUsageRaw {
    crate::spec::fixture::spoke_usage(e, crate::spec::fixture::SPOKE_ID, &hub0(asset))
}

/// The V-5 property itself, in the shape stream S1b generalizes.
///
/// Both sides are asserted, so an endpoint that moves the wrong side (or moves
/// a side it should not touch) fails here just as loudly as one that forgets
/// `apply_leg_usage` entirely.
fn assert_usage_tracks_scaled(
    before: &SpokeUsageRaw,
    after: &SpokeUsageRaw,
    scaled_before: Option<ScaledTotals>,
    scaled_after: Option<ScaledTotals>,
) {
    let (Some(scaled_before), Some(scaled_after)) = (scaled_before, scaled_after) else {
        // Unreachable for the bounded seeds used here; asserting rather than
        // assuming keeps an overflowing total from silently passing.
        cvlr_assert!(false);
        return;
    };

    match (
        after
            .supplied_scaled_ray
            .checked_sub(before.supplied_scaled_ray),
        scaled_after.supply.checked_sub(scaled_before.supply),
    ) {
        (Some(usage_delta), Some(scaled_delta)) => cvlr_assert!(usage_delta == scaled_delta),
        _ => cvlr_assert!(false),
    }

    match (
        after
            .borrowed_scaled_ray
            .checked_sub(before.borrowed_scaled_ray),
        scaled_after.debt.checked_sub(scaled_before.debt),
    ) {
        (Some(usage_delta), Some(scaled_delta)) => cvlr_assert!(usage_delta == scaled_delta),
        _ => cvlr_assert!(false),
    }

    // Negative usage would hand the spoke unbounded cap headroom: the cap
    // check compares `usage + delta <= cap_scaled`, so a negative accumulator
    // is indistinguishable from a raised cap.
    cvlr_assert!(after.supplied_scaled_ray >= 0);
    cvlr_assert!(after.borrowed_scaled_ray >= 0);
}

/// Seeds a live account holding both a supply and a debt position in `asset`,
/// plus a usage row that covers both, and returns the seeded usage row.
///
/// Seeding usage at or above the account's own scaled amounts is the
/// production-faithful state: the row is the sum over every account bound to
/// the spoke, and this account is only one of them.
fn seed_usage_scenario(
    e: &Env,
    account_id: u64,
    caller: &Address,
    asset: &Address,
    supply_scaled: i128,
    debt_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    crate::spec::fixture::seed_live_account(e, account_id, caller, asset);
    crate::spec::fixture::seed_supply_position(e, account_id, asset, supply_scaled);
    crate::spec::fixture::seed_debt_position(e, account_id, asset, debt_scaled);
    crate::spec::fixture::seed_spoke_usage(e, asset, usage_supply, usage_debt);
}

/// Constrains the four seed parameters shared by the scenario rules.
fn assume_usage_seeds(
    supply_scaled: i128,
    debt_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    cvlr_assume!(supply_scaled > 0 && supply_scaled <= USAGE_SEED_MAX);
    cvlr_assume!(debt_scaled > 0 && debt_scaled <= USAGE_SEED_MAX);
    cvlr_assume!(usage_supply >= supply_scaled && usage_supply <= USAGE_SEED_MAX);
    cvlr_assume!(usage_debt >= debt_scaled && usage_debt <= USAGE_SEED_MAX);
}

// ---------------------------------------------------------------------------
// Endpoint rules: process_supply, process_withdraw, process_borrow,
// process_repay. Each drives the real ABI entry point, so the proof covers
// the whole chain — gate, pool call, merge, `apply_leg_usage`, and the
// `finalize_position_flow` persist — not just the leg in isolation.
// ---------------------------------------------------------------------------

#[rule]
fn usage_supply_tracks_scaled_delta(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    cvlr_assume!(account_id != 0);
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(usage_supply >= 0 && usage_supply <= USAGE_SEED_MAX);
    cvlr_assume!(usage_debt >= 0 && usage_debt <= USAGE_SEED_MAX);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_spoke_usage(&e, &asset, usage_supply, usage_debt);

    let accounts = [account_id];
    let usage_before = usage_row(&e, &asset);
    let scaled_before = stored_scaled_totals(&e, &accounts, &asset);

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset.clone(), amount);

    let usage_after = usage_row(&e, &asset);
    let scaled_after = stored_scaled_totals(&e, &accounts, &asset);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);
}

#[rule]
fn usage_withdraw_tracks_scaled_delta(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    supply_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    cvlr_assume!(account_id != 0);
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(supply_scaled > 0 && supply_scaled <= USAGE_SEED_MAX);
    cvlr_assume!(usage_supply >= supply_scaled && usage_supply <= USAGE_SEED_MAX);
    cvlr_assume!(usage_debt >= 0 && usage_debt <= USAGE_SEED_MAX);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_supply_position(&e, account_id, &asset, supply_scaled);
    crate::spec::fixture::seed_spoke_usage(&e, &asset, usage_supply, usage_debt);

    let accounts = [account_id];
    let usage_before = usage_row(&e, &asset);
    let scaled_before = stored_scaled_totals(&e, &accounts, &asset);

    crate::spec::compat::withdraw_single(e.clone(), caller, account_id, asset.clone(), amount);

    let usage_after = usage_row(&e, &asset);
    let scaled_after = stored_scaled_totals(&e, &accounts, &asset);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);
}

#[rule]
fn usage_borrow_tracks_scaled_delta(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    supply_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    cvlr_assume!(account_id != 0);
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(supply_scaled > 0 && supply_scaled <= USAGE_SEED_MAX);
    cvlr_assume!(usage_supply >= supply_scaled && usage_supply <= USAGE_SEED_MAX);
    cvlr_assume!(usage_debt >= 0 && usage_debt <= USAGE_SEED_MAX);
    // Collateral in the same hub asset, so the borrow must move the borrow
    // side of this cell and leave the supply side of the same cell untouched.
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_supply_position(&e, account_id, &asset, supply_scaled);
    crate::spec::fixture::seed_spoke_usage(&e, &asset, usage_supply, usage_debt);

    let accounts = [account_id];
    let usage_before = usage_row(&e, &asset);
    let scaled_before = stored_scaled_totals(&e, &accounts, &asset);

    crate::spec::compat::borrow_single(e.clone(), caller, account_id, asset.clone(), amount);

    let usage_after = usage_row(&e, &asset);
    let scaled_after = stored_scaled_totals(&e, &accounts, &asset);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);
}

#[rule]
fn usage_repay_tracks_scaled_delta(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    debt_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    cvlr_assume!(account_id != 0);
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(debt_scaled > 0 && debt_scaled <= USAGE_SEED_MAX);
    cvlr_assume!(usage_debt >= debt_scaled && usage_debt <= USAGE_SEED_MAX);
    cvlr_assume!(usage_supply >= 0 && usage_supply <= USAGE_SEED_MAX);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_debt_position(&e, account_id, &asset, debt_scaled);
    crate::spec::fixture::seed_spoke_usage(&e, &asset, usage_supply, usage_debt);

    let accounts = [account_id];
    let usage_before = usage_row(&e, &asset);
    let scaled_before = stored_scaled_totals(&e, &accounts, &asset);

    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset.clone(), amount);

    let usage_after = usage_row(&e, &asset);
    let scaled_after = stored_scaled_totals(&e, &accounts, &asset);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);
}

// ---------------------------------------------------------------------------
// Strategy legs. Every strategy (multiply, swap-debt, swap-collateral,
// repay-with-collateral, migrate, close) is assembled from exactly four
// controller-custody primitives, and each one is proved here against an
// arbitrary pool outcome. Driving the primitives rather than the full
// strategies keeps the swap-router trust boundary out of the proof: what is
// at stake is the usage wiring of the leg, not the router.
//
// Usage is buffered in the `Cache` until `finalize_position_flow` runs, so
// these rules persist explicitly. `strategy_finalize` (strategies/mod.rs:65)
// is the single tail every strategy ends in, and it calls
// `finalize_position_flow`; the four endpoint rules above prove that tail
// really writes the buffered rows.
// ---------------------------------------------------------------------------

#[rule]
fn usage_strategy_borrow_leg_tracks_scaled_delta(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    supply_scaled: i128,
    debt_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    cvlr_assume!(account_id != 0);
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    assume_usage_seeds(supply_scaled, debt_scaled, usage_supply, usage_debt);
    seed_usage_scenario(
        &e,
        account_id,
        &caller,
        &asset,
        supply_scaled,
        debt_scaled,
        usage_supply,
        usage_debt,
    );

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);
    let charge_fee: bool = nondet();

    let usage_before = usage_row(&e, &asset);
    let scaled_before = account_scaled_totals(&[&account], &hub);

    crate::positions::borrow_into_controller(
        &e,
        &mut account,
        &hub,
        amount,
        charge_fee,
        PositionAction::Multiply,
        &mut cache,
    );

    cache.persist_spoke_usage();
    let usage_after = usage_row(&e, &asset);
    let scaled_after = account_scaled_totals(&[&account], &hub);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);
}

#[rule]
fn usage_strategy_withdraw_leg_tracks_scaled_delta(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    supply_scaled: i128,
    debt_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    cvlr_assume!(account_id != 0);
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    assume_usage_seeds(supply_scaled, debt_scaled, usage_supply, usage_debt);
    seed_usage_scenario(
        &e,
        account_id,
        &caller,
        &asset,
        supply_scaled,
        debt_scaled,
        usage_supply,
        usage_debt,
    );

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);

    let usage_before = usage_row(&e, &asset);
    let scaled_before = account_scaled_totals(&[&account], &hub);

    let position = crate::positions::get_supply_position_or_panic(&e, &account, &hub);
    crate::positions::execute_withdrawal(
        &e,
        &mut account,
        EventContext {
            counterparty: caller.clone(),
            action: PositionAction::SwColWd,
        },
        crate::positions::WithdrawalRequest {
            hub_asset: &hub,
            amount,
            position: &position,
        },
        &mut cache,
    );

    cache.persist_spoke_usage();
    let usage_after = usage_row(&e, &asset);
    let scaled_after = account_scaled_totals(&[&account], &hub);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);
}

#[rule]
fn usage_strategy_repay_leg_tracks_scaled_delta(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    supply_scaled: i128,
    debt_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    cvlr_assume!(account_id != 0);
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    assume_usage_seeds(supply_scaled, debt_scaled, usage_supply, usage_debt);
    seed_usage_scenario(
        &e,
        account_id,
        &caller,
        &asset,
        supply_scaled,
        debt_scaled,
        usage_supply,
        usage_debt,
    );

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);

    let usage_before = usage_row(&e, &asset);
    let scaled_before = account_scaled_totals(&[&account], &hub);

    let position = crate::positions::get_debt_position_or_panic(&e, &account, &hub);
    crate::positions::execute_repayment(
        &e,
        &mut account,
        EventContext {
            counterparty: caller.clone(),
            action: PositionAction::RpColR,
        },
        crate::positions::RepaymentRequest {
            hub_asset: &hub,
            position: &position,
            amount,
        },
        &mut cache,
    );

    cache.persist_spoke_usage();
    let usage_after = usage_row(&e, &asset);
    let scaled_after = account_scaled_totals(&[&account], &hub);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);
}

/// The net-settle leg is the interesting one: it moves the supply and the
/// debt side of the *same* cell in a single call, through two different merge
/// primitives. Both usage sides must track their own leg.
#[rule]
fn usage_strategy_net_settle_tracks_scaled_delta(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    supply_scaled: i128,
    debt_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    cvlr_assume!(account_id != 0);
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    assume_usage_seeds(supply_scaled, debt_scaled, usage_supply, usage_debt);
    seed_usage_scenario(
        &e,
        account_id,
        &caller,
        &asset,
        supply_scaled,
        debt_scaled,
        usage_supply,
        usage_debt,
    );

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);

    let usage_before = usage_row(&e, &asset);
    let scaled_before = account_scaled_totals(&[&account], &hub);

    crate::strategies::net_settle_collateral_against_debt(
        &e,
        &mut account,
        &mut cache,
        &hub,
        amount,
        PositionAction::RpColNet,
    );

    cache.persist_spoke_usage();
    let usage_after = usage_row(&e, &asset);
    let scaled_after = account_scaled_totals(&[&account], &hub);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);
}

// ---------------------------------------------------------------------------
// Coverage.
//
// The threat is not "one of today's verbs is wrong" — it is "tomorrow's verb
// forgets `apply_leg_usage` and nothing fails". Two mechanisms, one static and
// one semantic:
//
//  1. `usage_coverage_class` matches EXHAUSTIVELY over
//     `events::PositionAction`. Every verb in this codebase tags its position
//     updates with its own variant (Supply, Borrow, ..., RpColNet), so adding
//     a verb means adding a variant, and adding a variant without classifying
//     it here is a COMPILE ERROR under `--features certora-spoke-rules`. The
//     author cannot land the verb without reading this section.
//
//  2. Classifying it as `UsageCoverage::Wave0(..)` then obliges the new verb
//     to name the legs it drives, and `Wave0Legs::contains` / `run_wave0_leg`
//     match exhaustively over `Wave0Leg`. A verb that mutates positions
//     through a *new* primitive therefore needs a new `Wave0Leg` variant,
//     which breaks both matches until `usage_coverage_no_unwired_verb`
//     actually exercises it. Escaping coverage requires actively
//     mis-classifying a verb, not merely forgetting one.
// ---------------------------------------------------------------------------

/// One of the merge primitives every position mutation that touches a pool
/// leg funnels through.
#[derive(Clone, Copy, PartialEq)]
enum Wave0Leg {
    /// `positions::supply::merge_supply_leg`.
    SupplyEntry,
    /// `positions::supply::merge_withdraw_leg` with `WithdrawKind::Normal`.
    SupplyExit,
    /// `positions::supply::merge_withdraw_leg` with `WithdrawKind::Liquidation`
    /// — the transfer-mode seizure leg (stream S1b). A separate variant
    /// because `spoke_refresh_for_leg` (supply.rs:414) branches on the kind:
    /// the usage identity must hold on the branch that skips the risk-param
    /// restamp too.
    SupplyExitLiquidation,
    /// `positions::merge_debt_leg` with `LegDirection::Entry`.
    DebtEntry,
    /// `positions::merge_debt_leg` with `LegDirection::Exit`.
    DebtExit,
}

/// The set of merge legs an action's production call sites can drive.
#[derive(Clone, Copy)]
struct Wave0Legs {
    supply_entry: bool,
    supply_exit: bool,
    supply_exit_liquidation: bool,
    debt_entry: bool,
    debt_exit: bool,
}

impl Wave0Legs {
    const NONE: Self = Self {
        supply_entry: false,
        supply_exit: false,
        supply_exit_liquidation: false,
        debt_entry: false,
        debt_exit: false,
    };
    const SUPPLY_ENTRY: Self = Self {
        supply_entry: true,
        ..Self::NONE
    };
    const SUPPLY_EXIT: Self = Self {
        supply_exit: true,
        ..Self::NONE
    };
    const SUPPLY_EXIT_LIQUIDATION: Self = Self {
        supply_exit_liquidation: true,
        ..Self::NONE
    };
    const DEBT_ENTRY: Self = Self {
        debt_entry: true,
        ..Self::NONE
    };
    const DEBT_EXIT: Self = Self {
        debt_exit: true,
        ..Self::NONE
    };
    const DEBT_BOTH: Self = Self {
        debt_entry: true,
        debt_exit: true,
        ..Self::NONE
    };
    const SUPPLY_EXIT_AND_DEBT_EXIT: Self = Self {
        supply_exit: true,
        debt_exit: true,
        ..Self::NONE
    };

    /// Exhaustive over `Wave0Leg` — a new leg kind fails to compile here.
    fn contains(self, leg: Wave0Leg) -> bool {
        match leg {
            Wave0Leg::SupplyEntry => self.supply_entry,
            Wave0Leg::SupplyExit => self.supply_exit,
            Wave0Leg::SupplyExitLiquidation => self.supply_exit_liquidation,
            Wave0Leg::DebtEntry => self.debt_entry,
            Wave0Leg::DebtExit => self.debt_exit,
        }
    }
}

/// Coverage bucket for one `events::PositionAction`.
enum UsageCoverage {
    /// Position mutation that reaches a merge primitive, proved by the rules
    /// in this file. The `Wave0Legs` names every merge leg the action's
    /// production call sites can drive.
    Wave0(Wave0Legs),
    /// Emits a position update but never moves a scaled amount, so there is
    /// no usage delta to track. Proved by `usage_param_refresh_moves_neither`.
    NoScaledMove,
    /// Moves scaled amounts between two accounts without reaching any merge
    /// primitive, so the delta is only meaningful summed over both. Proved by
    /// `usage_liq_credit_seize_sums_over_two_accounts`.
    ///
    /// This is NOT an escape hatch: unlike the `Wave1Liquidation` bucket it
    /// replaces, it names a rule that exists and passes. Do not classify a new
    /// action here unless a cross-account rule actually covers it.
    CrossAccount,
}

/// COVERAGE GUARD — exhaustive over `events::PositionAction`.
///
/// Do not add a `_ =>` arm. The missing-arm compile error is the mechanism
/// that stops a new verb from silently escaping V-5 coverage.
///
/// Each classification is grounded in the action's production call sites:
/// `Supply` supply.rs:345, `Borrow` debt.rs:147, `Withdraw` supply.rs:215,
/// `Repay` debt.rs:96, `LiqRepay`/`LiqSeize` liquidation/apply.rs:78,116,
/// `LiqCredit` liquidation/apply.rs:294 (credit-mode receiver leg),
/// `Multiply` multiply.rs:79 (`borrow_into_controller`), `ParamUpd`
/// keepers.rs:186 (risk-parameter restamp only), `SwDebtR` swap_debt.rs:65,87,
/// `SwColWd` swap_collateral.rs:65, `RpColWd`/`RpColR`/`RpColNet`
/// repay_debt_with_collateral.rs:134,146,104, `CloseWd` legs.rs:138,
/// `Migrate` migrate_blend.rs:146,361.
fn usage_coverage_class(action: PositionAction) -> UsageCoverage {
    match action {
        PositionAction::Supply => UsageCoverage::Wave0(Wave0Legs::SUPPLY_ENTRY),
        PositionAction::Borrow => UsageCoverage::Wave0(Wave0Legs::DEBT_ENTRY),
        PositionAction::Withdraw => UsageCoverage::Wave0(Wave0Legs::SUPPLY_EXIT),
        PositionAction::Repay => UsageCoverage::Wave0(Wave0Legs::DEBT_EXIT),
        // Stream S1b. `apply_liquidation_repayments` (apply.rs:78) reaches
        // exactly one merge primitive, `merge_debt_leg`/Exit, via
        // `apply_repay_batch`.
        PositionAction::LiqRepay => UsageCoverage::Wave0(Wave0Legs::DEBT_EXIT),
        // `apply_liquidation_seizures` (apply.rs:116) reaches exactly one,
        // `merge_withdraw_leg` with `WithdrawKind::Liquidation`, via
        // `apply_withdraw_batch`. Since the receiver's leg was split onto its
        // own `LiqCredit` tag, `LiqSeize` now means exactly one thing: the
        // liquidated account's debit, gross of the protocol fee, in both
        // seize modes.
        PositionAction::LiqSeize => UsageCoverage::Wave0(Wave0Legs::SUPPLY_EXIT_LIQUIDATION),
        // The credit-mode receiver's leg. Drives NO merge primitive — it moves
        // scaled amounts between two accounts directly
        // (`apply_liquidation_share_credit`, apply.rs:143) and books the
        // protocol fee with a bare `apply_spoke_exit`, so the delta only
        // balances when summed over both accounts.
        PositionAction::LiqCredit => UsageCoverage::CrossAccount,
        PositionAction::Multiply => UsageCoverage::Wave0(Wave0Legs::DEBT_ENTRY),
        PositionAction::ParamUpd => UsageCoverage::NoScaledMove,
        PositionAction::SwDebtR => UsageCoverage::Wave0(Wave0Legs::DEBT_BOTH),
        PositionAction::SwColWd => UsageCoverage::Wave0(Wave0Legs::SUPPLY_EXIT),
        PositionAction::RpColWd => UsageCoverage::Wave0(Wave0Legs::SUPPLY_EXIT),
        PositionAction::RpColR => UsageCoverage::Wave0(Wave0Legs::DEBT_EXIT),
        PositionAction::CloseWd => UsageCoverage::Wave0(Wave0Legs::SUPPLY_EXIT),
        PositionAction::Migrate => UsageCoverage::Wave0(Wave0Legs::DEBT_BOTH),
        PositionAction::RpColNet => UsageCoverage::Wave0(Wave0Legs::SUPPLY_EXIT_AND_DEBT_EXIT),
    }
}

/// Decodes a nondeterministic selector into a `PositionAction`.
fn nondet_position_action(sel: u32) -> PositionAction {
    match sel {
        0 => PositionAction::Supply,
        1 => PositionAction::Borrow,
        2 => PositionAction::Withdraw,
        3 => PositionAction::Repay,
        4 => PositionAction::LiqRepay,
        5 => PositionAction::LiqSeize,
        6 => PositionAction::Multiply,
        7 => PositionAction::ParamUpd,
        8 => PositionAction::SwDebtR,
        9 => PositionAction::SwColWd,
        10 => PositionAction::RpColWd,
        11 => PositionAction::RpColR,
        12 => PositionAction::CloseWd,
        13 => PositionAction::Migrate,
        14 => PositionAction::RpColNet,
        _ => PositionAction::LiqCredit,
    }
}

/// Decodes a nondeterministic selector into a `Wave0Leg`.
fn nondet_wave0_leg(sel: u32) -> Wave0Leg {
    match sel {
        0 => Wave0Leg::SupplyEntry,
        1 => Wave0Leg::SupplyExit,
        2 => Wave0Leg::SupplyExitLiquidation,
        3 => Wave0Leg::DebtEntry,
        _ => Wave0Leg::DebtExit,
    }
}

/// Number of `Wave0Leg` variants `nondet_wave0_leg` can produce. Callers
/// constrain their selector with this so a new variant is reachable the
/// moment it is decoded.
const WAVE0_LEG_COUNT: u32 = 5;

/// A pool leg outcome with the index and decimals constrained exactly as the
/// shared pool summary constrains a real one, but with the resulting scaled
/// amount free in both directions and bounded only by `USAGE_SEED_MAX`.
///
/// Leaving the direction free is deliberate: the usage identity must follow
/// from the leg wiring alone, never from the pool's monotonicity. A leg that
/// happened to be correct only because the pool never moves scaled the other
/// way would be a latent break the moment the pool changes.
fn nondet_leg_outcome(amount: i128) -> (LegOutcome, u32) {
    let supply_index: i128 = nondet();
    let borrow_index: i128 = nondet();
    cvlr_assume!(supply_index >= common::constants::SUPPLY_INDEX_FLOOR_RAW);
    cvlr_assume!(supply_index <= common::constants::MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(borrow_index >= common::constants::RAY);
    cvlr_assume!(borrow_index <= common::constants::MAX_BORROW_INDEX_RAY);

    let new_scaled: i128 = nondet();
    cvlr_assume!(new_scaled >= 0 && new_scaled <= USAGE_SEED_MAX);

    let asset_decimals: u32 = nondet();
    cvlr_assume!(asset_decimals <= 27);

    (
        LegOutcome {
            new_scaled: common::math::fp::Ray::from(new_scaled),
            market_index: MarketIndexRaw {
                borrow_index,
                supply_index,
            },
            amount,
        },
        asset_decimals,
    )
}

/// Drives one merge primitive. Exhaustive over `Wave0Leg` — a new leg kind
/// fails to compile here until it is exercised.
fn run_wave0_leg(
    e: &Env,
    leg: Wave0Leg,
    account: &mut Account,
    hub_asset: &HubAssetKey,
    action: PositionAction,
    amount: i128,
    cache: &mut Cache,
) {
    let (outcome, asset_decimals) = nondet_leg_outcome(amount);
    match leg {
        Wave0Leg::SupplyEntry => {
            let pool_action = PoolAction {
                position: ScaledPositionRaw {
                    scaled_amount: account
                        .supply_positions
                        .get(hub_asset.clone())
                        .map_or(0, |p| p.scaled_amount),
                },
                amount,
                hub_asset: hub_asset.clone(),
            };
            let mutation = PoolPositionMutation {
                position: ScaledPositionRaw {
                    scaled_amount: outcome.new_scaled.raw(),
                },
                market_index: outcome.market_index.clone(),
                actual_amount: outcome.amount,
                asset_decimals,
            };
            crate::positions::supply::merge_supply_leg(e, account, &pool_action, &mutation, cache);
        }
        Wave0Leg::SupplyExit => {
            crate::positions::merge_withdraw_leg(
                e,
                account,
                action,
                hub_asset,
                WithdrawKind::Normal,
                &outcome,
                cache,
            );
        }
        Wave0Leg::SupplyExitLiquidation => {
            crate::positions::merge_withdraw_leg(
                e,
                account,
                action,
                hub_asset,
                WithdrawKind::Liquidation,
                &outcome,
                cache,
            );
        }
        Wave0Leg::DebtEntry => {
            crate::positions::merge_debt_leg(
                e,
                account,
                action,
                hub_asset,
                LegDirection::Entry { asset_decimals },
                &outcome,
                cache,
            );
        }
        Wave0Leg::DebtExit => {
            crate::positions::merge_debt_leg(
                e,
                account,
                action,
                hub_asset,
                LegDirection::Exit,
                &outcome,
                cache,
            );
        }
    }
}

/// No position moves without the matching usage move — for every
/// `PositionAction` this codebase defines, against an arbitrary pool outcome.
#[rule]
fn usage_coverage_no_unwired_verb(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    action_sel: u32,
    leg_sel: u32,
    supply_scaled: i128,
    debt_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    cvlr_assume!(account_id != 0);
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(action_sel < 16);
    cvlr_assume!(leg_sel < WAVE0_LEG_COUNT);
    assume_usage_seeds(supply_scaled, debt_scaled, usage_supply, usage_debt);
    seed_usage_scenario(
        &e,
        account_id,
        &caller,
        &asset,
        supply_scaled,
        debt_scaled,
        usage_supply,
        usage_debt,
    );

    let leg = nondet_wave0_leg(leg_sel);
    let action = nondet_position_action(action_sel);
    match usage_coverage_class(action) {
        // Restrict to the legs this action can actually drive in production.
        UsageCoverage::Wave0(legs) => cvlr_assume!(legs.contains(leg)),
        // Proved by `usage_param_refresh_moves_neither`.
        UsageCoverage::NoScaledMove => {
            cvlr_assume!(false);
            return;
        }
        // Proved by `usage_liq_credit_seize_sums_over_two_accounts`, which
        // sums over the liquidated account and the receiver. A single-account
        // rule cannot state this action's invariant, so excluding it here is
        // correct rather than a gap.
        UsageCoverage::CrossAccount => {
            cvlr_assume!(false);
            return;
        }
    }

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);

    let usage_before = usage_row(&e, &asset);
    let scaled_before = account_scaled_totals(&[&account], &hub);

    run_wave0_leg(&e, leg, &mut account, &hub, action, amount, &mut cache);

    cache.persist_spoke_usage();
    let usage_after = usage_row(&e, &asset);
    let scaled_after = account_scaled_totals(&[&account], &hub);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);
}

/// The `NoScaledMove` half of the coverage classification: the keeper
/// threshold refresh emits `ParamUpd` position updates but re-stamps risk
/// parameters only, so it must move neither the scaled amounts nor usage.
#[rule]
fn usage_param_refresh_moves_neither(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    has_risks: bool,
    supply_scaled: i128,
    debt_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    cvlr_assume!(account_id != 0);
    assume_usage_seeds(supply_scaled, debt_scaled, usage_supply, usage_debt);
    seed_usage_scenario(
        &e,
        account_id,
        &caller,
        &asset,
        supply_scaled,
        debt_scaled,
        usage_supply,
        usage_debt,
    );

    let accounts = [account_id];
    let usage_before = usage_row(&e, &asset);
    let scaled_before = stored_scaled_totals(&e, &accounts, &asset);

    let mut account_ids: Vec<u64> = Vec::new(&e);
    account_ids.push_back(account_id);
    crate::keepers::update_account_threshold(&e, caller, has_risks, account_ids);

    let usage_after = usage_row(&e, &asset);
    let scaled_after = stored_scaled_totals(&e, &accounts, &asset);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);
    cvlr_assert!(usage_after.supplied_scaled_ray == usage_before.supplied_scaled_ray);
    cvlr_assert!(usage_after.borrowed_scaled_ray == usage_before.borrowed_scaled_ray);
}

/// Pins the one asymmetry in `SpokeUsageContext`: `apply_entry` inserts a zero
/// row when storage has none (spoke_usage.rs:141), but `apply_exit` returns
/// without writing anything (`MissingUsage::Absent`, spoke_usage.rs:161). On a
/// cell whose usage row is absent, an exit therefore leaves usage at the zero
/// row instead of tracking the position down — the delta identity above holds
/// *given a row*, which is exactly the precondition every exit rule seeds.
///
/// Production only reaches an exit after an entry created the row, so this is
/// a documented carve-out rather than a gap. It is a rule so that it cannot
/// quietly become one.
#[rule]
fn usage_exit_without_usage_row_is_a_noop(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    supply_scaled: i128,
) {
    cvlr_assume!(account_id != 0);
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(supply_scaled > 0 && supply_scaled <= USAGE_SEED_MAX);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_supply_position(&e, account_id, &asset, supply_scaled);
    // Deliberately no `seed_spoke_usage`.
    cvlr_assume!(crate::storage::get_spoke_usage(
        &e,
        crate::spec::fixture::SPOKE_ID,
        &hub0(&asset)
    )
    .is_none());

    crate::spec::compat::withdraw_single(e.clone(), caller, account_id, asset.clone(), amount);

    let usage_after = usage_row(&e, &asset);
    cvlr_assert!(usage_after.supplied_scaled_ray == 0);
    cvlr_assert!(usage_after.borrowed_scaled_ray == 0);
}

// ---------------------------------------------------------------------------
// Reachability witnesses. Each satisfies that its endpoint both completes and
// actually MOVES usage in the expected direction — a bare `satisfy(true)`
// witness would leave a wholly unwired verb looking healthy.
// ---------------------------------------------------------------------------

#[rule]
fn usage_supply_reachable(e: Env, caller: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_spoke_usage(&e, &asset, 0, 0);

    let before = usage_row(&e, &asset);
    crate::spec::compat::supply_single(
        e.clone(),
        caller,
        account_id,
        asset.clone(),
        crate::constants::WAD,
    );
    let after = usage_row(&e, &asset);

    cvlr_satisfy!(after.supplied_scaled_ray > before.supplied_scaled_ray);
}

#[rule]
fn usage_withdraw_reachable(e: Env, caller: Address, asset: Address, supply_scaled: i128) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    cvlr_assume!(supply_scaled > 0 && supply_scaled <= USAGE_SEED_MAX);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_supply_position(&e, account_id, &asset, supply_scaled);
    crate::spec::fixture::seed_spoke_usage(&e, &asset, supply_scaled, 0);

    let before = usage_row(&e, &asset);
    crate::spec::compat::withdraw_single(
        e.clone(),
        caller,
        account_id,
        asset.clone(),
        crate::constants::WAD,
    );
    let after = usage_row(&e, &asset);

    cvlr_satisfy!(after.supplied_scaled_ray < before.supplied_scaled_ray);
}

#[rule]
fn usage_borrow_reachable(e: Env, caller: Address, asset: Address, supply_scaled: i128) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    cvlr_assume!(supply_scaled > 0 && supply_scaled <= USAGE_SEED_MAX);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_supply_position(&e, account_id, &asset, supply_scaled);
    crate::spec::fixture::seed_spoke_usage(&e, &asset, supply_scaled, 0);

    let before = usage_row(&e, &asset);
    crate::spec::compat::borrow_single(
        e.clone(),
        caller,
        account_id,
        asset.clone(),
        crate::constants::WAD,
    );
    let after = usage_row(&e, &asset);

    cvlr_satisfy!(after.borrowed_scaled_ray > before.borrowed_scaled_ray);
}

#[rule]
fn usage_repay_reachable(e: Env, caller: Address, asset: Address, debt_scaled: i128) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    cvlr_assume!(debt_scaled > 0 && debt_scaled <= USAGE_SEED_MAX);
    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_debt_position(&e, account_id, &asset, debt_scaled);
    crate::spec::fixture::seed_spoke_usage(&e, &asset, 0, debt_scaled);

    let before = usage_row(&e, &asset);
    crate::spec::compat::repay_single(
        e.clone(),
        caller,
        account_id,
        asset.clone(),
        crate::constants::WAD,
    );
    let after = usage_row(&e, &asset);

    cvlr_satisfy!(after.borrowed_scaled_ray < before.borrowed_scaled_ray);
}

#[rule]
fn usage_strategy_borrow_leg_reachable(e: Env, caller: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let seed = common::constants::RAY;
    seed_usage_scenario(&e, account_id, &caller, &asset, seed, seed, seed, seed);

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);

    let before = usage_row(&e, &asset);
    crate::positions::borrow_into_controller(
        &e,
        &mut account,
        &hub,
        crate::constants::WAD,
        false,
        PositionAction::Multiply,
        &mut cache,
    );
    cache.persist_spoke_usage();
    let after = usage_row(&e, &asset);

    cvlr_satisfy!(after.borrowed_scaled_ray > before.borrowed_scaled_ray);
}

#[rule]
fn usage_strategy_withdraw_leg_reachable(e: Env, caller: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let seed = common::constants::RAY;
    seed_usage_scenario(&e, account_id, &caller, &asset, seed, seed, seed, seed);

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);

    let before = usage_row(&e, &asset);
    let position = crate::positions::get_supply_position_or_panic(&e, &account, &hub);
    crate::positions::execute_withdrawal(
        &e,
        &mut account,
        EventContext {
            counterparty: caller.clone(),
            action: PositionAction::SwColWd,
        },
        crate::positions::WithdrawalRequest {
            hub_asset: &hub,
            amount: crate::constants::WAD,
            position: &position,
        },
        &mut cache,
    );
    cache.persist_spoke_usage();
    let after = usage_row(&e, &asset);

    cvlr_satisfy!(after.supplied_scaled_ray < before.supplied_scaled_ray);
}

#[rule]
fn usage_strategy_repay_leg_reachable(e: Env, caller: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let seed = common::constants::RAY;
    seed_usage_scenario(&e, account_id, &caller, &asset, seed, seed, seed, seed);

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);

    let before = usage_row(&e, &asset);
    let position = crate::positions::get_debt_position_or_panic(&e, &account, &hub);
    crate::positions::execute_repayment(
        &e,
        &mut account,
        EventContext {
            counterparty: caller.clone(),
            action: PositionAction::RpColR,
        },
        crate::positions::RepaymentRequest {
            hub_asset: &hub,
            position: &position,
            amount: crate::constants::WAD,
        },
        &mut cache,
    );
    cache.persist_spoke_usage();
    let after = usage_row(&e, &asset);

    cvlr_satisfy!(after.borrowed_scaled_ray < before.borrowed_scaled_ray);
}

#[rule]
fn usage_strategy_net_settle_reachable(e: Env, caller: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let seed = common::constants::RAY;
    seed_usage_scenario(&e, account_id, &caller, &asset, seed, seed, seed, seed);

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);

    let before = usage_row(&e, &asset);
    crate::strategies::net_settle_collateral_against_debt(
        &e,
        &mut account,
        &mut cache,
        &hub,
        crate::constants::WAD,
        PositionAction::RpColNet,
    );
    cache.persist_spoke_usage();
    let after = usage_row(&e, &asset);

    // Net settle burns both sides in one call.
    cvlr_satisfy!(
        after.supplied_scaled_ray < before.supplied_scaled_ray
            && after.borrowed_scaled_ray < before.borrowed_scaled_ray
    );
}

#[rule]
fn usage_coverage_dispatch_reachable(e: Env, caller: Address, asset: Address, leg_sel: u32) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let seed = common::constants::RAY;
    cvlr_assume!(leg_sel < WAVE0_LEG_COUNT);
    seed_usage_scenario(&e, account_id, &caller, &asset, seed, seed, seed, seed);

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);
    let leg = nondet_wave0_leg(leg_sel);

    let before = usage_row(&e, &asset);
    run_wave0_leg(
        &e,
        leg,
        &mut account,
        &hub,
        PositionAction::Withdraw,
        crate::constants::WAD,
        &mut cache,
    );
    cache.persist_spoke_usage();
    let after = usage_row(&e, &asset);

    cvlr_satisfy!(
        after.supplied_scaled_ray != before.supplied_scaled_ray
            || after.borrowed_scaled_ray != before.borrowed_scaled_ray
    );
}

// ---------------------------------------------------------------------------
// V-5, liquidation legs (stream S1b, `usage_liq_` prefix).
//
// Liquidation is the only verb family that moves positions on more than one
// account, and the only one that moves value OUT of the account system
// without a pool withdrawal. Three distinct shapes:
//
//  1. Repayment. `apply_liquidation_repayments` (apply.rs:30) pulls the
//     liquidator's tokens and hands the legs to `apply_repay_batch`. Ordinary
//     single-account exit: borrow usage falls by the repaid scaled debt.
//
//  2. `SeizeMode::Transfer`. `apply_liquidation_seizures` (apply.rs:92) hands
//     the legs to `apply_withdraw_batch`. The pool burns the whole seizure —
//     protocol fee included, since the fee is withheld from the *payout*, not
//     from the burn — so supply usage falls by the full seized scaled amount.
//
//  3. `SeizeMode::Credit`. `apply_liquidation_share_credit` (apply.rs:143)
//     debits the liquidated account by the whole scaled seizure `S`, credits
//     the receiver `S - fee`, and books `fee` as a bare `apply_spoke_exit`.
//     Per account nothing reconciles; summed over the pair, supply usage
//     falls by exactly `fee`. NOT by zero: `absorb_supply_as_revenue`
//     reclassifies those shares into pool revenue, so they leave the account
//     system for good. A rule pinning `delta == 0` here would pin a false
//     invariant, and an implementation matching it would ratchet usage up on
//     every credit-mode liquidation until `remove_asset_from_spoke` — which
//     requires a zero usage row — became unreachable for the asset.
//
//  4. Bad-debt cleanup. `execute_bad_debt_cleanup` (bad_debt.rs:14) absorbs
//     every remaining position into revenue or socialized debt and removes
//     the account entry, so usage must shed each wiped position in full.
//
// REACHABILITY NOTE, and the reason the shapes are driven at different
// levels: `positions::liquidation::{apply, bad_debt}` are private modules
// (liquidation/mod.rs:9,10). The spec mounts at `crate::spec` and is not
// their descendant, so `apply_liquidation_repayments`,
// `apply_liquidation_seizures`, `apply_liquidation_share_credit` and
// `execute_bad_debt_cleanup` are NOT callable from here (E0603). Shapes 1
// and 2 are therefore driven through the `pub(crate)` batch primitives those
// functions delegate to — `apply_repay_batch` and `apply_withdraw_batch`,
// which is where every position and usage mutation of those two legs
// happens; the wrappers themselves only run the flag gate, move tokens, and
// compute USD. Shapes 3 and 4 have no such seam — the share credit and the
// cleanup ARE the private functions — so they are driven end to end through
// `process_liquidation` and `clean_bad_debt_standalone`.
// ---------------------------------------------------------------------------

/// The scaled supply shares that left a pair of accounts entirely: what the
/// liquidated account lost, minus what the receiver gained.
///
/// In credit mode this is exactly the protocol fee `split_seized_shares`
/// (liquidation/math.rs:371) carves out of the seizure, because the debit is
/// `S` and the credit is `S - fee`. It is the only value that leaves the
/// account system on the seizure leg, and therefore the only spoke-usage
/// movement the leg may book.
///
/// Returns `None` when any total is unrepresentable, leaving it to the caller
/// to decide whether that is an assertion failure or a witness that simply
/// does not apply. Deliberately free of `cvlr_assert!`/`cvlr_satisfy!`: this
/// helper is shared by rules of both kinds, and `check_orphans.py` classifies
/// a rule by the macros in the source span that follows it.
fn supply_shares_that_left(
    liquidated_before: Option<ScaledTotals>,
    liquidated_after: Option<ScaledTotals>,
    receiver_before: Option<ScaledTotals>,
    receiver_after: Option<ScaledTotals>,
) -> Option<i128> {
    let lost = liquidated_before?
        .supply
        .checked_sub(liquidated_after?.supply)?;
    let gained = receiver_after?
        .supply
        .checked_sub(receiver_before?.supply)?;
    lost.checked_sub(gained)
}

/// Whether the pair's supply shares moved from the liquidated account to the
/// receiver — the observable signature of a share credit that really ran.
fn shares_moved_between(
    liquidated_before: Option<ScaledTotals>,
    liquidated_after: Option<ScaledTotals>,
    receiver_before: Option<ScaledTotals>,
    receiver_after: Option<ScaledTotals>,
) -> bool {
    match (
        liquidated_before,
        liquidated_after,
        receiver_before,
        receiver_after,
    ) {
        (Some(lost_from), Some(lost_to), Some(gained_from), Some(gained_to)) => {
            lost_to.supply < lost_from.supply && gained_to.supply > gained_from.supply
        }
        _ => false,
    }
}

/// Builds the single-leg payment vector the liquidation entry points take.
fn one_payment(e: &Env, asset: &Address, amount: i128) -> Vec<(HubAssetKey, i128)> {
    let mut payments: Vec<(HubAssetKey, i128)> = Vec::new(e);
    payments.push_back((hub0(asset), amount));
    payments
}

/// Seeds a liquidatable account plus a credit-mode receiver bound to the same
/// spoke (`fixture::SPOKE_ID`) and owned by the liquidator, so
/// `resolve_seize_receiver` (liquidation/mod.rs:158) admits it.
///
/// `receiver_holds` selects between the two receiver shapes
/// `credit_supply_shares` (apply.rs:255) distinguishes: merging into an
/// existing position, or stamping a fresh one from the current listing.
fn seed_credit_liquidation(
    e: &Env,
    account_id: u64,
    receiver_id: u64,
    owner: &Address,
    liquidator: &Address,
    collateral_asset: &Address,
    debt_asset: &Address,
    collateral_scaled: i128,
    debt_scaled: i128,
    receiver_holds: bool,
    receiver_scaled: i128,
) {
    crate::spec::fixture::seed_live_account(e, account_id, owner, collateral_asset);
    crate::spec::fixture::seed_market(e, debt_asset);
    crate::spec::fixture::seed_supply_position(e, account_id, collateral_asset, collateral_scaled);
    crate::spec::fixture::seed_debt_position(e, account_id, debt_asset, debt_scaled);

    crate::spec::fixture::seed_account(e, receiver_id, liquidator);
    if receiver_holds {
        crate::spec::fixture::seed_supply_position(
            e,
            receiver_id,
            collateral_asset,
            receiver_scaled,
        );
    }
}

// ---------------------------------------------------------------------------
// Leg 1 — repayment.
// ---------------------------------------------------------------------------

/// The repayment leg of a liquidation decreases borrow usage by exactly the
/// scaled debt the pool burned, and leaves supply usage alone.
///
/// Drives `apply_repay_batch` with `PositionAction::LiqRepay`, which is the
/// whole of `apply_liquidation_repayments`' position and usage effect
/// (apply.rs:78) — the enclosing function adds only the flag gate, the
/// measured token pull, and the USD arithmetic, none of which touch a
/// position map or a usage row.
#[rule]
fn usage_liq_repay_leg_tracks_scaled_delta(
    e: Env,
    liquidator: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    supply_scaled: i128,
    debt_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    cvlr_assume!(account_id != 0);
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    assume_usage_seeds(supply_scaled, debt_scaled, usage_supply, usage_debt);
    seed_usage_scenario(
        &e,
        account_id,
        &liquidator,
        &asset,
        supply_scaled,
        debt_scaled,
        usage_supply,
        usage_debt,
    );

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);

    let position = crate::positions::get_debt_position_or_panic(&e, &account, &hub);
    let mut actions: Vec<PoolAction> = Vec::new(&e);
    actions.push_back(crate::positions::make_pool_action(
        &position,
        amount,
        hub.clone(),
    ));

    let usage_before = usage_row(&e, &asset);
    let scaled_before = account_scaled_totals(&[&account], &hub);

    crate::positions::apply_repay_batch(
        &e,
        &mut account,
        &liquidator,
        PositionAction::LiqRepay,
        &actions,
        &mut cache,
    );

    cache.persist_spoke_usage();
    let usage_after = usage_row(&e, &asset);
    let scaled_after = account_scaled_totals(&[&account], &hub);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);

    // Direction, stated separately from the identity: a repayment that
    // *raised* borrow usage would still satisfy an identity written over a
    // position that moved the same wrong way.
    cvlr_assert!(usage_after.borrowed_scaled_ray < usage_before.borrowed_scaled_ray);
    cvlr_assert!(usage_after.supplied_scaled_ray == usage_before.supplied_scaled_ray);
}

// ---------------------------------------------------------------------------
// Leg 2 — `SeizeMode::Transfer` seizure.
// ---------------------------------------------------------------------------

/// The transfer-mode seizure leg decreases supply usage by the FULL seized
/// scaled amount — protocol fee included — and leaves borrow usage alone.
///
/// Drives `apply_withdraw_batch` with `WithdrawKind::Liquidation` and
/// `PositionAction::LiqSeize`, which is the whole of
/// `apply_liquidation_seizures`' position and usage effect (apply.rs:116).
///
/// The fee is not a second usage movement in this mode: `protocol_fee` rides
/// along on the `PoolWithdrawEntry` and is withheld by the pool from the
/// liquidator's *payout*, while the shares burned out of the liquidated
/// account are the whole seizure. Booking a separate fee exit here — the
/// credit-mode shape — would double-count it.
#[rule]
fn usage_liq_transfer_seize_leg_tracks_scaled_delta(
    e: Env,
    liquidator: Address,
    account_id: u64,
    asset: Address,
    amount: i128,
    protocol_fee: i128,
    supply_scaled: i128,
    debt_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    cvlr_assume!(account_id != 0);
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    // `LiquidationPlan::validate` (liquidation/math.rs:60) admits exactly this
    // range for a seizure entry's fee.
    cvlr_assume!(protocol_fee >= 0 && protocol_fee <= amount);
    assume_usage_seeds(supply_scaled, debt_scaled, usage_supply, usage_debt);
    seed_usage_scenario(
        &e,
        account_id,
        &liquidator,
        &asset,
        supply_scaled,
        debt_scaled,
        usage_supply,
        usage_debt,
    );

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);

    let position = crate::positions::get_supply_position_or_panic(&e, &account, &hub);
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(&e);
    entries.push_back(PoolWithdrawEntry {
        action: crate::positions::make_pool_action(&position, amount, hub.clone()),
        protocol_fee,
    });

    let usage_before = usage_row(&e, &asset);
    let scaled_before = account_scaled_totals(&[&account], &hub);

    crate::positions::apply_withdraw_batch(
        &e,
        &mut account,
        &liquidator,
        WithdrawKind::Liquidation,
        PositionAction::LiqSeize,
        &entries,
        &mut cache,
    );

    cache.persist_spoke_usage();
    let usage_after = usage_row(&e, &asset);
    let scaled_after = account_scaled_totals(&[&account], &hub);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);

    cvlr_assert!(usage_after.supplied_scaled_ray < usage_before.supplied_scaled_ray);
    cvlr_assert!(usage_after.borrowed_scaled_ray == usage_before.borrowed_scaled_ray);
}

// ---------------------------------------------------------------------------
// Leg 3 — `SeizeMode::Credit` seizure, summed over both principals.
// ---------------------------------------------------------------------------

/// Credit-mode liquidation moves collateral between two accounts on one
/// spoke, and supply usage falls by exactly the shares that left the pair —
/// the protocol fee — never by zero and never by the whole seizure.
///
/// Stated over the slice `[liquidated, receiver]`, because per account
/// neither delta reconciles with usage: the liquidated account loses `S`
/// while usage moves by `fee`, and the receiver gains `S - fee` while usage
/// does not move for it at all. The sum is what the accumulator tracks.
///
/// `fee` is not re-derived from the plan — the plan is built inside the call
/// against a nondeterministic market index and a nondeterministic price, so
/// recomputing it from a second invocation would compare two different
/// draws. It is instead *observed* as `lost - gained`, which is the fee by
/// construction of `split_seized_shares`, and the rule pins usage to it.
///
/// If `check_bad_debt_after_liquidation` fires on the residual, the wiped
/// positions are absorbed into revenue too and `lost - gained` grows to
/// cover them; the pinned equality is unchanged, since bad-debt cleanup
/// books its own matching exits.
#[rule]
fn usage_liq_credit_seize_sums_over_two_accounts(
    e: Env,
    liquidator: Address,
    owner: Address,
    collateral_asset: Address,
    debt_asset: Address,
    debt_amount: i128,
    collateral_scaled: i128,
    debt_scaled: i128,
    receiver_holds: bool,
    receiver_scaled: i128,
    usage_supply: i128,
    usage_debt: i128,
) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let receiver_id = account_id + 1;

    cvlr_assume!(owner != liquidator);
    cvlr_assume!(debt_amount > 0 && debt_amount <= crate::constants::WAD * 1000);
    cvlr_assume!(collateral_scaled > 0 && collateral_scaled <= USAGE_SEED_MAX);
    cvlr_assume!(debt_scaled > 0 && debt_scaled <= USAGE_SEED_MAX);
    cvlr_assume!(receiver_scaled > 0 && receiver_scaled <= USAGE_SEED_MAX);
    // The row is the sum over every account on the spoke, so it covers both
    // principals' holdings in the watched asset.
    cvlr_assume!(usage_supply >= collateral_scaled + receiver_scaled);
    cvlr_assume!(usage_supply <= 4 * USAGE_SEED_MAX);
    cvlr_assume!(usage_debt >= debt_scaled && usage_debt <= 4 * USAGE_SEED_MAX);

    seed_credit_liquidation(
        &e,
        account_id,
        receiver_id,
        &owner,
        &liquidator,
        &collateral_asset,
        &debt_asset,
        collateral_scaled,
        debt_scaled,
        receiver_holds,
        receiver_scaled,
    );
    crate::spec::fixture::seed_spoke_usage(&e, &collateral_asset, usage_supply, usage_debt);
    crate::spec::fixture::seed_spoke_usage(&e, &debt_asset, usage_supply, usage_debt);

    let both = [account_id, receiver_id];
    let liquidated = [account_id];
    let receiving = [receiver_id];

    let usage_before = usage_row(&e, &collateral_asset);
    let pair_before = stored_scaled_totals(&e, &both, &collateral_asset);
    let liquidated_before = stored_scaled_totals(&e, &liquidated, &collateral_asset);
    let receiver_before = stored_scaled_totals(&e, &receiving, &collateral_asset);

    let payments = one_payment(&e, &debt_asset, debt_amount);
    let returned = crate::positions::liquidation::process_liquidation(
        &e,
        &liquidator,
        account_id,
        &payments,
        SeizeMode::Credit(receiver_id),
    );

    let usage_after = usage_row(&e, &collateral_asset);
    let pair_after = stored_scaled_totals(&e, &both, &collateral_asset);
    let liquidated_after = stored_scaled_totals(&e, &liquidated, &collateral_asset);
    let receiver_after = stored_scaled_totals(&e, &receiving, &collateral_asset);

    // The V-5 identity in its slice form: usage tracks the pair, both sides.
    assert_usage_tracks_scaled(&usage_before, &usage_after, pair_before, pair_after);

    // The same statement with the fee named, which is the part that would be
    // wrong if it were written as `delta == 0`.
    match supply_shares_that_left(
        liquidated_before,
        liquidated_after,
        receiver_before,
        receiver_after,
    ) {
        Some(fee) => {
            cvlr_assert!(fee >= 0);
            cvlr_assert!(
                usage_before
                    .supplied_scaled_ray
                    .checked_sub(usage_after.supplied_scaled_ray)
                    == Some(fee)
            );
        }
        None => cvlr_assert!(false),
    }

    // The credit is a credit: the receiver may only gain supply, and gains no
    // debt. Without this, "usage fell by lost - gained" would also be
    // satisfied by an implementation that debited the receiver.
    match (receiver_before, receiver_after) {
        (Some(before), Some(after)) => {
            cvlr_assert!(after.supply >= before.supply);
            cvlr_assert!(after.debt == before.debt);
        }
        _ => cvlr_assert!(false),
    }
    cvlr_assert!(returned == receiver_id);
}

// ---------------------------------------------------------------------------
// Leg 4 — bad-debt cleanup.
// ---------------------------------------------------------------------------

/// Bad-debt cleanup sheds every wiped position from spoke usage in full, on
/// both sides, and leaves behind exactly the usage other accounts contributed.
///
/// Seeded as `account holdings + extra`, so `extra == 0` is the literal
/// "usage is driven to zero" case and `extra > 0` proves the cleanup takes
/// down its own positions and nothing else. `execute_bad_debt_cleanup`
/// (bad_debt.rs:14) removes the account entry outright, so a missing
/// `apply_spoke_exit` there is invisible everywhere else in the system: the
/// positions are gone and the usage they consumed would be stranded forever.
#[rule]
fn usage_liq_bad_debt_cleanup_sheds_every_wiped_position(
    e: Env,
    caller: Address,
    account_id: u64,
    asset: Address,
    supply_scaled: i128,
    debt_scaled: i128,
    extra_supply: i128,
    extra_debt: i128,
) {
    cvlr_assume!(account_id != 0);
    cvlr_assume!(supply_scaled > 0 && supply_scaled <= USAGE_SEED_MAX);
    cvlr_assume!(debt_scaled > 0 && debt_scaled <= USAGE_SEED_MAX);
    cvlr_assume!(extra_supply >= 0 && extra_supply <= USAGE_SEED_MAX);
    cvlr_assume!(extra_debt >= 0 && extra_debt <= USAGE_SEED_MAX);

    crate::spec::fixture::seed_live_account(&e, account_id, &caller, &asset);
    crate::spec::fixture::seed_supply_position(&e, account_id, &asset, supply_scaled);
    crate::spec::fixture::seed_debt_position(&e, account_id, &asset, debt_scaled);
    crate::spec::fixture::seed_spoke_usage(
        &e,
        &asset,
        supply_scaled + extra_supply,
        debt_scaled + extra_debt,
    );

    let accounts = [account_id];
    let usage_before = usage_row(&e, &asset);
    let scaled_before = stored_scaled_totals(&e, &accounts, &asset);

    crate::positions::liquidation::clean_bad_debt_standalone(&e, account_id);

    let usage_after = usage_row(&e, &asset);
    let scaled_after = stored_scaled_totals(&e, &accounts, &asset);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);

    // The account entry is removed, so its positions must read as gone.
    match scaled_after {
        Some(after) => {
            cvlr_assert!(after.supply == 0);
            cvlr_assert!(after.debt == 0);
        }
        None => cvlr_assert!(false),
    }
    // What remains is exactly what other accounts on the spoke contributed —
    // zero when this account was the only holder.
    cvlr_assert!(usage_after.supplied_scaled_ray == extra_supply);
    cvlr_assert!(usage_after.borrowed_scaled_ray == extra_debt);
}

// ---------------------------------------------------------------------------
// Reachability witnesses, one per liquidation leg. Each satisfies that usage
// moved in the direction its leg is supposed to move it — a `satisfy(true)`
// witness would look just as healthy on a leg that was never wired to
// `apply_leg_usage` at all, which is the exact failure V-5 exists to catch.
// ---------------------------------------------------------------------------

#[rule]
fn usage_liq_repay_leg_reachable(e: Env, liquidator: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let seed = common::constants::RAY;
    seed_usage_scenario(&e, account_id, &liquidator, &asset, seed, seed, seed, seed);

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);

    let position = crate::positions::get_debt_position_or_panic(&e, &account, &hub);
    let mut actions: Vec<PoolAction> = Vec::new(&e);
    actions.push_back(crate::positions::make_pool_action(
        &position,
        crate::constants::WAD,
        hub.clone(),
    ));

    let before = usage_row(&e, &asset);
    crate::positions::apply_repay_batch(
        &e,
        &mut account,
        &liquidator,
        PositionAction::LiqRepay,
        &actions,
        &mut cache,
    );
    cache.persist_spoke_usage();
    let after = usage_row(&e, &asset);

    cvlr_satisfy!(after.borrowed_scaled_ray < before.borrowed_scaled_ray);
}

#[rule]
fn usage_liq_transfer_seize_leg_reachable(e: Env, liquidator: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let seed = common::constants::RAY;
    seed_usage_scenario(&e, account_id, &liquidator, &asset, seed, seed, seed, seed);

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Cache::new(&e);

    let position = crate::positions::get_supply_position_or_panic(&e, &account, &hub);
    let mut entries: Vec<PoolWithdrawEntry> = Vec::new(&e);
    entries.push_back(PoolWithdrawEntry {
        action: crate::positions::make_pool_action(&position, crate::constants::WAD, hub.clone()),
        protocol_fee: 0,
    });

    let before = usage_row(&e, &asset);
    crate::positions::apply_withdraw_batch(
        &e,
        &mut account,
        &liquidator,
        WithdrawKind::Liquidation,
        PositionAction::LiqSeize,
        &entries,
        &mut cache,
    );
    cache.persist_spoke_usage();
    let after = usage_row(&e, &asset);

    cvlr_satisfy!(after.supplied_scaled_ray < before.supplied_scaled_ray);
}

/// Credit mode completes and really does move shares from the liquidated
/// account to the receiver while supply usage does not rise.
///
/// Kept separate from the fee witness below so that "the share transfer
/// happens at all" and "the fee exit fires" fail independently.
#[rule]
fn usage_liq_credit_seize_reachable(
    e: Env,
    liquidator: Address,
    owner: Address,
    collateral_asset: Address,
    debt_asset: Address,
) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let receiver_id = account_id + 1;
    let seed = 10 * common::constants::RAY;

    cvlr_assume!(owner != liquidator);
    seed_credit_liquidation(
        &e,
        account_id,
        receiver_id,
        &owner,
        &liquidator,
        &collateral_asset,
        &debt_asset,
        seed,
        seed,
        true,
        seed,
    );
    crate::spec::fixture::seed_spoke_usage(&e, &collateral_asset, 4 * seed, 4 * seed);
    crate::spec::fixture::seed_spoke_usage(&e, &debt_asset, 4 * seed, 4 * seed);

    let liquidated = [account_id];
    let receiving = [receiver_id];
    let usage_before = usage_row(&e, &collateral_asset);
    let liquidated_before = stored_scaled_totals(&e, &liquidated, &collateral_asset);
    let receiver_before = stored_scaled_totals(&e, &receiving, &collateral_asset);

    let payments = one_payment(&e, &debt_asset, crate::constants::WAD);
    crate::positions::liquidation::process_liquidation(
        &e,
        &liquidator,
        account_id,
        &payments,
        SeizeMode::Credit(receiver_id),
    );

    let usage_after = usage_row(&e, &collateral_asset);
    let liquidated_after = stored_scaled_totals(&e, &liquidated, &collateral_asset);
    let receiver_after = stored_scaled_totals(&e, &receiving, &collateral_asset);

    let moved = shares_moved_between(
        liquidated_before,
        liquidated_after,
        receiver_before,
        receiver_after,
    );
    cvlr_satisfy!(moved && usage_after.supplied_scaled_ray <= usage_before.supplied_scaled_ray);
}

/// The credit-mode fee exit is reachable: supply usage strictly falls even
/// though no pool withdrawal occurred.
///
/// This is the witness for the `apply_spoke_exit` at apply.rs:197. Were the
/// fee exit deleted, credit mode would leave usage untouched and this rule
/// would go unsatisfiable while every assert rule above still passed on a
/// zero-fee draw.
#[rule]
fn usage_liq_credit_fee_exits_usage_reachable(
    e: Env,
    liquidator: Address,
    owner: Address,
    collateral_asset: Address,
    debt_asset: Address,
) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let receiver_id = account_id + 1;
    let seed = 10 * common::constants::RAY;

    cvlr_assume!(owner != liquidator);
    seed_credit_liquidation(
        &e,
        account_id,
        receiver_id,
        &owner,
        &liquidator,
        &collateral_asset,
        &debt_asset,
        seed,
        seed,
        true,
        seed,
    );
    crate::spec::fixture::seed_spoke_usage(&e, &collateral_asset, 4 * seed, 4 * seed);
    crate::spec::fixture::seed_spoke_usage(&e, &debt_asset, 4 * seed, 4 * seed);

    let usage_before = usage_row(&e, &collateral_asset);

    let payments = one_payment(&e, &debt_asset, crate::constants::WAD);
    crate::positions::liquidation::process_liquidation(
        &e,
        &liquidator,
        account_id,
        &payments,
        SeizeMode::Credit(receiver_id),
    );

    let usage_after = usage_row(&e, &collateral_asset);

    cvlr_satisfy!(usage_after.supplied_scaled_ray < usage_before.supplied_scaled_ray);
}

#[rule]
fn usage_liq_bad_debt_cleanup_reachable(e: Env, caller: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let seed = common::constants::RAY;
    seed_usage_scenario(&e, account_id, &caller, &asset, seed, seed, seed, seed);

    let before = usage_row(&e, &asset);
    crate::positions::liquidation::clean_bad_debt_standalone(&e, account_id);
    let after = usage_row(&e, &asset);

    cvlr_satisfy!(
        after.supplied_scaled_ray < before.supplied_scaled_ray
            && after.borrowed_scaled_ray < before.borrowed_scaled_ray
    );
}
