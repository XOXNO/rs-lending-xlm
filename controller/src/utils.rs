use common::constants::WAD;
use common::errors::CollateralError;
use common::fp::Wad;
use common::types::{Account, Payment, PositionMode};
use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::cache::ControllerCache;
use crate::storage;

pub use crate::positions::account::{create_account, remove_account};

// ---------------------------------------------------------------------------
// Account Helpers
// ---------------------------------------------------------------------------

/// Creates a new account for the supply entry point, deriving the isolation flag from
/// the first asset in the batch.
pub fn create_account_for_first_asset(
    env: &Env,
    caller: &Address,
    e_mode_category: u32,
    assets: &Vec<Payment>,
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

/// Panics with `PositionNotFound` when either position map is non-empty.
pub fn validate_account_is_empty(env: &Env, account: &Account) {
    if !account.supply_positions.is_empty() || !account.borrow_positions.is_empty() {
        panic_with_error!(env, CollateralError::PositionNotFound);
    }
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
        let pool_client = pool_interface::LiquidityPoolClient::new(env, &pool_addr);
        let index = pool_client.update_indexes(&0);
        // Refresh the in-memory cache so any subsequent reads in this tx see
        // the now-persisted index instead of recomputing.
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
    use common::types::{AccountPosition, AccountPositionType};
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

    #[test]
    #[should_panic(expected = "Error(Contract, #110)")]
    fn test_validate_account_is_empty_rejects_open_positions() {
        let env = Env::default();
        let mut account = empty_account(&env, None);
        let asset = Address::generate(&env);

        account.supply_positions.set(
            asset.clone(),
            AccountPosition {
                position_type: AccountPositionType::Deposit,
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
