use soroban_sdk::{
    contract, contractimpl,
    testutils::{MockAuth, MockAuthInvoke},
    token, Address, Env, Error as HostError, IntoVal, MuxedAddress, Vec,
};

use super::curve::{curve_hop, deploy_pool, pool_abi, Fixture, INPUT, LIQUIDITY};
use crate::errors::Error;

const LP_INPUT: i128 = 1_000_000_000_000_000_000;
const RESERVE: i128 = 1_234;

#[contract]
struct UnauthenticatedMint;

#[contractimpl]
impl UnauthenticatedMint {
    pub fn mint(env: Env, pool: Address, amount: i128) -> i128 {
        let caller = env.current_contract_address();
        pool_abi::Client::new(&env, &pool).add_liquidity(
            &caller,
            &Vec::from_array(&env, [amount, 0]),
            &0,
            MuxedAddress::from(&caller),
        )
    }
}

fn transfer_to_router(f: &Fixture, token: &Address, amount: i128) {
    token::Client::new(&f.env, token)
        .mock_auths(&[MockAuth {
            address: &f.user,
            invoke: &MockAuthInvoke {
                contract: token,
                fn_name: "transfer",
                args: (&f.user, &f.router, amount).into_val(&f.env),
                sub_invokes: &[],
            },
        }])
        .transfer(&f.user, &f.router, &amount);
}

fn seed_reserves(f: &Fixture) {
    for address in f.assets().iter() {
        transfer_to_router(f, &address, RESERVE);
    }
}

fn assert_reserves(f: &Fixture) {
    for address in f.assets().iter() {
        assert_eq!(
            token::Client::new(&f.env, &address).balance(&f.router),
            RESERVE
        );
    }
}

/// Mints LP from each coin, then burns LP to each coin. An `i128::MAX` floor must fail and
/// roll back everything; a floor of one must credit exactly the measured deltas and leave the
/// router's reserves untouched. The burn passes with no auth beyond the user's input transfer.
#[test]
fn curve_lp_real_one_sided_mint_and_one_coin_burn_each_coin_floor_rollback_and_reserve_isolation() {
    for count in [2, 3] {
        let f = Fixture::new(count);
        seed_reserves(&f);
        let assets = f.assets();
        for coin in 1..=count {
            for (input, output, amount) in [(coin, 0, INPUT), (0, coin, LP_INPUT)] {
                let token_in = assets.get(input).unwrap();
                let token_out = assets.get(output).unwrap();
                let before = f.state();
                assert_eq!(
                    f.swap(&token_in, &token_out, amount, i128::MAX),
                    Err(Error::SlippageExceeded.into())
                );
                assert_eq!(
                    f.state(),
                    before,
                    "floor failure must roll back LP, coins, active/admin balances and supply"
                );
                let received = f.swap(&token_in, &token_out, amount, 1).unwrap();
                let after = f.state();
                let (i, o) = (input as usize * 3, output as usize * 3);
                assert_eq!(after.0[i], before.0[i] - amount);
                assert_eq!(after.0[o], before.0[o] + received);
                let supply_change = if output == 0 { received } else { -amount };
                assert_eq!(after.1.lp_supply, before.1.lp_supply + supply_change);
                assert_reserves(&f);
            }
        }
    }
}

fn other_state(f: &Fixture, two: &Address) -> (pool_abi::PoolSnapshot, alloc::vec::Vec<i128>) {
    let pool = pool_abi::Client::new(&f.env, two);
    let mut balances = alloc::vec![pool.balance(&f.user), pool.balance(&f.router)];
    for coin in f.tokens.iter() {
        balances.push(token::Client::new(&f.env, &coin).balance(two));
    }
    (pool.pool_snapshot(), balances)
}

fn add_two_coin_pool(f: &Fixture) -> Address {
    let coins = f.tokens.slice(0..2);
    deploy_pool(&f.env, &f.user, &coins, LIQUIDITY / 2)
}

#[test]
fn curve_lp_real_two_and_three_coin_lp_routes_floor_rollback_and_reserve_isolation() {
    let f = Fixture::new(3);
    let two = add_two_coin_pool(&f);
    let coin = f.tokens.get(0).unwrap();
    seed_reserves(&f);
    transfer_to_router(&f, &two, RESERVE);
    for (input, output) in [(&two, &f.pool), (&f.pool, &two)] {
        let xdr = |floor| {
            let hops = alloc::vec![
                curve_hop(input, input, &coin),
                curve_hop(output, &coin, output)
            ];
            f.payload(hops, floor)
        };
        let before = (f.state(), other_state(&f, &two));
        let source = pool_abi::Client::new(&f.env, input);
        let destination = pool_abi::Client::new(&f.env, output);
        let source_before = (source.total_supply(), source.balance(&f.user));
        let destination_before = (destination.total_supply(), destination.balance(&f.user));
        assert_eq!(
            f.execute(input, LP_INPUT, &xdr(i128::MAX)),
            Err(Error::SlippageExceeded.into())
        );
        assert_eq!((f.state(), other_state(&f, &two)), before);
        let received = f.execute(input, LP_INPUT, &xdr(1)).unwrap();
        assert_eq!(source.total_supply(), source_before.0 - LP_INPUT);
        assert_eq!(source.balance(&f.user), source_before.1 - LP_INPUT);
        assert_eq!(destination.total_supply(), destination_before.0 + received);
        assert_eq!(
            destination.balance(&f.user),
            destination_before.1 + received
        );
        assert_eq!(token::Client::new(&f.env, &two).balance(&f.router), RESERVE);
        assert_reserves(&f);
    }
}

#[test]
fn curve_lp_real_rejects_other_pool_lp_and_absent_constituent_with_complete_rollback() {
    let f = Fixture::new(3);
    let two = add_two_coin_pool(&f);
    let before = f.state();
    let other_before = other_state(&f, &two);
    for other in [f.pool.clone(), f.tokens.get(2).unwrap()] {
        let amount = if other == f.pool { LP_INPUT } else { INPUT };
        for (token_in, token_out, amount) in [(&two, &other, LP_INPUT), (&other, &two, amount)] {
            let xdr = f.payload(alloc::vec![curve_hop(&two, token_in, token_out)], 1);
            assert_eq!(
                f.execute(token_in, amount, &xdr),
                Err(Error::BrokenTokenChain.into())
            );
            assert_eq!(f.state(), before);
            assert_eq!(other_state(&f, &two), other_before);
        }
    }
}

#[test]
fn curve_lp_real_mint_without_nested_transfer_authorization_fails_and_rolls_back() {
    let mut f = Fixture::new(2);
    f.router = f.env.register(UnauthenticatedMint, ());
    let input = f.tokens.get(0).unwrap();
    transfer_to_router(&f, &input, INPUT);
    let before = f.state();
    let error = UnauthenticatedMintClient::new(&f.env, &f.router)
        .try_mint(&f.pool, &INPUT)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, HostError::from_contract_error(10400));
    assert_eq!(f.state(), before);
}
