//! Buffers per-account supply and debt position deltas on `Cache` and
//! publishes them as a single batched event.

use common::types::{Account, AccountPosition, DebtPosition, HubAssetKey};
use soroban_sdk::Vec;

use crate::context::Cache;
use crate::events::{
    EventBorrowDelta, EventDepositDelta, PositionAction, UpdatePositionBatchEvent,
};

impl Cache {
    /// Appends a supply-position delta for `hub_asset` to the pending
    /// supply-update queue.
    pub(crate) fn record_supply_position_update(
        &mut self,
        action: PositionAction,
        hub_asset: &HubAssetKey,
        index_ray: i128,
        amount: i128,
        position: &AccountPosition,
    ) {
        self.supply_updates.push_back(EventDepositDelta::new(
            action,
            hub_asset.hub_id,
            hub_asset.asset.clone(),
            index_ray,
            amount,
            position,
        ));
    }

    /// Appends a debt-position delta for `hub_asset` to the pending
    /// debt-update queue.
    pub(crate) fn record_debt_position_update(
        &mut self,
        action: PositionAction,
        hub_asset: &HubAssetKey,
        index_ray: i128,
        amount: i128,
        position: &DebtPosition,
    ) {
        self.debt_updates.push_back(EventBorrowDelta::new(
            action,
            hub_asset.hub_id,
            hub_asset.asset.clone(),
            index_ray,
            amount,
            position,
        ));
    }

    /// Publishes the queued supply and debt updates for `account_id` as one
    /// `UpdatePositionBatchEvent`, then clears both queues. Does nothing if
    /// both queues are empty.
    pub(crate) fn emit_position_batch(&mut self, account_id: u64, account: &Account) {
        if self.supply_updates.is_empty() && self.debt_updates.is_empty() {
            return;
        }
        UpdatePositionBatchEvent {
            account_id,
            account_attributes: account.into(),
            deposits: self.supply_updates.clone(),
            borrows: self.debt_updates.clone(),
        }
        .publish(&self.env);
        self.supply_updates = Vec::new(&self.env);
        self.debt_updates = Vec::new(&self.env);
    }
}
