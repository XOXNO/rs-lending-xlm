use controller::constants::WAD;
use controller::types::ControllerKey;
use soroban_sdk::{Address, Env, Map};

use crate::context::LendingTest;
use crate::view::PositionType;

fn side_count(env: &Env, account_id: u64, pos_type: PositionType) -> u32 {
    let key = match pos_type {
        PositionType::Supply => ControllerKey::SupplyPositions(account_id),
        PositionType::Borrow => ControllerKey::BorrowPositions(account_id),
    };
    env.storage()
        .persistent()
        .get::<_, Map<Address, controller::types::AccountPositionRaw>>(&key)
        .map(|m| m.len())
        .unwrap_or(0)
}
/// Assert that a Result contains a specific contract error code.
///
/// Usage: `assert_contract_error(result, errors::INSUFFICIENT_COLLATERAL);`
pub fn assert_contract_error<T: std::fmt::Debug>(
    result: Result<T, soroban_sdk::Error>,
    expected_code: u32,
) {
    match result {
        Ok(val) => panic!(
            "expected contract error {} but got Ok({:?})",
            expected_code, val
        ),
        Err(err) => {
            let expected = soroban_sdk::Error::from_contract_error(expected_code);
            assert_eq!(
                err, expected,
                "expected contract error {} but got {:?}",
                expected_code, err
            );
        }
    }
}

impl LendingTest {
    // Health factor assertions

    pub fn assert_healthy(&self, user: &str) {
        let hf = self.health_factor_raw(user);
        assert!(
            hf >= WAD,
            "'{}' should be healthy (HF >= 1.0) but HF = {}",
            user,
            hf as f64 / WAD as f64
        );
    }

    pub fn assert_liquidatable(&self, user: &str) {
        let hf = self.health_factor_raw(user);
        assert!(
            hf < WAD,
            "'{}' should be liquidatable (HF < 1.0) but HF = {}",
            user,
            hf as f64 / WAD as f64
        );
    }

    pub fn assert_healthy_for(&self, user: &str, account_id: u64) {
        let hf = self.health_factor_for_raw(user, account_id);
        assert!(
            hf >= WAD,
            "'{}' account {} should be healthy but HF = {}",
            user,
            account_id,
            hf as f64 / WAD as f64
        );
    }

    pub fn assert_health_factor_near(&self, user: &str, expected: f64, tolerance: f64) {
        let actual = self.health_factor(user);
        assert!(
            (actual - expected).abs() <= tolerance,
            "'{}' HF expected ~{} (+-{}) but got {}",
            user,
            expected,
            tolerance,
            actual
        );
    }
    // Position assertions

    pub fn assert_position_exists(&self, user: &str, asset_name: &str, pos_type: PositionType) {
        let account_id = self.resolve_account_id(user);
        self.assert_position_exists_for(user, account_id, asset_name, pos_type);
    }

    pub fn assert_position_exists_for(
        &self,
        user: &str,
        account_id: u64,
        asset_name: &str,
        pos_type: PositionType,
    ) {
        let asset = self.resolve_asset(asset_name);

        let type_label = match pos_type {
            PositionType::Supply => "supply",
            PositionType::Borrow => "borrow",
        };

        self.env.as_contract(&self.controller, || {
            let map_key = match pos_type {
                PositionType::Supply => ControllerKey::SupplyPositions(account_id),
                PositionType::Borrow => ControllerKey::BorrowPositions(account_id),
            };
            let has_pos = self
                .env
                .storage()
                .persistent()
                .get::<_, soroban_sdk::Map<soroban_sdk::Address, controller::types::AccountPositionRaw>>(
                    &map_key,
                )
                .map(|m| m.contains_key(asset.clone()))
                .unwrap_or(false);
            assert!(
                has_pos,
                "'{}' account {} should have {} position for '{}'",
                user, account_id, type_label, asset_name
            );
        });
    }

    pub fn assert_no_positions(&self, user: &str) {
        if let Some(account_id) = self.find_account_id(user) {
            self.assert_no_positions_for(user, account_id);
        }
    }

    pub fn assert_no_positions_for(&self, user: &str, account_id: u64) {
        self.env.as_contract(&self.controller, || {
            let supply_count = side_count(&self.env, account_id, PositionType::Supply);
            let borrow_count = side_count(&self.env, account_id, PositionType::Borrow);
            assert!(
                supply_count == 0 && borrow_count == 0,
                "'{}' account {} should have no positions but has {} supply, {} borrow",
                user,
                account_id,
                supply_count,
                borrow_count
            );
        });
    }

    pub fn assert_supply_count(&self, user: &str, expected: u32) {
        let count = self.find_account_id(user).map_or(0u32, |account_id| {
            self.env.as_contract(&self.controller, || {
                side_count(&self.env, account_id, PositionType::Supply)
            })
        });
        assert_eq!(
            count, expected,
            "'{}' should have {} supply positions, got {}",
            user, expected, count
        );
    }

    pub fn assert_borrow_count(&self, user: &str, expected: u32) {
        let count = self.find_account_id(user).map_or(0u32, |account_id| {
            self.env.as_contract(&self.controller, || {
                side_count(&self.env, account_id, PositionType::Borrow)
            })
        });
        assert_eq!(
            count, expected,
            "'{}' should have {} borrow positions, got {}",
            user, expected, count
        );
    }
    // Balance assertions

    pub fn assert_balance_eq(&self, user: &str, asset_name: &str, expected: f64) {
        let actual = self.token_balance(user, asset_name);
        assert!(
            (actual - expected).abs() < 0.001,
            "'{}' balance of '{}' expected {} but got {}",
            user,
            asset_name,
            expected,
            actual
        );
    }

    pub fn assert_balance_gt(&self, user: &str, asset_name: &str, threshold: f64) {
        let actual = self.token_balance(user, asset_name);
        assert!(
            actual > threshold,
            "'{}' balance of '{}' should be > {} but got {}",
            user,
            asset_name,
            threshold,
            actual
        );
    }

    pub fn assert_supply_near(&self, user: &str, asset_name: &str, expected: f64, tolerance: f64) {
        let actual = self.supply_balance(user, asset_name);
        assert!(
            (actual - expected).abs() <= tolerance,
            "'{}' supply of '{}' expected ~{} (+-{}) but got {}",
            user,
            asset_name,
            expected,
            tolerance,
            actual
        );
    }

    pub fn assert_borrow_near(&self, user: &str, asset_name: &str, expected: f64, tolerance: f64) {
        let actual = self.borrow_balance(user, asset_name);
        assert!(
            (actual - expected).abs() <= tolerance,
            "'{}' borrow of '{}' expected ~{} (+-{}) but got {}",
            user,
            asset_name,
            expected,
            tolerance,
            actual
        );
    }
    // Pool assertions

    pub fn assert_pool_has_liquidity(&self, asset_name: &str) {
        let reserves = self.pool_reserves(asset_name);
        assert!(
            reserves > 0.0,
            "pool '{}' should have liquidity but reserves = {}",
            asset_name,
            reserves
        );
    }

    pub fn assert_revenue_increased_since(&self, asset_name: &str, snapshot: i128) {
        let current = self.snapshot_revenue(asset_name);
        assert!(
            current > snapshot,
            "pool '{}' revenue should have increased: before={}, after={}",
            asset_name,
            snapshot,
            current
        );
    }
}
