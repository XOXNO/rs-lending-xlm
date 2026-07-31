use common::constants::WAD;
use common::errors::OracleError;
use common::oracle::providers::redstone::{RedStonePriceData, REDSTONE_DECIMALS};
use common::oracle::providers::reflector::{ReflectorAsset, ReflectorOracle, ReflectorPriceData};
use mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{
    contract, contractimpl, contracttype, panic_with_error, Address, Env, String, Symbol, Vec, U256,
};

use crate::PriceAggregator;

const WAD_TO_REDSTONE: i128 = 10_000_000_000;

const REFLECTOR_DECIMALS: u32 = 14;

const REFLECTOR_ONE_RAW: i128 = 100_000_000_000_000;

pub(crate) const REFLECTOR_RESOLUTION_SECS: u32 = 300;

pub(crate) const TWAP_NEWER_AGE_SECS: u64 = 100;

pub(crate) const TWAP_OLDER_AGE_SECS: u64 = TWAP_NEWER_AGE_SECS + 300;

pub(crate) const TWAP_TIGHT_SPACING_SECS: u64 = REFLECTOR_RESOLUTION_SECS as u64 - 1;

const TWAP_SAMPLES_RAW: [i128; 2] = [REFLECTOR_ONE_RAW, 3 * REFLECTOR_ONE_RAW];

const REFLECTOR_HUGE_RAW: i128 = 10i128.pow(25);

pub(crate) fn in_contract<T>(env: &Env, body: impl FnOnce() -> T) -> T {
    let id = env.register(PriceAggregator, (Address::generate(env),));
    env.as_contract(&id, body)
}

pub(crate) fn register_redstone_feed(env: &Env) -> (Address, MockRedStonePriceFeedClient<'_>) {
    let id = env.register(MockRedStonePriceFeed, ());
    (id.clone(), MockRedStonePriceFeedClient::new(env, &id))
}

#[contract]
pub(crate) struct EmptyReflector;

#[contractimpl]
impl ReflectorOracle for EmptyReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        14
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(_env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        None
    }

    fn prices(_env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        None
    }
}

#[contract]
pub(crate) struct EmptyWindowReflector;

#[contractimpl]
impl ReflectorOracle for EmptyWindowReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(_env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        None
    }

    fn prices(env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        Some(Vec::new(&env))
    }
}

#[contract]
pub(crate) struct PricedReflector;

#[contractimpl]
impl ReflectorOracle for PricedReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        Some(ReflectorPriceData {
            price: REFLECTOR_ONE_RAW,
            timestamp: env.ledger().timestamp(),
        })
    }

    fn prices(_env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        None
    }
}

#[contract]
pub(crate) struct CountingReflector;

#[contractimpl]
impl ReflectorOracle for CountingReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        let key = Symbol::new(&env, "reads");
        let reads = env.storage().instance().get::<_, i128>(&key).unwrap_or(0) + 1;
        env.storage().instance().set(&key, &reads);
        Some(ReflectorPriceData {
            price: reads * REFLECTOR_ONE_RAW,
            timestamp: env.ledger().timestamp(),
        })
    }

    fn prices(_env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        None
    }
}

#[contract]
pub(crate) struct HugeReflector;

#[contractimpl]
impl ReflectorOracle for HugeReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        Some(ReflectorPriceData {
            price: REFLECTOR_HUGE_RAW,
            timestamp: env.ledger().timestamp(),
        })
    }

    fn prices(_env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        None
    }
}

#[contract]
pub(crate) struct TwapReflector;

#[contractimpl]
impl ReflectorOracle for TwapReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        REFLECTOR_RESOLUTION_SECS
    }

    fn lastprice(_env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        None
    }

    fn prices(env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        Some(twap_history(&env))
    }
}

fn twap_history(env: &Env) -> Vec<ReflectorPriceData> {
    let now = env.ledger().timestamp();
    Vec::from_array(
        env,
        [
            ReflectorPriceData {
                price: TWAP_SAMPLES_RAW[0],
                timestamp: now.saturating_sub(TWAP_NEWER_AGE_SECS),
            },
            ReflectorPriceData {
                price: TWAP_SAMPLES_RAW[1],
                timestamp: now.saturating_sub(TWAP_OLDER_AGE_SECS),
            },
        ],
    )
}

#[contract]
pub(crate) struct LongHistoryReflector;

#[contractimpl]
impl ReflectorOracle for LongHistoryReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        REFLECTOR_RESOLUTION_SECS
    }

    fn lastprice(_env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        None
    }

    fn prices(env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        let now = env.ledger().timestamp();
        let sample = |age: u64| ReflectorPriceData {
            price: REFLECTOR_ONE_RAW,
            timestamp: now.saturating_sub(age),
        };
        Some(Vec::from_array(
            &env,
            [
                sample(TWAP_NEWER_AGE_SECS),
                sample(TWAP_OLDER_AGE_SECS),
                sample(TWAP_OLDER_AGE_SECS + u64::from(REFLECTOR_RESOLUTION_SECS)),
            ],
        ))
    }
}

