use common::types::HubAssetKey;
use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, xdr::FromXdr, Address, Bytes, Env,
    Symbol, Vec,
};

use crate::helpers::{HARNESS_HUB, HARNESS_SPOKE};
use crate::strategy::MockSwapPayload;

#[contractclient(name = "ReenterControllerClient")]
pub trait ReenterController {
    fn supply(
        env: Env,
        caller: Address,
        account_id: u64,
        spoke_id: u32,
        assets: Vec<(HubAssetKey, i128)>,
    ) -> u64;

    fn flash_loan(
        env: Env,
        caller: Address,
        asset: HubAssetKey,
        amount: i128,
        receiver: Address,
        data: Bytes,
    );

    fn borrow(
        env: Env,
        caller: Address,
        account_id: u64,
        borrows: Vec<(HubAssetKey, i128)>,
        to: Option<Address>,
    );
}

#[contract]
pub struct MockAggregator;

#[contractimpl]
impl MockAggregator {
    pub fn __constructor(_env: Env, _admin: Address) {}

    pub fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128 {
        sender.require_auth();
        let router = env.current_contract_address();
        let payload =
            MockSwapPayload::from_xdr(&env, &swap_xdr).expect("mock swap payload must decode");

        let in_client = soroban_sdk::token::Client::new(&env, &payload.token_in);
        in_client.transfer(&sender, &router, &total_in);

        if payload.min_out > 0 {
            let out_client = soroban_sdk::token::Client::new(&env, &payload.token_out);
            out_client.transfer(&router, &sender, &payload.min_out);
        }

        payload.min_out
    }
}

#[contracttype]
#[derive(Clone, Copy)]
pub enum BadMode {
    Refund,
    OverPull,
    UnderPull,
    OutputShortfall,
}

#[contract]
pub struct BadAggregator;

#[contractimpl]
impl BadAggregator {
    pub fn __constructor(env: Env, _admin: Address, mode: BadMode) {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "MODE"), &mode);
    }

    pub fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128 {
        sender.require_auth();
        let router = env.current_contract_address();
        let payload =
            MockSwapPayload::from_xdr(&env, &swap_xdr).expect("mock swap payload must decode");
        let mode: BadMode = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "MODE"))
            .expect("mode must be set by constructor");

        let in_client = soroban_sdk::token::Client::new(&env, &payload.token_in);
        let out_client = soroban_sdk::token::Client::new(&env, &payload.token_out);

        match mode {
            BadMode::Refund => {
                if payload.min_out > 0 {
                    out_client.transfer(&router, &sender, &payload.min_out);
                }
                in_client.transfer(&router, &sender, &total_in);
            }
            BadMode::OverPull => {
                let overshoot = total_in.saturating_mul(2);
                in_client.transfer(&sender, &router, &overshoot);
                if payload.min_out > 0 {
                    out_client.transfer(&router, &sender, &payload.min_out);
                }
            }
            BadMode::UnderPull => {
                in_client.transfer(&sender, &router, &(total_in / 2));
                if payload.min_out > 0 {
                    out_client.transfer(&router, &sender, &payload.min_out);
                }
            }
            BadMode::OutputShortfall => {
                in_client.transfer(&sender, &router, &total_in);
            }
        }

        payload.min_out
    }
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ReenterMode {
    Supply = 0,
    Borrow = 1,
    FlashLoan = 2,
    Panic = 3,
}

#[contract]
pub struct ReenteringAggregator;

#[contractimpl]
impl ReenteringAggregator {
    pub fn __constructor(env: Env, controller: Address, mode: ReenterMode) {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "CTRL"), &controller);
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "MODE"), &mode);
    }

    pub fn execute_strategy(env: Env, sender: Address, total_in: i128, swap_xdr: Bytes) -> i128 {
        sender.require_auth();
        let mode: ReenterMode = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "MODE"))
            .expect("mode must be set by constructor");
        let controller: Address = env
            .storage()
            .instance()
            .get(&Symbol::new(&env, "CTRL"))
            .expect("controller must be set by constructor");
        let payload =
            MockSwapPayload::from_xdr(&env, &swap_xdr).expect("mock swap payload must decode");

        reenter(&env, &controller, &payload.token_out, mode);

        let router = env.current_contract_address();
        let in_client = soroban_sdk::token::Client::new(&env, &payload.token_in);
        in_client.transfer(&sender, &router, &total_in);
        if payload.min_out > 0 {
            let out_client = soroban_sdk::token::Client::new(&env, &payload.token_out);
            out_client.transfer(&router, &sender, &payload.min_out);
        }
        payload.min_out
    }
}

fn reenter(env: &Env, controller: &Address, asset: &Address, mode: ReenterMode) {
    let caller = env.current_contract_address();
    let mut assets: Vec<(HubAssetKey, i128)> = Vec::new(env);
    assets.push_back((
        HubAssetKey {
            hub_id: HARNESS_HUB,
            asset: asset.clone(),
        },
        1i128,
    ));
    let client = ReenterControllerClient::new(env, controller);
    match mode {
        ReenterMode::Supply => {
            let _ = client.supply(&caller, &0u64, &HARNESS_SPOKE, &assets);
        }
        ReenterMode::Borrow => {
            client.borrow(&caller, &0u64, &assets, &None);
        }
        ReenterMode::FlashLoan => {
            client.flash_loan(
                &caller,
                &HubAssetKey {
                    hub_id: HARNESS_HUB,
                    asset: asset.clone(),
                },
                &1i128,
                &caller,
                &Bytes::new(env),
            );
        }
        ReenterMode::Panic => {
            panic!("reentering aggregator panic");
        }
    }
}
