use soroban_sdk::token;
use test_harness::LendingTest;

pub fn router_allowance(t: &LendingTest, asset_name: &str) -> i128 {
    let asset = t.resolve_asset(asset_name);
    let tok = token::Client::new(&t.env, &asset);
    tok.allowance(&t.controller, &t.aggregator)
}

pub fn flash_guard_cleared(t: &LendingTest) -> bool {
    t.env.as_contract(&t.controller, || {
        !controller::test_support::is_flash_loan_ongoing(&t.env)
    })
}