use cvlr::macros::rule;
use cvlr::nondet::nondet;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Vec};

use crate::context::Context;
use crate::events::PositionAction;
use crate::positions::{LegDirection, LegOutcome, WithdrawKind};
use crate::spec::fixture;
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
    fixture::seed_empty_books(&e, account_id);
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

    let mut cache = crate::context::Context::new(&e);
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

/// The listing arguments the `add_asset_to_spoke` rules share: a valid
/// LTV/threshold pair and uncapped supply and borrow, so nothing but the gate
/// under test can reject the call.
fn listing_args(asset: &Address, spoke_id: u32) -> SpokeAssetArgs {
    SpokeAssetArgs {
        hub_id: crate::spec::fixture::HUB_ID,
        asset: asset.clone(),
        spoke_id,
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
    }
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

/// Listing an asset that the spoke already lists reverts with
/// `AssetAlreadyInSpoke`. A second `add_asset_to_spoke` would otherwise
/// overwrite the risk parameters and caps of a market with live positions and
/// `SpokeUsage`. Mirrors `spoke_add_asset_to_deprecated_category`.
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

/// Satisfy twin of `spoke_add_asset_to_deprecated_category`: the same listing
/// on an *active* spoke completes, so the revert rule is a statement about
/// deprecation and not about an unreachable listing call.
#[rule]
fn spoke_add_asset_to_deprecated_category_fixture_completes(e: Env, asset: Address) {
    let category_id = fixture::SPOKE_ID;
    fixture::seed_protocol(&e);

    let spoke = crate::storage::try_get_spoke(&e, category_id);
    cvlr_assume!(matches!(&spoke, Some(cfg) if !cfg.is_deprecated));
    cvlr_assume!(crate::storage::get_spoke_asset(&e, category_id, &hub0(&asset)).is_none());

    crate::config::add_asset_to_spoke(&e, &listing_args(&asset, category_id));

    cvlr_satisfy!(crate::storage::get_spoke_asset(&e, category_id, &hub0(&asset)).is_some());
}

/// Satisfy twin of `spoke_add_asset_to_listed_asset`: a spoke that already
/// lists one asset still admits a *different*, unlisted one, so the revert
/// rule is a statement about re-listing one asset and not about a spoke that
/// refuses every listing.
#[rule]
fn spoke_add_asset_to_listed_asset_fixture_completes(e: Env, listed: Address, fresh: Address) {
    let category_id = fixture::SPOKE_ID;
    cvlr_assume!(listed != fresh);
    fixture::seed_market(&e, &listed);

    let spoke = crate::storage::try_get_spoke(&e, category_id);
    cvlr_assume!(matches!(&spoke, Some(cfg) if !cfg.is_deprecated));
    cvlr_assume!(crate::storage::get_spoke_asset(&e, category_id, &hub0(&listed)).is_some());
    cvlr_assume!(crate::storage::get_spoke_asset(&e, category_id, &hub0(&fresh)).is_none());

    crate::config::add_asset_to_spoke(&e, &listing_args(&fresh, category_id));

    cvlr_satisfy!(crate::storage::get_spoke_asset(&e, category_id, &hub0(&fresh)).is_some());
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
// `validate_bulk_position_limits` (`risk/validation.rs`) de-duplicates
// repeated assets *within one call* (`seen` map) before comparing the new
// unique-position count against the configured limits. These rules pin that
// the duplicated-leg bulk flow at the exact boundary succeeds (supply) or
// reverts only when a *second distinct* leg would exceed the cap (supply and
// borrow), and that a fresh multi-leg supply persists both records.
// ---------------------------------------------------------------------------

/// A duplicated leg is counted once against the position cap: a book one slot
/// below the cap plus two legs naming the same new asset lands exactly at the
/// cap, and the new record is persisted.
#[rule]
#[allow(clippy::too_many_arguments)]
fn bulk_supply_duplicate_asset_counted_once(
    e: Env,
    caller: Address,
    account_id: u64,
    asset_a: Address,
    s1: Address,
    s2: Address,
    s3: Address,
    s4: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    fixture::seed_protocol(&e);
    fixture::seed_account(&e, account_id, &caller);
    fixture::seed_market(&e, &asset_a);
    fixture::seed_empty_books(&e, account_id);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let assets: [Address; fixture::POSITION_LIMIT - 1] = [s1, s2, s3, s4];
    fixture::assume_pairwise_distinct(&assets, &asset_a);
    let seeded = fixture::seed_supply_positions(&e, account_id, &assets);
    cvlr_assume!(seeded + 1 == common::constants::POSITION_LIMIT_MAX);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((fixture::hub_asset(&asset_a), amount));
    legs.push_back((fixture::hub_asset(&asset_a), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &legs);

    let book = crate::storage::get_supply_positions(&e, account_id);
    cvlr_assert!(book.get(fixture::hub_asset(&asset_a)).is_some());
    cvlr_assert!(book.len() == common::constants::POSITION_LIMIT_MAX);
}

/// Two *distinct* new assets on a book one slot below the cap open two slots
/// and breach it.
#[rule]
#[allow(clippy::too_many_arguments)]
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
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(asset_a != asset_b);
    fixture::seed_protocol(&e);
    fixture::seed_account(&e, account_id, &caller);
    fixture::seed_market(&e, &asset_a);
    fixture::seed_market(&e, &asset_b);
    fixture::seed_empty_books(&e, account_id);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let assets: [Address; fixture::POSITION_LIMIT - 1] = [s1, s2, s3, s4];
    fixture::assume_pairwise_distinct(&assets, &asset_a);
    fixture::assume_pairwise_distinct(&assets, &asset_b);
    let seeded = fixture::seed_supply_positions(&e, account_id, &assets);
    cvlr_assume!(seeded + 1 == common::constants::POSITION_LIMIT_MAX);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((fixture::hub_asset(&asset_a), amount));
    legs.push_back((fixture::hub_asset(&asset_b), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &legs);

    // Two distinct new assets bring the account one past POSITION_LIMIT_MAX.
    cvlr_assert!(false);
}

/// Satisfy twin of `bulk_supply_distinct_legs_exceed_limit_reverts`: the same
/// book with both legs naming the *same* new asset opens one slot, not two, so
/// the call reaches the cap instead of breaching it.
///
/// This is the witness that the revert rule is about the slot past the cap and
/// not about a bulk supply that cannot complete at all.
#[rule]
#[allow(clippy::too_many_arguments)]
fn bulk_supply_distinct_legs_exceed_limit_reverts_fixture_completes(
    e: Env,
    caller: Address,
    account_id: u64,
    asset_a: Address,
    s1: Address,
    s2: Address,
    s3: Address,
    s4: Address,
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    fixture::seed_protocol(&e);
    fixture::seed_account(&e, account_id, &caller);
    fixture::seed_market(&e, &asset_a);
    fixture::seed_empty_books(&e, account_id);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let assets: [Address; fixture::POSITION_LIMIT - 1] = [s1, s2, s3, s4];
    fixture::assume_pairwise_distinct(&assets, &asset_a);
    let seeded = fixture::seed_supply_positions(&e, account_id, &assets);
    cvlr_assume!(seeded + 1 == common::constants::POSITION_LIMIT_MAX);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((fixture::hub_asset(&asset_a), amount));
    legs.push_back((fixture::hub_asset(&asset_a), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &legs);

    cvlr_satisfy!(
        crate::storage::get_supply_positions(&e, account_id).len()
            == common::constants::POSITION_LIMIT_MAX
    );
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
    fixture::seed_protocol(&e);
    fixture::seed_account(&e, account_id, &caller);
    fixture::seed_market(&e, &asset_a);
    fixture::seed_market(&e, &asset_b);
    // `book.len() == 2` counts the whole book, so the rule has to start from a
    // book whose size it knows. Excludes accounts that already hold a
    // position; `bulk_supply_duplicate_asset_counted_once` covers the book at
    // the cap.
    fixture::seed_empty_books(&e, account_id);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((fixture::hub_asset(&asset_a), amount));
    legs.push_back((fixture::hub_asset(&asset_b), amount));
    crate::positions::supply::process_supply(&e, &caller, account_id, attrs.spoke_id, &legs);

    let book = crate::storage::get_supply_positions(&e, account_id);
    cvlr_assert!(book.len() == 2);
    cvlr_assert!(book.get(fixture::hub_asset(&asset_a)).is_some());
    cvlr_assert!(book.get(fixture::hub_asset(&asset_b)).is_some());
}

/// The borrow side of `bulk_supply_duplicate_asset_counted_once`: a duplicated
/// leg counts once, so a debt book one slot below the cap still admits it.
#[rule]
#[allow(clippy::too_many_arguments)]
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
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(asset_a != collateral);
    fixture::seed_protocol(&e);
    fixture::seed_account(&e, account_id, &caller);
    fixture::seed_market(&e, &asset_a);
    fixture::seed_market(&e, &collateral);
    fixture::seed_empty_books(&e, account_id);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let assets: [Address; fixture::POSITION_LIMIT - 1] = [s1, s2, s3, s4];
    fixture::assume_pairwise_distinct(&assets, &asset_a);
    fixture::assume_pairwise_distinct(&assets, &collateral);

    // Positive collateral book so a borrow path can pass the health gate.
    fixture::seed_supply_position(&e, account_id, &collateral, common::constants::RAY);
    let seeded = fixture::seed_debt_positions(&e, account_id, &assets);
    cvlr_assume!(seeded + 1 == common::constants::POSITION_LIMIT_MAX);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((fixture::hub_asset(&asset_a), amount));
    legs.push_back((fixture::hub_asset(&asset_a), amount));
    crate::positions::process_borrow(&e, &caller, account_id, &legs, None);

    cvlr_satisfy!(crate::storage::get_debt_positions(&e, account_id)
        .get(fixture::hub_asset(&asset_a))
        .is_some());
}

/// The borrow side of `bulk_supply_distinct_legs_exceed_limit_reverts`.
#[rule]
#[allow(clippy::too_many_arguments)]
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
    amount: i128,
) {
    cvlr_assume!(amount > 0 && amount <= crate::constants::WAD * 1000);
    cvlr_assume!(asset_a != asset_b);
    fixture::seed_protocol(&e);
    fixture::seed_account(&e, account_id, &caller);
    fixture::seed_market(&e, &asset_a);
    fixture::seed_market(&e, &asset_b);
    fixture::seed_empty_books(&e, account_id);

    let attrs = crate::storage::get_account_attrs(&e, account_id);
    cvlr_assume!(attrs.spoke_id > 0);

    let assets: [Address; fixture::POSITION_LIMIT - 1] = [s1, s2, s3, s4];
    fixture::assume_pairwise_distinct(&assets, &asset_a);
    fixture::assume_pairwise_distinct(&assets, &asset_b);
    let seeded = fixture::seed_debt_positions(&e, account_id, &assets);
    cvlr_assume!(seeded + 1 == common::constants::POSITION_LIMIT_MAX);

    let mut legs: Vec<(HubAssetKey, i128)> = Vec::new(&e);
    legs.push_back((fixture::hub_asset(&asset_a), amount));
    legs.push_back((fixture::hub_asset(&asset_b), amount));
    // The borrow position-limit gate runs inside validate_position_entry_gates
    // before any health computation: two distinct new slots on a book at
    // POSITION_LIMIT_MAX - 1 -> PositionLimitExceeded.
    crate::positions::process_borrow(&e, &caller, account_id, &legs, None);

    cvlr_assert!(false);
}

// ---------------------------------------------------------------------------
// Spoke usage reconciliation (`usage_` prefix).
//
// `SpokeUsage(spoke_id, hub_asset)` is the only per-spoke record of cap
// consumption; the pool keeps no per-spoke book. `spoke_usage.rs` maintains it
// apart from the position maps it shadows. A verb that moves a position but
// skips `apply_leg_usage` breaks cap enforcement (an under-count lets a spoke
// exceed its cap, an over-count blocks valid supply), and no other check sees it.
//
// The property proved below, for one `(spoke_id, hub_asset)` cell:
//
//     usage_after - usage_before == scaled_after - scaled_before
//     usage_after >= 0
//
// on both sides at once: the side a verb does not touch stays put, and the side
// it touches moves by exactly the leg delta.
//
// The scaled term is a sum over affected accounts (`stored_scaled_totals` and
// `account_scaled_totals` take a slice). `SeizeMode::Credit` moves collateral
// between two accounts on one spoke in one call, so only the sum over both
// tracks usage, and the credit-mode rule passes both in one slice. That sum is
// not constant: it falls by the protocol fee (shape 3 of the liquidation
// section).
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
/// `Context`; only `finalize_position_flow` writes both out. Leg-level rules
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

/// Asserts the usage reconciliation property for one cell over the given scaled totals.
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
/// plus a usage row that covers both.
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
// Strategy legs. Every strategy (multiply, flash-position, swap-debt,
// swap-collateral, repay-with-collateral, migrate, close) moves positions
// through the four controller-custody primitives below plus `process_deposit`,
// which the supply endpoint rule covers. Each primitive is proved here against
// an arbitrary pool outcome. Driving the primitives rather than the full
// strategies keeps the swap router out of the proof.
//
// Usage is buffered in the `Context` until `finalize_position_flow` runs, so
// these rules persist explicitly. Every strategy ends in `strategy_finalize`
// (`strategies/mod.rs`), which calls `finalize_position_flow`; the four
// endpoint rules above prove that call writes the buffered rows.
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
    let mut cache = Context::new(&e);
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
    let mut cache = Context::new(&e);

    let usage_before = usage_row(&e, &asset);
    let scaled_before = account_scaled_totals(&[&account], &hub);

    let position = crate::positions::get_supply_position_or_panic(&e, &account, &hub);
    crate::positions::execute_withdrawal(
        &e,
        &mut account,
        &caller,
        PositionAction::SwColWd,
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
    let mut cache = Context::new(&e);

    let usage_before = usage_row(&e, &asset);
    let scaled_before = account_scaled_totals(&[&account], &hub);

    let position = crate::positions::get_debt_position_or_panic(&e, &account, &hub);
    crate::positions::repay_prefunded_position(
        &e,
        &mut account,
        &caller,
        PositionAction::RpColR,
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

/// The net-settle leg moves the supply and the debt side of the *same* cell in
/// a single call, through two different merge primitives. Both usage sides
/// must track their own leg.
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
    let mut cache = Context::new(&e);

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
// Two mechanisms stop a new verb from skipping `apply_leg_usage` unnoticed:
//
//  1. `usage_coverage_class` matches exhaustively over
//     `events::PositionAction`. Verbs tag their position updates with a
//     variant, so a new verb usually adds one, and an unclassified variant is
//     a compile error under `--features certora-spoke-rules`.
//
//  2. A `UsageCoverage::Wave0(..)` classification names the legs the verb
//     drives, and `Wave0Legs::contains` / `run_wave0_leg` match exhaustively
//     over `Wave0Leg`. A verb that moves positions through a new primitive
//     needs a new `Wave0Leg` variant, which breaks both matches until
//     `usage_coverage_no_unwired_verb` exercises it. Escaping coverage takes a
//     deliberate misclassification.
// ---------------------------------------------------------------------------

/// One of the merge primitives every position mutation that touches a pool
/// leg funnels through.
#[derive(Clone, Copy, PartialEq)]
enum Wave0Leg {
    /// `positions::supply::merge_supply_leg`.
    SupplyEntry,
    /// `positions::supply::merge_withdraw_leg` with `WithdrawKind::Normal`.
    SupplyExit,
    /// `positions::supply::merge_withdraw_leg` with `WithdrawKind::Liquidation`,
    /// the transfer-mode seizure leg. A separate variant because
    /// `leg_may_restamp_risk_params` skips the risk-parameter restamp for this
    /// kind, and the usage identity must hold on that branch too.
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
    /// Classify a new action here only when a cross-account rule covers it.
    CrossAccount,
}

/// Coverage guard: exhaustive over `events::PositionAction`.
///
/// Do not add a `_ =>` arm. The missing-arm compile error stops a new verb
/// from escaping spoke usage coverage.
///
/// Each classification follows the action's production call sites:
/// `Supply` and `Withdraw` in `positions/supply.rs`, `Borrow` and `Repay` in
/// `positions/debt.rs`, `LiqRepay`/`LiqSeize`/`LiqCredit` in
/// `liquidation/apply.rs`, `Multiply` and `FlashPos` (`borrow_into_controller`)
/// in `multiply.rs` and `flash_position.rs`, `ParamUpd` in `risk/params.rs`
/// (risk-parameter restamp only), `SwDebtR` in `swap_debt.rs`, `SwColWd` in
/// `swap_collateral.rs`, `RpColWd`/`RpColR`/`RpColNet` in
/// `repay_debt_with_collateral.rs`, `CloseWd` in `legs.rs`, `Migrate` in
/// `migrate_blend.rs`.
fn usage_coverage_class(action: PositionAction) -> UsageCoverage {
    match action {
        PositionAction::Supply => UsageCoverage::Wave0(Wave0Legs::SUPPLY_ENTRY),
        PositionAction::Borrow => UsageCoverage::Wave0(Wave0Legs::DEBT_ENTRY),
        PositionAction::Withdraw => UsageCoverage::Wave0(Wave0Legs::SUPPLY_EXIT),
        PositionAction::Repay => UsageCoverage::Wave0(Wave0Legs::DEBT_EXIT),
        // `apply_liquidation_repayments` reaches one merge primitive,
        // `merge_debt_leg` with `LegDirection::Exit`, via `apply_repay_batch`.
        PositionAction::LiqRepay => UsageCoverage::Wave0(Wave0Legs::DEBT_EXIT),
        // The liquidated account's debit, gross of the protocol fee. Transfer
        // mode reaches one merge primitive, `merge_withdraw_leg` with
        // `WithdrawKind::Liquidation`, via `apply_withdraw_batch`. The
        // credit-mode debit reaches none;
        // `usage_liq_credit_seize_sums_over_two_accounts` covers it.
        PositionAction::LiqSeize => UsageCoverage::Wave0(Wave0Legs::SUPPLY_EXIT_LIQUIDATION),
        // The credit-mode receiver's leg. It reaches no merge primitive:
        // `apply_liquidation_share_credit` moves scaled amounts between two
        // accounts and books the protocol fee with a bare `apply_spoke_exit`,
        // so the delta balances only when summed over both accounts.
        PositionAction::LiqCredit => UsageCoverage::CrossAccount,
        PositionAction::Multiply => UsageCoverage::Wave0(Wave0Legs::DEBT_ENTRY),
        PositionAction::FlashPos => UsageCoverage::Wave0(Wave0Legs::DEBT_ENTRY),
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
        16 => PositionAction::FlashPos,
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

/// A pool leg outcome with indexes in the shared pool summary's domain, asset
/// decimals in `0..=27` (a superset of `MIN_ASSET_DECIMALS..=MAX_ASSET_DECIMALS`),
/// and a scaled amount free in both directions, bounded only by `USAGE_SEED_MAX`.
///
/// The direction stays free so the usage identity follows from the leg wiring
/// alone, never from the pool's monotonicity.
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
    cache: &mut Context,
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
        // Restrict to the legs this action can drive in production.
        UsageCoverage::Wave0(legs) => cvlr_assume!(legs.contains(leg)),
        // Proved by `usage_param_refresh_moves_neither`.
        UsageCoverage::NoScaledMove => {
            cvlr_assume!(false);
            return;
        }
        // Proved by `usage_liq_credit_seize_sums_over_two_accounts`, which
        // sums over the liquidated account and the receiver. A single-account
        // rule cannot state this action's invariant.
        UsageCoverage::CrossAccount => {
            cvlr_assume!(false);
            return;
        }
    }

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Context::new(&e);

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
    crate::risk::params::update_account_threshold(&e, caller, has_risks, account_ids);

    let usage_after = usage_row(&e, &asset);
    let scaled_after = stored_scaled_totals(&e, &accounts, &asset);
    assert_usage_tracks_scaled(&usage_before, &usage_after, scaled_before, scaled_after);
    cvlr_assert!(usage_after.supplied_scaled_ray == usage_before.supplied_scaled_ray);
    cvlr_assert!(usage_after.borrowed_scaled_ray == usage_before.borrowed_scaled_ray);
}

/// Pins the one asymmetry in `SpokeUsageContext`: `apply_entry` treats an
/// absent row as zero and writes it, but `apply_exit` on an absent row writes
/// nothing. On a cell with no usage row, an exit leaves usage at the zero row
/// instead of tracking the position down. The delta identity above holds
/// *given a row*, which is the precondition every exit rule seeds.
///
/// Production reaches an exit only after an entry created the row; this rule
/// pins that carve-out.
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
// Reachability witnesses. Each requires its endpoint to complete and to move
// usage in the expected direction; a bare `satisfy(true)` would pass on an
// unwired verb.
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
    let mut cache = Context::new(&e);

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
    let mut cache = Context::new(&e);

    let before = usage_row(&e, &asset);
    let position = crate::positions::get_supply_position_or_panic(&e, &account, &hub);
    crate::positions::execute_withdrawal(
        &e,
        &mut account,
        &caller,
        PositionAction::SwColWd,
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
    let mut cache = Context::new(&e);

    let before = usage_row(&e, &asset);
    let position = crate::positions::get_debt_position_or_panic(&e, &account, &hub);
    crate::positions::repay_prefunded_position(
        &e,
        &mut account,
        &caller,
        PositionAction::RpColR,
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
    let mut cache = Context::new(&e);

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
    let mut cache = Context::new(&e);
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
// Spoke usage reconciliation, liquidation legs (`usage_liq_` prefix).
//
// Liquidation is the only verb family that moves positions on more than one
// account, and the only one that moves value out of the account system
// without a pool withdrawal. Four shapes:
//
//  1. Repayment. `apply_liquidation_repayments` pulls the liquidator's tokens
//     and passes the legs to `apply_repay_batch`. Single-account exit: borrow
//     usage falls by the repaid scaled debt.
//
//  2. `SeizeMode::Transfer`. `apply_liquidation_seizures` passes the legs to
//     `apply_withdraw_batch`. The pool burns the whole seizure, protocol fee
//     included, and withholds the fee from the payout. Supply usage falls by
//     the full seized scaled amount.
//
//  3. `SeizeMode::Credit`. `apply_liquidation_share_credit` debits the
//     liquidated account by the whole scaled seizure `S`, credits the receiver
//     `S - fee`, and books `fee` as a bare `apply_spoke_exit`. Per account
//     nothing reconciles; summed over the pair, supply usage falls by exactly
//     `fee`, because `absorb_supply_as_revenue` reclassifies those shares into
//     pool revenue. Without that exit, usage ratchets up on every
//     credit-mode liquidation until `remove_asset_from_spoke`, which requires
//     a zero usage row, becomes unreachable for the asset.
//
//  4. Bad-debt cleanup. `execute_bad_debt_cleanup` absorbs every remaining
//     position into revenue or socialized debt and removes the account entry,
//     so usage must shed each wiped position in full.
//
// Shapes 1 and 2 are driven through `apply_repay_batch` and
// `apply_withdraw_batch`, where every position and usage mutation of those
// legs happens; the wrappers add only flag gates, the repayment token pull and
// USD arithmetic. Shapes 3 and 4 are driven end to end through
// `process_liquidation` and `clean_bad_debt_standalone`.
// ---------------------------------------------------------------------------

/// The scaled supply shares that left a pair of accounts entirely: what the
/// liquidated account lost, minus what the receiver gained.
///
/// In credit mode this is exactly the protocol fee `split_seized_shares`
/// carves out of the seizure, because the debit is `S` and the credit is
/// `S - fee`. It is the only value that leaves the account system on the
/// seizure leg, and therefore the only spoke-usage movement the leg may book.
///
/// Returns `None` when any total is unrepresentable; the caller decides how to
/// treat it.
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
/// receiver — the observable signature of a share credit that ran.
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

/// Seeds an account holding collateral and debt plus a credit-mode receiver
/// bound to the same spoke (`fixture::SPOKE_ID`) and owned by the liquidator,
/// so `resolve_seize_receiver` admits it.
///
/// `receiver_holds` selects between the two receiver shapes
/// `credit_supply_shares` distinguishes: merging into an existing position, or
/// stamping a fresh one from the current listing.
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
/// whole of `apply_liquidation_repayments`' position and usage effect.
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
    let mut cache = Context::new(&e);

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

/// The transfer-mode seizure leg decreases supply usage by the full seized
/// scaled amount, protocol fee included, and leaves borrow usage alone.
///
/// Drives `apply_withdraw_batch` with `WithdrawKind::Liquidation` and
/// `PositionAction::LiqSeize`, which is the whole of
/// `apply_liquidation_seizures`' position and usage effect.
///
/// The fee is not a second usage movement in this mode: the pool withholds
/// `protocol_fee` from the payout and burns the whole seizure. A separate fee
/// exit here would count the fee twice.
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
    // `LiquidationPlan::validate` admits exactly this range for a seizure
    // entry's fee.
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
    let mut cache = Context::new(&e);

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
/// spoke, and supply usage falls by exactly the shares that left the pair:
/// the protocol fee, not the whole seizure.
///
/// Stated over the slice `[liquidated, receiver]`, because per account
/// neither delta reconciles with usage: the liquidated account loses `S`
/// while usage moves by `fee`, and the receiver gains `S - fee` while usage
/// does not move for it at all. The sum is what the accumulator tracks.
///
/// The rule observes `fee` as `lost - gained` rather than recomputing the plan.
/// This is the fee by construction of `split_seized_shares`; the rule pins
/// usage to it. Repeated price and index reads reuse the rule's snapshots.
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

    // The reconciliation identity in its slice form: usage tracks the pair, both sides.
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
/// removes the account entry, so no other check sees a missing
/// `apply_spoke_exit` there: the usage those positions consumed stays stranded.
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
// Reachability witnesses, one per liquidation leg. Each requires usage to move
// in its leg's direction; a `satisfy(true)` witness would also pass on a leg
// never wired to `apply_leg_usage`.
// ---------------------------------------------------------------------------

#[rule]
fn usage_liq_repay_leg_reachable(e: Env, liquidator: Address, asset: Address) {
    let account_id = crate::spec::fixture::ACCOUNT_ID;
    let seed = common::constants::RAY;
    seed_usage_scenario(&e, account_id, &liquidator, &asset, seed, seed, seed, seed);

    let hub = hub0(&asset);
    let mut account = crate::storage::get_account(&e, account_id);
    let mut cache = Context::new(&e);

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
    let mut cache = Context::new(&e);

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

/// Credit mode completes and moves shares from the liquidated account to the
/// receiver while supply usage does not rise.
///
/// Kept separate from the fee witness below so that "the share transfer
/// happens" and "the fee exit fires" fail independently.
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
/// Witness for the fee `apply_spoke_exit` in `apply_liquidation_share_credit`:
/// a positive-fee path exists, so the credit-mode assert rule is not proved
/// on zero-fee draws alone.
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

/// `Credit(0)` liquidation opens its receiving account even when the spoke is
/// deprecated.
///
/// `remove_spoke` performs no usage check and deprecation is one-way, so a
/// deprecated spoke can hold live positions forever. If `Credit(0)` refused a
/// deprecated spoke, a liquidator with no account there would have only
/// `Transfer` mode, which depends on pool cash. The witness is the returned
/// receiver id: reaching it means the call completed, and a non-zero id means
/// an account was minted.
#[rule]
fn credit_zero_liquidation_creates_receiver_in_deprecated_spoke(
    e: Env,
    liquidator: Address,
    owner: Address,
    collateral_asset: Address,
    debt_asset: Address,
) {
    let account_id = fixture::ACCOUNT_ID;
    let seed = 10 * common::constants::RAY;

    cvlr_assume!(owner != liquidator);
    seed_credit_liquidation(
        &e,
        account_id,
        account_id + 1,
        &owner,
        &liquidator,
        &collateral_asset,
        &debt_asset,
        seed,
        seed,
        false,
        0,
    );
    fixture::seed_spoke_usage(&e, &collateral_asset, 4 * seed, 4 * seed);
    fixture::seed_spoke_usage(&e, &debt_asset, 4 * seed, 4 * seed);
    // The minted id is otherwise drawn from a havoced counter and could
    // collide with the liquidated account; pin it and empty its books.
    fixture::seed_empty_books(&e, fixture::seed_next_account_id(&e, 100));

    // Deprecate last: `seed_market` re-seeds the spoke as active.
    let mut deprecated = crate::storage::get_spoke(&e, fixture::SPOKE_ID);
    deprecated.is_deprecated = true;
    crate::storage::set_spoke(&e, fixture::SPOKE_ID, &deprecated);

    let payments = one_payment(&e, &debt_asset, crate::constants::WAD);
    let receiver = crate::positions::liquidation::process_liquidation(
        &e,
        &liquidator,
        account_id,
        &payments,
        SeizeMode::Credit(0),
    );

    cvlr_satisfy!(receiver != 0);
}
