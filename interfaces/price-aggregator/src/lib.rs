#![no_std]

use common::types::{AssetOracle, OracleTolerance, PriceFeedRaw, PriceKey, PriceStatus};
use soroban_sdk::{contractclient, Address, Env, Map, Vec};

#[contractclient(name = "PriceAggregatorClient")]
pub trait PriceAggregatorInterface {
    fn get_owner(env: Env) -> Option<Address>;

    fn prices(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw>;

    fn price(env: Env, key: PriceKey) -> PriceFeedRaw;

    fn quote(env: Env, key: PriceKey) -> PriceStatus;

    fn quotes(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceStatus>;

    fn price_spread(env: Env, key: PriceKey) -> (i128, i128);

    fn oracle(env: Env, key: PriceKey) -> Option<AssetOracle>;

    fn set_oracle(env: Env, key: PriceKey, oracle: AssetOracle);

    fn set_sanity_band(env: Env, key: PriceKey, min_wad: i128, max_wad: i128);

    fn set_tolerance(env: Env, key: PriceKey, tolerance: OracleTolerance);
}
