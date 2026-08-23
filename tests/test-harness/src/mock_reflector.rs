use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};

pub use common::oracle::providers::reflector::{ReflectorAsset, ReflectorPriceData};

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
    pub fn set_price(env: Env, asset: Address, price_wad: i128) {
        let timestamp = env.ledger().timestamp();
        Self::set_price_at(env, asset, price_wad, timestamp);
    }

    pub fn set_price_at(env: Env, asset: Address, price_wad: i128, timestamp: u64) {
        let price_14 = price_wad / 10_000;
        env.storage()
            .temporary()
            .set(&MockKey::Spot(asset), &(price_14, timestamp));
    }

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
            .set(&MockKey::Base, &ReflectorAsset::Other(symbol));
    }

    pub fn set_base_stellar(env: Env, asset: Address) {
        env.storage()
            .temporary()
            .set(&MockKey::Base, &ReflectorAsset::Stellar(asset));
    }

    pub fn set_decimals(env: Env, decimals: u32) {
        env.storage().temporary().set(&MockKey::Decimals, &decimals);
    }

    pub fn set_resolution(env: Env, resolution: u32) {
        env.storage()
            .temporary()
            .set(&MockKey::Resolution, &resolution);
    }

    pub fn set_twap_history_mode(env: Env, asset: Address, mode: u32) {
        env.storage()
            .temporary()
            .set(&MockKey::TwapHistoryMode(asset), &mode);
    }

    pub fn base(env: Env) -> ReflectorAsset {
        env.storage()
            .temporary()
            .get(&MockKey::Base)
            .unwrap_or_else(|| ReflectorAsset::Other(Symbol::new(&env, "USD")))
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

    pub fn lastprice(env: Env, asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        let addr = match asset {
            ReflectorAsset::Stellar(a) => a,
            _ => return None,
        };
        let (price, timestamp): (i128, u64) =
            env.storage().temporary().get(&MockKey::Spot(addr))?;
        Some(ReflectorPriceData { price, timestamp })
    }

    pub fn prices(
        env: Env,
        asset: ReflectorAsset,
        records: u32,
    ) -> Option<Vec<ReflectorPriceData>> {
        let addr = match asset.clone() {
            ReflectorAsset::Stellar(a) => a,
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
            Some((price, timestamp)) => ReflectorPriceData { price, timestamp },
            None => Self::lastprice(env.clone(), asset)?,
        };

        let mut out = Vec::new(&env);
        let len = match mode {
            3 => records.saturating_sub(2).max(1),
            // Exact-minimum window: a TWAP must use its full configured window, so
            // `records` observations is the smallest history that is accepted.
            6 => records,
            _ => records,
        };
        for i in 0..len {
            let mut entry = twap_pd.clone();
            let resolution = u64::from(Self::resolution(env.clone()));
            entry.timestamp = match mode {
                7 => twap_pd.timestamp,
                8 => twap_pd.timestamp.saturating_sub(u64::from(i)),
                9 if i == 1 => twap_pd.timestamp.saturating_sub(1),
                _ => twap_pd
                    .timestamp
                    .saturating_sub(u64::from(i).saturating_mul(resolution)),
            };

            if mode == 4 && i == 0 {
                entry.price = 0;
            }

            if mode == 5 && i.saturating_add(1) == len {
                entry.timestamp = 1;
            }
            out.push_back(entry);
        }
        Some(out)
    }
}
