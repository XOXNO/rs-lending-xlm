use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, Address, Env};

#[test]
fn test_to_reflector_asset_symbol_maps_to_other() {
    let env = Env::default();
    let symbol = soroban_sdk::Symbol::new(&env, "USD");
    let asset = OracleAssetRef::Symbol(symbol.clone());
    let result = to_reflector_asset(&env, &asset);
    match result {
        ReflectorAsset::Other(s) => assert_eq!(s, symbol),
        _ => panic!("expected ReflectorAsset::Other"),
    }
}

#[test]
#[should_panic]
fn test_to_reflector_asset_string_panics() {
    let env = Env::default();
    let asset = OracleAssetRef::String(soroban_sdk::String::from_str(&env, "USDC"));
    let _ = to_reflector_asset(&env, &asset);
}

fn pd(env: &soroban_sdk::Env, price: i128) -> ReflectorPriceData {
    let _ = env;
    ReflectorPriceData {
        price,
        timestamp: 0,
    }
}

#[test]
fn try_twap_mean_price_averages_positive_samples() {
    let env = Env::default();
    let history = soroban_sdk::vec![&env, pd(&env, 100), pd(&env, 200), pd(&env, 300)];
    assert_eq!(try_twap_mean_price(&history), Some(200));
}

#[test]
fn try_twap_mean_price_rejects_non_positive_sample() {
    let env = Env::default();

    let zero = soroban_sdk::vec![&env, pd(&env, 100), pd(&env, 0)];
    assert_eq!(try_twap_mean_price(&zero), None);
    let negative = soroban_sdk::vec![&env, pd(&env, 100), pd(&env, -1)];
    assert_eq!(try_twap_mean_price(&negative), None);
}

#[test]
fn try_twap_mean_price_softens_overflow_and_empty() {
    let env = Env::default();

    let overflow = soroban_sdk::vec![&env, pd(&env, i128::MAX), pd(&env, i128::MAX)];
    assert_eq!(try_twap_mean_price(&overflow), None);

    let empty: soroban_sdk::Vec<ReflectorPriceData> = soroban_sdk::Vec::new(&env);
    assert_eq!(try_twap_mean_price(&empty), None);
}

#[contract]
struct StubReflector;

#[contractimpl]
impl StubReflector {
    pub fn resolution(_env: Env) -> u32 {
        300_000
    }

    pub fn lastprice(_env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        Some(ReflectorPriceData {
            price: 12,
            timestamp: 99,
        })
    }

    pub fn prices(
        env: Env,
        _asset: ReflectorAsset,
        _records: u32,
    ) -> Option<soroban_sdk::Vec<ReflectorPriceData>> {
        Some(soroban_sdk::vec![
            &env,
            ReflectorPriceData {
                price: 10,
                timestamp: 1,
            },
            ReflectorPriceData {
                price: 20,
                timestamp: 2,
            },
        ])
    }
}

#[test]
fn missing_oracle_try_helpers_fail_open() {
    let env = Env::default();
    let missing = Address::generate(&env);
    let asset = ReflectorAsset::Other(soroban_sdk::Symbol::new(&env, "USD"));
    assert!(reflector_last_price(&env, &missing, &asset).is_none());
    assert!(reflector_prices(&env, &missing, &asset, 2).is_none());
    assert!(try_reflector_resolution(&env, &missing).is_none());
}

#[test]
fn live_oracle_returns_non_default_values() {
    let env = Env::default();
    let oracle = env.register(StubReflector, ());
    let asset = ReflectorAsset::Other(soroban_sdk::Symbol::new(&env, "USD"));
    assert_eq!(try_reflector_resolution(&env, &oracle), Some(300_000));
    let last = reflector_last_price(&env, &oracle, &asset).expect("last");
    assert_eq!(last.price, 12);
    assert_eq!(last.timestamp, 99);
    let history = reflector_prices(&env, &oracle, &asset, 2).expect("prices");
    assert_eq!(history.len(), 2);
    assert_eq!(history.get_unchecked(0).price, 10);
    assert_eq!(history.get_unchecked(1).price, 20);
}
