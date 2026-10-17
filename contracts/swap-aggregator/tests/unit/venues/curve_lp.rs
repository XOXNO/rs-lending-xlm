use soroban_sdk::{
    contract, contractimpl,
    testutils::{Events as _, MockAuth, MockAuthInvoke},
    token,
    xdr::{ContractEventBody, ToXdr},
    Address, Bytes, Env, IntoVal, Map, MuxedAddress, String, Symbol, TryFromVal, Val, Vec,
};

use super::curve::{pool_abi, Factory, FactoryClient, Fixture, THREE_COIN, TWO_COIN};
use crate::{
    errors::Error,
    program::encode::{self, RawOp},
    types::StrategyPayload,
    RouterClient,
};

const INPUT: i128 = 10_000_000;
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

fn assets(f: &Fixture) -> Vec<Address> {
    let mut result = Vec::from_array(&f.env, [f.pool.clone()]);
    for coin in f.tokens.iter() {
        result.push_back(coin);
    }
    result
}

fn payload(f: &Fixture, token_in: u8, token_out: u8, floor: i128) -> Bytes {
    StrategyPayload {
        assets: assets(f),
        amounts: Vec::from_array(&f.env, [floor]),
        ops: encode::program(
            &f.env,
            token_in,
            token_out,
            0,
            0,
            &[RawOp {
                opcode: 7,
                mode: encode::ALL,
                idx_a: 0,
                idx_b: token_in,
                idx_c: token_out,
            }],
            &[],
        ),
    }
    .to_xdr(&f.env)
}

