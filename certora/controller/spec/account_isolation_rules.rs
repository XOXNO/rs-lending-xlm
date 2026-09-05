use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use crate::constants::WAD;
use crate::spec::fixture;
use crate::storage::{get_debt_positions, get_supply_positions};
use crate::types::HubAssetKey;
use common::constants::RAY;
use common::types::Payment;

fn hub0(asset: &Address) -> HubAssetKey {
    HubAssetKey {
        hub_id: crate::spec::fixture::HUB_ID,
        asset: asset.clone(),
    }
}

/// The frame rules keep both accounts' books arbitrary — that is their whole
/// point — but an arbitrary book is not a *reachable* book: havoced storage can
/// hold a threshold above `BPS` or a loan-to-value above the threshold, which
/// no listing can produce. `assume_wellformed_book` states the premise the
/// risk-totals summary already encodes implicitly, and nothing more: lengths,
/// keys and scaled amounts stay unbounded.
fn assume_reachable_books(e: &Env, target_account: u64, other_account: u64) {
    fixture::assume_wellformed_book(e, target_account);
    fixture::assume_wellformed_book(e, other_account);
}

/// Both readers go to the position maps directly rather than through
/// `get_account`, which panics with `AccountNotFound` once bad-debt cleanup
/// has removed the account. A panic drops the path from the rule, so the
/// post state of a liquidation that ended in cleanup would silently leave it.
fn scaled_supply_at(env: &Env, account_id: u64, asset: &Address) -> i128 {
    get_supply_positions(env, account_id)
        .get(hub0(asset))
        .map(|p| p.scaled_amount)
        .unwrap_or(0)
}

fn scaled_borrow_at(env: &Env, account_id: u64, asset: &Address) -> i128 {
    get_debt_positions(env, account_id)
        .get(hub0(asset))
        .map(|p| p.scaled_amount)
        .unwrap_or(0)
}

#[rule]
fn supply_does_not_change_other_account_positions(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let target_account: u64 = 1;
    let other_account: u64 = 2;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, target_account, &caller, &asset);
    crate::spec::fixture::seed_account(&e, other_account, &caller);
    assume_reachable_books(&e, target_account, other_account);

    let other_supply_before = scaled_supply_at(&e, other_account, &asset);
    let other_borrow_before = scaled_borrow_at(&e, other_account, &asset);

    crate::spec::compat::supply_single(e.clone(), caller, target_account, asset.clone(), amount);

    cvlr_assert!(scaled_supply_at(&e, other_account, &asset) == other_supply_before);
    cvlr_assert!(scaled_borrow_at(&e, other_account, &asset) == other_borrow_before);
}

#[rule]
fn borrow_does_not_change_other_account_positions(
    e: Env,
    caller: Address,
    asset: Address,
    amount: i128,
) {
    let target_account: u64 = 1;
    let other_account: u64 = 2;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, target_account, &caller, &asset);
    crate::spec::fixture::seed_account(&e, other_account, &caller);
    assume_reachable_books(&e, target_account, other_account);

    let other_supply_before = scaled_supply_at(&e, other_account, &asset);
    let other_borrow_before = scaled_borrow_at(&e, other_account, &asset);

    crate::spec::compat::borrow_single(e.clone(), caller, target_account, asset.clone(), amount);

    cvlr_assert!(scaled_supply_at(&e, other_account, &asset) == other_supply_before);
    cvlr_assert!(scaled_borrow_at(&e, other_account, &asset) == other_borrow_before);
}

#[rule]
fn repay_only_changes_target_account_debt(e: Env, caller: Address, asset: Address, amount: i128) {
    let target_account: u64 = 1;
    let other_account: u64 = 2;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    crate::spec::fixture::seed_live_account(&e, target_account, &caller, &asset);
    crate::spec::fixture::seed_account(&e, other_account, &caller);
    assume_reachable_books(&e, target_account, other_account);

    let other_supply_before = scaled_supply_at(&e, other_account, &asset);
    let other_borrow_before = scaled_borrow_at(&e, other_account, &asset);

    crate::spec::compat::repay_single(e.clone(), caller, target_account, asset.clone(), amount);

    cvlr_assert!(scaled_supply_at(&e, other_account, &asset) == other_supply_before);
    cvlr_assert!(scaled_borrow_at(&e, other_account, &asset) == other_borrow_before);
}

