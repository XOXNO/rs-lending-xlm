use soroban_sdk::{
    contract, contractimpl, symbol_short,
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    token, vec, Address, Bytes, Env, IntoVal, Map, MuxedAddress, Symbol, Val, Vec,
};

use crate::{
    errors::Error,
    program::MAX_ASSETS,
    types::{SwapHop, SwapVenue},
    Router, RouterClient,
};

use super::super::support::{new_asset, path, strategy_xdr};

#[path = "../../fixtures/curve/factory.rs"]
mod factory;

pub(super) use factory::pool_abi;

pub(super) const LIQUIDITY: i128 = 1_000_000_000;
pub(super) const INPUT: i128 = 10_000_000;

/// Deploys a real Curve pool over `tokens` and seeds it with `seed` of each coin from `user`.
pub(super) fn deploy_pool(env: &Env, user: &Address, tokens: &Vec<Address>, seed: i128) -> Address {
    let pool = factory::deploy(env, tokens);
    let amounts = Vec::from_iter(env, tokens.iter().map(|_| seed));
    let receiver = MuxedAddress::from(user);
    let token_addresses: alloc::vec::Vec<_> = tokens.iter().collect();
    let transfers: alloc::vec::Vec<_> = token_addresses
        .iter()
        .map(|token| MockAuthInvoke {
            contract: token,
            fn_name: "transfer",
            args: (user, &pool, seed).into_val(env),
            sub_invokes: &[],
        })
        .collect();
    pool_abi::Client::new(env, &pool)
        .mock_auths(&[MockAuth {
            address: user,
            invoke: &MockAuthInvoke {
                contract: &pool,
                fn_name: "add_liquidity",
                args: (user, &amounts, 0_i128, &receiver).into_val(env),
                sub_invokes: &transfers,
            },
        }])
        .add_liquidity(user, &amounts, &0, &receiver);
    pool
}

pub(super) fn curve_hop(pool: &Address, token_in: &Address, token_out: &Address) -> SwapHop {
    SwapHop {
        venue: SwapVenue::Curve,
        pool: pool.clone(),
        token_in: token_in.clone(),
        token_out: token_out.clone(),
    }
}

pub(super) struct Fixture {
    pub(super) env: Env,
    pub(super) user: Address,
    pub(super) router: Address,
    pub(super) pool: Address,
    pub(super) tokens: Vec<Address>,
}

impl Fixture {
    pub(super) fn new(count: u32) -> Self {
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
        let pool = deploy_pool(&env, &user, &tokens, LIQUIDITY);
        Self {
            env,
            user,
            router,
            pool,
            tokens,
        }
    }

    /// The pool's LP token followed by its coins, so asset `k + 1` is coin `k`.
    pub(super) fn assets(&self) -> Vec<Address> {
        let mut assets = vec![&self.env, self.pool.clone()];
        assets.append(&self.tokens);
        assets
    }

    /// Encodes `hops` as one full-weight path with strategy floor `floor`.
    pub(super) fn payload(&self, hops: alloc::vec::Vec<SwapHop>, floor: i128) -> Bytes {
        let input = hops[0].token_in.clone();
        let output = hops[hops.len() - 1].token_out.clone();
        strategy_xdr(
            &self.env,
            input,
            output,
            floor,
            alloc::vec![path(hops, 1_000_000)],
        )
    }

    /// Runs `xdr` for the user, authorizing exactly the strategy call and its input transfer.
    pub(super) fn execute(
        &self,
        input: &Address,
        amount: i128,
        xdr: &Bytes,
    ) -> Result<i128, soroban_sdk::Error> {
        let result = RouterClient::new(&self.env, &self.router)
            .mock_auths(&[MockAuth {
                address: &self.user,
                invoke: &MockAuthInvoke {
                    contract: &self.router,
                    fn_name: "execute_strategy",
                    args: (&self.user, amount, xdr).into_val(&self.env),
                    sub_invokes: &[MockAuthInvoke {
                        contract: input,
                        fn_name: "transfer",
                        args: (&self.user, &self.router, amount).into_val(&self.env),
                        sub_invokes: &[],
                    }],
                },
            }])
            .try_execute_strategy(&self.user, &amount, xdr);
        match result {
            Ok(value) => Ok(value.unwrap()),
            Err(error) => Err(error.unwrap()),
        }
    }

