//! Reflector SEP-40 mock for live testnet runs.
//! Stores 14-decimal prices; setters accept USD WAD.

#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReflectorAsset {
    Stellar(Address),
    Other(Symbol),
}

#[contracttype]
#[derive(Clone)]
pub struct PriceData {
    pub price: i128,
    pub timestamp: u64,
}

#[contracttype]
pub enum MockKey {
    Price(ReflectorAsset),
    Ts(ReflectorAsset),
    Base,
    Decimals,
    Resolution,
}

const WAD_TO_14_DECIMALS: i128 = 10_000;

#[contract]
pub struct MockReflectorOracle;

#[contractimpl]
impl MockReflectorOracle {
    pub fn __constructor(env: Env) {
        env.storage().instance().set(
            &MockKey::Base,
            &ReflectorAsset::Other(Symbol::new(&env, "USD")),
        );
        env.storage().instance().set(&MockKey::Decimals, &14u32);
        env.storage().instance().set(&MockKey::Resolution, &300u32);
    }

    /// Sets price at current ledger timestamp.
    pub fn set_price(env: Env, asset: ReflectorAsset, price_wad: i128) {
        let now = env.ledger().timestamp();
        Self::set_price_at(env, asset, price_wad, now);
    }

    /// Sets price with explicit timestamp for stale-price tests.
    pub fn set_price_at(env: Env, asset: ReflectorAsset, price_wad: i128, timestamp: u64) {
        let price_14 = price_wad / WAD_TO_14_DECIMALS;
        env.storage()
            .persistent()
            .set(&MockKey::Price(asset.clone()), &price_14);
        env.storage()
            .persistent()
            .set(&MockKey::Ts(asset), &timestamp);
    }

    /// Overrides only the stored timestamp for `asset`.
    pub fn set_ts(env: Env, asset: ReflectorAsset, timestamp: u64) {
        env.storage()
            .persistent()
            .set(&MockKey::Ts(asset), &timestamp);
    }

    pub fn base(env: Env) -> ReflectorAsset {
        env.storage().instance().get(&MockKey::Base).unwrap()
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage().instance().get(&MockKey::Decimals).unwrap()
    }

    pub fn resolution(env: Env) -> u32 {
        env.storage().instance().get(&MockKey::Resolution).unwrap()
    }

    pub fn lastprice(env: Env, asset: ReflectorAsset) -> Option<PriceData> {
        let price: i128 = env
            .storage()
            .persistent()
            .get(&MockKey::Price(asset.clone()))?;
        let timestamp: u64 = env.storage().persistent().get(&MockKey::Ts(asset))?;
        Some(PriceData { price, timestamp })
    }

    /// Returns repeated samples for deterministic TWAP.
    pub fn prices(env: Env, asset: ReflectorAsset, records: u32) -> Option<Vec<PriceData>> {
        let entry = Self::lastprice(env.clone(), asset)?;
        let mut out = Vec::new(&env);
        for _ in 0..records {
            out.push_back(entry.clone());
        }
        Some(out)
    }
}
