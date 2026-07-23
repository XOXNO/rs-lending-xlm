use common::math::fp::Ray;
use controller::constants::RAY;
use controller::types::{AccountPositionType, ControllerKey, PositionLimits};
use soroban_sdk::token;

use crate::context::LendingTest;
use crate::helpers::{hub_asset, i128_to_f64, wad_to_f64, HARNESS_SPOKE};

pub enum PositionType {
    Supply,
    Borrow,
}

/// Risk params from base spoke listing; flash-loan/decimals from pool `MarketParamsRaw`.
pub struct AssetConfigView {
    pub loan_to_value: u32,
    pub liquidation_threshold: u32,
    pub liquidation_bonus: u32,
    pub liquidation_fees: u32,
    pub is_collateralizable: bool,
    pub is_borrowable: bool,
    pub is_flashloanable: bool,
    pub flashloan_fee: u32,
    pub asset_decimals: u32,
}

impl LendingTest {
    pub fn health_factor(&self, user: &str) -> f64 {
        wad_to_f64(self.health_factor_raw(user))
    }

    pub fn health_factor_for(&self, _user: &str, account_id: u64) -> f64 {
        wad_to_f64(self.health_factor_for_raw(_user, account_id))
    }

    pub fn health_factor_raw(&self, user: &str) -> i128 {
        self.find_account_id(user)
            .map(|account_id| self.ctrl_client().get_health_factor(&account_id))
            .unwrap_or(i128::MAX)
    }

    pub fn health_factor_for_raw(&self, _user: &str, account_id: u64) -> i128 {
        self.ctrl_client().get_health_factor(&account_id)
    }

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

    pub fn supply_balance(&self, user: &str, asset_name: &str) -> f64 {
        let decimals = self.resolve_market(asset_name).decimals;
        i128_to_f64(self.supply_balance_raw(user, asset_name), decimals)
    }

    pub fn supply_balance_raw(&self, user: &str, asset_name: &str) -> i128 {
        let asset = self.resolve_asset(asset_name);
        self.find_account_id(user)
            .map(|account_id| {
                self.position_balance_raw(account_id, &asset, AccountPositionType::Deposit)
            })
            .unwrap_or(0)
    }

