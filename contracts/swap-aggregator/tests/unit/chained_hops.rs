//! `Mode::Prev` end to end: a hop sized from what the hop before it produced.
//!
//! `Program::validate` proves the chain structurally at decode time, but the
//! executor re-checks the token identity before it spends. Both halves need a
//! route that actually chains and settles, not just one that is rejected.

use crate::types::{SwapHop, SwapVenue};
use crate::{Router, RouterClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env};

use super::support::{aquarius_mock, new_asset, path, strategy_xdr};

#[test]
fn a_two_hop_route_sizes_the_second_hop_from_the_first_hops_output() {
    let env = Env::default();
    env.mock_all_auths();

    let router_addr = env.register(Router, (Address::generate(&env),));
    let sender = Address::generate(&env);
    let admin = Address::generate(&env);
    let (token_a, sac_a) = new_asset(&env, &admin);
    let (token_b, sac_b) = new_asset(&env, &admin);
    let (token_c, sac_c) = new_asset(&env, &admin);

    // Two 1:1 books: a -> b, then b -> c.
    let pool_ab = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool_ab).init(&token_a, &token_b);
    sac_b.mint(&pool_ab, &10_000);

    let pool_bc = env.register(aquarius_mock::AqPool, ());
    aquarius_mock::AqPoolClient::new(&env, &pool_bc).init(&token_b, &token_c);
    sac_c.mint(&pool_bc, &10_000);

    sac_a.mint(&sender, &1_000);

    let swap_xdr = strategy_xdr(
        &env,
        token_a.clone(),
        token_c.clone(),
        1_000,
        alloc::vec![path(
            alloc::vec![
                SwapHop {
                    venue: SwapVenue::Aquarius,
                    pool: pool_ab,
                    token_in: token_a.clone(),
                    token_out: token_b.clone(),
                },
                SwapHop {
                    venue: SwapVenue::Aquarius,
                    pool: pool_bc,
                    token_in: token_b.clone(),
                    token_out: token_c.clone(),
                },
            ],
            1_000_000,
        )],
    );

    let out = RouterClient::new(&env, &router_addr).execute_strategy(&sender, &1_000, &swap_xdr);

    assert_eq!(out, 1_000, "both 1:1 hops must fill in full");
    assert_eq!(token::Client::new(&env, &token_c).balance(&sender), 1_000);
    assert_eq!(token::Client::new(&env, &token_a).balance(&sender), 0);
    assert_eq!(
        token::Client::new(&env, &token_b).balance(&router_addr),
        0,
        "the intermediate token must not be left behind"
    );
}
