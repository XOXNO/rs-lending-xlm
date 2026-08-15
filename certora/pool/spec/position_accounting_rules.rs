use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume};
use soroban_sdk::{Address, Env};

use common::constants::{
    MAX_BORROW_INDEX_RAY, MAX_SUPPLY_INDEX_RAY, RAY, RAY_DECIMALS, SUPPLY_INDEX_FLOOR_RAW,
};
use common::math::fp::Ray;
use common::math::fp_core;
use common::types::{PoolBorrowEntry, PoolNetSettleEntry, PoolSupplyEntry, PoolWithdrawEntry};

use super::fixture::{
    action, hub, params_with_decimals, position, read_state, seed, state, write_market,
    MAX_FLOW_AMOUNT, ONE_TOKEN,
};

#[rule]
fn supply_scaled_balance_matches_index(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    supply_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= 10 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    let amount_ray = Ray::from_asset(amount, asset_decimals);
    let expected = fp_core::mul_div_floor(&e, amount_ray.raw(), RAY, supply_index);
    cvlr_assume!(expected > 0);
    seed(
        &e,
        admin,
        asset.clone(),
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
        state(
            100 * RAY,
            100 * RAY,
            RAY,
            supply_index.max(RAY),
            supply_index,
            200 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let entry = PoolSupplyEntry {
        action: action(asset.clone(), position_before, amount),
    };
    let (result, _) = crate::ops::supply::apply(&e, &entry);
    let post = read_state(&e, &asset);
    let pre_claim = Ray::from(pre.supplied)
        .mul_floor(&e, Ray::from(pre.supply_index))
        .to_asset_floor(asset_decimals);
    let pre_debt = Ray::from(pre.borrowed)
        .mul_ceil(&e, Ray::from(pre.borrow_index))
        .to_asset_ceil(asset_decimals);
    let post_claim = Ray::from(post.supplied)
        .mul_floor(&e, Ray::from(post.supply_index))
        .to_asset_floor(asset_decimals);
    let post_debt = Ray::from(post.borrowed)
        .mul_ceil(&e, Ray::from(post.borrow_index))
        .to_asset_ceil(asset_decimals);
    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount - position_before == expected);
    cvlr_assert!(Ray::from(expected).mul_floor(&e, Ray::from(supply_index)) <= amount_ray);
    cvlr_assert!(post.supplied - pre.supplied == expected);
    cvlr_assert!(post.cash - pre.cash == amount);
    cvlr_assert!(post.borrowed == pre.borrowed && post.revenue == pre.revenue);
    cvlr_assert!(post.supply_index == pre.supply_index && post.borrow_index == pre.borrow_index);
    cvlr_assert!(pre_claim <= pre.cash.saturating_add(pre_debt));
    cvlr_assert!(post_claim <= post.cash.saturating_add(post_debt));
}

#[rule]
fn borrow_scaled_debt_matches_index(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    debt_before: i128,
    borrow_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(debt_before >= 0 && debt_before <= 10 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    let amount_ray = Ray::from_asset(amount, asset_decimals);
    let expected = fp_core::mul_div_ceil(&e, amount_ray.raw(), RAY, borrow_index);
    cvlr_assume!(expected > 0 && expected <= i128::MAX - debt_before);
    seed(
        &e,
        admin,
        asset.clone(),
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
        state(
            100 * RAY,
            debt_before,
            RAY,
            borrow_index,
            RAY,
            200 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let entry = PoolBorrowEntry {
        action: action(asset.clone(), debt_before, amount),
    };
    let result = crate::ops::borrow::accounting(&e, &entry).mutation;
    let post = read_state(&e, &asset);
    cvlr_assert!(result.actual_amount == amount);
    cvlr_assert!(result.position.scaled_amount - debt_before == expected);
    cvlr_assert!(Ray::from(expected).mul_floor(&e, Ray::from(borrow_index)) >= amount_ray);
    cvlr_assert!(post.borrowed - pre.borrowed == expected);
    cvlr_assert!(pre.cash - post.cash == amount);
    cvlr_assert!(post.supplied == pre.supplied && post.revenue == pre.revenue);
    cvlr_assert!(post.supply_index == pre.supply_index && post.borrow_index == pre.borrow_index);
}

#[rule]
fn partial_withdraw_burns_scaled_supply(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    position_before: i128,
    supply_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before > 0 && position_before <= 20 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    let current_actual = Ray::from(position_before)
        .mul(&e, Ray::from(supply_index))
        .to_asset(asset_decimals);
    cvlr_assume!(amount < current_actual);
    let amount_ray = Ray::from_asset(amount, asset_decimals);
    let expected_burn = fp_core::mul_div_ceil(&e, amount_ray.raw(), RAY, supply_index);
    cvlr_assume!(expected_burn > 0);
    seed(
        &e,
        admin,
        asset.clone(),
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
        state(
            100 * RAY,
            0,
            RAY,
            RAY,
            supply_index,
            i128::MAX,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, amount),
        protocol_fee: 0,
    };
    let outcome = crate::ops::withdraw::accounting(&e, false, &entry);
    let (result, net) = (outcome.mutation, outcome.net_transfer);
    let post = read_state(&e, &asset);
    cvlr_assert!(result.actual_amount == amount && net == amount);
    cvlr_assert!(position_before - result.position.scaled_amount == expected_burn);
    cvlr_assert!(Ray::from(expected_burn).mul_floor(&e, Ray::from(supply_index)) >= amount_ray);
    cvlr_assert!(pre.supplied - post.supplied == expected_burn);
    cvlr_assert!(pre.cash - post.cash == amount);
    cvlr_assert!(post.borrowed == pre.borrowed && post.revenue == pre.revenue);
}

#[rule]
fn full_withdraw_burns_entire_position(
    e: Env,
    admin: Address,
    asset: Address,
    position_before: i128,
    supply_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(position_before > 0 && position_before <= 20 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    seed(
        &e,
        admin,
        asset.clone(),
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
        state(
            100 * RAY,
            0,
            RAY,
            RAY,
            supply_index,
            i128::MAX,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let entry = PoolWithdrawEntry {
        action: action(asset.clone(), position_before, i128::MAX),
        protocol_fee: 0,
    };
    let outcome = crate::ops::withdraw::accounting(&e, false, &entry);
    let (result, net) = (outcome.mutation, outcome.net_transfer);
    let post = read_state(&e, &asset);
    let expected_gross = Ray::from(position_before)
        .mul_floor(&e, Ray::from(supply_index))
        .to_asset_floor(asset_decimals);

    cvlr_assert!(result.position.scaled_amount == 0);
    cvlr_assert!(pre.supplied - post.supplied == position_before);
    cvlr_assert!(result.actual_amount == expected_gross && net == expected_gross);
    cvlr_assert!(pre.cash - post.cash == expected_gross);
}

#[rule]
fn partial_repay_burns_scaled_debt(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    debt_before: i128,
    borrow_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_FLOW_AMOUNT);
    cvlr_assume!(debt_before > 0 && debt_before <= 20 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    let debt_ceil = Ray::from(debt_before)
        .mul_ceil(&e, Ray::from(borrow_index))
        .to_asset_ceil(asset_decimals);
    cvlr_assume!(amount < debt_ceil);
    let amount_ray = Ray::from_asset(amount, asset_decimals);
    let expected_burn = fp_core::mul_div_floor(&e, amount_ray.raw(), RAY, borrow_index);
    cvlr_assume!(expected_burn > 0);
    seed(
        &e,
        admin,
        asset.clone(),
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
        state(
            100 * RAY,
            debt_before,
            RAY,
            borrow_index,
            RAY,
            100 * ONE_TOKEN,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let act = action(asset.clone(), debt_before, amount);
    let outcome = crate::ops::repay::accounting(&e, &act);
    let (result, overpayment) = (outcome.mutation, outcome.overpayment);
    let post = read_state(&e, &asset);
    cvlr_assert!(overpayment == 0 && result.actual_amount == amount);
    cvlr_assert!(debt_before - result.position.scaled_amount == expected_burn);
    cvlr_assert!(Ray::from(expected_burn).mul_ceil(&e, Ray::from(borrow_index)) <= amount_ray);
    cvlr_assert!(pre.borrowed - post.borrowed == expected_burn);
    cvlr_assert!(post.cash - pre.cash == amount);
    cvlr_assert!(post.supplied == pre.supplied && post.revenue == pre.revenue);
}

#[rule]
fn full_repay_refunds_overpayment(
    e: Env,
    admin: Address,
    asset: Address,
    debt_before: i128,
    borrow_index: i128,
    extra: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(debt_before > 0 && debt_before <= 20 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(extra >= 0 && extra <= MAX_FLOW_AMOUNT);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    let debt_ceil = Ray::from(debt_before)
        .mul_ceil(&e, Ray::from(borrow_index))
        .to_asset_ceil(asset_decimals);
    let amount = debt_ceil + extra;
    seed(
        &e,
        admin,
        asset.clone(),
        params_with_decimals(asset.clone(), 0, false, asset_decimals),
        state(
            100 * RAY,
            debt_before,
            RAY,
            borrow_index,
            RAY,
            0,
            e.ledger().timestamp(),
        ),
    );

    let pre = read_state(&e, &asset);
    let act = action(asset.clone(), debt_before, amount);
    let outcome = crate::ops::repay::accounting(&e, &act);
    let (result, overpayment) = (outcome.mutation, outcome.overpayment);
    let post = read_state(&e, &asset);

    cvlr_assert!(result.position.scaled_amount == 0);
    cvlr_assert!(pre.borrowed - post.borrowed == debt_before);
    cvlr_assert!(result.actual_amount == debt_ceil);
    cvlr_assert!(post.cash - pre.cash == debt_ceil);
    cvlr_assert!(overpayment == extra);
}

// ---------------------------------------------------------------------------
// Anti-splitting / additivity — Aave Hub `*Additivity` analogue, docs
// `explanation/aave-v4-audit-comparison.md` §5 V-6.
//
// Shape of every rule below: run one operation as two sequential calls, then
// replay the *byte-identical* starting market and run one call for the summed
// amount. Assert the direction that leaves a caller no better off for having
// split, plus the exact slack.
//
// Why the slack constants are exact and not fudge factors. Both legs execute at
// the same ledger timestamp, so `Cache::needs_accrual` is false and
// `interest::global_sync` returns without touching either index
// (`contracts/pool/src/interest.rs:24`). Both runs therefore see literally the
// same `supply_index` / `borrow_index`, and the only difference is the single
// rounding step named per rule in `contracts/pool/src/cache/scale.rs`:
//
//   floor is sub-additive:   floor(x) + floor(y) ∈ [floor(x+y) − 1, floor(x+y)]
//   ceil  is super-additive: ceil(x)  + ceil(y)  ∈ [ceil(x+y), ceil(x+y) + 1]
//
// so one extra rounding boundary is crossed by splitting, worth at most one
// ray-scaled share. `Ray::from_asset` is an exact upscale by `10^(27−decimals)`
// for every `decimals <= RAY_DECIMALS`, so it contributes no slack of its own.
//
// Derivation of the constant, in full, for one leg. Write `K = 10^(27−decimals)`
// and let `I` be the live index. `Ray::from_asset(a) = a·K` exactly, so
//
//   calculate_scaled_supply(a)       = floor(a·K·RAY / I)      (supply mint)
//   calculate_scaled_borrow(a)       = ceil (a·K·RAY / I)      (borrow mint)
//   calculate_scaled_supply_ceil(a)  = ceil (a·K·RAY / I)      (withdraw / settle burn)
//   calculate_scaled_borrow_floor(a) = floor(a·K·RAY / I)      (repay / settle burn)
//
// `a ↦ a·K·RAY/I` is exactly additive over the rationals, so with
// `x = a1·K·RAY/I` and `y = a2·K·RAY/I` the whole slack is the classical
// `floor(x)+floor(y) − floor(x+y) ∈ {−1, 0}` and
// `ceil(x)+ceil(y) − ceil(x+y) ∈ {0, +1}`. Hence **exactly 1 ray share**, in
// the direction that is unfavourable to a splitting caller, for every rule below.
//
// PROOF STATUS: these rules are COMPILE-VERIFIED ONLY — they have not been run
// through the Certora prover. The bounds rest on the derivation above plus a
// randomized exact-integer model of `common/src/rates/scaling.rs`. Assertions
// that are *not* a single rounding step are flagged individually below.
// ---------------------------------------------------------------------------

/// `calculate_scaled_supply` **floors**, so two supplies mint at most as many
/// shares as one supply of the sum, and lose at most 1 ray share to the extra
/// truncation. Splitting a deposit can never mint extra claim.
#[rule]
#[allow(clippy::too_many_arguments)]
fn additivity_supply_split_never_mints_more(
    e: Env,
    admin: Address,
    asset: Address,
    first: i128,
    second: i128,
    position_before: i128,
    supply_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(first > 0 && first <= MAX_FLOW_AMOUNT);
    cvlr_assume!(second > 0 && second <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before >= 0 && position_before <= 10 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);

    let market = params_with_decimals(asset.clone(), 0, false, asset_decimals);
    let initial = state(
        100 * RAY,
        100 * RAY,
        0,
        supply_index.max(RAY),
        supply_index,
        200 * ONE_TOKEN,
        e.ledger().timestamp(),
    );
    seed(&e, admin, asset.clone(), market.clone(), initial.clone());

    let leg_one = crate::ops::supply::apply(
        &e,
        &PoolSupplyEntry {
            action: action(asset.clone(), position_before, first),
        },
    )
    .0;
    let leg_two = crate::ops::supply::apply(
        &e,
        &PoolSupplyEntry {
            action: action(asset.clone(), leg_one.position.scaled_amount, second),
        },
    )
    .0;
    let split = read_state(&e, &asset);
    let split_minted = leg_two.position.scaled_amount - position_before;

    write_market(&e, asset.clone(), market, initial);
    let whole = crate::ops::supply::apply(
        &e,
        &PoolSupplyEntry {
            action: action(asset.clone(), position_before, first + second),
        },
    )
    .0;
    let single = read_state(&e, &asset);
    let single_minted = whole.position.scaled_amount - position_before;

    cvlr_assert!(leg_one.actual_amount == first && leg_two.actual_amount == second);
    cvlr_assert!(whole.actual_amount == first + second);
    // floor sub-additivity: splitting never mints more, and loses at most 1 ray share.
    cvlr_assert!(split_minted <= single_minted);
    cvlr_assert!(single_minted - split_minted <= 1);
    cvlr_assert!(split.supplied <= single.supplied);
    cvlr_assert!(single.supplied - split.supplied <= 1);
    // Identical cash paid in either way — the caller funds the same amount.
    cvlr_assert!(split.cash == single.cash);
    cvlr_assert!(split.supply_index == single.supply_index);
    cvlr_assert!(split.borrow_index == single.borrow_index);
}

/// `calculate_scaled_borrow` **ceils**, so two borrows mint at least as much
/// debt as one borrow of the sum, exceeding it by at most 1 ray share, while
/// paying out exactly the same cash. Splitting a draw can never shrink the debt.
#[rule]
#[allow(clippy::too_many_arguments)]
fn additivity_borrow_split_never_reduces_debt(
    e: Env,
    admin: Address,
    asset: Address,
    first: i128,
    second: i128,
    debt_before: i128,
    borrow_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(first > 0 && first <= MAX_FLOW_AMOUNT);
    cvlr_assume!(second > 0 && second <= MAX_FLOW_AMOUNT);
    cvlr_assume!(debt_before >= 0 && debt_before <= 10 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);

    let market = params_with_decimals(asset.clone(), 0, false, asset_decimals);
    let initial = state(
        100 * RAY,
        debt_before,
        0,
        borrow_index,
        RAY,
        1_000_000 * ONE_TOKEN,
        e.ledger().timestamp(),
    );
    seed(&e, admin, asset.clone(), market.clone(), initial.clone());

    let leg_one = crate::ops::borrow::accounting(
        &e,
        &PoolBorrowEntry {
            action: action(asset.clone(), debt_before, first),
        },
    )
    .mutation;
    let leg_two = crate::ops::borrow::accounting(
        &e,
        &PoolBorrowEntry {
            action: action(asset.clone(), leg_one.position.scaled_amount, second),
        },
    )
    .mutation;
    let split = read_state(&e, &asset);
    let split_debt = leg_two.position.scaled_amount - debt_before;

    write_market(&e, asset.clone(), market, initial);
    let whole = crate::ops::borrow::accounting(
        &e,
        &PoolBorrowEntry {
            action: action(asset.clone(), debt_before, first + second),
        },
    )
    .mutation;
    let single = read_state(&e, &asset);
    let single_debt = whole.position.scaled_amount - debt_before;

    cvlr_assert!(leg_one.actual_amount == first && leg_two.actual_amount == second);
    cvlr_assert!(whole.actual_amount == first + second);
    // ceil super-additivity: splitting never owes less, and at most 1 ray share more.
    cvlr_assert!(split_debt >= single_debt);
    cvlr_assert!(split_debt - single_debt <= 1);
    cvlr_assert!(split.borrowed >= single.borrowed);
    cvlr_assert!(split.borrowed - single.borrowed <= 1);
    // Identical cash drawn either way.
    cvlr_assert!(split.cash == single.cash);
    cvlr_assert!(split.supply_index == single.supply_index);
    cvlr_assert!(split.borrow_index == single.borrow_index);
}

/// The partial branch of `resolve_withdrawal` **ceils** the burn and pays the
/// requested amount verbatim, so two partial withdrawals hand over exactly the
/// same tokens as one withdrawal of the sum while burning at least as many
/// shares — at most 1 ray share more. Splitting an exit is never cheaper.
#[rule]
#[allow(clippy::too_many_arguments)]
fn additivity_withdraw_split_never_pays_more(
    e: Env,
    admin: Address,
    asset: Address,
    first: i128,
    second: i128,
    position_before: i128,
    supply_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(first > 0 && first <= MAX_FLOW_AMOUNT);
    cvlr_assume!(second > 0 && second <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before > 0 && position_before <= 20 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);
    let held = Ray::from(position_before)
        .mul(&e, Ray::from(supply_index))
        .to_asset(asset_decimals);
    cvlr_assume!(first + second < held);

    let market = params_with_decimals(asset.clone(), 0, false, asset_decimals);
    let initial = state(
        100 * RAY,
        0,
        0,
        RAY,
        supply_index,
        i128::MAX,
        e.ledger().timestamp(),
    );
    seed(&e, admin, asset.clone(), market.clone(), initial.clone());

    let leg_one = crate::ops::withdraw::accounting(
        &e,
        false,
        &PoolWithdrawEntry {
            action: action(asset.clone(), position_before, first),
            protocol_fee: 0,
        },
    );
    let leg_two = crate::ops::withdraw::accounting(
        &e,
        false,
        &PoolWithdrawEntry {
            action: action(
                asset.clone(),
                leg_one.mutation.position.scaled_amount,
                second,
            ),
            protocol_fee: 0,
        },
    );
    let split_burn = position_before - leg_two.mutation.position.scaled_amount;
    let split_paid = leg_one.net_transfer + leg_two.net_transfer;

    write_market(&e, asset.clone(), market, initial);
    let whole = crate::ops::withdraw::accounting(
        &e,
        false,
        &PoolWithdrawEntry {
            action: action(asset.clone(), position_before, first + second),
            protocol_fee: 0,
        },
    );
    let single_burn = position_before - whole.mutation.position.scaled_amount;
    let single_paid = whole.net_transfer;

    // Proceeds never favour the split caller, in every branch mix. Not a single
    // rounding step: `first + second < held` pins the single call to the partial
    // branch (`single_paid == first + second`), and leg two pays `second` when
    // partial or `floor_value(p1) <= hu_value(p1) <= second` when it full-closes.
    cvlr_assert!(split_paid <= single_paid);
    if leg_one.mutation.actual_amount == first
        && leg_two.mutation.actual_amount == second
        && whole.mutation.actual_amount == first + second
    {
        // Partial regime: identical proceeds, ceil super-additive burn.
        cvlr_assert!(split_paid == single_paid);
        cvlr_assert!(split_burn >= single_burn);
        cvlr_assert!(split_burn - single_burn <= 1);
    }
}

/// The interesting boundary: a partial withdrawal followed by a full close can
/// never extract more than a single full close. `resolve_withdrawal` ceils the
/// partial burn and floors the closing payout, so the ceiling removes at least
/// the value the caller already took. This is the withdraw-side shape of
/// CS-AAVE4-009's "split the operation to cross a rounding boundary twice".
#[rule]
#[allow(clippy::too_many_arguments)]
fn additivity_withdraw_partial_then_close_never_exceeds_full_close(
    e: Env,
    admin: Address,
    asset: Address,
    first: i128,
    position_before: i128,
    supply_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(first > 0 && first <= MAX_FLOW_AMOUNT);
    cvlr_assume!(position_before > 0 && position_before <= 20 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);

    let market = params_with_decimals(asset.clone(), 0, false, asset_decimals);
    let initial = state(
        100 * RAY,
        0,
        0,
        RAY,
        supply_index,
        i128::MAX,
        e.ledger().timestamp(),
    );
    seed(&e, admin, asset.clone(), market.clone(), initial.clone());

    let leg_one = crate::ops::withdraw::accounting(
        &e,
        false,
        &PoolWithdrawEntry {
            action: action(asset.clone(), position_before, first),
            protocol_fee: 0,
        },
    );
    let leg_two = crate::ops::withdraw::accounting(
        &e,
        false,
        &PoolWithdrawEntry {
            action: action(
                asset.clone(),
                leg_one.mutation.position.scaled_amount,
                i128::MAX,
            ),
            protocol_fee: 0,
        },
    );
    let split_paid = leg_one.net_transfer + leg_two.net_transfer;

    write_market(&e, asset.clone(), market, initial);
    let whole = crate::ops::withdraw::accounting(
        &e,
        false,
        &PoolWithdrawEntry {
            action: action(asset.clone(), position_before, i128::MAX),
            protocol_fee: 0,
        },
    );

    // Both routes end with an empty position, and the split route never pays more.
    cvlr_assert!(leg_two.mutation.position.scaled_amount == 0);
    cvlr_assert!(whole.mutation.position.scaled_amount == 0);
    cvlr_assert!(split_paid <= whole.net_transfer);
}

/// The partial branch of `resolve_repay` **floors** the burn, so two repayments
/// retire at most as much debt as one repayment of the sum — at most 1 ray share
/// less — for exactly the same cash. Splitting a repayment never buys more relief.
#[rule]
#[allow(clippy::too_many_arguments)]
fn additivity_repay_split_never_burns_more_debt(
    e: Env,
    admin: Address,
    asset: Address,
    first: i128,
    second: i128,
    debt_before: i128,
    borrow_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(first > 0 && first <= MAX_FLOW_AMOUNT);
    cvlr_assume!(second > 0 && second <= MAX_FLOW_AMOUNT);
    cvlr_assume!(debt_before > 0 && debt_before <= 20 * RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);

    let market = params_with_decimals(asset.clone(), 0, false, asset_decimals);
    let initial = state(
        100 * RAY,
        debt_before,
        0,
        borrow_index,
        RAY,
        100 * ONE_TOKEN,
        e.ledger().timestamp(),
    );
    seed(&e, admin, asset.clone(), market.clone(), initial.clone());

    let leg_one = crate::ops::repay::accounting(&e, &action(asset.clone(), debt_before, first));
    let leg_two = crate::ops::repay::accounting(
        &e,
        &action(
            asset.clone(),
            leg_one.mutation.position.scaled_amount,
            second,
        ),
    );
    let split_burn = debt_before - leg_two.mutation.position.scaled_amount;
    let split_paid = leg_one.mutation.actual_amount + leg_two.mutation.actual_amount;

    write_market(&e, asset.clone(), market, initial);
    let whole =
        crate::ops::repay::accounting(&e, &action(asset.clone(), debt_before, first + second));
    let single_burn = debt_before - whole.mutation.position.scaled_amount;

    // Splitting never retires more debt, in every branch mix. Not a single
    // rounding step: when leg two full-closes, `unscale_borrow_ceil` composes as
    // `ceil(p·I/(RAY·K))`, so `DC(p0) <= first + DC(p1)` and the single call is
    // driven into its own full-close branch, burning the same `debt_before`.
    cvlr_assert!(split_burn <= single_burn);
    // And the caller never buys the same relief for less cash.
    if split_burn == single_burn {
        cvlr_assert!(split_paid >= whole.mutation.actual_amount);
    }
    if leg_one.overpayment == 0 && leg_two.overpayment == 0 && whole.overpayment == 0 {
        // Partial regime: same cash in, floor sub-additive burn — exactly 1 ray share.
        cvlr_assert!(single_burn - split_burn <= 1);
    }
}

/// `resolve_net_settle` sizes the burn from the conservative overlap: supply is
/// **ceiled** and debt is **floored**. Splitting therefore burns at least as much
/// collateral (≤ +1 ray share) and retires at most as much debt (≥ −1 ray share)
/// for the same settled tokens — both directions against the caller.
///
/// Deliberately *not* asserted: `split_settled <= single_settled`. When the
/// overlap is debt-bound, the floored debt burn leaves a residue smaller than one
/// debt share, and the next call's ceiled debt valuation re-exposes it, so a split
/// can settle up to the asset value of one debt share more than a single call.
/// That is not a caller gain — the extra settled units are paid for with extra
/// ceiled supply shares — so the property that matters is the per-side ordering
/// below, plus the unguarded "never strictly better on both axes" check.
#[rule]
#[allow(clippy::too_many_arguments)]
fn additivity_net_settle_split_never_favours_caller(
    e: Env,
    admin: Address,
    asset: Address,
    first: i128,
    second: i128,
    supply_before: i128,
    debt_before: i128,
    supply_index: i128,
    borrow_index: i128,
    asset_decimals: u32,
) {
    cvlr_assume!(first > 0 && first <= MAX_FLOW_AMOUNT);
    cvlr_assume!(second > 0 && second <= MAX_FLOW_AMOUNT);
    cvlr_assume!(supply_before > 0 && supply_before <= 20 * RAY);
    cvlr_assume!(debt_before > 0 && debt_before <= 20 * RAY);
    cvlr_assume!(supply_index >= SUPPLY_INDEX_FLOOR_RAW && supply_index <= MAX_SUPPLY_INDEX_RAY);
    cvlr_assume!(borrow_index >= RAY && borrow_index <= MAX_BORROW_INDEX_RAY);
    cvlr_assume!(asset_decimals <= RAY_DECIMALS);
    cvlr_assume!(e.ledger().timestamp() <= u64::MAX / 1_000);

    let market = params_with_decimals(asset.clone(), 0, false, asset_decimals);
    let initial = state(
        100 * RAY,
        100 * RAY,
        0,
        borrow_index,
        supply_index,
        200 * ONE_TOKEN,
        e.ledger().timestamp(),
    );
    seed(&e, admin, asset.clone(), market.clone(), initial.clone());

    let leg_one = crate::ops::net_settle::apply(
        &e,
        &PoolNetSettleEntry {
            hub_asset: hub(asset.clone()),
            amount: first,
            supply_position: position(supply_before),
            debt_position: position(debt_before),
        },
    )
    .0;
    let leg_two = crate::ops::net_settle::apply(
        &e,
        &PoolNetSettleEntry {
            hub_asset: hub(asset.clone()),
            amount: second,
            supply_position: leg_one.supply_position.clone(),
            debt_position: leg_one.debt_position.clone(),
        },
    )
    .0;
    let split_supply_burn = supply_before - leg_two.supply_position.scaled_amount;
    let split_debt_burn = debt_before - leg_two.debt_position.scaled_amount;

    write_market(&e, asset.clone(), market, initial);
    let whole = crate::ops::net_settle::apply(
        &e,
        &PoolNetSettleEntry {
            hub_asset: hub(asset.clone()),
            amount: first + second,
            supply_position: position(supply_before),
            debt_position: position(debt_before),
        },
    )
    .0;
    let single_supply_burn = supply_before - whole.supply_position.scaled_amount;
    let single_debt_burn = debt_before - whole.debt_position.scaled_amount;

    // Splitting is never a strict improvement on both axes at once: the caller
    // can never give up less collateral *and* retire more debt.
    //
    // LOWEST-CONFIDENCE ASSERTION IN THIS FILE. It is not a single rounding step
    // and it spans every branch mix of `resolve_net_settle` (amount-bound,
    // supply-bound, debt-bound, and either side fully closing). It survived
    // ~590k randomized samples of an exact integer model but has no hand proof,
    // so expect this one to be the first to fail if any does.
    cvlr_assert!(!(split_supply_burn < single_supply_burn && split_debt_burn > single_debt_burn));
    if leg_one.settled_amount == first
        && leg_two.settled_amount == second
        && whole.settled_amount == first + second
    {
        // Collateral side ceils: splitting burns at least as much, ≤ +1 ray share.
        cvlr_assert!(split_supply_burn >= single_supply_burn);
        cvlr_assert!(split_supply_burn - single_supply_burn <= 1);
        // Debt side floors: splitting retires at most as much, ≥ −1 ray share.
        cvlr_assert!(split_debt_burn <= single_debt_burn);
        cvlr_assert!(single_debt_burn - split_debt_burn <= 1);
    }
}
