use super::{read_mode_parts, EventOracleProvider};
use common::types::{
    AssetOracleConfig, OracleAssetRef, OracleReadMode, OracleSourceConfig,
    OracleSourceConfigOption, OracleStrategy, OracleTolerance, RedStoneSourceConfig,
    ReflectorBase, ReflectorSourceConfig,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String};

#[test]
fn read_mode_parts_maps_spot_and_twap() {
    assert_eq!(read_mode_parts(&OracleReadMode::Spot), (0, 0));
    assert_eq!(read_mode_parts(&OracleReadMode::Twap(12)), (1, 12));
}

// Anchored Reflector source: every anchor_* leg of the flat event must carry
// the source values (not the absent-anchor zero defaults).
#[test]
fn from_oracle_flattens_anchored_reflector_source() {
    let env = Env::default();
    let asset = Address::generate(&env);
    let quote = Address::generate(&env);
    let primary_feed = Address::generate(&env);
    let anchor_oracle = Address::generate(&env);

    let config = AssetOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_200,
            lower_ratio_bps: 9_800,
        },
        strategy: OracleStrategy::PrimaryWithAnchor,
        primary: OracleSourceConfig::RedStone(RedStoneSourceConfig {
            contract: primary_feed.clone(),
            feed_id: String::from_str(&env, "BTC/USD"),
            decimals: 8,
            max_stale_seconds: 600,
        }),
        anchor: OracleSourceConfigOption::Some(OracleSourceConfig::Reflector(
            ReflectorSourceConfig {
                contract: anchor_oracle.clone(),
                asset: OracleAssetRef::Stellar(asset.clone()),
                read_mode: OracleReadMode::Twap(12),
                decimals: 14,
                resolution_seconds: 300,
                base: ReflectorBase::Quoted(quote.clone()),
            },
        )),
        min_sanity_price_wad: 1,
        max_sanity_price_wad: 2,
    };

    let event = EventOracleProvider::from_oracle(&asset, &config);

    assert_eq!(event.base_token_id, asset);
    assert_eq!(event.strategy, OracleStrategy::PrimaryWithAnchor as u32);
    assert_eq!(event.primary_contract, primary_feed);
    assert_eq!(event.primary_max_stale_seconds, 600);

    assert_eq!(event.anchor_contract, Some(anchor_oracle));
    assert_eq!(event.anchor_asset, Some(asset));
    assert_eq!(event.anchor_quote_token, Some(quote));
    assert_eq!(event.anchor_read_mode, 1);
    assert_eq!(event.anchor_twap_records, 12);
    assert_eq!(event.anchor_decimals, 14);
    assert_eq!(event.anchor_resolution_seconds, 300);
    // Reflector legs carry the market-level staleness limit.
    assert_eq!(event.anchor_max_stale_seconds, 900);
}
