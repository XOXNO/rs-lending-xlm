extern crate std;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Bytes, BytesN, Env};

use common::types::{ControllerKey, PositionLimits};

use crate::constants::TIMELOCK_MIN_DELAY_LEDGERS;
use crate::{Governance, GovernanceClient};

pub const ZERO_SALT: [u8; 32] = [0u8; 32];

pub fn zero_salt(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &ZERO_SALT)
}

pub fn register(env: &Env, min_delay: u32) -> (Address, Address, GovernanceClient<'_>) {
    let admin = Address::generate(env);
    let gov_id = env.register(Governance, (admin.clone(), min_delay));
    let gov = GovernanceClient::new(env, &gov_id);
    (admin, gov_id, gov)
}

pub fn register_governance(env: &Env) -> (Address, Address, GovernanceClient<'_>) {
    register(env, TIMELOCK_MIN_DELAY_LEDGERS)
}

pub fn register_with_controller(
    env: &Env,
    min_delay: u32,
) -> (Address, Address, GovernanceClient<'_>) {
    let (admin, _gov_id, gov) = register(env, min_delay);
    let controller_id = env.register(controller::Controller, (gov.address.clone(),));
    gov.set_controller(&controller_id);
    (admin, controller_id, gov)
}

pub fn fresh_governance(env: &Env) -> Address {
    let (_admin, gov_id, _gov) = register_governance(env);
    gov_id
}

pub fn read_controller_position_limits(env: &Env, controller_id: &Address) -> PositionLimits {
    env.as_contract(controller_id, || {
        env.storage()
            .instance()
            .get(&ControllerKey::PositionLimits)
            .expect("position limits set")
    })
}

pub fn upload_controller_wasm(env: &Env) -> BytesN<32> {
    let path = std::env::var("CONTROLLER_WASM_PATH").unwrap_or_else(|_| {
        std::string::String::from("target/wasm32v1-none/release/controller.wasm")
    });
    let bytes = std::fs::read(&path)
        .or_else(|_| std::fs::read(std::format!("../{path}")))
        .or_else(|_| std::fs::read(std::format!("../../{path}")))
        .unwrap_or_else(|_| panic!("Controller WASM not found. Run 'make build' first."));
    env.deployer()
        .upload_contract_wasm(Bytes::from_slice(env, &bytes))
}

pub fn upload_price_aggregator_wasm(env: &Env) -> BytesN<32> {
    let path = std::env::var("PRICE_AGGREGATOR_WASM_PATH").unwrap_or_else(|_| {
        std::string::String::from("target/wasm32v1-none/release/price_aggregator.wasm")
    });
    let bytes = std::fs::read(&path)
        .or_else(|_| std::fs::read(std::format!("../{path}")))
        .or_else(|_| std::fs::read(std::format!("../../{path}")))
        .unwrap_or_else(|_| panic!("Price aggregator WASM not found. Run 'make build' first."));
    env.deployer()
        .upload_contract_wasm(Bytes::from_slice(env, &bytes))
}
