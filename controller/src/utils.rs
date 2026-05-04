use common::constants::WAD;
use common::errors::GenericError;
use common::fp::Wad;
use common::types::{Account, Payment, PositionMode};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

use crate::cache::ControllerCache;
use crate::storage;

pub use crate::positions::account::{create_account, remove_account};

// ---------------------------------------------------------------------------
// Payment Helpers
// ---------------------------------------------------------------------------

/// Deduplicates `(asset, amount)` payments in first-seen asset order and sums
/// duplicate amounts. Rejects zero, negative, and overflowing totals.
pub fn aggregate_positive_payments(env: &Env, payments: &Vec<Payment>) -> Vec<Payment> {
    aggregate_payments(env, payments, false)
}

/// Deduplicates withdrawal requests. Positive duplicates are summed, while a
/// zero amount remains the "withdraw all" sentinel for that asset.
pub fn aggregate_withdrawal_payments(env: &Env, payments: &Vec<Payment>) -> Vec<Payment> {
    aggregate_payments(env, payments, true)
}

/// Transfers `asset` from `from` to `to` and returns the positive balance
/// increase observed at `to`.
pub fn transfer_and_measure_received(
    env: &Env,
    asset: &Address,
    from: &Address,
    to: &Address,
    amount: i128,
    balance_decrease_error: GenericError,
) -> i128 {
    let balance_before = sac_balance_call(env, asset, to);

    sac_transfer_call(env, asset, from, to, &amount);

    let received = sac_balance_call(env, asset, to)
        .checked_sub(balance_before)
        .unwrap_or_else(|| panic_with_error!(env, balance_decrease_error));
    if received <= 0 {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }

    received
}

// ---------------------------------------------------------------------------
// Summarised SAC wrappers (used across the controller; the macro is a no-op
// outside `--features certora`).
// ---------------------------------------------------------------------------

crate::summarized!(
    crate::spec::summaries::sac::balance_summary,
    pub(crate) fn sac_balance_call(env: &Env, token: &Address, account: &Address) -> i128 {
        soroban_sdk::token::Client::new(env, token).balance(account)
    }
);

crate::summarized!(
    crate::spec::summaries::sac::transfer_summary,
    pub(crate) fn sac_transfer_call(
        env: &Env,
        token: &Address,
        from: &Address,
        to: &Address,
        amount: &i128,
    ) {
        soroban_sdk::token::Client::new(env, token).transfer(from, to, amount)
    }
);

// `sac::approve_summary` and `sac::allowance_summary` carry the matching
// arity (leading `_token: &Address`) for direct wrapper call sites. Production
// `approve` and `allowance` calls live in strategy helpers that reuse a
// `token::Client`, so they remain raw client calls unless the router approval
// flow is summarized explicitly.

fn aggregate_payments(
    env: &Env,
    payments: &Vec<Payment>,
    zero_is_withdraw_all: bool,
) -> Vec<Payment> {
    let mut order: Vec<Address> = Vec::new(env);
    let mut totals: Map<Address, i128> = Map::new(env);

    for (asset, amount) in payments {
        let next =
            aggregate_payment_amount(env, totals.get(asset.clone()), amount, zero_is_withdraw_all);

        if !totals.contains_key(asset.clone()) {
            order.push_back(asset.clone());
        }
        totals.set(asset, next);
    }

    let mut result = Vec::new(env);
    for asset in order {
        let amount = totals.get(asset.clone()).unwrap();
        result.push_back((asset, amount));
    }

    result
}

fn aggregate_payment_amount(
    env: &Env,
    previous: Option<i128>,
    amount: i128,
    zero_is_withdraw_all: bool,
) -> i128 {
    if amount < 0 || (!zero_is_withdraw_all && amount == 0) {
        panic_with_error!(env, GenericError::AmountMustBePositive);
    }

    if zero_is_withdraw_all && (amount == 0 || previous == Some(0)) {
        return 0;
    }

    previous
        .unwrap_or(0)
        .checked_add(amount)
        .unwrap_or_else(|| panic_with_error!(env, GenericError::MathOverflow))
}

// ---------------------------------------------------------------------------
// Account Helpers
// ---------------------------------------------------------------------------

/// Creates a new account for the supply entry point, deriving the isolation flag from
/// the first asset in the batch. Returns both the new id and the in-memory snapshot
/// so the caller can skip a redundant re-read.
pub fn create_account_for_first_asset(
    env: &Env,
    caller: &Address,
    e_mode_category: u32,
    assets: &Vec<Payment>,
) -> (u64, Account) {
    let (first_asset, _) = assets.get(0).unwrap();
    let first_config = storage::get_market_config(env, &first_asset).asset_config;
    let is_isolated = first_config.is_isolated_asset;
    let isolated_asset = if is_isolated {
        Some(first_asset.clone())
    } else {
        None
    };
    create_account(
        env,
        caller,
        e_mode_category,
        PositionMode::Normal,
        is_isolated,
        isolated_asset,
    )
}

