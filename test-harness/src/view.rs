use common::constants::{RAY, WAD};
use common::fp::Ray;
use common::types::{ControllerKey, PositionLimits, POSITION_TYPE_BORROW, POSITION_TYPE_DEPOSIT};
use soroban_sdk::token;

use crate::context::LendingTest;
use crate::helpers::{i128_to_f64, wad_to_f64};

/// Re-export for use in assertions.
pub enum PositionType {
    Supply,
    Borrow,
}

impl LendingTest {
    // -----------------------------------------------------------------------
    // Health factor
    // -----------------------------------------------------------------------

    pub fn health_factor(&self, user: &str) -> f64 {
        wad_to_f64(self.health_factor_raw(user))
    }

    pub fn health_factor_for(&self, _user: &str, account_id: u64) -> f64 {
        wad_to_f64(self.health_factor_for_raw(_user, account_id))
    }

    pub fn health_factor_raw(&self, user: &str) -> i128 {
        self.find_account_id(user)
            .map(|account_id| self.ctrl_client().health_factor(&account_id))
            .unwrap_or(i128::MAX)
    }

    pub fn health_factor_for_raw(&self, _user: &str, account_id: u64) -> i128 {
        self.ctrl_client().health_factor(&account_id)
    }

    // -----------------------------------------------------------------------
    // Token balances (wallet)
    // -----------------------------------------------------------------------

    pub fn token_balance(&self, user: &str, asset_name: &str) -> f64 {
        let decimals = self.resolve_market(asset_name).decimals;
        i128_to_f64(self.token_balance_raw(user, asset_name), decimals)
    }

    pub fn token_balance_raw(&self, user: &str, asset_name: &str) -> i128 {
        let user_state = self
            .users
            .get(user)
            .unwrap_or_else(|| panic!("user '{}' not found", user));
        let market = self.resolve_market(asset_name);
        let tok = token::Client::new(&self.env, &market.asset);
        tok.balance(&user_state.address)
    }

    // -----------------------------------------------------------------------
    // Position balances (protocol)
    // -----------------------------------------------------------------------

    pub fn supply_balance(&self, user: &str, asset_name: &str) -> f64 {
        let decimals = self.resolve_market(asset_name).decimals;
        i128_to_f64(self.supply_balance_raw(user, asset_name), decimals)
    }

    pub fn supply_balance_raw(&self, user: &str, asset_name: &str) -> i128 {
        let asset = self.resolve_asset(asset_name);
        self.find_account_id(user)
            .map(|account_id| self.position_balance_raw(account_id, &asset, POSITION_TYPE_DEPOSIT))
            .unwrap_or(0)
    }

    pub fn supply_balance_for(&self, _user: &str, account_id: u64, asset_name: &str) -> f64 {
        let decimals = self.resolve_market(asset_name).decimals;
        let asset = self.resolve_asset(asset_name);
        i128_to_f64(
            self.position_balance_raw(account_id, &asset, POSITION_TYPE_DEPOSIT),
            decimals,
        )
    }

    pub fn borrow_balance(&self, user: &str, asset_name: &str) -> f64 {
        let decimals = self.resolve_market(asset_name).decimals;
        i128_to_f64(self.borrow_balance_raw(user, asset_name), decimals)
    }

    pub fn borrow_balance_raw(&self, user: &str, asset_name: &str) -> i128 {
        let asset = self.resolve_asset(asset_name);
        self.find_account_id(user)
            .map(|account_id| self.position_balance_raw(account_id, &asset, POSITION_TYPE_BORROW))
            .unwrap_or(0)
    }

    pub fn borrow_balance_for(&self, _user: &str, account_id: u64, asset_name: &str) -> f64 {
        let decimals = self.resolve_market(asset_name).decimals;
        let asset = self.resolve_asset(asset_name);
        i128_to_f64(
            self.position_balance_raw(account_id, &asset, POSITION_TYPE_BORROW),
            decimals,
        )
    }

    fn position_balance_raw(
        &self,
        account_id: u64,
        asset: &soroban_sdk::Address,
        position_type: u32,
    ) -> i128 {
        let (supplies, borrows) = self.ctrl_client().get_account_positions(&account_id);
        let positions = if position_type == POSITION_TYPE_DEPOSIT {
            supplies
        } else {
            borrows
        };

        if let Some(position) = positions.get(asset.clone()) {
            let pool = self.resolve_market_by_asset(asset).pool.clone();
            let sync = pool::LiquidityPoolClient::new(&self.env, &pool).get_sync_data();
            let index = if position_type == POSITION_TYPE_DEPOSIT {
                sync.state.supply_index_ray
            } else {
                sync.state.borrow_index_ray
            };
            let decimals = self.resolve_market_by_asset(asset).decimals;
            return Ray::from_raw(position.scaled_amount_ray)
                .mul(&self.env, Ray::from_raw(index))
                .to_asset(decimals);
        }

        0
    }

