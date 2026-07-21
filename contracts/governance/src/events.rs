//! Controller deployment event only; config events come from the controller.

use soroban_sdk::{contractevent, Address, BytesN};

#[contractevent(topics = ["governance", "deploy_controller"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeployControllerEvent {
    pub controller: Address,
    pub wasm_hash: BytesN<32>,
}

#[contractevent(topics = ["governance", "deploy_price_aggregator"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeployPriceAggregatorEvent {
    pub price_aggregator: Address,
    pub wasm_hash: BytesN<32>,
}
