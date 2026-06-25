use super::*;

// Direct coverage for `OracleAssetRef::Symbol` mapping in `to_reflector_asset`.
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

// `OracleAssetRef::String` is unsupported on Reflector and panics with
// `InvalidOracleTokenType`.
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
