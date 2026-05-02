use common::types::{
    AssetConfig, ExchangeSource, MarketConfig, MarketStatus, OraclePriceFluctuation,
    OracleProviderConfig, OracleType, ReflectorAssetKind,
};
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
    ) -> Option<Vec<TestReflectorPriceData>> {
        let mut out = Vec::new(&env);
        let spot = Self::lastprice(env.clone(), asset)?;
        for _ in 0..records {
            out.push_back(spot.clone());
        }
        Some(out)
    }
}

pub fn test_market_config(
    env: &Env,
    asset: &Address,
    pool: &Address,
    asset_config: AssetConfig,
) -> MarketConfig {
    MarketConfig {
        status: MarketStatus::Active,
        asset_config,
        pool_address: pool.clone(),
        oracle_config: OracleProviderConfig {
            base_asset: asset.clone(),
            oracle_type: OracleType::Normal,
            exchange_source: ExchangeSource::SpotOnly,
            asset_decimals: 7,
            tolerance: OraclePriceFluctuation {
                first_upper_ratio_bps: 10_200,
                first_lower_ratio_bps: 9_800,
                last_upper_ratio_bps: 11_000,
                last_lower_ratio_bps: 9_000,
            },
            max_price_stale_seconds: 900,
        },
        cex_oracle: None,
        cex_asset_kind: ReflectorAssetKind::Stellar,
        cex_symbol: Symbol::new(env, ""),
        cex_decimals: 0,
        dex_oracle: None,
        dex_asset_kind: ReflectorAssetKind::Stellar,
        dex_symbol: Symbol::new(env, ""),
        dex_decimals: 0,
        twap_records: 0,
    }
}
