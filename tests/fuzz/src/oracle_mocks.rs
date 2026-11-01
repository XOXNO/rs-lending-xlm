//! Provider mocks whose every returned field is set by the fuzz input, including
//! malformed values, missing data and panics.

use common::oracle::providers::chainlink::ChainlinkRound;
use common::oracle::providers::redstone::RedStonePriceData;
use common::oracle::providers::reflector::{ReflectorAsset, ReflectorPriceData};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, Address, BytesN, Env,
    String, Symbol, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum MockError {
    Unavailable = 1,
    WrongDecimals = 2,
}

#[contracttype]
#[derive(Clone)]
enum Key {
    Panics,
    Base,
    Decimals,
    Resolution,
    Spot,
    History,
    Feed(String),
    ShortBulk,
    MaxAge,
    Round,
    Shares,
    Reserves,
    Tokens,
    PoolType,
    Share,
    Amp,
}

fn set<V: soroban_sdk::IntoVal<Env, soroban_sdk::Val>>(env: &Env, key: Key, value: &V) {
    env.storage().instance().set(&key, value);
}

fn get<V: soroban_sdk::TryFromVal<Env, soroban_sdk::Val>>(env: &Env, key: Key) -> Option<V> {
    env.storage().instance().get(&key)
}

fn guard(env: &Env) {
    if get::<bool>(env, Key::Panics).unwrap_or(false) {
        panic_with_error!(env, MockError::Unavailable);
    }
}

#[contract]
pub struct FuzzReflector;

#[contractimpl]
impl FuzzReflector {
    pub fn setup(
        env: Env,
        base: ReflectorAsset,
        decimals: u32,
        resolution: u32,
        spot: Option<ReflectorPriceData>,
        history: Option<Vec<ReflectorPriceData>>,
        panics: bool,
    ) {
        set(&env, Key::Base, &base);
        set(&env, Key::Decimals, &decimals);
        set(&env, Key::Resolution, &resolution);
        set(&env, Key::Spot, &spot);
        set(&env, Key::History, &history);
        set(&env, Key::Panics, &panics);
    }

    pub fn base(env: Env) -> ReflectorAsset {
        guard(&env);
        get(&env, Key::Base).unwrap()
    }

    pub fn decimals(env: Env) -> u32 {
        guard(&env);
        get(&env, Key::Decimals).unwrap()
    }

    pub fn resolution(env: Env) -> u32 {
        guard(&env);
        get(&env, Key::Resolution).unwrap()
    }

    pub fn lastprice(env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        guard(&env);
        get::<Option<ReflectorPriceData>>(&env, Key::Spot).flatten()
    }

    pub fn prices(
        env: Env,
        _asset: ReflectorAsset,
        _records: u32,
    ) -> Option<Vec<ReflectorPriceData>> {
        guard(&env);
        get::<Option<Vec<ReflectorPriceData>>>(&env, Key::History).flatten()
    }
}

/// Serves both RedStone and Xoxno references: same read interface, plus the
/// `decimals` and `max_submission_age_seconds` reads Xoxno attestation makes.
#[contract]
pub struct FuzzMultiFeed;

#[contractimpl]
impl FuzzMultiFeed {
    pub fn setup(env: Env, decimals: u32, max_age: u64, short_bulk: bool, panics: bool) {
        set(&env, Key::Decimals, &decimals);
        set(&env, Key::MaxAge, &max_age);
        set(&env, Key::ShortBulk, &short_bulk);
        set(&env, Key::Panics, &panics);
    }

    pub fn set_feed(env: Env, feed_id: String, data: Option<RedStonePriceData>) {
        set(&env, Key::Feed(feed_id), &data);
    }

    pub fn read_price_data_for_feed(
        env: Env,
        feed_id: String,
    ) -> Result<RedStonePriceData, MockError> {
        guard(&env);
        get::<Option<RedStonePriceData>>(&env, Key::Feed(feed_id))
            .flatten()
            .ok_or(MockError::Unavailable)
    }

    pub fn read_price_data(
        env: Env,
        feed_ids: Vec<String>,
    ) -> Result<Vec<RedStonePriceData>, MockError> {
        guard(&env);
        let mut out = Vec::new(&env);
        if get::<bool>(&env, Key::ShortBulk).unwrap_or(false) {
            return Ok(out);
        }
        for id in feed_ids.iter() {
            out.push_back(Self::read_price_data_for_feed(env.clone(), id)?);
        }
        Ok(out)
    }

    pub fn decimals(env: Env) -> u32 {
        guard(&env);
        get(&env, Key::Decimals).unwrap()
    }

    pub fn max_submission_age_seconds(env: Env) -> u64 {
        guard(&env);
        get(&env, Key::MaxAge).unwrap()
    }
}

#[contract]
pub struct FuzzChainlink;

#[contractimpl]
impl FuzzChainlink {
    pub fn setup(env: Env, round: Option<ChainlinkRound>, decimals: u32, panics: bool) {
        set(&env, Key::Round, &round);
        set(&env, Key::Decimals, &decimals);
        set(&env, Key::Panics, &panics);
    }

    pub fn latest_round(
        env: Env,
        _data_id: BytesN<32>,
        decimals: u32,
    ) -> Result<ChainlinkRound, MockError> {
        guard(&env);
        if get::<u32>(&env, Key::Decimals) != Some(decimals) {
            return Err(MockError::WrongDecimals);
        }
        get::<Option<ChainlinkRound>>(&env, Key::Round)
            .flatten()
            .ok_or(MockError::Unavailable)
    }
}

#[contract]
pub struct FuzzAquarius;

#[contractimpl]
impl FuzzAquarius {
    #[allow(clippy::too_many_arguments)]
    pub fn setup(
        env: Env,
        shares: u128,
        reserves: Vec<u128>,
        tokens: Vec<Address>,
        pool_type: Symbol,
        share: Address,
        amp: u128,
        panics: bool,
    ) {
        set(&env, Key::Shares, &shares);
        set(&env, Key::Reserves, &reserves);
        set(&env, Key::Tokens, &tokens);
        set(&env, Key::PoolType, &pool_type);
        set(&env, Key::Share, &share);
        set(&env, Key::Amp, &amp);
        set(&env, Key::Panics, &panics);
    }

    pub fn get_total_shares(env: Env) -> u128 {
        guard(&env);
        get(&env, Key::Shares).unwrap()
    }

    pub fn get_reserves(env: Env) -> Vec<u128> {
        guard(&env);
        get(&env, Key::Reserves).unwrap()
    }

    pub fn get_tokens(env: Env) -> Vec<Address> {
        guard(&env);
        get(&env, Key::Tokens).unwrap()
    }

    pub fn pool_type(env: Env) -> Symbol {
        guard(&env);
        get(&env, Key::PoolType).unwrap()
    }

    pub fn share_id(env: Env) -> Address {
        guard(&env);
        get(&env, Key::Share).unwrap()
    }

    pub fn a(env: Env) -> u128 {
        guard(&env);
        get(&env, Key::Amp).unwrap()
    }
}

/// Exposes only `decimals`, the one token read LP attestation makes.
#[contract]
pub struct FuzzToken;

#[contractimpl]
impl FuzzToken {
    pub fn setup(env: Env, decimals: u32) {
        set(&env, Key::Decimals, &decimals);
    }

    pub fn decimals(env: Env) -> u32 {
        get(&env, Key::Decimals).unwrap()
    }
}