    // -----------------------------------------------------------------------
    // USD totals
    // -----------------------------------------------------------------------

    pub fn total_collateral(&self, user: &str) -> f64 {
        wad_to_f64(self.total_collateral_raw(user))
    }

    pub fn total_collateral_raw(&self, user: &str) -> i128 {
        self.find_account_id(user)
            .map(|account_id| self.ctrl_client().total_collateral_in_usd(&account_id))
            .unwrap_or(0)
    }

    pub fn total_debt(&self, user: &str) -> f64 {
        wad_to_f64(self.total_debt_raw(user))
    }

    pub fn total_debt_raw(&self, user: &str) -> i128 {
        self.find_account_id(user)
            .map(|account_id| self.ctrl_client().total_borrow_in_usd(&account_id))
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Pool state
    // -----------------------------------------------------------------------

    pub fn pool_utilization(&self, asset_name: &str) -> f64 {
        let raw = self.pool_client(asset_name).capital_utilisation();
        raw as f64 / RAY as f64
    }

    pub fn pool_reserves(&self, asset_name: &str) -> f64 {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw = self.pool_client(asset_name).reserves();
        i128_to_f64(raw, decimals)
    }

    pub fn pool_borrow_rate(&self, asset_name: &str) -> f64 {
        let raw = self.pool_client(asset_name).borrow_rate();
        raw as f64 / RAY as f64
    }

    pub fn pool_supply_rate(&self, asset_name: &str) -> f64 {
        let raw = self.pool_client(asset_name).deposit_rate();
        raw as f64 / RAY as f64
    }

    // -----------------------------------------------------------------------
    // Revenue snapshots
    // -----------------------------------------------------------------------

    pub fn snapshot_revenue(&self, asset_name: &str) -> i128 {
        self.pool_client(asset_name).protocol_revenue()
    }

    // -----------------------------------------------------------------------
    // Liquidation status
    // -----------------------------------------------------------------------

    pub fn can_be_liquidated(&self, user: &str) -> bool {
        self.find_account_id(user)
            .map(|account_id| self.ctrl_client().health_factor(&account_id) < WAD)
            .unwrap_or(false)
    }

    pub fn can_be_liquidated_by_id(&self, account_id: u64) -> bool {
        self.ctrl_client().health_factor(&account_id) < WAD
    }

    // -----------------------------------------------------------------------
    // Isolated debt
    // -----------------------------------------------------------------------

    pub fn get_isolated_debt(&self, asset_name: &str) -> i128 {
        let asset = self.resolve_asset(asset_name);
        self.ctrl_client().get_isolated_debt(&asset)
    }

    // -----------------------------------------------------------------------
    // Account info
    // -----------------------------------------------------------------------

    pub fn get_account_attributes(&self, user: &str) -> common::types::AccountAttributes {
        let account_id = self.resolve_account_id(user);
        self.ctrl_client().get_account_attributes(&account_id)
    }

    pub fn get_active_accounts(&self, user: &str) -> soroban_sdk::Vec<u64> {
        let mut accounts = soroban_sdk::Vec::new(&self.env);
        if let Some(user_state) = self.users.get(user) {
            for account in &user_state.accounts {
                if self.account_exists(account.account_id) {
                    accounts.push_back(account.account_id);
                }
            }
        }
        accounts
    }

    pub fn get_asset_config(&self, asset_name: &str) -> common::types::AssetConfig {
        let asset = self.resolve_asset(asset_name);
        self.ctrl_client().get_market_config(&asset).asset_config
    }

    pub fn get_pool_address(&self, asset_name: &str) -> soroban_sdk::Address {
        let asset = self.resolve_asset(asset_name);
        self.ctrl_client().get_market_config(&asset).pool_address
    }

    pub fn get_position_limits(&self) -> common::types::PositionLimits {
        self.env.as_contract(&self.controller, || {
            self.env
                .storage()
                .instance()
                .get::<_, PositionLimits>(&ControllerKey::PositionLimits)
                .expect("position limits must exist")
        })
    }

    pub fn get_account_owner(&self, account_id: u64) -> soroban_sdk::Address {
        self.env.as_contract(&self.controller, || {
            self.env
                .storage()
                .persistent()
                .get::<_, common::types::AccountMeta>(&ControllerKey::AccountMeta(account_id))
                .expect("account must exist")
                .owner
        })
    }
}
