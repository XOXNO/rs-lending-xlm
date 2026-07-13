//! Governance contract events. Only the controller deployment is emitted here;
//! most configuration events are emitted by the controller itself.

use soroban_sdk::{contractevent, Address, BytesN};

// ################## EVENTS ##################

/// Emitted when governance deploys a new controller instance.
#[contractevent(topics = ["governance", "deploy_controller"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeployControllerEvent {
    pub controller: Address,
    pub wasm_hash: BytesN<32>,
}
