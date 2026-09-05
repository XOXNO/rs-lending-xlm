use common::types::{AccountMeta, DebtPositionRaw, HubAssetKey, SpokeUsageRaw};
use controller::constants::WAD;
use controller::types::ControllerKey;
use position_nft::PositionNftClient;
use soroban_sdk::{Env, Map};
use std::collections::HashMap;

use crate::context::LendingTest;
use crate::helpers::hub_asset;
use crate::view::PositionType;

fn side_count(env: &Env, account_id: u64, pos_type: PositionType) -> u32 {
    let key = match pos_type {
        PositionType::Supply => ControllerKey::SupplyPositions(account_id),
        PositionType::Borrow => ControllerKey::BorrowPositions(account_id),
    };
    env.storage()
        .persistent()
        .get::<_, Map<HubAssetKey, controller::types::AccountPositionRaw>>(&key)
        .map(|m| m.len())
        .unwrap_or(0)
}

/// `HubAssetKey` is neither `Hash` nor `Ord`, so the map is keyed by the
/// spoke id and the key's `Debug` form; the key itself rides along as a value.
type UsageRows = HashMap<(u32, String), (HubAssetKey, i128, i128)>;

fn bump_usage(
    rows: &mut UsageRows,
    spoke_id: u32,
    key: HubAssetKey,
    supplied: i128,
    borrowed: i128,
) {
    let row = rows
        .entry((spoke_id, format!("{key:?}")))
        .or_insert((key, 0, 0));
    row.1 += supplied;
    row.2 += borrowed;
}

pub fn assert_contract_error<T: std::fmt::Debug>(
    result: Result<T, soroban_sdk::Error>,
    expected_code: u32,
) {
    match result {
        Ok(val) => panic!(
            "expected contract error {} but got Ok({:?})",
            expected_code, val
        ),
        Err(err) => {
            let expected = soroban_sdk::Error::from_contract_error(expected_code);
            assert_eq!(
                err, expected,
                "expected contract error {} but got {:?}",
                expected_code, err
            );
        }
    }
}

impl LendingTest {
    pub fn assert_healthy(&self, user: &str) {
        let hf = self.health_factor_raw(user);
        assert!(
            hf >= WAD,
            "'{}' should be healthy (HF >= 1.0) but HF = {}",
            user,
            hf as f64 / WAD as f64
        );
    }

    pub fn assert_liquidatable(&self, user: &str) {
        let hf = self.health_factor_raw(user);
        assert!(
            hf < WAD,
            "'{}' should be liquidatable (HF < 1.0) but HF = {}",
            user,
            hf as f64 / WAD as f64
        );
    }

    pub fn assert_position_exists(&self, user: &str, asset_name: &str, pos_type: PositionType) {
        let account_id = self.resolve_account_id(user);
        self.assert_position_exists_for(user, account_id, asset_name, pos_type);
    }

    fn assert_position_exists_for(
        &self,
        user: &str,
        account_id: u64,
        asset_name: &str,
        pos_type: PositionType,
    ) {
        let asset = self.resolve_asset(asset_name);

        let type_label = match pos_type {
            PositionType::Supply => "supply",
            PositionType::Borrow => "borrow",
        };

        self.env.as_contract(&self.controller, || {
            let map_key = match pos_type {
                PositionType::Supply => ControllerKey::SupplyPositions(account_id),
                PositionType::Borrow => ControllerKey::BorrowPositions(account_id),
            };
            let has_pos = self
                .env
                .storage()
                .persistent()
                .get::<_, soroban_sdk::Map<HubAssetKey, controller::types::AccountPositionRaw>>(
                    &map_key,
                )
                .map(|m| m.contains_key(hub_asset(asset.clone())))
                .unwrap_or(false);
            assert!(
                has_pos,
                "'{}' account {} should have {} position for '{}'",
                user, account_id, type_label, asset_name
            );
        });
    }

    pub fn assert_no_positions(&self, user: &str) {
        if let Some(account_id) = self.find_account_id(user) {
            self.assert_no_positions_for(user, account_id);
        }
    }

    pub fn assert_no_positions_for(&self, user: &str, account_id: u64) {
        self.env.as_contract(&self.controller, || {
            let supply_count = side_count(&self.env, account_id, PositionType::Supply);
            let borrow_count = side_count(&self.env, account_id, PositionType::Borrow);
            assert!(
                supply_count == 0 && borrow_count == 0,
                "'{}' account {} should have no positions but has {} supply, {} borrow",
                user,
                account_id,
                supply_count,
                borrow_count
            );
        });
    }

    pub fn assert_supply_count(&self, user: &str, expected: u32) {
        let count = self.find_account_id(user).map_or(0u32, |account_id| {
            self.env.as_contract(&self.controller, || {
                side_count(&self.env, account_id, PositionType::Supply)
            })
        });
        assert_eq!(
            count, expected,
            "'{}' should have {} supply positions, got {}",
            user, expected, count
        );
    }

    pub fn assert_borrow_count(&self, user: &str, expected: u32) {
        let count = self.find_account_id(user).map_or(0u32, |account_id| {
            self.env.as_contract(&self.controller, || {
                side_count(&self.env, account_id, PositionType::Borrow)
            })
        });
        assert_eq!(
            count, expected,
            "'{}' should have {} borrow positions, got {}",
            user, expected, count
        );
    }

