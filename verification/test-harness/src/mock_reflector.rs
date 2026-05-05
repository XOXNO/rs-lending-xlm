//! Minimal SEP-40 mock for integration tests.
//! Replaces oracle_adapter test helpers.
//! Prices are stored at 14 decimals (matching real Reflector), so
//! the controller's rescale_to_wad() path is exercised correctly.

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};

#[contracttype]
#[derive(Clone)]
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
}

#[contract]
pub struct MockReflector;

#[contractimpl]
impl MockReflector {
    /// Test helper: set price (WAD input is converted to 14-decimal storage
    /// so controller's rescale_to_wad() is fully exercised in tests).
    pub fn set_price(env: Env, asset: Address, price_wad: i128) {
        let price_14 = price_wad / 10_000; // WAD(18) -> 14 decimals
        env.storage()
            .temporary()
            .set(&MockKey::Spot(asset), &(price_14, env.ledger().timestamp()));
    }

    /// Test helper: set a separate TWAP ("safe") price for tolerance testing.
    pub fn set_twap_price(env: Env, asset: Address, price_wad: i128) {
        let price_14 = price_wad / 10_000;
        env.storage()
            .temporary()
            .set(&MockKey::Twap(asset), &(price_14, env.ledger().timestamp()));
    }

    pub fn decimals(_env: Env) -> u32 {
        14
    }
    pub fn resolution(_env: Env) -> u32 {
        300
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
        let twap_pd = match env.storage().temporary().get(&MockKey::Twap(addr)) {
            Some((price, timestamp)) => PriceData { price, timestamp },
            None => Self::lastprice(env.clone(), asset)?,
        };

        let mut out = Vec::new(&env);
        for _ in 0..records {
            out.push_back(twap_pd.clone());
        }
        Some(out)
    }
}
