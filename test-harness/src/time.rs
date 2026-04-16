use soroban_sdk::testutils::{Ledger, LedgerInfo};
use soroban_sdk::{Address, Vec};

use crate::context::LendingTest;

impl LendingTest {
    /// Advance the ledger timestamp by `duration_secs` seconds.
    /// Also bumps sequence_number proportionally.
    /// Re-sets oracle prices at the new timestamp to prevent staleness errors.
    pub fn advance_time(&mut self, duration_secs: u64) {
        let current = self.env.ledger().timestamp();
        let current_seq = self.env.ledger().sequence();
        let new_timestamp = current + duration_secs;
        // Roughly 5 seconds per ledger
        let new_seq = current_seq + (duration_secs / 5) as u32;

        self.env.ledger().set(LedgerInfo {
            timestamp: new_timestamp,
            protocol_version: 25,
            sequence_number: new_seq,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 3_110_400,
        });

        // Refresh oracle prices to prevent staleness
        self.refresh_all_prices();
    }

    /// Advance the ledger timestamp WITHOUT refreshing oracle prices.
    /// Useful for testing staleness behavior.
    pub fn advance_time_no_refresh(&self, duration_secs: u64) {
        let current = self.env.ledger().timestamp();
        let current_seq = self.env.ledger().sequence();
        let new_timestamp = current + duration_secs;
        let new_seq = current_seq + (duration_secs / 5) as u32;

        self.env.ledger().set(LedgerInfo {
            timestamp: new_timestamp,
            protocol_version: 25,
            sequence_number: new_seq,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 3_110_400,
        });
    }

    /// Advance time AND call update_indexes on all markets.
    pub fn advance_and_sync(&mut self, duration_secs: u64) {
        self.advance_time(duration_secs);
        self.sync_all_markets();
    }

    /// Advance time AND call update_indexes on specific markets.
    pub fn advance_and_sync_markets(&mut self, duration_secs: u64, market_names: &[&str]) {
        self.advance_time(duration_secs);

        let assets: Vec<Address> = {
            let mut v = Vec::new(&self.env);
            for name in market_names {
                v.push_back(self.resolve_asset(name));
            }
            v
        };

        let ctrl = self.ctrl_client();
        ctrl.update_indexes(&self.keeper, &assets);
    }

    fn refresh_all_prices(&self) {
        let mock_reflector = self.mock_reflector_client();
        for market in self.markets.values() {
            mock_reflector.set_price(&market.asset, &market.price_wad);
            mock_reflector.set_twap_price(&market.asset, &market.price_wad);
        }
    }

    fn sync_all_markets(&self) {
        let assets: Vec<Address> = {
            let mut v = Vec::new(&self.env);
            for market in self.markets.values() {
                v.push_back(market.asset.clone());
            }
            v
        };

        let ctrl = self.ctrl_client();
        ctrl.update_indexes(&self.keeper, &assets);
    }
}