/// A liquidation writes at most the two accounts it names.
///
/// Share-credit liquidation deliberately writes a second account: the receiver the liquidator
/// declares through `SeizeMode::Credit`. The rule therefore names both principals — the
/// liquidated account and the declared receiver — and still forbids any *third* account from
/// moving. The declared receiver is the strongest form available: it is chosen in the call
/// itself, so the rule holds a liquidation to exactly the accounts its arguments identify, and
/// a liquidation that touched an undeclared account would still be caught.
#[rule]
fn liquidation_does_not_change_other_account_positions(
    e: Env,
    liquidator: Address,
    owner: Address,
    debt_asset: Address,
    debt_amount: i128,
) {
    let target_account: u64 = 1;
    let receiver_account: u64 = 2;
    let other_account: u64 = 3;
    cvlr_assume!(debt_amount > 0 && debt_amount <= WAD * 1000);
    cvlr_assume!(owner != liquidator);
    crate::spec::fixture::seed_live_account(&e, target_account, &owner, &debt_asset);
    // The receiver is the liquidator's own account, which is what `Credit(id)` requires.
    crate::spec::fixture::seed_account(&e, receiver_account, &liquidator);
    crate::spec::fixture::seed_account(&e, other_account, &owner);
    assume_reachable_books(&e, target_account, other_account);
    fixture::assume_wellformed_book(&e, receiver_account);

    let other_supply_before = scaled_supply_at(&e, other_account, &debt_asset);
    let other_borrow_before = scaled_borrow_at(&e, other_account, &debt_asset);

    let mut payments: soroban_sdk::Vec<Payment> = soroban_sdk::Vec::new(&e);
    payments.push_back((debt_asset.clone(), debt_amount));
    crate::spec::compat::liquidate_with_mode(
        e.clone(),
        liquidator,
        target_account,
        payments,
        crate::types::SeizeMode::Credit(receiver_account),
    );

    cvlr_assert!(scaled_supply_at(&e, other_account, &debt_asset) == other_supply_before);
    cvlr_assert!(scaled_borrow_at(&e, other_account, &debt_asset) == other_borrow_before);
}

