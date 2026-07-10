use super::ResolvedOracleComponents;

#[test]
fn abi_prices_preserve_present_legs_and_default_missing_ones() {
    let dual = ResolvedOracleComponents {
        primary_price_wad: Some(100),
        anchor_price_wad: Some(120),
        final_price_wad: 110,
        timestamp: 1,
    };
    assert_eq!(dual.to_abi_prices(), (100, 120));

    let single = ResolvedOracleComponents {
        primary_price_wad: Some(100),
        anchor_price_wad: None,
        final_price_wad: 100,
        timestamp: 1,
    };
    assert_eq!(single.to_abi_prices(), (100, 100));
}
