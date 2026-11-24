//! Contract events published when the controller and price aggregator
//! contracts are deployed, and when an operation is proposed.

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

/// Event published when an operation is scheduled, naming who proposed it.
/// The execution's `OperationExecuted` carries the same id, so a change made
/// at execution traces back to its proposer.
#[contractevent(topics = ["governance", "operation_proposed"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationProposedEvent {
    pub op_id: BytesN<32>,
    pub proposer: Address,
}
