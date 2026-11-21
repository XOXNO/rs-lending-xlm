//! Reflector stand-in that answers `prices` as the public Reflector contract
//! does.
//!
//! `prices(asset, records)` returns `records + 1` rounds on a contiguous
//! history, newest first. A missing round is filled with the round before it.
//! A round stamped after the ledger time is refused. `MockReflector` returns
//! `records` samples instead.

use soroban_sdk::{contract, contractimpl, contracttype, Env, Map, Symbol, Vec};

pub use common::oracle::providers::reflector::{ReflectorAsset, ReflectorPriceData};

const DECIMALS: u32 = 14;
const WAD_TO_RAW: i128 = 10_000;
const TTL_LEDGERS: u32 = 3_000_000;

#[contracttype]
pub enum RoundKey {
    Resolution,
    LastRound,
    Rounds(ReflectorAsset),
}

#[contract]
pub struct RoundReflector;

impl RoundReflector {
    fn rounds(env: &Env, asset: &ReflectorAsset) -> Map<u64, i128> {
        env.storage()
            .instance()
            .get(&RoundKey::Rounds(asset.clone()))
            .unwrap_or_else(|| Map::new(env))
    }

    fn last_round(env: &Env) -> Option<u64> {
        env.storage().instance().get(&RoundKey::LastRound)
    }

    /// The asset's newest round at or before `slot`.
    fn round_at_or_before(rounds: &Map<u64, i128>, slot: u64) -> Option<ReflectorPriceData> {
        let timestamp = rounds.keys().iter().rev().find(|round| *round <= slot)?;
        let price = rounds.get(timestamp)?;
        Some(ReflectorPriceData { price, timestamp })
    }
}

#[contractimpl]
impl RoundReflector {
    pub fn __constructor(env: Env, resolution: u32) {
        env.storage()
            .instance()
            .set(&RoundKey::Resolution, &resolution);
    }

    /// Stores `price_wad` for `asset` in the round stamped `timestamp`
    /// (seconds). Panics on a round stamped after the ledger time, off the
    /// resolution grid, or older than the last round.
    pub fn publish(env: Env, asset: ReflectorAsset, timestamp: u64, price_wad: i128) {
        let resolution = u64::from(Self::resolution(env.clone()));
        let last = Self::last_round(&env).unwrap_or(0);
        assert!(
            timestamp <= env.ledger().timestamp(),
            "round is stamped after the ledger time"
        );
        assert!(
            timestamp.is_multiple_of(resolution),
            "round is off the resolution grid"
        );
        assert!(timestamp >= last, "round is older than the last round");

        let mut rounds = Self::rounds(&env, &asset);
        rounds.set(timestamp, price_wad / WAD_TO_RAW);
        let store = env.storage().instance();
        store.set(&RoundKey::Rounds(asset), &rounds);
        store.set(&RoundKey::LastRound, &timestamp);
        store.extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
    }

    pub fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    pub fn decimals(_env: Env) -> u32 {
        DECIMALS
    }

    pub fn resolution(env: Env) -> u32 {
        env.storage().instance().get(&RoundKey::Resolution).unwrap()
    }

    /// The asset's price in the last round, if that round carries one.
    pub fn lastprice(env: Env, asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        let timestamp = Self::last_round(&env)?;
        let price = Self::rounds(&env, &asset).get(timestamp)?;
        Some(ReflectorPriceData { price, timestamp })
    }

    pub fn prices(
        env: Env,
        asset: ReflectorAsset,
        records: u32,
    ) -> Option<Vec<ReflectorPriceData>> {
        let last = Self::last_round(&env)?;
        let resolution = u64::from(Self::resolution(env.clone()));
        let rounds = Self::rounds(&env, &asset);
        let mut out: Vec<ReflectorPriceData> = Vec::new(&env);
        for back in 0..=u64::from(records) {
            let Some(slot) = last.checked_sub(back * resolution) else {
                break;
            };
            let Some(round) = Self::round_at_or_before(&rounds, slot) else {
                break;
            };
            if out
                .last()
                .is_none_or(|newer| newer.timestamp != round.timestamp)
            {
                out.push_back(round);
            }
        }
        (!out.is_empty()).then_some(out)
    }
}
