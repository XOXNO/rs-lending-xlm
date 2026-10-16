use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    token, vec, Address, Bytes, BytesN, Env, IntoVal, Map, MuxedAddress, String, Symbol, Val, Vec,
};

use crate::{
    errors::Error, program::MAX_ASSETS, types::SwapVenue, venues::auth_entry, Router, RouterClient,
};

use super::super::support::{new_asset, one_hop_path, strategy_xdr};

mod pool_abi {
    soroban_sdk::contractimport!(file = "tests/fixtures/curve/pool-2coin.wasm");
}

const TWO_COIN: &[u8] = include_bytes!("../../fixtures/curve/pool-2coin.wasm");
const THREE_COIN: &[u8] = include_bytes!("../../fixtures/curve/pool-3coin.wasm");
const LIQUIDITY: i128 = 1_000_000_000;
const INPUT: i128 = 10_000_000;

#[contract]
struct Factory;

#[contractimpl]
impl Factory {
    pub fn __constructor(env: Env, admin: Address) {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "admin"), &admin);
    }

    pub fn admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&Symbol::new(&env, "admin"))
            .unwrap()
    }

    pub fn fee_receiver(env: Env) -> MuxedAddress {
        Self::admin(env).into()
    }

    pub fn new_pool(env: Env, hash: BytesN<32>, args: pool_abi::PoolConstructorArgs) -> Address {
        let deployer = env
            .deployer()
            .with_current_contract(BytesN::from_array(&env, &[0; 32]));
        env.authorize_as_current_contract(vec![
            &env,
            auth_entry(
                &env,
                &deployer.deployed_address(),
                "__constructor",
                (args.clone(),).into_val(&env),
                vec![&env],
            ),
        ]);
        deployer.deploy_contract(soroban_sdk::ContractExecutable::Wasm(hash), (args,))
    }
}

struct Fixture {
    env: Env,
    user: Address,
    router: Address,
    pool: Address,
    tokens: Vec<Address>,
}

impl Fixture {
    fn new(wasm: &[u8], count: u32) -> Self {
        let env = Env::default();
        env.cost_estimate().budget().reset_unlimited();
        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let router = env.register(Router, (&admin,));
        let mut tokens = Vec::new(&env);
        for _ in 0..count {
            let (address, client) = new_asset(&env, &admin);
            client
                .mock_auths(&[MockAuth {
                    address: &admin,
                    invoke: &MockAuthInvoke {
                        contract: &address,
                        fn_name: "mint",
                        args: (&user, 2 * LIQUIDITY).into_val(&env),
                        sub_invokes: &[],
                    },
                }])
                .mint(&user, &(2 * LIQUIDITY));
            tokens.push_back(address);
        }
        let factory = env.register(Factory, (&admin,));
        let args = pool_abi::PoolConstructorArgs {
            admin_fee: 5_000_000_000,
            amplification: 100,
            asset_types: Vec::from_iter(&env, (0..count).map(|_| 0)),
            factory: factory.clone(),
            fee: 30_000_000,
            methods: vec![&env],
            min_locked_liquidity: 1_000,
            name: String::from_str(&env, "Test"),
            offpeg_fee_multiplier: 20_000_000_000,
            oracles: vec![&env],
            symbol: String::from_str(&env, "T"),
            tokens: tokens.clone(),
        };
        let hash = env.deployer().upload_contract_wasm(wasm);
        let pool = FactoryClient::new(&env, &factory).new_pool(&hash, &args);
        let amounts = Vec::from_iter(&env, (0..count).map(|_| LIQUIDITY));
        let receiver = MuxedAddress::from(&user);
        let token_addresses: alloc::vec::Vec<_> = tokens.iter().collect();
        let transfers: alloc::vec::Vec<_> = token_addresses
            .iter()
            .map(|token| MockAuthInvoke {
                contract: token,
                fn_name: "transfer",
                args: (&user, &pool, LIQUIDITY).into_val(&env),
                sub_invokes: &[],
            })
            .collect();
        pool_abi::Client::new(&env, &pool)
            .mock_auths(&[MockAuth {
                address: &user,
                invoke: &MockAuthInvoke {
                    contract: &pool,
                    fn_name: "add_liquidity",
                    args: (&user, &amounts, 0_i128, &receiver).into_val(&env),
                    sub_invokes: &transfers,
                },
            }])
            .add_liquidity(&user, &amounts, &0, &receiver);
        Self {
            env,
            user,
            router,
            pool,
            tokens,
        }
    }