fn execute(
    f: &Fixture,
    input: &Address,
    amount: i128,
    xdr: &Bytes,
) -> Result<i128, soroban_sdk::Error> {
    match RouterClient::new(&f.env, &f.router)
        .mock_auths(&[MockAuth {
            address: &f.user,
            invoke: &MockAuthInvoke {
                contract: &f.router,
                fn_name: "execute_strategy",
                args: (&f.user, amount, xdr).into_val(&f.env),
                sub_invokes: &[MockAuthInvoke {
                    contract: input,
                    fn_name: "transfer",
                    args: (&f.user, &f.router, amount).into_val(&f.env),
                    sub_invokes: &[],
                }],
            },
        }])
        .try_execute_strategy(&f.user, &amount, xdr)
    {
        Ok(result) => Ok(result.unwrap()),
        Err(error) => Err(error.unwrap()),
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
    for address in assets(f).iter() {
        transfer_to_router(f, &address, RESERVE);
    }
}

fn state(f: &Fixture) -> (alloc::vec::Vec<i128>, pool_abi::PoolSnapshot) {
    let balances = assets(f)
        .iter()
        .flat_map(|address| {
            [&f.user, &f.router, &f.pool]
                .map(|owner| token::Client::new(&f.env, &address).balance(owner))
        })
        .collect();
    (
        balances,
        pool_abi::Client::new(&f.env, &f.pool).pool_snapshot(),
    )
}

fn assert_reserves(f: &Fixture) {
    for address in assets(f).iter() {
        assert_eq!(
            token::Client::new(&f.env, &address).balance(&f.router),
            RESERVE
        );
    }
}

fn assert_event_post_state(f: &Fixture, name: &str) {
    let events = f.env.events().all().filter_by_contract(&f.pool);
    let body = events
        .events()
        .iter()
        .rev()
        .find_map(|event| {
            let ContractEventBody::V0(body) = &event.body;
            let topic = Val::try_from_val(&f.env, body.topics.first()?).ok()?;
            (Symbol::try_from_val(&f.env, &topic).ok() == Some(Symbol::new(&f.env, name)))
                .then_some(body)
        })
        .expect("pool operation must emit its accounting post-state");
    let value = Val::try_from_val(&f.env, &body.data).unwrap();
    let data = Map::<Symbol, Val>::try_from_val(&f.env, &value).unwrap();
    let post = data.get(Symbol::new(&f.env, "post_state")).unwrap();
    let post = pool_abi::PoolAccountingState::try_from_val(&f.env, &post).unwrap();
    let snapshot = pool_abi::Client::new(&f.env, &f.pool).pool_snapshot();
    assert_eq!(post.lp_supply, snapshot.lp_supply);
    assert_eq!(post.active_balances, snapshot.active_balances);
    assert_eq!(post.admin_balances, snapshot.admin_balances);
    if name == "remove_liquidity_one" {
        let receiver = data.get(Symbol::new(&f.env, "receiver")).unwrap();
        assert_eq!(Address::try_from_val(&f.env, &receiver).unwrap(), f.router);
    }
}

#[test]
fn curve_lp_real_one_sided_mint_each_coin_exact_auth_floor_rollback_and_reserve_isolation() {
    for (wasm, count) in [(TWO_COIN, 2), (THREE_COIN, 3)] {
        for coin in 0..count {
            let reference = Fixture::new(wasm, count);
            let input = reference.tokens.get(coin).unwrap();
            let shares = execute(
                &reference,
                &input,
                INPUT,
                &payload(&reference, (coin + 1) as u8, 0, 1),
            )
            .unwrap();
            assert!(shares > 0);
            let f = Fixture::new(wasm, count);
            seed_reserves(&f);
            let input = f.tokens.get(coin).unwrap();
            let before = state(&f);
            assert_eq!(
                execute(
                    &f,
                    &input,
                    INPUT,
                    &payload(&f, (coin + 1) as u8, 0, shares + 1)
                ),
                Err(Error::SlippageExceeded.into())
            );
            assert_eq!(
                state(&f),
                before,
                "mint floor failure must roll back LP, coins, supply and fees"
            );
            assert_eq!(
                execute(&f, &input, INPUT, &payload(&f, (coin + 1) as u8, 0, shares)),
                Ok(shares)
            );
            assert_event_post_state(&f, "add_liquidity");
            let after = state(&f);
            assert_eq!(after.0[0], before.0[0] + shares);
            assert_eq!(after.1.lp_supply, before.1.lp_supply + shares);
            assert_eq!(
                after.0[(coin as usize + 1) * 3],
                before.0[(coin as usize + 1) * 3] - INPUT
            );
            assert_reserves(&f);
        }
    }
}

#[test]
fn curve_lp_real_one_coin_burn_each_coin_without_external_burn_auth_and_floor_rollback() {
    for (wasm, count) in [(TWO_COIN, 2), (THREE_COIN, 3)] {
        for output in 0..count {
            let f = Fixture::new(wasm, count);
            seed_reserves(&f);
            let before = state(&f);
            let quote =
                pool_abi::Client::new(&f.env, &f.pool).calc_withdraw_one_coin(&LP_INPUT, &output);
            assert!(quote > 0);
            assert_eq!(
                execute(
                    &f,
                    &f.pool,
                    LP_INPUT,
                    &payload(&f, 0, (output + 1) as u8, quote + 1)
                ),
                Err(Error::SlippageExceeded.into())
            );
            assert_eq!(
                state(&f),
                before,
                "burn floor failure must restore LP, coins, active/admin balances and supply"
            );
            assert_eq!(
                execute(
                    &f,
                    &f.pool,
                    LP_INPUT,
                    &payload(&f, 0, (output + 1) as u8, quote)
                ),
                Ok(quote)
            );
            assert_event_post_state(&f, "remove_liquidity_one");
            let after = state(&f);
            assert_eq!(after.0[0], before.0[0] - LP_INPUT);
            assert_eq!(after.1.lp_supply, before.1.lp_supply - LP_INPUT);
            assert_eq!(
                after.0[(output as usize + 1) * 3],
                before.0[(output as usize + 1) * 3] + quote
            );
            assert_reserves(&f);
        }
    }
}

fn add_two_coin_pool(f: &Fixture) -> Address {
    let env = &f.env;
    let factory = env.register(Factory, (&f.user,));
    let coins = Vec::from_array(env, [f.tokens.get(0).unwrap(), f.tokens.get(1).unwrap()]);
    let args = pool_abi::PoolConstructorArgs {
        admin_fee: 5_000_000_000,
        amplification: 100,
        asset_types: Vec::from_array(env, [0, 0]),
        factory: factory.clone(),
        fee: 30_000_000,
        methods: Vec::new(env),
        min_locked_liquidity: 1_000,
        name: String::from_str(env, "Two"),
        offpeg_fee_multiplier: 20_000_000_000,
        oracles: Vec::new(env),
        symbol: String::from_str(env, "TWO"),
        tokens: coins.clone(),
    };
    let pool = FactoryClient::new(env, &factory)
        .new_pool(&env.deployer().upload_contract_wasm(TWO_COIN), &args);
    let seed = 500_000_000_i128;
    let amounts = Vec::from_array(env, [seed, seed]);
    let receiver = MuxedAddress::from(&f.user);
    let addresses: alloc::vec::Vec<_> = coins.iter().collect();
    let transfers: alloc::vec::Vec<_> = addresses
        .iter()
        .map(|coin| MockAuthInvoke {
            contract: coin,
            fn_name: "transfer",
            args: (&f.user, &pool, seed).into_val(env),
            sub_invokes: &[],
        })
        .collect();
    pool_abi::Client::new(env, &pool)
        .mock_auths(&[MockAuth {
            address: &f.user,
            invoke: &MockAuthInvoke {
                contract: &pool,
                fn_name: "add_liquidity",
                args: (&f.user, &amounts, 0_i128, &receiver).into_val(env),
                sub_invokes: &transfers,
            },
        }])
        .add_liquidity(&f.user, &amounts, &0, &receiver);
    pool
}

fn lp_to_lp_payload(f: &Fixture, input: &Address, output: &Address, floor: i128) -> Bytes {
    StrategyPayload {
        assets: Vec::from_array(
            &f.env,
            [input.clone(), f.tokens.get(0).unwrap(), output.clone()],
        ),
        amounts: Vec::from_array(&f.env, [floor]),
        ops: encode::program(
            &f.env,
            0,
            2,
            0,
            0,
            &[
                RawOp {
                    opcode: 7,
                    mode: encode::ALL,
                    idx_a: 0,
                    idx_b: 0,
                    idx_c: 1,
                },
                RawOp {
                    opcode: 7,
                    mode: encode::PREV,
                    idx_a: 2,
                    idx_b: 1,
                    idx_c: 2,
                },
            ],
            &[],
        ),
    }
    .to_xdr(&f.env)
}

fn other_state(f: &Fixture, two: &Address) -> (pool_abi::PoolSnapshot, alloc::vec::Vec<i128>) {
    let pool = pool_abi::Client::new(&f.env, two);
    let mut balances = alloc::vec![pool.balance(&f.user), pool.balance(&f.router)];
    for coin in f.tokens.iter() {
        balances.push(token::Client::new(&f.env, &coin).balance(two));
    }
    (pool.pool_snapshot(), balances)
}

#[test]
fn curve_lp_real_two_and_three_coin_lp_routes_exact_auth_floor_rollback_and_reserve_isolation() {
    for reverse in [false, true] {
        let reference = Fixture::new(THREE_COIN, 3);
        let reference_two = add_two_coin_pool(&reference);
        let (input, output) = if reverse {
            (&reference.pool, &reference_two)
        } else {
            (&reference_two, &reference.pool)
        };
        let expected = execute(
            &reference,
            input,
            LP_INPUT,
            &lp_to_lp_payload(&reference, input, output, 1),
        )
        .unwrap();
        assert!(expected > 0);
        let f = Fixture::new(THREE_COIN, 3);
        let two = add_two_coin_pool(&f);
        seed_reserves(&f);
        transfer_to_router(&f, &two, RESERVE);
        let (input, output) = if reverse {
            (&f.pool, &two)
        } else {
            (&two, &f.pool)
        };
        let before = state(&f);
        let other_before = other_state(&f, &two);
        let source = pool_abi::Client::new(&f.env, input);
        let destination = pool_abi::Client::new(&f.env, output);
        let source_before = (source.total_supply(), source.balance(&f.user));
        let destination_before = (destination.total_supply(), destination.balance(&f.user));
        assert_eq!(
            execute(
                &f,
                input,
                LP_INPUT,
                &lp_to_lp_payload(&f, input, output, expected + 1)
            ),
            Err(Error::SlippageExceeded.into())
        );
        assert_eq!(state(&f), before);
        assert_eq!(other_state(&f, &two), other_before);
        assert_eq!(
            execute(
                &f,
                input,
                LP_INPUT,
                &lp_to_lp_payload(&f, input, output, expected)
            ),
            Ok(expected)
        );
        assert_eq!(source.total_supply(), source_before.0 - LP_INPUT);
        assert_eq!(source.balance(&f.user), source_before.1 - LP_INPUT);
        assert_eq!(destination.total_supply(), destination_before.0 + expected);
        assert_eq!(
            destination.balance(&f.user),
            destination_before.1 + expected
        );
        assert_eq!(token::Client::new(&f.env, &two).balance(&f.router), RESERVE);
        assert_reserves(&f);
    }
}

#[test]
fn curve_lp_real_rejects_other_pool_lp_and_absent_constituent_with_complete_rollback() {
    let f = Fixture::new(THREE_COIN, 3);
    let two = add_two_coin_pool(&f);
    let before = state(&f);
    let other_before = other_state(&f, &two);
    for other in [f.pool.clone(), f.tokens.get(2).unwrap()] {
        for (input, output) in [(0, 1), (1, 0)] {
            let xdr = StrategyPayload {
                assets: Vec::from_array(&f.env, [two.clone(), other.clone()]),
                amounts: Vec::from_array(&f.env, [1]),
                ops: encode::program(
                    &f.env,
                    input,
                    output,
                    0,
                    0,
                    &[RawOp {
                        opcode: 7,
                        mode: encode::ALL,
                        idx_a: 0,
                        idx_b: input,
                        idx_c: output,
                    }],
                    &[],
                ),
            }
            .to_xdr(&f.env);
            let (token, amount) = if input == 0 {
                (&two, LP_INPUT)
            } else if other == f.pool {
                (&other, LP_INPUT)
            } else {
                (&other, INPUT)
            };
            assert_eq!(
                execute(&f, token, amount, &xdr),
                Err(Error::BrokenTokenChain.into())
            );
            assert_eq!(state(&f), before);
            assert_eq!(other_state(&f, &two), other_before);
        }
    }
}

#[test]
fn curve_lp_real_mint_without_nested_transfer_authorization_fails_and_rolls_back() {
    let mut f = Fixture::new(TWO_COIN, 2);
    f.router = f.env.register(UnauthenticatedMint, ());
    let input = f.tokens.get(0).unwrap();
    transfer_to_router(&f, &input, INPUT);
    let before = state(&f);
    let error = UnauthenticatedMintClient::new(&f.env, &f.router)
        .try_mint(&f.pool, &INPUT)
        .unwrap_err()
        .unwrap();
    assert_eq!(error, soroban_sdk::Error::from_contract_error(10400));
    assert_eq!(state(&f), before);
}
