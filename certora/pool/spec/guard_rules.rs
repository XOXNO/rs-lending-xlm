use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use stellar_access::ownable;

use common::constants::{MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, RAY, SUPPLY_INDEX_FLOOR_RAW};
use common::math::fp::Ray;
use common::types::{PoolBorrowEntry, PoolNetSettleEntry, PoolWithdrawEntry};

use super::fixture::{
    action, hub, params, params_with_max_util, position, read_state, seed, state, MAX_FLOW_AMOUNT,
    ONE_TOKEN,
};

fn utilization_after(e: &Env, supplied: i128, borrowed: i128, s_idx: i128, b_idx: i128) -> Ray {
    if supplied == 0 {
        return Ray::ZERO;
    }
    let debt_value = Ray::from(borrowed).mul(e, Ray::from(b_idx));
    let supply_value = Ray::from(supplied).mul(e, Ray::from(s_idx));
    common::rates::utilization(e, debt_value, supply_value)
}

#[rule]
fn borrow_respects_utilization_cap(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    supplied: i128,
    borrowed: i128,
    position_before: i128,
) {
    // `fixture::state` stamps `last_timestamp = e.ledger().timestamp() * 1_000`
    // and `Cache::load` recomputes the same product through `time::now_ms`.
    // Both are checked multiplications, so a ledger clock past `u64::MAX /
    // 1_000` panics and Sunbeam prunes the path as `assume(false)`. Stating the
    // bound makes that pruning visible instead of hidden, and drops the
    // overflow branch from every rule below.
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    let max_util = RAY * 9 / 10;
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supplied > 0 && supplied <= 100 * RAY);
    cvlr_assume!(borrowed >= 0 && borrowed <= supplied);
    cvlr_assume!(position_before >= 0 && position_before <= borrowed);

    seed(
        &e,
        admin,
        asset.clone(),
        params_with_max_util(asset.clone(), max_util),
        state(
            supplied,
            borrowed,
            0,
            RAY,
            RAY,
            1_000 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let entry = PoolBorrowEntry {
        action: action(asset.clone(), position_before, amount),
    };
    crate::ops::borrow::accounting(&e, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(
        utilization_after(
            &e,
            post.supplied,
            post.borrowed,
            post.supply_index,
            post.borrow_index
        )
        .raw()
            <= max_util
    );
}

#[rule]
fn user_withdraw_respects_utilization_cap(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    supplied: i128,
    borrowed: i128,
    position_before: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    let max_util = RAY * 9 / 10;
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supplied > 0 && supplied <= 100 * RAY);
    cvlr_assume!(borrowed >= 0 && borrowed <= supplied);
    cvlr_assume!(position_before > 0 && position_before <= supplied);

    seed(
        &e,
        admin,
        asset.clone(),
        params_with_max_util(asset.clone(), max_util),
        state(
            supplied,
            borrowed,
            0,
            RAY,
            RAY,
            1_000 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, amount),
        protocol_fee: 0,
    };
    crate::ops::withdraw::accounting(&e, false, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(
        utilization_after(
            &e,
            post.supplied,
            post.borrowed,
            post.supply_index,
            post.borrow_index
        )
        .raw()
            <= max_util
    );
}

#[rule]
fn withdraw_never_overdraws_cash(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    cash_before: i128,
    supply_index: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before > 0 && position_before <= 20 * RAY);
    cvlr_assume!(cash_before >= 0 && cash_before <= 1_000 * ONE_TOKEN);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            0,
            0,
            RAY,
            supply_index,
            cash_before,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, amount),
        protocol_fee: 0,
    };
    let outcome = crate::ops::withdraw::accounting(&e, false, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(outcome.net_transfer <= pre.cash);
    cvlr_assert!(pre.cash - post.cash == outcome.net_transfer);
    cvlr_assert!(post.cash >= 0);
}

