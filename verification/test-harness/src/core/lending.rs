use common::errors::GenericError;
use soroban_sdk::testutils::Address as _;

use crate::core::types::{LendingTest, MarketState};

impl LendingTest {
    pub fn get_or_create_user(&mut self, name: &str) -> soroban_sdk::Address {
        if let Some(user) = self.users.get(name) {
            return user.address.clone();
        }
        let address = soroban_sdk::Address::generate(&self.env);
        self.users.insert(
            name.to_string(),
            crate::core::types::UserState {
                address: address.clone(),
                default_account_id: None,
                accounts: Vec::new(),
            },
        );
        address
    }

    pub fn find_account_id(&self, name: &str) -> Option<u64> {
        let user = self.users.get(name)?;
        user.default_account_id
            .filter(|id| self.account_exists(*id))
            .or_else(|| {
                user.accounts.iter().find_map(|account| {
                    self.account_exists(account.account_id)
                        .then_some(account.account_id)
                })
            })
    }

    pub fn resolve_account_id(&self, name: &str) -> u64 {
        match self.find_account_id(name) {
            Some(id) => id,
            None => panic!(
                "'{}' has no account -- call supply() or create_account() first",
                name
            ),
        }
    }

    pub fn try_resolve_account_id(&self, name: &str) -> Result<u64, soroban_sdk::Error> {
        self.find_account_id(name).ok_or_else(|| {
            soroban_sdk::Error::from_contract_error(GenericError::AccountNotInMarket as u32)
        })
    }

    pub fn resolve_market(&self, asset_name: &str) -> &MarketState {
        self.markets.get(asset_name).unwrap_or_else(|| {
            panic!(
                "market '{}' not found -- add it with .with_market()",
                asset_name
            )
        })
    }

    pub fn resolve_market_by_asset(&self, asset: &soroban_sdk::Address) -> &MarketState {
        self.markets
            .values()
            .find(|market| market.asset == *asset)
            .unwrap_or_else(|| panic!("market for asset '{:?}' not found", asset))
    }

    pub fn resolve_asset(&self, asset_name: &str) -> soroban_sdk::Address {
        self.resolve_market(asset_name).asset.clone()
    }

    pub fn ctrl_client(&self) -> controller::ControllerClient<'_> {
        controller::ControllerClient::new(&self.env, &self.controller)
    }

    pub fn mock_reflector_client(&self) -> crate::mock_reflector::MockReflectorClient<'_> {
        crate::mock_reflector::MockReflectorClient::new(&self.env, &self.mock_reflector)
    }

    pub fn account_exists(&self, account_id: u64) -> bool {
        use common::types::ControllerKey;

        self.env.as_contract(&self.controller, || {
            self.env
                .storage()
                .persistent()
                .has(&ControllerKey::AccountMeta(account_id))
        })
    }

    pub fn default_account_id_or_zero(&self, user: &str) -> u64 {
        self.find_account_id(user).unwrap_or(0)
    }

    #[allow(dead_code)]
    pub fn pool_client(&self, asset_name: &str) -> pool::LiquidityPoolClient<'_> {
        let market = self.resolve_market(asset_name);
        pool::LiquidityPoolClient::new(&self.env, &market.pool)
    }
}
