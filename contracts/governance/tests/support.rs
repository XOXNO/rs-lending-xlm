//! Shared governance test fixtures.

extern crate std;

use soroban_sdk::{Bytes, BytesN, Env};

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