#[rule]
fn withdraw_keeps_revenue_backed(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    revenue_before: i128,
    protocol_fee: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before > 0 && position_before <= 20 * RAY);
    cvlr_assume!(revenue_before >= 0 && revenue_before <= 100 * RAY);
    cvlr_assume!(protocol_fee >= 0 && protocol_fee <= MAX_FLOW_AMOUNT);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            0,
            revenue_before,
            RAY,
            RAY,
            1_000 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, amount),
        protocol_fee,
    };
    crate::ops::withdraw::accounting(&e, true, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(post.revenue >= 0);
    cvlr_assert!(post.revenue <= post.supplied);
}

#[rule]
fn net_settle_keeps_revenue_backed(
    e: Env,
    admin: Address,
    asset: Address,
    requested: i128,
    supply_before: i128,
    debt_before: i128,
    revenue_before: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(requested >= 0 && requested <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supply_before > 0 && supply_before <= 20 * RAY);
    cvlr_assume!(debt_before > 0 && debt_before <= 20 * RAY);
    cvlr_assume!(revenue_before >= 0 && revenue_before <= 20 * RAY);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            50 * RAY,
            revenue_before,
            RAY,
            RAY,
            1_000 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let entry = PoolNetSettleEntry {
        hub_asset: hub(asset.clone()),
        amount: requested,
        supply_position: position(supply_before),
        debt_position: position(debt_before),
    };
    crate::ops::net_settle::apply(&e, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(post.revenue >= 0);
    cvlr_assert!(post.revenue <= post.supplied);
}

#[rule]
fn withdraw_leaves_no_orphan_debt(
    e: Env,
    admin: Address,
    asset: Address,
    position_before: i128,
    borrowed: i128,
    borrow_index: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(position_before > 0 && position_before <= 20 * RAY);
    cvlr_assume!(borrowed > 0 && borrowed <= 20 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            position_before,
            borrowed,
            0,
            borrow_index,
            RAY,
            1_000 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, i128::MAX),
        protocol_fee: 0,
    };
    crate::ops::withdraw::accounting(&e, true, &entry);
    let post = read_state(&e, &asset);

    cvlr_assert!(!(post.supplied == 0 && post.borrowed != 0));
}

#[rule]
fn claim_revenue_leaves_no_orphan_debt(
    e: Env,
    admin: Address,
    asset: Address,
    revenue_before: i128,
    borrowed: i128,
    cash_before: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(revenue_before > 0 && revenue_before <= 20 * RAY);
    cvlr_assume!(borrowed > 0 && borrowed <= 20 * RAY);
    cvlr_assume!(cash_before >= 0 && cash_before <= 1_000 * ONE_TOKEN);

    seed(
        &e,
        admin,
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            revenue_before,
            borrowed,
            revenue_before,
            RAY,
            RAY,
            cash_before,
            e.ledger().timestamp(),
        ),
    );

    crate::ops::revenue::accounting(&e, hub(asset.clone()));
    let post = read_state(&e, &asset);

    cvlr_assert!(!(post.supplied == 0 && post.borrowed != 0));
    cvlr_assert!(post.revenue <= post.supplied);
    cvlr_assert!(post.cash >= 0);
}

// --- Token-authority guards (Certora Hub L-02 analogue) -------------------
//
// Aave's Hub called `transferFrom` with a caller-supplied `from`, so anyone who
// had approved the Hub could be drained. Our pool has no such pull: the
// controller moves tokens in and reports the measured receipt, and every pool
// payout is a `transfer_out` from the pool's own balance, sized by the pool's
// own cash book. The single `transfer_from` in the pool (`ops/flash.rs:198`)
// names the flash receiver the pool just funded, for exactly principal + fee —
// pinned by `flash_repayment_terms_recover_principal_and_fee`, not here.
//
// The rules below pin the accounting half of that claim on the three paths not
// already covered elsewhere in the pool suite:
//
//   * repay      — inbound leg with a refund, the only path that returns value
//                  to a payer address without debiting the cash book;
//   * borrow     — payout to a controller-named receiver;
//   * revenue    — the one payout whose recipient the *pool* picks, not the
//                  controller (it is the Ownable owner).
//
// Each also asserts the Ownable owner is unchanged: `#[only_owner]` gates every
// mutator on that stored value, so no operation may widen the authorized set.