    /// Routes `amount` of `input` to `output` in one hop through the fixture pool.
    pub(super) fn swap(
        &self,
        input: &Address,
        output: &Address,
        amount: i128,
        floor: i128,
    ) -> Result<i128, soroban_sdk::Error> {
        let hops = alloc::vec![curve_hop(&self.pool, input, output)];
        self.execute(input, amount, &self.payload(hops, floor))
    }

    /// User, router and pool balances of every asset, in [`Self::assets`] order, plus the pool's
    /// accounting snapshot.
    pub(super) fn state(&self) -> (alloc::vec::Vec<i128>, pool_abi::PoolSnapshot) {
        let balances = self
            .assets()
            .iter()
            .flat_map(|address| {
                let client = token::Client::new(&self.env, &address);
                [&self.user, &self.router, &self.pool].map(|owner| client.balance(owner))
            })
            .collect();
        (
            balances,
            pool_abi::Client::new(&self.env, &self.pool).pool_snapshot(),
        )
    }

    fn check_real_swap(&self, i: u32, j: u32) {
        let input = self.tokens.get(i).unwrap();
        let output = self.tokens.get(j).unwrap();
        let quote = pool_abi::Client::new(&self.env, &self.pool).get_dy(&i, &j, &INPUT);
        let before = self.state();
        assert!(quote > 0);
        assert_eq!(
            self.swap(&input, &output, INPUT, quote + 1),
            Err(Error::SlippageExceeded.into())
        );
        assert_eq!(
            self.state(),
            before,
            "failed floor must roll back user, pool and router balances"
        );
        assert_eq!(self.swap(&input, &output, INPUT, quote), Ok(quote));
        let (after, _) = self.state();
        let (i, j) = (i as usize + 1, j as usize + 1);
        assert_eq!(after[i * 3], before.0[i * 3] - INPUT);
        assert_eq!(after[j * 3], before.0[j * 3] + quote);
        assert_eq!(after[i * 3 + 2], before.0[i * 3 + 2] + INPUT);
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
    let fixture = Fixture::new(2);
    fixture.check_real_swap(0, 1);
    fixture.check_real_swap(1, 0);
}

#[test]
fn curve_real_three_coin_index_two_to_zero_exact_auth_and_floor_rollback() {
    Fixture::new(3).check_real_swap(2, 0);
}

/// A pool that only reports `coins`, for routing checks that must fail before any pool call.
#[contract]
struct AdversarialPool;

#[contractimpl]
impl AdversarialPool {
    pub fn __constructor(env: Env, coins: Vec<Address>) {
        env.storage()
            .instance()
            .set(&symbol_short!("coins"), &coins);
    }

    pub fn pool_info(env: Env) -> Map<Symbol, Val> {
        let coins: Vec<Address> = env
            .storage()
            .instance()
            .get(&symbol_short!("coins"))
            .unwrap();
        Map::from_array(&env, [(symbol_short!("coins"), coins.into_val(&env))])
    }
}

#[test]
fn curve_rejects_invalid_coins_and_missing_route_token_before_exchange() {
    let fixture = Fixture::new(2);
    let env = &fixture.env;
    let a = fixture.tokens.get(0).unwrap();
    let b = fixture.tokens.get(1).unwrap();
    let absent = Address::generate(env);
    let oversized = Vec::from_iter(env, (0..=MAX_ASSETS).map(|_| Address::generate(env)));
    for coins in [
        vec![env],
        vec![env, a.clone()],
        vec![env, a.clone(), a.clone()],
        vec![env, a.clone(), absent.clone()],
        vec![env, b.clone(), absent],
        oversized,
    ] {
        let pool = env.register(AdversarialPool, (&coins,));
        let before = fixture.state();
        let xdr = fixture.payload(alloc::vec![curve_hop(&pool, &a, &b)], 1);
        assert_eq!(
            fixture.execute(&a, INPUT, &xdr),
            Err(Error::BrokenTokenChain.into())
        );
        assert_eq!(fixture.state(), before);
    }
}
