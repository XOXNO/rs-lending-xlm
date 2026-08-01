use crate::context::LendingTest;
use crate::helpers::hub_asset;

impl LendingTest {
    pub fn claim_revenue(&self, asset_name: &str) -> i128 {
        let asset = self.resolve_asset(asset_name);
        let assets = soroban_sdk::vec![&self.env, hub_asset(asset)];
        self.ctrl_client()
            .claim_revenue(&self.admin, &assets)
            .get(0)
            .unwrap()
    }

    pub fn try_claim_revenue(&self, asset_name: &str) -> Result<i128, soroban_sdk::Error> {
        let asset = self.resolve_asset(asset_name);
        let assets = soroban_sdk::vec![&self.env, hub_asset(asset)];
        match self.ctrl_client().try_claim_revenue(&self.admin, &assets) {
            Ok(Ok(amounts)) => Ok(amounts.get(0).unwrap()),
            Ok(Err(err)) => Err(err.into()),
            Err(e) => Err(e.expect("expected contract error, got InvokeError")),
        }
    }

}
