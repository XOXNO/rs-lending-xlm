use super::*;

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

#[test]
fn test_min_twap_observations_clamps_and_rounds_up() {
    assert_eq!(min_twap_observations(0), 2);
    assert_eq!(min_twap_observations(1), 2);
    assert_eq!(min_twap_observations(2), 2);
    assert_eq!(min_twap_observations(3), 2);
    assert_eq!(min_twap_observations(4), 2);
    assert_eq!(min_twap_observations(5), 3);
    assert_eq!(min_twap_observations(12), 6);
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

#[test]
fn twap_mean_price_divides_the_sum_by_the_sample_count() {
    let env = Env::default();
    let history = soroban_sdk::vec![&env, pd(&env, 100), pd(&env, 200), pd(&env, 300)];
    assert_eq!(twap_mean_price(&env, &history), 200);
}

#[test]
fn twap_mean_price_matches_the_soft_variant() {
    let env = Env::default();
    let history = soroban_sdk::vec![&env, pd(&env, 7), pd(&env, 11)];
    assert_eq!(
        twap_mean_price(&env, &history),
        try_twap_mean_price(&history).unwrap()
    );
}

#[test]
#[should_panic(expected = "#217")]
fn twap_mean_price_rejects_non_positive_sample() {
    let env = Env::default();
    let history = soroban_sdk::vec![&env, pd(&env, 100), pd(&env, 0)];
    let _ = twap_mean_price(&env, &history);
}

#[test]
#[should_panic(expected = "#33")]
fn twap_mean_price_panics_on_overflow() {
    let env = Env::default();
    let history = soroban_sdk::vec![&env, pd(&env, i128::MAX), pd(&env, i128::MAX)];
    let _ = twap_mean_price(&env, &history);
}
