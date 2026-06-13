//! Governance-emitted events.
//!
//! Forwarded admin operations stay observable through the controller's own
//! config events; governance only announces the controller deployment.

use soroban_sdk::{contractevent, Address, BytesN};

#[contractevent(topics = ["governance", "deploy_controller"])]
#[derive(Clone, Debug)]
pub struct DeployControllerEvent {
    pub controller: Address,
    pub wasm_hash: BytesN<32>,
}