    fn payload(&self, input: &Address, output: &Address, floor: i128) -> Bytes {
        strategy_xdr(
            &self.env,
            input.clone(),
            output.clone(),
            floor,
            alloc::vec![one_hop_path(
                &self.env,
                SwapVenue::Curve,
                self.pool.clone(),
                input.clone(),
                output.clone(),
                1_000_000
            ),],
        )
    }

    fn swap(
        &self,
        input: &Address,
        output: &Address,
        floor: i128,
    ) -> Result<i128, soroban_sdk::Error> {
        let xdr = self.payload(input, output, floor);
        let result = RouterClient::new(&self.env, &self.router)
            .mock_auths(&[MockAuth {
                address: &self.user,
                invoke: &MockAuthInvoke {
                    contract: &self.router,
                    fn_name: "execute_strategy",
                    args: (&self.user, INPUT, &xdr).into_val(&self.env),
                    sub_invokes: &[MockAuthInvoke {
                        contract: input,
                        fn_name: "transfer",
                        args: (&self.user, &self.router, INPUT).into_val(&self.env),
                        sub_invokes: &[],
                    }],
                },
            }])
            .try_execute_strategy(&self.user, &INPUT, &xdr);
        match result {
            Ok(value) => Ok(value.unwrap()),
            Err(error) => Err(error.unwrap()),
        }
    }

    fn balances(&self) -> alloc::vec::Vec<i128> {
        self.tokens
            .iter()
            .flat_map(|address| {
                let client = token::Client::new(&self.env, &address);
                [&self.user, &self.router, &self.pool].map(|owner| client.balance(owner))
            })
            .collect()
    }

    fn check_real_swap(&self, i: u32, j: u32) {
        let input = self.tokens.get(i).unwrap();
        let output = self.tokens.get(j).unwrap();
        let quote = pool_abi::Client::new(&self.env, &self.pool).get_dy(&i, &j, &INPUT);
        let before = self.balances();
        assert!(quote > 0);
        assert_eq!(
            self.swap(&input, &output, quote + 1),
            Err(Error::SlippageExceeded.into())
        );
        assert_eq!(
            self.balances(),
            before,
            "failed floor must roll back user, pool and router balances"
        );
        assert_eq!(self.swap(&input, &output, quote), Ok(quote));
        let after = self.balances();
        assert_eq!(after[i as usize * 3], before[i as usize * 3] - INPUT);
        assert_eq!(after[j as usize * 3], before[j as usize * 3] + quote);
        assert_eq!(
            after[i as usize * 3 + 2],
            before[i as usize * 3 + 2] + INPUT
        );
        for address in self.tokens.iter() {
            assert_eq!(
                token::Client::new(&self.env, &address).balance(&self.router),
                0
            );
        }
    }
}

#[test]
fn curve_real_two_coin_both_directions_exact_auth_and_floor_rollback() {
    let fixture = Fixture::new(TWO_COIN, 2);
    fixture.check_real_swap(0, 1);
    fixture.check_real_swap(1, 0);
}

#[test]
fn curve_real_three_coin_index_two_to_zero_exact_auth_and_floor_rollback() {
    Fixture::new(THREE_COIN, 3).check_real_swap(2, 0);
}

#[contract]
struct AdversarialPool;

