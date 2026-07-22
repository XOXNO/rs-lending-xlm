use super::ResolvedPrice;

#[test]
fn abi_prices_preserve_present_legs_and_default_missing_ones() {
    let dual = ResolvedPrice {
        primary_price_wad: 100,
        anchor_price_wad: Some(120),
        final_price_wad: 110,
        timestamp: 1,
    };
    assert_eq!(dual.primary_and_secondary(), (100, 120));

    let single = ResolvedPrice {
        primary_price_wad: 100,
        anchor_price_wad: None,
        final_price_wad: 100,
        timestamp: 1,
    };
    assert_eq!(single.primary_and_secondary(), (100, 100));
}
