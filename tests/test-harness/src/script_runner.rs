use script_runner::{Op, ScriptRunner, ScriptRunnerClient};
use soroban_sdk::{token, Address, Vec};

use crate::context::LendingTest;

impl LendingTest {
    pub fn deploy_script_runner(&self) -> Address {
        self.env.register(ScriptRunner, ())
    }

    pub fn fund_runner(&self, runner: &Address, asset_name: &str, amount: i128) {
        self.resolve_market(asset_name)
            .token_admin
            .mint(runner, &amount);
    }

    pub fn runner_wallet(&self, runner: &Address, asset_name: &str) -> i128 {
        token::Client::new(&self.env, &self.resolve_asset(asset_name)).balance(runner)
    }

    /// Runs the script from the runner's frame. `Err` carries the contract
    /// error of the failing op; a host error (auth, re-entry) surfaces as a
    /// panic with its text.
    pub fn run_script(&self, runner: &Address, ops: &Vec<Op>) -> Result<u64, soroban_sdk::Error> {
        match ScriptRunnerClient::new(&self.env, runner).try_run(
            &self.controller,
            &self.position_nft,
            ops,
        ) {
            Ok(Ok(id)) => Ok(id),
            Ok(Err(_)) => panic!("script runner returned an unconvertible value"),
            Err(e) => Err(e.expect("expected a contract error, got an InvokeError")),
        }
    }
}
