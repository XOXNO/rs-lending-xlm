use soroban_sdk::testutils::{Ledger, LedgerInfo};
use soroban_sdk::Vec;

use common::types::HubAssetKey;

use crate::context::LendingTest;
use crate::helpers::hub_asset;
use crate::presets::LEDGER_PROTOCOL_VERSION;

impl LendingTest {
    pub fn advance_time(&mut self, duration_secs: u64) {
        self.advance_ledger(duration_secs);
        self.refresh_oracle_prices();
    }

    pub fn advance_time_no_refresh(&self, duration_secs: u64) {
        self.advance_ledger(duration_secs);
    }

    /// Moves the ledger forward by `duration_secs`, at 5 seconds per ledger.
    fn advance_ledger(&self, duration_secs: u64) {
        let current = self.env.ledger().timestamp();
        let current_seq = self.env.ledger().sequence();

        self.env.ledger().set(LedgerInfo {
            timestamp: current + duration_secs,
            protocol_version: LEDGER_PROTOCOL_VERSION,
            sequence_number: current_seq + (duration_secs / 5) as u32,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 3_110_400,
        });
    }

    pub fn advance_and_sync(&mut self, duration_secs: u64) {
        self.advance_time(duration_secs);
        self.sync_all_markets();
    }

    pub fn advance_and_sync_markets(&mut self, duration_secs: u64, market_names: &[&str]) {
        self.advance_time(duration_secs);

        let assets: Vec<HubAssetKey> = {
            let mut v = Vec::new(&self.env);
            for name in market_names {
                v.push_back(hub_asset(self.resolve_asset(name)));
            }
            v
        };

        let ctrl = self.ctrl_client();
        ctrl.update_indexes(&self.keeper, &assets);
    }

    fn sync_all_markets(&self) {
        let assets: Vec<HubAssetKey> = {
            let mut v = Vec::new(&self.env);
            for market in self.markets.values() {
                v.push_back(hub_asset(market.asset.clone()));
            }
            v
        };

        let ctrl = self.ctrl_client();
        ctrl.update_indexes(&self.keeper, &assets);
    }
}
