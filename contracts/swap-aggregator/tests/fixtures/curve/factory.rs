//! Test-only factory that deploys the real Curve pool fixtures. Shared with
//! `#[path]` by the router, price-aggregator and harness tests. Every consumer's
//! manifest sits two directories below the workspace root, so one
//! `contractimport!` path serves them all.

use soroban_sdk::{
    auth::{ContractContext, InvokerContractAuthEntry, SubContractInvocation},
    contract, contractimpl,
    testutils::Address as _,
    vec, Address, BytesN, ContractExecutable, Env, IntoVal, MuxedAddress, String, Symbol, Vec,
};

pub mod pool_abi {
    soroban_sdk::contractimport!(
        file = "../../contracts/swap-aggregator/tests/fixtures/curve/pool-2coin.wasm"
    );
}

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
            InvokerContractAuthEntry::Contract(SubContractInvocation {
                context: ContractContext {
                    contract: deployer.deployed_address(),
                    fn_name: Symbol::new(&env, "__constructor"),
                    args: (args.clone(),).into_val(&env),
                },
                sub_invocations: vec![&env],
            }),
        ]);
        deployer.deploy_contract(ContractExecutable::Wasm(hash), (args,))
    }
}

/// Deploys the real two- or three-coin Curve pool over `tokens` through a fresh
/// factory, with no liquidity.
pub fn deploy(env: &Env, tokens: &Vec<Address>) -> Address {
    let wasm: &[u8] = match tokens.len() {
        2 => include_bytes!("pool-2coin.wasm"),
        3 => include_bytes!("pool-3coin.wasm"),
        n => panic!("no {n}-coin Curve fixture"),
    };
    let factory = env.register(Factory, (Address::generate(env),));
    let args = pool_abi::PoolConstructorArgs {
        admin_fee: 5_000_000_000,
        amplification: 100,
        asset_types: Vec::from_iter(env, tokens.iter().map(|_| 0)),
        factory: factory.clone(),
        fee: 30_000_000,
        methods: vec![env],
        min_locked_liquidity: 1_000,
        name: String::from_str(env, "Curve LP"),
        offpeg_fee_multiplier: 20_000_000_000,
        oracles: vec![env],
        symbol: String::from_str(env, "LP"),
        tokens: tokens.clone(),
    };
    let hash = env.deployer().upload_contract_wasm(wasm);
    FactoryClient::new(env, &factory).new_pool(&hash, &args)
}