/// Repay never pays out pool funds: the refund handed back to the payer is
/// bounded by the amount that payer just sent in, and the cash book only ever
/// rises on this path (by exactly the net repay).
#[rule]
#[allow(clippy::too_many_arguments)]
fn pool_trust_repay_refunds_only_payer_surplus(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    borrowed: i128,
    borrow_index: i128,
    cash_before: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(amount >= 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= 20 * RAY);
    cvlr_assume!(borrowed >= position_before && borrowed <= 100 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(cash_before >= 0 && cash_before <= 1_000 * ONE_TOKEN);

    seed(
        &e,
        admin.clone(),
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            borrowed,
            0,
            borrow_index,
            RAY,
            cash_before,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let outcome =
        crate::ops::repay::accounting(&e, &action(asset.clone(), position_before, amount));
    let post = read_state(&e, &asset);

    // `apply` transfers exactly `overpayment` back to the payer; it can never
    // exceed what the payer supplied, so no third party's funds are reachable.
    cvlr_assert!(outcome.overpayment >= 0 && outcome.overpayment <= amount);
    cvlr_assert!(outcome.mutation.actual_amount == amount - outcome.overpayment);
    // The refund is paid out of the payer's own inbound amount, never the book.
    cvlr_assert!(post.cash - pre.cash == outcome.mutation.actual_amount);
    cvlr_assert!(post.cash >= pre.cash);
    cvlr_assert!(ownable::get_owner(&e) == Some(admin));
}

/// A borrow payout to the controller-named receiver is fully backed by the
/// pool's own cash: the amount sent never exceeds pre-call cash, is debited
/// exactly once, and cannot drive the book negative.
#[rule]
#[allow(clippy::too_many_arguments)]
fn pool_trust_borrow_payout_is_backed_by_pool_cash(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    borrowed: i128,
    cash_before: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= 20 * RAY);
    cvlr_assume!(borrowed >= 0 && borrowed <= 50 * RAY);
    cvlr_assume!(cash_before >= 0 && cash_before <= 1_000 * ONE_TOKEN);

    seed(
        &e,
        admin.clone(),
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            borrowed,
            0,
            RAY,
            RAY,
            cash_before,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let entry = PoolBorrowEntry {
        action: action(asset.clone(), position_before, amount),
    };
    let outcome = crate::ops::borrow::accounting(&e, &entry);
    let post = read_state(&e, &asset);

    // `apply` transfers `mutation.actual_amount` out of the pool's own balance.
    cvlr_assert!(outcome.mutation.actual_amount == amount);
    cvlr_assert!(outcome.mutation.actual_amount <= pre.cash);
    cvlr_assert!(pre.cash - post.cash == outcome.mutation.actual_amount);
    cvlr_assert!(post.cash >= 0);
    cvlr_assert!(ownable::get_owner(&e) == Some(admin));
}

/// The revenue claim is the only payout whose recipient the pool chooses. It
/// pays the Ownable owner set at construction — which the claim path itself
/// cannot change — and never more than the pool's own cash.
#[rule]
#[allow(clippy::too_many_arguments)]
fn pool_trust_revenue_claim_pays_owner_from_pool_cash(
    e: Env,
    admin: Address,
    asset: Address,
    revenue_before: i128,
    cash_before: i128,
    supply_index: i128,
) {
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    cvlr_assume!(revenue_before >= 0 && revenue_before <= 20 * RAY);
    cvlr_assume!(cash_before >= 0 && cash_before <= 1_000 * ONE_TOKEN);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);

    seed(
        &e,
        admin.clone(),
        asset.clone(),
        params(asset.clone(), 0, false),
        state(
            100 * RAY,
            20 * RAY,
            revenue_before,
            RAY,
            supply_index,
            cash_before,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let claimed = crate::ops::revenue::accounting(&e, hub(asset.clone()))
        .mutation
        .actual_amount;
    let post = read_state(&e, &asset);

    cvlr_assert!(claimed >= 0 && claimed <= pre.cash);
    cvlr_assert!(pre.cash - post.cash == claimed);
    cvlr_assert!(post.cash >= 0);
    // `apply` sends `claimed` to `ownable::get_owner`, still the constructor owner.
    cvlr_assert!(ownable::get_owner(&e) == Some(admin));
}