#[contract]
pub(crate) struct TightWindowReflector;

#[contractimpl]
impl ReflectorOracle for TightWindowReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        REFLECTOR_RESOLUTION_SECS
    }

    fn lastprice(_env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        None
    }

    fn prices(env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        let now = env.ledger().timestamp();
        let sample = |age: u64| ReflectorPriceData {
            price: REFLECTOR_ONE_RAW,
            timestamp: now.saturating_sub(age),
        };
        Some(Vec::from_array(
            &env,
            [
                sample(TWAP_NEWER_AGE_SECS),
                sample(TWAP_NEWER_AGE_SECS + TWAP_TIGHT_SPACING_SECS),
            ],
        ))
    }
}

#[contract]
pub(crate) struct NonUsdReflector;

#[contractimpl]
impl ReflectorOracle for NonUsdReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "EUR"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        REFLECTOR_RESOLUTION_SECS
    }

    fn lastprice(_env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        None
    }

    fn prices(env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        Some(twap_history(&env))
    }
}

#[contract]
pub(crate) struct RevertingReflector;

#[contractimpl]
impl ReflectorOracle for RevertingReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        panic_with_error!(&env, OracleError::OracleNotConfigured)
    }

    fn prices(env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        panic_with_error!(&env, OracleError::OracleNotConfigured)
    }
}

#[contracttype]
#[derive(Clone)]
pub(crate) enum CountKey {
    Price(String),

    Single,

    Bulk,

    LastBatch,

    Short,
}

#[contract]
pub(crate) struct CountingRedStoneAdapter;

#[contractimpl]
impl CountingRedStoneAdapter {
    pub fn set_price(env: Env, feed_id: String, price_wad: i128) {
        let now_ms = env.ledger().timestamp() * 1_000;
        let data = RedStonePriceData {
            price: U256::from_u128(&env, (price_wad / WAD_TO_REDSTONE) as u128),
            package_timestamp: now_ms,
            write_timestamp: now_ms,
        };
        env.storage()
            .persistent()
            .set(&CountKey::Price(feed_id), &data);
    }

    pub fn set_short(env: Env, short: bool) {
        env.storage().instance().set(&CountKey::Short, &short);
    }

    pub fn counts(env: Env) -> (u32, u32, u32) {
        (
            Self::counter(&env, CountKey::Single),
            Self::counter(&env, CountKey::Bulk),
            Self::counter(&env, CountKey::LastBatch),
        )
    }

    pub fn read_price_data_for_feed(env: Env, feed_id: String) -> RedStonePriceData {
        Self::bump(&env, CountKey::Single);
        Self::payload(&env, &feed_id)
    }

    pub fn read_price_data(env: Env, feed_ids: Vec<String>) -> Vec<RedStonePriceData> {
        Self::bump(&env, CountKey::Bulk);
        env.storage()
            .instance()
            .set(&CountKey::LastBatch, &feed_ids.len());

        let short: bool = env
            .storage()
            .instance()
            .get(&CountKey::Short)
            .unwrap_or(false);
        let serve = if short {
            feed_ids.len().saturating_sub(1)
        } else {
            feed_ids.len()
        };

        let mut out = Vec::new(&env);
        for (index, feed_id) in feed_ids.iter().enumerate() {
            if (index as u32) < serve {
                out.push_back(Self::payload(&env, &feed_id));
            }
        }
        out
    }

    fn payload(env: &Env, feed_id: &String) -> RedStonePriceData {
        env.storage()
            .persistent()
            .get(&CountKey::Price(feed_id.clone()))
            .unwrap_or_else(|| panic_with_error!(env, OracleError::NoLastPrice))
    }

    fn counter(env: &Env, key: CountKey) -> u32 {
        env.storage().instance().get(&key).unwrap_or(0)
    }

    fn bump(env: &Env, key: CountKey) {
        let next = Self::counter(env, key.clone()) + 1;
        env.storage().instance().set(&key, &next);
    }
}

#[contract]
pub(crate) struct StubXoxnoAdapter;

#[contractimpl]
impl StubXoxnoAdapter {
    pub fn decimals(_env: Env) -> u32 {
        REDSTONE_DECIMALS
    }

    pub fn max_submission_age_seconds(_env: Env) -> u64 {
        XOXNO_SUBMISSION_WINDOW_SECS
    }

    pub fn max_stale_seconds(_env: Env) -> u64 {
        XOXNO_SUBMISSION_WINDOW_SECS
    }

    pub fn max_relative_skew_seconds(_env: Env) -> u64 {
        60
    }

    pub fn read_price_data_for_feed(env: Env, _feed_id: String) -> RedStonePriceData {
        let now_ms = env.ledger().timestamp() * 1_000;
        RedStonePriceData {
            price: U256::from_u128(&env, (WAD / WAD_TO_REDSTONE) as u128),
            package_timestamp: now_ms,
            write_timestamp: now_ms,
        }
    }
}

pub(crate) const XOXNO_SUBMISSION_WINDOW_SECS: u64 = 1_800;
