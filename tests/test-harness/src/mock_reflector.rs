//! Minimal SEP-40 mock for integration tests.
//! Replaces oracle_adapter test helpers.
//! Prices are stored at 14 decimals (matching real Reflector), so
//! the controller's rescale_to_wad() path is exercised correctly.

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Sep40Asset {
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
    Spot(Address),
    Twap(Address),
    Base,
    Decimals,
    Resolution,
    TwapHistoryMode(Address),
}

#[contract]
pub struct MockReflector;

#[contractimpl]
impl MockReflector {
    /// Test helper: set price (WAD input is converted to 14-decimal storage
    /// so controller's rescale_to_wad() is fully exercised in tests).
    pub fn set_price(env: Env, asset: Address, price_wad: i128) {
        let timestamp = env.ledger().timestamp();
        Self::set_price_at(env, asset, price_wad, timestamp);
    }

    pub fn set_price_at(env: Env, asset: Address, price_wad: i128, timestamp: u64) {
        let price_14 = price_wad / 10_000; // WAD(18) -> 14 decimals
        env.storage()
            .temporary()
            .set(&MockKey::Spot(asset), &(price_14, timestamp));
    }

    /// Test helper: set a separate TWAP ("safe") price for tolerance testing.
    pub fn set_twap_price(env: Env, asset: Address, price_wad: i128) {
        let timestamp = env.ledger().timestamp();
        Self::set_twap_price_at(env, asset, price_wad, timestamp);
    }

    pub fn set_twap_price_at(env: Env, asset: Address, price_wad: i128, timestamp: u64) {
        let price_14 = price_wad / 10_000;
        env.storage()
            .temporary()
            .set(&MockKey::Twap(asset), &(price_14, timestamp));
    }

    pub fn set_base_other(env: Env, symbol: Symbol) {
        env.storage()
            .temporary()
            .set(&MockKey::Base, &Sep40Asset::Other(symbol));
    }

    pub fn set_base_stellar(env: Env, asset: Address) {
        env.storage()
            .temporary()
            .set(&MockKey::Base, &Sep40Asset::Stellar(asset));
    }

    pub fn set_decimals(env: Env, decimals: u32) {
        env.storage().temporary().set(&MockKey::Decimals, &decimals);
    }

    pub fn set_resolution(env: Env, resolution: u32) {
        env.storage()
            .temporary()
            .set(&MockKey::Resolution, &resolution);
    }

    /// 0 = normal, 1 = None, 2 = empty, 3 = insufficient,
    /// 4 = invalid-price (one entry has price <= 0),
    /// 5 = stale (oldest timestamp is far in the past),
    /// 6 = exactly the minimum accepted observation count.
    pub fn set_twap_history_mode(env: Env, asset: Address, mode: u32) {
        env.storage()
            .temporary()
            .set(&MockKey::TwapHistoryMode(asset), &mode);
    }

    pub fn base(env: Env) -> Sep40Asset {
        env.storage()
            .temporary()
            .get(&MockKey::Base)
            .unwrap_or_else(|| Sep40Asset::Other(Symbol::new(&env, "USD")))
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage()
            .temporary()
            .get(&MockKey::Decimals)
            .unwrap_or(14)
    }
    pub fn resolution(env: Env) -> u32 {
        env.storage()
            .temporary()
            .get(&MockKey::Resolution)
            .unwrap_or(300)
    }

    pub fn lastprice(env: Env, asset: Sep40Asset) -> Option<PriceData> {
        let addr = match asset {
            Sep40Asset::Stellar(a) => a,
            _ => return None,
        };
        let (price, timestamp): (i128, u64) =
            env.storage().temporary().get(&MockKey::Spot(addr))?;
        Some(PriceData { price, timestamp })
    }

    pub fn prices(env: Env, asset: Sep40Asset, records: u32) -> Option<Vec<PriceData>> {
        let addr = match asset.clone() {
            Sep40Asset::Stellar(a) => a,
            _ => return None,
        };
        let mode: u32 = env
            .storage()
            .temporary()
            .get(&MockKey::TwapHistoryMode(addr.clone()))
            .unwrap_or(0);
        if mode == 1 {
            return None;
        }
        if mode == 2 {
            return Some(Vec::new(&env));
        }
        let twap_pd = match env.storage().temporary().get(&MockKey::Twap(addr)) {
            Some((price, timestamp)) => PriceData { price, timestamp },
            None => Self::lastprice(env.clone(), asset)?,
        };

        let mut out = Vec::new(&env);
        let len = match mode {
            3 => records.saturating_sub(2).max(1),
            6 => common::oracle::providers::reflector::min_twap_observations(records),
            _ => records,
        };
        for i in 0..len {
            let mut entry = twap_pd.clone();
            // Mode 4: poison the first entry with a non-positive price so
            // the reader's `has_invalid_price` path fires.
            if mode == 4 && i == 0 {
                entry.price = 0;
            }
            // Mode 5: backdate the oldest entry so the staleness check in
            // `read_twap` rejects the whole window.
            if mode == 5 && i == 0 {
                entry.timestamp = 1;
            }
            out.push_back(entry);
        }
        Some(out)
    }
}
