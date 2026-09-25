use super::*;
use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::token;

#[contract]
struct MockController;

#[contractimpl]
impl MockController {
    pub fn get_pool_address(env: Env) -> Address {
        env.current_contract_address()
    }
}

#[contract]
struct MockStrategy;

#[contractimpl]
impl MockStrategy {
    pub fn __constructor(env: Env, asset: Address) {
        env.storage()
            .instance()
            .set(&symbol_short!("asset"), &asset);
    }

    pub fn asset(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&symbol_short!("asset"))
            .unwrap()
    }

    pub fn deposit(env: Env, amount: i128, from: Address) -> i128 {
        from.require_auth();
        token::Client::new(&env, &Self::asset(env.clone())).transfer(
            &from,
            &env.current_contract_address(),
            &amount,
        );
        amount
    }

    pub fn harvest(_env: Env, from: Address, _data: Option<Bytes>) {
        from.require_auth();
    }

    pub fn withdraw(env: Env, amount: i128, from: Address, to: Address) -> i128 {
        from.require_auth();
        token::Client::new(&env, &Self::asset(env.clone())).transfer(
            &env.current_contract_address(),
            &to,
            &amount,
        );
        0
    }
}

#[test]
fn vault_owner_auth_and_nested_token_authorization() {
    let env = Env::default();
    let owner = Address::generate(&env);
    let recipient = Address::generate(&env);
    let issuer = Address::generate(&env);
    let asset = env
        .register_stellar_asset_contract_v2(issuer.clone())
        .address();
    let controller = env.register(MockController, ());
    let nft = Address::generate(&env);
    let runner = env.register(ScriptRunner, ());
    let strategy = env.register(MockStrategy, (&asset,));
    let client = ScriptRunnerClient::new(&env, &runner);

    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &runner,
                fn_name: "configure_vault",
                args: (&owner,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .configure_vault(&owner);
    token::StellarAssetClient::new(&env, &asset)
        .mock_auths(&[MockAuth {
            address: &issuer,
            invoke: &MockAuthInvoke {
                contract: &asset,
                fn_name: "mint",
                args: (&runner, 100i128).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .mint(&runner, &100);

    let ops = vec![
        &env,
        Op::StrategyDeposit(StrategyDepositOp {
            strategy: strategy.clone(),
            amount: 100,
        }),
        Op::StrategyHarvest(strategy.clone()),
        Op::StrategyWithdraw(StrategyWithdrawOp {
            strategy: strategy.clone(),
            amount: 100,
            to: recipient.clone(),
        }),
    ];
    assert!(client.try_run(&controller, &nft, &ops).is_err());
    assert_eq!(token::Client::new(&env, &asset).balance(&runner), 100);
    client
        .mock_auths(&[MockAuth {
            address: &owner,
            invoke: &MockAuthInvoke {
                contract: &runner,
                fn_name: "run",
                args: (&controller, &nft, &ops).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .run(&controller, &nft, &ops);
    let token = token::Client::new(&env, &asset);
    assert_eq!(token.balance(&runner), 0);
    assert_eq!(token.balance(&strategy), 0);
    assert_eq!(token.balance(&recipient), 100);

    // Configured vaults also gate legacy operations and cannot replace their owner.
    assert!(client.try_run(&controller, &nft, &vec![&env]).is_err());
    assert!(client
        .mock_auths(&[MockAuth {
            address: &recipient,
            invoke: &MockAuthInvoke {
                contract: &runner,
                fn_name: "configure_vault",
                args: (&recipient,).into_val(&env),
                sub_invokes: &[],
            },
        }])
        .try_configure_vault(&recipient)
        .is_err());

    // Existing unconfigured controller scripts remain permissionless, but cannot
    // execute vault strategy operations before the owner has been pinned.
    let unconfigured = env.register(ScriptRunner, ());
    let legacy_client = ScriptRunnerClient::new(&env, &unconfigured);
    assert_eq!(legacy_client.run(&controller, &nft, &vec![&env]), 0);
    assert!(legacy_client.try_run(&controller, &nft, &ops).is_err());
}
