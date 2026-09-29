#![no_std]

use common::constants::RAY;
use common::types::{
    HubAssetKey, MarketParamsRaw, PoolAction, PoolPositionMutation, PoolSupplyEntry,
    ScaledPositionRaw,
};
use pool::LiquidityPool;
use pool_interface::LiquidityPoolInterface;
use soroban_sdk::{contract, contractimpl, vec, Address, Env, Vec};

const ASSET_DECIMALS: u32 = 7;
// Independent integer specification: one native unit is 10^(27 - 7) RAY units.
const NATIVE_TO_SCALED: i128 = 100_000_000_000_000_000_000;
pub const MAX_SUPPLY: i128 = i128::MAX / NATIVE_TO_SCALED;

#[contract]
pub struct PoolStateProof;

fn params(asset: Address) -> MarketParamsRaw {
    MarketParamsRaw {
        max_borrow_rate: RAY,
        base_borrow_rate: 0,
        slope1: 0,
        slope2: 0,
        slope3: 0,
        mid_utilization: RAY / 2,
        optimal_utilization: RAY * 8 / 10,
        max_utilization: RAY,
        reserve_factor: 0,
        is_flashloanable: false,
        flashloan_fee: 0,
        asset_id: asset,
        asset_decimals: ASSET_DECIMALS,
    }
}

// Retain the real entrypoint call boundary in the proof Wasm.
#[inline(never)]
fn production_supply(env: Env, entry: PoolSupplyEntry) -> Vec<PoolPositionMutation> {
    LiquidityPool::supply(env.clone(), vec![&env, entry])
}

#[contractimpl]
impl PoolStateProof {
    /// Komet runs initialization once, in a separate frame before each test.
    pub fn init(env: Env) {
        let address = env.current_contract_address();
        LiquidityPool::__constructor(env.clone(), address.clone());
        LiquidityPool::create_market(env, 0, params(address));
    }

    /// Same-frame production call; authorized controller and prior token receipt are assumed.
    pub fn test_supply_exact(env: Env, amount: i128) -> bool {
        if amount <= 0 || amount > MAX_SUPPLY || env.ledger().timestamp() > u64::MAX / 1_000 {
            return true;
        }

        let key = HubAssetKey {
            hub_id: 0,
            asset: env.current_contract_address(),
        };
        let before = LiquidityPool::get_sync_data(env.clone(), key.clone()).state;

        let result = production_supply(
            env.clone(),
            PoolSupplyEntry {
                action: PoolAction {
                    hub_asset: key.clone(),
                    amount,
                    position: ScaledPositionRaw { scaled_amount: 0 },
                },
            },
        );
        let after = LiquidityPool::get_sync_data(env.clone(), key).state;
        let expected_scaled = amount * NATIVE_TO_SCALED;
        if result.len() != 1 {
            return false;
        }
        let mutation = result.get_unchecked(0);

        before.supplied == 0
            && before.borrowed == 0
            && before.revenue == 0
            && before.cash == 0
            && before.supply_index == RAY
            && before.borrow_index == RAY
            && before.last_timestamp == env.ledger().timestamp() * 1_000
            && mutation.position.scaled_amount == expected_scaled
            && mutation.actual_amount == amount
            && mutation.asset_decimals == ASSET_DECIMALS
            && mutation.market_index.supply_index == RAY
            && mutation.market_index.borrow_index == RAY
            && after.supplied == expected_scaled
            && after.cash == amount
            && after.borrowed == before.borrowed
            && after.revenue == before.revenue
            && after.supply_index == before.supply_index
            && after.borrow_index == before.borrow_index
            && after.last_timestamp == before.last_timestamp
    }

    /// Concrete witness that cannot pass by skipping a premise.
    pub fn test_supply_one_unit(env: Env) -> bool {
        env.ledger().timestamp() <= u64::MAX / 1_000 && Self::test_supply_exact(env, 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Ledger, LedgerInfo};

    #[test]
    fn actual_supply_reaches_small_and_scaling_boundary_states() {
        for amount in [1, 10_000_000, MAX_SUPPLY - 1, MAX_SUPPLY] {
            assert!(amount > 0 && amount <= MAX_SUPPLY);
            let env = Env::default();
            env.ledger().set(LedgerInfo {
                timestamp: 1_000,
                protocol_version: 28,
                sequence_number: 100,
                network_id: Default::default(),
                base_reserve: 10,
                min_temp_entry_ttl: 10,
                min_persistent_entry_ttl: 10,
                max_entry_ttl: 3_110_400,
            });
            env.mock_all_auths();
            let harness = env.register(PoolStateProof, ());
            let client = PoolStateProofClient::new(&env, &harness);
            client.init();
            assert!(client.test_supply_exact(&amount));
        }
    }
}
