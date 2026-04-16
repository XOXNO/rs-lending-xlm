use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};

#[contracttype]
#[derive(Clone)]
pub enum TestReflectorAsset {
    Stellar(Address),
    Other(Symbol),
}

#[contracttype]
#[derive(Clone)]
pub struct TestReflectorPriceData {
    pub price: i128,
    pub timestamp: u64,
}

#[contract]
pub struct TestReflector;

#[contractimpl]
impl TestReflector {
    pub fn set_spot(env: Env, asset: TestReflectorAsset, price: i128, timestamp: u64) {
        env.storage()
            .temporary()
            .set(&asset, &TestReflectorPriceData { price, timestamp });
    }

    pub fn decimals(_env: Env) -> u32 {
        14
    }

    pub fn resolution(_env: Env) -> u32 {
        300
    }

    pub fn lastprice(env: Env, asset: TestReflectorAsset) -> Option<TestReflectorPriceData> {
        env.storage().temporary().get(&asset)
    }

    pub fn prices(
        env: Env,
        asset: TestReflectorAsset,
        records: u32,
    ) -> Vec<Option<TestReflectorPriceData>> {
        let mut out = Vec::new(&env);
        let spot = Self::lastprice(env.clone(), asset);
        for _ in 0..records {
            out.push_back(spot.clone());
        }
        out
    }
}
