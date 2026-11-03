#![no_std]
//! Komet claims on real pool entrypoints. The production `pool` crate is
//! linked into this harness contract (the komet/pool-state pattern): the
//! harness is the pool, so a test can install a market state at
//! `PoolKey::State` with fixed and symbolic fields and then call the real
//! entrypoint in the same frame. Owner authorization always succeeds in the
//! Komet host model, events are no-ops, and `last_timestamp` is set to the
//! current ledger time, so `global_sync` takes its no-accrual path.

#[cfg(feature = "seize")]
pub mod seize;

use common::constants::RAY;
use common::types::{HubAssetKey, MarketParamsRaw, PoolKey, PoolStateRaw};
use pool::LiquidityPool;
use soroban_sdk::{contract, contractimpl, Address, Env};

pub const ASSET_DECIMALS: u32 = 7;

#[contract]
pub struct PoolEntryProof;

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

pub fn market_key(env: &Env) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: env.current_contract_address(),
    }
}

/// Installs `state` as the market's stored state (accrual-free: the caller
/// passes `last_timestamp = ledger seconds * 1000`).
pub fn install_state(env: &Env, state: &PoolStateRaw) {
    env.storage()
        .persistent()
        .set(&PoolKey::State(market_key(env)), state);
}

pub fn read_state(env: &Env) -> PoolStateRaw {
    use pool_interface::LiquidityPoolInterface;
    LiquidityPool::get_sync_data(env.clone(), market_key(env)).state
}

/// `None` when the ledger time cannot be expressed in milliseconds.
pub fn now_ms(env: &Env) -> Option<u64> {
    env.ledger().timestamp().checked_mul(1_000)
}

#[contractimpl]
impl PoolEntryProof {
    /// Komet runs initialization once, in a separate frame before each test.
    pub fn init(env: Env) {
        use pool_interface::LiquidityPoolInterface;
        let address = env.current_contract_address();
        LiquidityPool::__constructor(env.clone(), address.clone());
        LiquidityPool::create_market(env, 0, params(address));
    }

    /// Market (S, I) = (10^33, RAY), 2^64 <= position, see seize.rs.
    #[cfg(feature = "seize")]
    pub fn test_seize_bad_debt_a(env: Env, position: i128) -> bool {
        seize::bad_debt_claims(&env, 0, position)
    }

    /// Market (S, I) = (3*10^33 + 7, 1.25*RAY + 3), 2^64 <= position.
    #[cfg(feature = "seize")]
    pub fn test_seize_bad_debt_b(env: Env, position: i128) -> bool {
        seize::bad_debt_claims(&env, 1, position)
    }

    /// Zero bad debt leaves the supply index unchanged (market a).
    #[cfg(feature = "seize")]
    pub fn test_seize_zero_a(env: Env) -> bool {
        seize::zero_debt(&env, 0)
    }

    /// Zero bad debt leaves the supply index unchanged (market b).
    #[cfg(feature = "seize")]
    pub fn test_seize_zero_b(env: Env) -> bool {
        seize::zero_debt(&env, 1)
    }

    /// Deliberately false: seizing a real debt leaves the supply index unchanged.
    #[cfg(feature = "seize")]
    pub fn test_seize_wrong(env: Env) -> bool {
        seize::no_loss_wrong(&env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Ledger, LedgerInfo};

    fn client(env: &Env) -> PoolEntryProofClient<'_> {
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
        let harness = env.register(PoolEntryProof, ());
        let c = PoolEntryProofClient::new(env, &harness);
        c.init();
        c
    }

    #[cfg(feature = "seize")]
    #[test]
    fn seize_witnesses() {
        use seize::{BORROWED, MARKETS, WIDE};
        for k in 0..2 {
            let t = MARKETS[k].0 / RAY * MARKETS[k].1 + MARKETS[k].0 % RAY * MARKETS[k].1 / RAY;
            for pos in [WIDE, t / 3, t / 2 + 1, t - t / 10, t, t + 1, BORROWED] {
                let env = Env::default();
                let c = client(&env);
                let ok = if k == 0 {
                    c.test_seize_bad_debt_a(&pos)
                } else {
                    c.test_seize_bad_debt_b(&pos)
                };
                assert!(ok, "market {k} pos {pos}");
            }
        }
        for k in 0..2 {
            let env = Env::default();
            let c = client(&env);
            assert!(if k == 0 {
                c.test_seize_zero_a()
            } else {
                c.test_seize_zero_b()
            });
        }
        let env = Env::default();
        let c = client(&env);
        assert!(!c.test_seize_wrong());
    }
}
