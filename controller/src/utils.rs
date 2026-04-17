use common::constants::WAD;
use common::errors::CollateralError;
use common::fp::Wad;
use common::types::{Account, PositionMode};
use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::cache::ControllerCache;
use crate::storage;

pub use crate::positions::account::{create_account, remove_account};

// ---------------------------------------------------------------------------
// Account Helpers
// ---------------------------------------------------------------------------

pub fn create_account_for_first_asset(
    env: &Env,
    caller: &Address,
    e_mode_category: u32,
    assets: &Vec<(Address, i128)>,
) -> u64 {
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

pub fn validate_account_is_empty(env: &Env, account: &Account) {
    if !account.supply_positions.is_empty() || !account.borrow_positions.is_empty() {
        panic_with_error!(env, CollateralError::PositionNotFound);
    }
}

// ---------------------------------------------------------------------------
// Market Helpers
// ---------------------------------------------------------------------------

pub fn sync_market_indexes(_env: &Env, cache: &mut ControllerCache, assets: &Vec<Address>) {
    for asset in assets {
        cache.cached_market_index(&asset);
    }
}

// ---------------------------------------------------------------------------
// Isolated debt adjustment
// ---------------------------------------------------------------------------

/// Decrements the isolated-debt tracker by the repaid fraction of
/// outstanding debt: `new_debt = current * (outstanding - repaid) /
/// outstanding`. Oracle-independent. Full repayment (repaid >=
/// outstanding) zeros the tracker. Sub-$1 residuals are also zeroed to
/// prevent dust lockup.
///
/// `price_wad` and `asset_decimals` remain in the signature for call-site
/// compatibility but do not affect the decrement.
pub fn adjust_isolated_debt_usd(
    env: &Env,
    account: &Account,
    token_amount: i128,
    _price_wad: &i128,
    _asset_decimals: u32,
    outstanding_before: i128,
    cache: &mut ControllerCache,
) {
    let Some(isolated_asset) = account.isolated_asset.clone() else {
        return;
    };

    if token_amount <= 0 || outstanding_before <= 0 {
        return;
    }

    let current = cache.get_isolated_debt(&isolated_asset);
    if current == 0 {
        return;
    }

    let mut new_debt = if token_amount >= outstanding_before {
        0
    } else {
        // Computed via Wad to bound intermediate width at extreme values.
        let remaining = outstanding_before - token_amount;
        let fraction_wad = Wad::from_raw(common::fp_core::mul_div_floor(
            env,
            remaining,
            WAD,
            outstanding_before,
        ));
        Wad::from_raw(current).mul(env, fraction_wad).raw()
    };

    // Zero sub-$1 residuals so dust cannot keep the isolated flag live.
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

        // Non-isolated account: no-op regardless of outstanding_before.
        adjust_isolated_debt_usd(&env, &account, 10_000_000, &WAD, 7, 10_000_000, &mut cache);

        assert_eq!(cache.get_isolated_debt(&tracked_asset), 77);
    }

    #[test]
    fn test_adjust_isolated_debt_usd_erases_sub_dollar_dust() {
        let env = Env::default();
        let isolated_asset = Address::generate(&env);
        let account = empty_account(&env, Some(isolated_asset.clone()));
        let mut cache = ControllerCache::new(&env, true);

        cache.set_isolated_debt(&isolated_asset, WAD + (WAD / 2));
        // Full clear: outstanding == repaid -> decrement fraction = 1 -> tracker zeroed.
        adjust_isolated_debt_usd(&env, &account, 10_000_000, &WAD, 7, 10_000_000, &mut cache);

        assert_eq!(cache.get_isolated_debt(&isolated_asset), 0);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #110)")]
    fn test_validate_account_is_empty_rejects_open_positions() {
        let env = Env::default();
        let mut account = empty_account(&env, None);
        let asset = Address::generate(&env);

        account.supply_positions.set(
            asset.clone(),
            common::types::AccountPosition {
                position_type: common::types::AccountPositionType::Deposit,
                asset,
                scaled_amount_ray: 1,
                account_id: 1,
                liquidation_threshold_bps: 8_000,
                liquidation_bonus_bps: 500,
                liquidation_fees_bps: 100,
                loan_to_value_bps: 7_500,
            },
        );

        validate_account_is_empty(&env, &account);
    }
}
