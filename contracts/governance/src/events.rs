//! Contract events published when the controller and price aggregator
//! contracts are deployed, and when an expired operation is cleared.

use soroban_sdk::{contractevent, Address, BytesN};

/// Event published after the controller contract is deployed, carrying its
/// address and the wasm hash it was deployed from.
#[contractevent(topics = ["governance", "deploy_controller"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeployControllerEvent {
    pub controller: Address,
    pub wasm_hash: BytesN<32>,
}

/// Event published after the price aggregator contract is deployed,
/// carrying its address and the wasm hash it was deployed from.
#[contractevent(topics = ["governance", "deploy_price_aggregator"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeployPriceAggregatorEvent {
    pub price_aggregator: Address,
    pub wasm_hash: BytesN<32>,
}

/// Event published when a proposal clears an expired operation with the same
/// id, together with its sidecar state, before scheduling it again.
#[contractevent(topics = ["governance", "expired_operation_cleared"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpiredOperationClearedEvent {
    pub operation_id: BytesN<32>,
}
