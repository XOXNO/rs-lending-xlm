//! The pinned Blend pool fixture activates against this backstop and lends.

use mock_blend_backstop::MockBlendBackstop;
use mock_oracle::{MockReflectorOracle, MockReflectorOracleClient, ReflectorAsset};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{vec, Address, Env, String};

// Generated bindings mirror Blend's constructor, which takes eight arguments.
#[allow(clippy::too_many_arguments)]
mod blend {
    soroban_sdk::contractimport!(file = "../../tests/integration/fixtures/blend/blend_pool.wasm");
}

const WAD: i128 = 1_000_000_000_000_000_000;

fn reserve_config(index: u32) -> blend::ReserveConfig {
    blend::ReserveConfig {
        index,
        decimals: 7,
        c_factor: 9_800_000,
        l_factor: 9_800_000,
        util: 9_000_000,
        max_util: 9_800_000,
        r_base: 50_000,
        r_one: 500_000,
        r_two: 1_000_000,
        r_three: 10_000_000,
        reactivity: 1_000,
        supply_cap: i128::MAX,
        enabled: true,
    }
}

#[test]
fn pinned_pool_activates_and_lends_with_the_mock_backstop() {
    let env = Env::default();
    // The pool constructor requires the admin signature; the live lane signs as admin.
    env.mock_all_auths_allowing_non_root_auth();
    env.cost_estimate().budget().reset_unlimited();

    let admin = Address::generate(&env);
    let backstop = env.register(MockBlendBackstop, ());
    let oracle = env.register(MockReflectorOracle, ());
    let xlm = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let usdc = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let prices = MockReflectorOracleClient::new(&env, &oracle);
    prices.set_price(&ReflectorAsset::Stellar(xlm.clone()), &(3 * WAD / 10));
    prices.set_price(&ReflectorAsset::Stellar(usdc.clone()), &WAD);

    let pool_id = env.register(
        blend::WASM,
        (
            admin.clone(),
            String::from_str(&env, "e2e"),
            oracle,
            5_000_000u32,
            6u32,
            0i128,
            backstop.clone(),
            backstop,
        ),
    );
    let pool = blend::Client::new(&env, &pool_id);
    for (index, asset) in [(0, &xlm), (1, &usdc)] {
        pool.queue_set_reserve(asset, &reserve_config(index));
        pool.set_reserve(asset);
    }
    pool.set_status(&0);
    assert_eq!(pool.get_config().status, 0);

    let lender = Address::generate(&env);
    let borrower = Address::generate(&env);
    StellarAssetClient::new(&env, &usdc).mint(&lender, &(1_000 * 10_000_000));
    StellarAssetClient::new(&env, &xlm).mint(&borrower, &(1_000 * 10_000_000));
    pool.submit(
        &lender,
        &lender,
        &lender,
        &vec![
            &env,
            blend::Request {
                request_type: 0,
                address: usdc.clone(),
                amount: 1_000 * 10_000_000,
            },
        ],
    );
    let positions = pool.submit(
        &borrower,
        &borrower,
        &borrower,
        &vec![
            &env,
            blend::Request {
                request_type: 2,
                address: xlm.clone(),
                amount: 1_000 * 10_000_000,
            },
            blend::Request {
                request_type: 4,
                address: usdc.clone(),
                amount: 10 * 10_000_000,
            },
        ],
    );
    assert_eq!(positions.liabilities.len(), 1);
    assert_eq!(
        soroban_sdk::token::Client::new(&env, &usdc).balance(&borrower),
        10 * 10_000_000
    );
}

#[test]
fn every_other_backstop_call_fails() {
    let env = Env::default();
    let backstop = env.register(MockBlendBackstop, ());
    let result = env.try_invoke_contract::<soroban_sdk::Val, soroban_sdk::Error>(
        &backstop,
        &soroban_sdk::Symbol::new(&env, "deposit"),
        vec![&env],
    );
    assert!(result.is_err());
}
