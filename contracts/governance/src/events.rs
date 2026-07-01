//! Governance-emitted events.

use soroban_sdk::{contractevent, Address, BytesN};

#[contractevent(topics = ["governance", "deploy_controller"])]
#[derive(Clone, Debug)]
pub struct DeployControllerEvent {
    pub controller: Address,
    pub wasm_hash: BytesN<32>,
}
