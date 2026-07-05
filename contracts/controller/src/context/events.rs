//! Buffered position event context methods.

use common::types::{Account, AccountPosition, DebtPosition, HubAssetKey};
use soroban_sdk::Vec;

use crate::context::Cache;
use crate::events::{
    EventBorrowDelta, EventDepositDelta, PositionAction, UpdatePositionBatchEvent,
};

impl Cache {
    /// Buffers a supply-position delta for the next batch event emission.
    pub fn record_position_update(
        &mut self,
        action: PositionAction,
        hub_asset: &HubAssetKey,
        index_ray: i128,
        amount: i128,
        position: &AccountPosition,
    ) {
        self.deposit_updates.push_back(EventDepositDelta::new(
            action,
            hub_asset.hub_id,
            hub_asset.asset.clone(),
            index_ray,
            amount,
            position,
        ));
    }

    /// Buffers a debt-position delta for the next batch event emission.
    pub fn record_debt_position_update(
        &mut self,
        action: PositionAction,
        hub_asset: &HubAssetKey,
        index_ray: i128,
        amount: i128,
        position: &DebtPosition,
    ) {
        self.borrow_updates.push_back(EventBorrowDelta::new(
            action,
            hub_asset.hub_id,
            hub_asset.asset.clone(),
            index_ray,
            amount,
            position,
        ));
    }

    /// Publishes buffered supply and debt deltas as one position-batch event, then clears the buffers.
    pub fn emit_position_batch(&mut self, account_id: u64, account: &Account) {
        if self.deposit_updates.is_empty() && self.borrow_updates.is_empty() {
            return;
        }
        UpdatePositionBatchEvent {
            account_id,
            account_attributes: account.into(),
            deposits: self.deposit_updates.clone(),
            borrows: self.borrow_updates.clone(),
        }
        .publish(&self.env);
        self.deposit_updates = Vec::new(&self.env);
        self.borrow_updates = Vec::new(&self.env);
    }
}