/// A share credit moves the receiver's book by at most what the liquidated account lost.
///
/// `liquidation_does_not_change_other_account_positions` names the receiver so it can exempt
/// it, which leaves the receiver itself unbounded — the weakening ADR-0019 records. This rule
/// bounds it, per hub asset: the receiver's debt book does not move at all (a credit is
/// supply-side only), its supply shares never fall, and they never rise by more than the
/// liquidated account's supply shares fell in the same asset.
///
/// That bound is exactly `credited <= seized`: `split_seized_shares` returns
/// `(fee, seized - fee)` with `fee >= 0`, `apply_liquidation_share_credit` debits the whole
/// `seized` and credits `seized - fee`, and the legs are keyed by hub asset, so one asset is
/// debited at most once per liquidation.
///
/// The residue `seized - credited` is the protocol fee, and this rule does *not* pin it to the
/// pool's revenue: the fee leaves the controller through `pool_seize_positions_call`, a
/// cross-contract call the prover havocs, so pool revenue is not observable here. The one
/// controller-side trace of it — the spoke usage exit — is not the pool's revenue either, and
/// pinning it would need a seeded usage row plus a no-bad-debt-cleanup assumption. Left out on
/// purpose rather than assumed into existence.
#[rule]
fn liquidation_share_credit_bounded_by_target_loss(
    e: Env,
    liquidator: Address,
    owner: Address,
    debt_asset: Address,
    collateral_asset: Address,
    debt_amount: i128,
) {
    let target_account: u64 = 1;
    let receiver_account: u64 = 2;
    cvlr_assume!(debt_amount > 0 && debt_amount <= WAD * 1000);
    cvlr_assume!(owner != liquidator);
    crate::spec::fixture::seed_live_account(&e, target_account, &owner, &debt_asset);
    // The receiver is the liquidator's own account, which is what `Credit(id)` requires.
    crate::spec::fixture::seed_account(&e, receiver_account, &liquidator);
    assume_reachable_books(&e, target_account, receiver_account);

    let target_supply_before = scaled_supply_at(&e, target_account, &collateral_asset);
    let receiver_supply_before = scaled_supply_at(&e, receiver_account, &collateral_asset);
    let receiver_borrow_before = scaled_borrow_at(&e, receiver_account, &collateral_asset);
    // Keeps the two differences below inside `i128`. Excludes only a single scaled position
    // above 2^125 shares, which no listing can reach: every scaled write is bounded by the
    // pool's own RAY arithmetic, which overflows far below that.
    cvlr_assume!(target_supply_before <= i128::MAX / 4);
    cvlr_assume!(receiver_supply_before <= i128::MAX / 4);

    let mut payments: soroban_sdk::Vec<Payment> = soroban_sdk::Vec::new(&e);
    payments.push_back((debt_asset.clone(), debt_amount));
    crate::spec::compat::liquidate_with_mode(
        e.clone(),
        liquidator,
        target_account,
        payments,
        crate::types::SeizeMode::Credit(receiver_account),
    );

    let target_supply_after = scaled_supply_at(&e, target_account, &collateral_asset);
    let receiver_supply_after = scaled_supply_at(&e, receiver_account, &collateral_asset);

    cvlr_assert!(
        scaled_borrow_at(&e, receiver_account, &collateral_asset) == receiver_borrow_before
    );
    cvlr_assert!(receiver_supply_after >= receiver_supply_before);
    cvlr_assert!(
        receiver_supply_after - receiver_supply_before
            <= target_supply_before - target_supply_after
    );
}

#[rule]
fn account_isolation_reachability(e: Env, caller: Address, asset: Address) {
    let amount = WAD;
    crate::spec::fixture::seed_live_account(&e, 1, &caller, &asset);
    crate::spec::compat::supply_single(e, caller, 1, asset, amount);
    cvlr_satisfy!(true);
}

/// Witness that a share-credit liquidation actually credits the receiver.
///
/// `account_isolation_reachability` only drives `supply`, so it says nothing about the credit
/// path. This is the non-vacuity alarm for `liquidation_share_credit_bounded_by_target_loss`:
/// that rule's asserts are trivially true on every path where no leg touches
/// `collateral_asset`, so a domain that closed around the credit itself would leave it green.
/// If this witness stops being satisfiable, the credit path stopped being reachable.
#[rule]
fn liquidation_share_credit_reachability(
    e: Env,
    liquidator: Address,
    owner: Address,
    debt_asset: Address,
    collateral_asset: Address,
) {
    let target_account: u64 = 1;
    let receiver_account: u64 = 2;
    cvlr_assume!(owner != liquidator);
    crate::spec::fixture::seed_live_account(&e, target_account, &owner, &debt_asset);
    crate::spec::fixture::seed_market(&e, &collateral_asset);
    crate::spec::fixture::seed_account(&e, receiver_account, &liquidator);
    crate::spec::fixture::seed_debt_position(&e, target_account, &debt_asset, 10 * RAY);
    crate::spec::fixture::seed_supply_position(&e, target_account, &collateral_asset, 10 * RAY);

    let receiver_supply_before = scaled_supply_at(&e, receiver_account, &collateral_asset);

    let mut payments: soroban_sdk::Vec<Payment> = soroban_sdk::Vec::new(&e);
    payments.push_back((debt_asset.clone(), WAD));
    crate::spec::compat::liquidate_with_mode(
        e.clone(),
        liquidator,
        target_account,
        payments,
        crate::types::SeizeMode::Credit(receiver_account),
    );

    cvlr_satisfy!(
        scaled_supply_at(&e, receiver_account, &collateral_asset) > receiver_supply_before
    );
}