#[contractimpl]
impl AdversarialPool {
    pub fn __constructor(env: Env, coins: Vec<Address>, spend: i128, output: i128, report: i128) {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "coins"), &coins);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "spend"), &spend);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "output"), &output);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "report"), &report);
    }

    pub fn pool_info(env: Env) -> Map<Symbol, Val> {
        let coins: Vec<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "coins"))
            .unwrap();
        Map::from_array(&env, [(Symbol::new(&env, "coins"), coins.into_val(&env))])
    }

    pub fn exchange(
        env: Env,
        sender: Address,
        i: u32,
        j: u32,
        dx: i128,
        min_dy: i128,
        receiver: Address,
    ) -> i128 {
        assert_eq!(dx, INPUT);
        assert_eq!(min_dy, 0);
        assert_eq!(sender, receiver);
        let coins: Vec<Address> = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "coins"))
            .unwrap();
        let spend: i128 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "spend"))
            .unwrap();
        let output: i128 = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "output"))
            .unwrap();
        let pool = env.current_contract_address();
        if spend > 0 {
            token::Client::new(&env, &coins.get(i).unwrap()).transfer(&sender, &pool, &spend);
        }
        token::Client::new(&env, &coins.get(j).unwrap()).transfer(&pool, &receiver, &output);
        env.storage()
            .instance()
            .get(&Symbol::new(&env, "report"))
            .unwrap()
    }
}

#[test]
fn curve_rejects_invalid_coins_and_missing_route_token_before_exchange() {
    let mut fixture = Fixture::new(TWO_COIN, 2);
    let a = fixture.tokens.get(0).unwrap();
    let b = fixture.tokens.get(1).unwrap();
    let absent = Address::generate(&fixture.env);
    let oversized = Vec::from_iter(
        &fixture.env,
        (0..=MAX_ASSETS).map(|_| Address::generate(&fixture.env)),
    );
    for coins in [
        vec![&fixture.env],
        vec![&fixture.env, a.clone()],
        vec![&fixture.env, a.clone(), a.clone()],
        vec![&fixture.env, a.clone(), absent.clone()],
        vec![&fixture.env, b.clone(), absent],
        oversized,
    ] {
        fixture.pool = fixture
            .env
            .register(AdversarialPool, (&coins, INPUT, 10_i128, 10_i128));
        let before = fixture.balances();
        assert_eq!(fixture.swap(&a, &b, 1), Err(Error::BrokenTokenChain.into()));
        assert_eq!(fixture.balances(), before);
    }
}

#[test]
fn curve_measured_output_overrides_reported_amount() {
    let mut fixture = Fixture::new(TWO_COIN, 2);
    let a = fixture.tokens.get(0).unwrap();
    let b = fixture.tokens.get(1).unwrap();
    fixture.pool = fixture
        .env
        .register(AdversarialPool, (&fixture.tokens, INPUT, INPUT, i128::MAX));
    // Fund the adversarial pool with a direct user transfer and exact account auth.
    token::Client::new(&fixture.env, &b)
        .mock_auths(&[MockAuth {
            address: &fixture.user,
            invoke: &MockAuthInvoke {
                contract: &b,
                fn_name: "transfer",
                args: (&fixture.user, &fixture.pool, INPUT).into_val(&fixture.env),
                sub_invokes: &[],
            },
        }])
        .transfer(&fixture.user, &fixture.pool, &INPUT);
    let before = fixture.balances();
    assert_eq!(
        fixture.swap(&a, &b, INPUT + 1),
        Err(Error::SlippageExceeded.into())
    );
    assert_eq!(fixture.balances(), before);
    assert_eq!(fixture.swap(&a, &b, INPUT), Ok(INPUT));
    let after = fixture.balances();
    assert_eq!(after[0], before[0] - INPUT);
    assert_eq!(after[3], before[3] + INPUT);
    assert_eq!((after[1], after[4]), (0, 0));
}

#[test]
fn curve_rejects_output_without_exact_input_spend() {
    let mut fixture = Fixture::new(TWO_COIN, 2);
    let a = fixture.tokens.get(0).unwrap();
    let b = fixture.tokens.get(1).unwrap();
    fixture.pool = fixture
        .env
        .register(AdversarialPool, (&fixture.tokens, 0_i128, INPUT, INPUT));
    token::Client::new(&fixture.env, &b)
        .mock_auths(&[MockAuth {
            address: &fixture.user,
            invoke: &MockAuthInvoke {
                contract: &b,
                fn_name: "transfer",
                args: (&fixture.user, &fixture.pool, INPUT).into_val(&fixture.env),
                sub_invokes: &[],
            },
        }])
        .transfer(&fixture.user, &fixture.pool, &INPUT);
    let before = fixture.balances();
    assert_eq!(fixture.swap(&a, &b, 1), Err(Error::InvalidAmount.into()));
    assert_eq!(fixture.balances(), before);
}