    pub fn supply_balance_for(&self, _user: &str, account_id: u64, asset_name: &str) -> f64 {
        let decimals = self.resolve_market(asset_name).decimals;
        let asset = self.resolve_asset(asset_name);
        i128_to_f64(
            self.position_balance_raw(account_id, &asset, AccountPositionType::Deposit),
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
            .map(|account_id| {
                self.position_balance_raw(account_id, &asset, AccountPositionType::Borrow)
            })
            .unwrap_or(0)
    }

    pub fn borrow_balance_for(&self, _user: &str, account_id: u64, asset_name: &str) -> f64 {
        let decimals = self.resolve_market(asset_name).decimals;
        let asset = self.resolve_asset(asset_name);
        i128_to_f64(
            self.position_balance_raw(account_id, &asset, AccountPositionType::Borrow),
            decimals,
        )
    }

    fn position_balance_raw(
        &self,
        account_id: u64,
        asset: &soroban_sdk::Address,
        position_type: AccountPositionType,
    ) -> i128 {
        let (supplies, borrows) = self.ctrl_client().get_account_positions(&account_id);
        // Supply and debt maps hold different value types; extract the
        // scaled share each carries.
        let scaled_ray = match position_type {
            AccountPositionType::Deposit => supplies
                .get(hub_asset(asset.clone()))
                .map(|p| p.scaled_amount),
            AccountPositionType::Borrow => borrows
                .get(hub_asset(asset.clone()))
                .map(|p| p.scaled_amount),
        };

        if let Some(scaled_amount) = scaled_ray {
            let pool = self.resolve_market_by_asset(asset).pool.clone();
            let sync = pool::LiquidityPoolClient::new(&self.env, &pool)
                .get_sync_data(&hub_asset(asset.clone()));
            let index = match position_type {
                AccountPositionType::Deposit => sync.state.supply_index,
                AccountPositionType::Borrow => sync.state.borrow_index,
            };
            let decimals = self.resolve_market_by_asset(asset).decimals;
            return Ray::from(scaled_amount)
                .mul(&self.env, Ray::from(index))
                .to_asset(decimals);
        }

        0
    }

    pub fn total_collateral(&self, user: &str) -> f64 {
        wad_to_f64(self.total_collateral_raw(user))
    }

    pub fn total_collateral_raw(&self, user: &str) -> i128 {
        self.find_account_id(user)
            .map(|account_id| self.ctrl_client().get_total_collateral_usd(&account_id))
            .unwrap_or(0)
    }

    pub fn total_debt(&self, user: &str) -> f64 {
        wad_to_f64(self.total_debt_raw(user))
    }

    pub fn total_debt_raw(&self, user: &str) -> i128 {
        self.find_account_id(user)
            .map(|account_id| self.ctrl_client().get_total_borrow_usd(&account_id))
            .unwrap_or(0)
    }

    pub fn pool_utilization(&self, asset_name: &str) -> f64 {
        let asset = self.resolve_asset(asset_name);
        let raw = self
            .pool_client(asset_name)
            .get_utilisation(&hub_asset(asset));
        raw as f64 / RAY as f64
    }

    pub fn pool_reserves(&self, asset_name: &str) -> f64 {
        let decimals = self.resolve_market(asset_name).decimals;
        let asset = self.resolve_asset(asset_name);
        let raw = self.pool_client(asset_name).get_reserves(&hub_asset(asset));
        i128_to_f64(raw, decimals)
    }

    pub fn pool_borrow_rate(&self, asset_name: &str) -> f64 {
        let asset = self.resolve_asset(asset_name);
        let raw = self
            .pool_client(asset_name)
            .get_borrow_rate(&hub_asset(asset));
        raw as f64 / RAY as f64
    }

    pub fn pool_supply_rate(&self, asset_name: &str) -> f64 {
        let asset = self.resolve_asset(asset_name);
        let raw = self
            .pool_client(asset_name)
            .get_deposit_rate(&hub_asset(asset));
        raw as f64 / RAY as f64
    }

    pub fn snapshot_revenue(&self, asset_name: &str) -> i128 {
        let asset = self.resolve_asset(asset_name);
        self.pool_client(asset_name).get_revenue(&hub_asset(asset))
    }

    pub fn can_be_liquidated(&self, user: &str) -> bool {
        self.find_account_id(user)
            .map(|account_id| self.ctrl_client().is_liquidatable(&account_id))
            .unwrap_or(false)
    }

    pub fn can_be_liquidated_by_id(&self, account_id: u64) -> bool {
        self.ctrl_client().is_liquidatable(&account_id)
    }

    pub fn get_account_attributes(&self, user: &str) -> controller::types::AccountAttributes {
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

    pub fn get_asset_config(&self, asset_name: &str) -> AssetConfigView {
        let asset = self.resolve_asset(asset_name);
        let spoke = self
            .ctrl_client()
            .get_spoke_asset(&HARNESS_SPOKE, &hub_asset(asset.clone()));
        let params = self
            .pool_client(asset_name)
            .get_sync_data(&hub_asset(asset))
            .params;
        AssetConfigView {
            loan_to_value: spoke.loan_to_value,
            liquidation_threshold: spoke.liquidation_threshold,
            liquidation_bonus: spoke.liquidation_bonus,
            liquidation_fees: spoke.liquidation_fees,
            is_collateralizable: spoke.is_collateralizable,
            is_borrowable: spoke.is_borrowable,
            is_flashloanable: params.is_flashloanable,
            flashloan_fee: params.flashloan_fee,
            asset_decimals: params.asset_decimals,
        }
    }

    pub fn get_pool_address(&self, _asset_name: &str) -> soroban_sdk::Address {
        self.ctrl_client().get_pool_address()
    }

    /// True when the price-aggregator holds a token-rooted oracle for `asset`
    /// (absence = pending/disabled).
    pub fn market_is_active(&self, asset: &soroban_sdk::Address) -> bool {
        self.price_agg_client().oracle_config(asset).is_some()
    }

    pub fn market_oracle_config(
        &self,
        asset: &soroban_sdk::Address,
    ) -> controller::types::AssetOracleConfig {
        self.price_agg_client()
            .oracle_config(asset)
            .expect("market oracle config must exist")
    }

    pub fn get_position_limits(&self) -> controller::types::PositionLimits {
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
                .get::<_, controller::types::AccountMeta>(&ControllerKey::AccountMeta(account_id))
                .expect("account must exist")
                .owner
        })
    }
}
