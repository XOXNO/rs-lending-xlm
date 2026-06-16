use crate::config::config;
use crate::strategy_helpers::flash_guard_cleared;
use controller::types::PositionMode;
use proptest::prelude::*;
use soroban_sdk::{contract, contractimpl, token, xdr::FromXdr, Address, Bytes, Env};
use test_harness::{build_aggregator_swap, LendingTest, MockSwapPayload, ALICE};

#[contract]
pub struct ShortAggregator;

#[contractimpl]
impl ShortAggregator {
    pub fn __constructor(_env: Env, _admin: Address) {}

    pub fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128 {
        sender.require_auth();
        let router = env.current_contract_address();
        let payload = MockSwapPayload::from_xdr(&env, &swap_xdr).expect("mock payload must decode");
        let in_client = token::Client::new(&env, &payload.token_in);
        in_client.transfer(&sender, &router, &total_in);
        0
    }
}

proptest! {
    #![proptest_config(config(8))]

    #[test]
    fn prop_short_aggregator_rejected(debt_units in 1u32..5u32) {
        let mut t = LendingTest::new().standard_two_asset().build();
        let admin = t.admin.clone();
        let short = t.env.register(ShortAggregator, (admin,));
        t.ctrl_client().set_aggregator(&short);

        let eth_amount = debt_units as f64;
        let usdc_amount = eth_amount * 2_000.0;
        let usdc_decimals = t.resolve_market("USDC").decimals;
        let eth_decimals = t.resolve_market("ETH").decimals;
        let min_out_raw = (usdc_amount as i128) * 10i128.pow(usdc_decimals);
        let amount_in_raw = (eth_amount as i128) * 10i128.pow(eth_decimals);

        let usdc_addr = t.resolve_asset("USDC");
        let usdc_admin = soroban_sdk::token::StellarAssetClient::new(&t.env, &usdc_addr);
        usdc_admin.mint(&short, &(min_out_raw * 2));

        let steps = build_aggregator_swap(&t, "ETH", "USDC", amount_in_raw, min_out_raw);
        let result = t.try_multiply(ALICE, "USDC", eth_amount, "ETH", PositionMode::Multiply, &steps);

        prop_assert!(result.is_err(), "zero-output aggregator must be rejected");
        prop_assert!(flash_guard_cleared(&t));
    }
}
