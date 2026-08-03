use crate::types::SwapVenue;
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Env};

use super::super::support::{new_asset, one_hop_path, phoenix_mock, strategy_xdr};

#[test]
fn phoenix_single_hop_happy_path() {
    let env = Env::default();
    env.mock_all_auths();
    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let pool = env.register(phoenix_mock::PhoenixPool, ());
    phoenix_mock::PhoenixPoolClient::new(&env, &pool).init(&token_a, &token_b);
    sac_a.mint(&sender, &1_000);
    sac_b.mint(&pool, &1_000);
    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_b.clone(),
        400,
        vec![
            &env,
            one_hop_path(
                &env,
                SwapVenue::Phoenix,
                pool,
                token_a.clone(),
                token_b.clone(),
                1_000_000,
            ),
        ],
    );
    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &400, &swap_xdr);
    assert_eq!(out, 400);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 600);
    assert_eq!(token::Client::new(&env, &token_b).balance(&sender), 400);
}
