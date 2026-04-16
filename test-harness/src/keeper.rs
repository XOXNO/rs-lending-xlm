use soroban_sdk::Vec;

use crate::context::LendingTest;

impl LendingTest {
    // -----------------------------------------------------------------------
    // Index management
    // -----------------------------------------------------------------------

    /// Update indexes for specific markets (uses internal keeper).
    pub fn update_indexes_for(&self, assets: &[&str]) {
        let mut addrs = Vec::new(&self.env);
        for name in assets {
            addrs.push_back(self.resolve_asset(name));
        }
        self.ctrl_client().update_indexes(&self.keeper, &addrs);
    }

    // -----------------------------------------------------------------------
    // Bad debt cleanup
    // -----------------------------------------------------------------------

    /// Clean bad debt for a specific account (by user name).
    pub fn clean_bad_debt_for(&self, target_user: &str) {
        let account_id = self.resolve_account_id(target_user);
        self.ctrl_client().clean_bad_debt(&self.keeper, &account_id);
    }

    /// Clean bad debt for a specific account ID directly.
    pub fn clean_bad_debt_by_id(&self, account_id: u64) {
        self.ctrl_client().clean_bad_debt(&self.keeper, &account_id);
    }

    /// Try clean bad debt -- returns Result.
    pub fn try_clean_bad_debt_by_id(&self, account_id: u64) -> Result<(), soroban_sdk::Error> {
        match self
            .ctrl_client()
            .try_clean_bad_debt(&self.keeper, &account_id)
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

    // -----------------------------------------------------------------------
    // Account threshold propagation
    // -----------------------------------------------------------------------

    /// Update account thresholds for a set of accounts on a given asset.
    pub fn update_account_threshold(&self, asset_name: &str, has_risks: bool, account_ids: &[u64]) {
        let asset = self.resolve_asset(asset_name);
        let mut ids = Vec::new(&self.env);
        for id in account_ids {
            ids.push_back(*id);
        }
        self.ctrl_client()
            .update_account_threshold(&self.keeper, &asset, &has_risks, &ids);
    }

    /// Try update account threshold -- returns Result.
    pub fn try_update_account_threshold(
        &self,
        asset_name: &str,
        has_risks: bool,
        account_ids: &[u64],
    ) -> Result<(), soroban_sdk::Error> {
        let asset = self.resolve_asset(asset_name);
        let mut ids = Vec::new(&self.env);
        for id in account_ids {
            ids.push_back(*id);
        }
        match self.ctrl_client().try_update_account_threshold(
            &self.keeper,
            &asset,
            &has_risks,
            &ids,
        ) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }
}
