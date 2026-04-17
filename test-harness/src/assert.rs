use common::constants::WAD;
use common::types::ControllerKey;

use crate::context::LendingTest;
use crate::view::PositionType;

// ---------------------------------------------------------------------------
// Error code constants for assertions (from common/src/errors.rs)
// ---------------------------------------------------------------------------

pub mod errors {
    // GenericError
    pub const ASSET_NOT_SUPPORTED: u32 = 1;
    pub const ASSET_ALREADY_SUPPORTED: u32 = 2;
    pub const ASSETS_ARE_THE_SAME: u32 = 7;
    pub const ACCOUNT_NOT_IN_MARKET: u32 = 13;
    pub const AMOUNT_MUST_BE_POSITIVE: u32 = 14;
    pub const INVALID_PAYMENTS: u32 = 16;
    pub const ACCOUNT_MODE_MISMATCH: u32 = 25;
    pub const INTERNAL_ERROR: u32 = 34;
    pub const CONTRACT_PAUSED: u32 = 1000; // Pausable enforced pause

    // CollateralError
    pub const INSUFFICIENT_COLLATERAL: u32 = 100;
    pub const HEALTH_FACTOR_TOO_HIGH: u32 = 101;
    pub const HEALTH_FACTOR_TOO_LOW: u32 = 102;
    pub const NOT_COLLATERAL: u32 = 104;
    pub const SUPPLY_CAP_REACHED: u32 = 105;
    pub const BORROW_CAP_REACHED: u32 = 106;
    pub const ASSET_NOT_BORROWABLE: u32 = 107;
    pub const NOT_BORROWABLE_SILOED: u32 = 108;
    pub const POSITION_LIMIT_EXCEEDED: u32 = 109;
    pub const POSITION_NOT_FOUND: u32 = 110;
    pub const INVALID_POSITION_MODE: u32 = 111;
    pub const INSUFFICIENT_LIQUIDITY: u32 = 112;
    pub const INVALID_LIQ_THRESHOLD: u32 = 113;
    pub const CANNOT_CLEAN_BAD_DEBT: u32 = 114;
    pub const DEBT_POSITION_NOT_FOUND: u32 = 120;
    pub const COLLATERAL_POSITION_NOT_FOUND: u32 = 121;
    pub const CANNOT_CLOSE_WITH_REMAINING_DEBT: u32 = 122;

    // OracleError
    pub const UNSAFE_PRICE: u32 = 205;
    pub const PRICE_FEED_STALE: u32 = 206;
    pub const BAD_FIRST_TOLERANCE: u32 = 207;
    pub const REFLECTOR_NOT_CONFIGURED: u32 = 215;
    pub const ORACLE_NOT_CONFIGURED: u32 = 216;

    // EModeError
    pub const EMODE_CATEGORY_NOT_FOUND: u32 = 300;
    pub const EMODE_CATEGORY_DEPRECATED: u32 = 301;
    pub const EMODE_WITH_ISOLATED: u32 = 302;
    pub const MIX_ISOLATED_COLLATERAL: u32 = 303;
    pub const DEBT_CEILING_REACHED: u32 = 304;
    pub const NOT_BORROWABLE_ISOLATION: u32 = 305;
    pub const ASSET_NOT_IN_EMODE: u32 = 307;

    // FlashLoanError
    pub const FLASH_LOAN_ONGOING: u32 = 400;
    pub const FLASHLOAN_NOT_ENABLED: u32 = 401;
    pub const INVALID_FLASHLOAN_REPAY: u32 = 402;
    pub const SWAP_COLLATERAL_NO_ISO: u32 = 404;
    pub const SWAP_DEBT_NOT_SUPPORTED: u32 = 406;
    pub const NO_DEBT_PAYMENTS: u32 = 407;
    pub const MULTIPLY_EXTRA_STEPS: u32 = 408;

    // StrategyError
    pub const CONVERT_STEPS_REQUIRED: u32 = 500;
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
    // -----------------------------------------------------------------------
    // Health factor assertions
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Position assertions
    // -----------------------------------------------------------------------

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
            let has_pos = match pos_type {
                PositionType::Supply => self
                    .env
                    .storage()
                    .persistent()
                    .has(&ControllerKey::SupplyPosition(account_id, asset.clone())),
                PositionType::Borrow => self
                    .env
                    .storage()
                    .persistent()
                    .has(&ControllerKey::BorrowPosition(account_id, asset.clone())),
            };
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
            let account: Option<common::types::AccountMeta> = self
                .env
                .storage()
                .persistent()
                .get(&ControllerKey::AccountMeta(account_id));
            let (supply_count, borrow_count) = account.map_or((0u32, 0u32), |acct| {
                (acct.supply_assets.len(), acct.borrow_assets.len())
            });
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
                let account: Option<common::types::AccountMeta> = self
                    .env
                    .storage()
                    .persistent()
                    .get(&ControllerKey::AccountMeta(account_id));
                account.map_or(0u32, |acct| acct.supply_assets.len())
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
                let account: Option<common::types::AccountMeta> = self
                    .env
                    .storage()
                    .persistent()
                    .get(&ControllerKey::AccountMeta(account_id));
                account.map_or(0u32, |acct| acct.borrow_assets.len())
            })
        });
        assert_eq!(
            count, expected,
            "'{}' should have {} borrow positions, got {}",
            user, expected, count
        );
    }

    // -----------------------------------------------------------------------
    // Balance assertions
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Pool assertions
    // -----------------------------------------------------------------------

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