// ---------------------------------------------------------------------------
// Market Helpers
// ---------------------------------------------------------------------------

/// Advances each pool's stored `last_timestamp` and persists accrued indices
/// by invoking `pool::update_indexes` directly. Updates the in-memory cache
/// so subsequent reads in the same transaction see the persisted index.
pub fn sync_market_indexes(env: &Env, cache: &mut ControllerCache, assets: &Vec<Address>) {
    for asset in assets {
        let pool_addr = cache.cached_pool_address(&asset);
        let index = crate::router::pool_update_indexes_call(env, &pool_addr, 0);
        // Refresh the in-memory cache so subsequent reads in this transaction
        // use the persisted index.
        cache.market_indexes.set(asset.clone(), index);
    }
}

// ---------------------------------------------------------------------------
// Isolated debt adjustment
// ---------------------------------------------------------------------------

/// Decrements the isolated-debt tracker by the USD value of `token_amount`:
/// `new_debt = max(0, current - token_amount × price_wad)`. Zeros residuals
/// below `WAD` ($1). No-op for non-isolated accounts. The decrement is
/// unconditional; under a permissive oracle cache (repay) accepts a
/// slightly off USD value rather than letting the global ceiling drift.
pub fn adjust_isolated_debt_usd(
    env: &Env,
    account: &Account,
    token_amount: i128,
    price_wad: &i128,
    asset_decimals: u32,
    cache: &mut ControllerCache,
) {
    let Some(isolated_asset) = account.isolated_asset.clone() else {
        return;
    };

    let amount_wad = Wad::from_token(token_amount, asset_decimals);
    let usd_wad = amount_wad.mul(env, Wad::from_raw(*price_wad)).raw();

    let current = cache.get_isolated_debt(&isolated_asset);
    let mut new_debt = if usd_wad >= current {
        0
    } else {
        current - usd_wad
    };

    if new_debt > 0 && new_debt < WAD {
        new_debt = 0;
    }

    cache.set_isolated_debt(&isolated_asset, new_debt);
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, Map};

    fn empty_account(env: &Env, isolated_asset: Option<Address>) -> Account {
        Account {
            owner: Address::generate(env),
            is_isolated: isolated_asset.is_some(),
            e_mode_category_id: 0,
            mode: PositionMode::Normal,
            isolated_asset,
            supply_positions: Map::new(env),
            borrow_positions: Map::new(env),
        }
    }

    #[test]
    fn test_adjust_isolated_debt_usd_noops_for_non_isolated_accounts() {
        let env = Env::default();
        let mut cache = ControllerCache::new(&env, true);
        let account = empty_account(&env, None);
        let tracked_asset = Address::generate(&env);

        cache.set_isolated_debt(&tracked_asset, 77);

        adjust_isolated_debt_usd(&env, &account, 10_000_000, &WAD, 7, &mut cache);

        assert_eq!(cache.get_isolated_debt(&tracked_asset), 77);
    }

    #[test]
    fn test_aggregate_positive_payments_sums_duplicates_in_first_seen_order() {
        let env = Env::default();
        let usdc = Address::generate(&env);
        let eth = Address::generate(&env);
        let payments =
            soroban_sdk::vec![&env, (usdc.clone(), 5), (eth.clone(), 7), (usdc.clone(), 3)];

        let aggregated = aggregate_positive_payments(&env, &payments);

        assert_eq!(aggregated.len(), 2);
        assert_eq!(aggregated.get(0).unwrap(), (usdc, 8));
        assert_eq!(aggregated.get(1).unwrap(), (eth, 7));
    }

    #[test]
    fn test_aggregate_withdrawal_payments_keeps_zero_sentinel() {
        let env = Env::default();
        let usdc = Address::generate(&env);
        let payments = soroban_sdk::vec![&env, (usdc.clone(), 5), (usdc.clone(), 0)];

        let aggregated = aggregate_withdrawal_payments(&env, &payments);

        assert_eq!(aggregated.len(), 1);
        assert_eq!(aggregated.get(0).unwrap(), (usdc, 0));
    }

    #[test]
    #[should_panic]
    fn test_aggregate_positive_payments_rejects_overflow() {
        let env = Env::default();
        let usdc = Address::generate(&env);
        let payments = soroban_sdk::vec![&env, (usdc.clone(), i128::MAX), (usdc, 1)];

        let _ = aggregate_positive_payments(&env, &payments);
    }

    #[test]
    fn test_adjust_isolated_debt_usd_erases_sub_dollar_dust() {
        let env = Env::default();
        let isolated_asset = Address::generate(&env);
        let account = empty_account(&env, Some(isolated_asset.clone()));
        let mut cache = ControllerCache::new(&env, true);

        cache.set_isolated_debt(&isolated_asset, WAD + (WAD / 2));
        adjust_isolated_debt_usd(&env, &account, 10_000_000, &WAD, 7, &mut cache);

        // 0.5 WAD residual is below the dust floor.
        assert_eq!(cache.get_isolated_debt(&isolated_asset), 0);
    }
}
