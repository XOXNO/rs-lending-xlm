use crate::context::LendingTest;
use crate::helpers::{f64_to_i128, hub_asset};

impl LendingTest {
    // Revenue claims

    /// Claim accrued protocol revenue for an asset on hub 0.
    /// Uses admin as the signed caller (permissionless; no REVENUE role).
    pub fn claim_revenue(&self, asset_name: &str) -> i128 {
        let asset = self.resolve_asset(asset_name);
        let assets = soroban_sdk::vec![&self.env, hub_asset(asset)];
        self.ctrl_client()
            .claim_revenue(&self.admin, &assets)
            .get(0)
            .unwrap()
    }

    /// Try claim revenue on hub 0 -- returns Result.
    pub fn try_claim_revenue(&self, asset_name: &str) -> Result<i128, soroban_sdk::Error> {
        let asset = self.resolve_asset(asset_name);
        let assets = soroban_sdk::vec![&self.env, hub_asset(asset)];
        match self.ctrl_client().try_claim_revenue(&self.admin, &assets) {
            Ok(Ok(amounts)) => Ok(amounts.get(0).unwrap()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }
    // External rewards

    /// Add external reward tokens to a hub-0 pool (increases supply index).
    /// Auto-mints tokens to admin before calling.
    pub fn add_rewards(&self, asset_name: &str, amount: f64) {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw = f64_to_i128(amount, decimals);
        let asset = self.resolve_asset(asset_name);
        let market = self.resolve_market(asset_name);

        // Mint tokens to admin so the transfer succeeds
        market.token_admin.mint(&self.admin, &raw);
        let rewards = soroban_sdk::vec![&self.env, (hub_asset(asset), raw)];
        self.ctrl_client().add_rewards(&self.admin, &rewards);
    }

    /// Add external reward tokens with raw i128 amount on hub 0.
    pub fn add_rewards_raw(&self, asset_name: &str, amount: i128) {
        let asset = self.resolve_asset(asset_name);
        let market = self.resolve_market(asset_name);

        market.token_admin.mint(&self.admin, &amount);
        let rewards = soroban_sdk::vec![&self.env, (hub_asset(asset), amount)];
        self.ctrl_client().add_rewards(&self.admin, &rewards);
    }

    /// Try add rewards on hub 0 -- returns Result.
    pub fn try_add_rewards(&self, asset_name: &str, amount: f64) -> Result<(), soroban_sdk::Error> {
        let decimals = self.resolve_market(asset_name).decimals;
        let raw = f64_to_i128(amount, decimals);
        let asset = self.resolve_asset(asset_name);
        let market = self.resolve_market(asset_name);

        market.token_admin.mint(&self.admin, &raw);
        let rewards = soroban_sdk::vec![&self.env, (hub_asset(asset), raw)];
        match self.ctrl_client().try_add_rewards(&self.admin, &rewards) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }
}
