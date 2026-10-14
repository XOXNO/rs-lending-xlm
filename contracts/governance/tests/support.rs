extern crate std;

use std::env::var as env_var;
use std::format;
use std::fs;
use std::path::Path;
use std::string::String;
use std::vec::Vec;

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

/// Reads the release WASM `file` from `override_var` when it is set, else from
/// `$CARGO_TARGET_DIR/wasm32v1-none/release/<file>` (CI's mutation job builds into
/// `target-mutants`), else from `target/wasm32v1-none/release/<file>`. A relative
/// path is tried from the crate directory and up to two levels above it.
pub fn release_wasm_bytes(file: &str, override_var: &str) -> Vec<u8> {
    let mut bases = Vec::new();
    if let Ok(path) = env_var(override_var) {
        bases.push(path);
    }
    if let Ok(dir) = env_var("CARGO_TARGET_DIR") {
        bases.push(format!("{dir}/wasm32v1-none/release/{file}"));
    }
    bases.push(format!("target/wasm32v1-none/release/{file}"));
    let mut candidates: Vec<String> = Vec::new();
    for base in bases {
        if Path::new(&base).is_absolute() {
            candidates.push(base);
        } else {
            for prefix in ["", "../", "../../"] {
                candidates.push(format!("{prefix}{base}"));
            }
        }
    }
    candidates
        .iter()
        .find_map(|path| fs::read(path).ok())
        .unwrap_or_else(|| {
            panic!("{file} not found (tried {candidates:?}). Run 'make build' first.")
        })
}

pub fn controller_wasm_bytes() -> Vec<u8> {
    release_wasm_bytes("controller.wasm", "CONTROLLER_WASM_PATH")
}

pub fn upload_controller_wasm(env: &Env) -> BytesN<32> {
    let bytes = controller_wasm_bytes();
    env.deployer()
        .upload_contract_wasm(Bytes::from_slice(env, &bytes))
}

pub fn upload_price_aggregator_wasm(env: &Env) -> BytesN<32> {
    let bytes = release_wasm_bytes("price_aggregator.wasm", "PRICE_AGGREGATOR_WASM_PATH");
    env.deployer()
        .upload_contract_wasm(Bytes::from_slice(env, &bytes))
}