    pub fn assert_balance_eq(&self, user: &str, asset_name: &str, expected: f64) {
        let actual = self.token_balance(user, asset_name);
        assert!(
            (actual - expected).abs() < 0.001,
            "'{}' balance of '{}' expected {} but got {}",
            user,
            asset_name,
            expected,
            actual
        );
    }

    pub fn assert_supply_near(&self, user: &str, asset_name: &str, expected: f64, tolerance: f64) {
        let actual = self.supply_balance(user, asset_name);
        assert!(
            (actual - expected).abs() <= tolerance,
            "'{}' supply of '{}' expected ~{} (+-{}) but got {}",
            user,
            asset_name,
            expected,
            tolerance,
            actual
        );
    }

    pub fn assert_borrow_near(&self, user: &str, asset_name: &str, expected: f64, tolerance: f64) {
        let actual = self.borrow_balance(user, asset_name);
        assert!(
            (actual - expected).abs() <= tolerance,
            "'{}' borrow of '{}' expected ~{} (+-{}) but got {}",
            user,
            asset_name,
            expected,
            tolerance,
            actual
        );
    }

    pub fn assert_revenue_increased_since(&self, asset_name: &str, snapshot: i128) {
        let current = self.snapshot_revenue(asset_name);
        assert!(
            current > snapshot,
            "pool '{}' revenue should have increased: before={}, after={}",
            asset_name,
            snapshot,
            current
        );
    }

    /// Every spoke usage row equals the sum of live scaled positions in that
    /// spoke, per hub asset and side, and a row exists only while that sum is
    /// non-zero on at least one side.
    ///
    /// Usage and positions are written by the same leg merge, so the only way
    /// they can drift is a writer that moves shares without `apply_leg_usage`.
    /// That is the A080 precondition: the exit path no-ops on a missing row, so
    /// a drift is never healed once it exists. Accounts are enumerated from the
    /// position NFT, which includes the receivers `SeizeMode::Credit(0)` opens.
    pub fn assert_spoke_usage_matches_positions(&self) {
        let nft = PositionNftClient::new(&self.env, &self.position_nft);
        let mut expected = UsageRows::new();

        self.env.as_contract(&self.controller, || {
            let storage = self.env.storage().persistent();
            for index in 0..nft.total_supply() {
                let account_id = u64::from(nft.get_token_id(&index));
                let Some(meta) =
                    storage.get::<_, AccountMeta>(&ControllerKey::AccountMeta(account_id))
                else {
                    continue;
                };
                let supplies = storage
                    .get::<_, Map<HubAssetKey, controller::types::AccountPositionRaw>>(
                        &ControllerKey::SupplyPositions(account_id),
                    )
                    .unwrap_or_else(|| Map::new(&self.env));
                for (key, position) in supplies.iter() {
                    bump_usage(&mut expected, meta.spoke_id, key, position.scaled_amount, 0);
                }
                let borrows = storage
                    .get::<_, Map<HubAssetKey, DebtPositionRaw>>(&ControllerKey::BorrowPositions(
                        account_id,
                    ))
                    .unwrap_or_else(|| Map::new(&self.env));
                for (key, position) in borrows.iter() {
                    bump_usage(&mut expected, meta.spoke_id, key, 0, position.scaled_amount);
                }
            }

            // Every (spoke, hub, asset) triple is checked, not only the ones
            // with live positions, so a row that outlived its last position
            // is caught too. Hubs are enumerated from the counter because a
            // market can be listed on a hub other than the default one and
            // `self.markets` keeps only the asset.
            let instance = self.env.storage().instance();
            let last_spoke: u32 = instance.get(&ControllerKey::LastSpokeId).unwrap_or(0);
            let last_hub: u32 = instance.get(&ControllerKey::LastHubId).unwrap_or(0);
            for spoke_id in 1..=last_spoke {
                for hub_id in 1..=last_hub {
                    for market in self.markets.values() {
                        let key = HubAssetKey {
                            hub_id,
                            asset: market.asset.clone(),
                        };
                        bump_usage(&mut expected, spoke_id, key, 0, 0);
                    }
                }
            }

            for ((spoke_id, _), (key, supplied, borrowed)) in &expected {
                let row = storage.get::<_, SpokeUsageRaw>(&ControllerKey::SpokeUsage(
                    *spoke_id,
                    key.clone(),
                ));
                match row {
                    Some(usage) => {
                        assert_eq!(
                            (usage.supplied_scaled_ray, usage.borrowed_scaled_ray),
                            (*supplied, *borrowed),
                            "spoke {spoke_id} usage for {key:?} drifted from the positions it tracks"
                        );
                        assert!(
                            *supplied != 0 || *borrowed != 0,
                            "spoke {spoke_id} keeps a usage row for {key:?} with no positions behind it"
                        );
                    }
                    None => assert_eq!(
                        (*supplied, *borrowed),
                        (0, 0),
                        "spoke {spoke_id} has live positions in {key:?} but no usage row"
                    ),
                }
            }
        });
    }
}
