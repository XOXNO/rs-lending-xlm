use crate::context::LendingTest;
use crate::oracle::config::{
    tight_single_source_band, DEFAULT_MAX_SANITY_PRICE_WAD, DEFAULT_MIN_SANITY_PRICE_WAD,
};
use crate::presets::TolerancePreset;
use controller::types::{
    AssetOracle, FeedSource, OracleAssetRef, OracleReadMode, PriceKey, PriceSource, ProviderRef,
    ReflectorFeedRef,
};
use soroban_sdk::{Address, Vec};

impl LendingTest {
    pub fn set_price(&mut self, asset_name: &str, price_wad: i128) {
        let asset = self.record_price(asset_name, price_wad);
        self.sync_sanity_band_for_test_price(&asset, price_wad);
    }

    pub fn set_price_keeping_sanity_band(&mut self, asset_name: &str, price_wad: i128) {
        self.record_price(asset_name, price_wad);
    }

    /// Records `price_wad` as the market's price and pushes it to the mock
    /// feeds. Returns the market's asset address.
    fn record_price(&mut self, asset_name: &str, price_wad: i128) -> Address {
        let market = self
            .markets
            .get_mut(asset_name)
            .unwrap_or_else(|| panic!("market '{}' not found", asset_name));
        let asset = market.asset.clone();
        market.price_wad = price_wad;
        self.push_oracle_prices(&asset, price_wad);
        asset
    }

    pub fn refresh_oracle_prices(&self) {
        for market in self.markets.values() {
            self.push_oracle_prices(&market.asset, market.price_wad);
        }
    }

    pub(crate) fn push_oracle_prices(&self, asset: &Address, price_wad: i128) {
        let mock_reflector = self.mock_reflector_client();
        mock_reflector.set_price(asset, &price_wad);
        mock_reflector.set_twap_price(asset, &price_wad);
    }

    fn sync_sanity_band_for_test_price(&self, asset: &Address, price_wad: i128) {
        if price_wad <= 0 {
            return;
        }
        let key = PriceKey::Token(asset.clone());
        let Some(mut oracle) = self.price_agg_client().oracle(&key) else {
            return;
        };
        if oracle.is_dual() {
            oracle.min_sanity_price_wad = DEFAULT_MIN_SANITY_PRICE_WAD;
            oracle.max_sanity_price_wad = DEFAULT_MAX_SANITY_PRICE_WAD;
        } else {
            let (min_wad, max_wad) = tight_single_source_band(price_wad);
            oracle.min_sanity_price_wad = min_wad;
            oracle.max_sanity_price_wad = max_wad;
        }
        self.price_agg_client().seed_oracle(&key, &oracle);
    }

    pub fn set_prices(&mut self, pairs: &[(&str, i128)]) {
        for (asset_name, price_wad) in pairs {
            self.set_price(asset_name, *price_wad);
        }
    }

    pub fn set_tolerance(&self, asset_name: &str, preset: TolerancePreset) {
        let asset = self.resolve_asset(asset_name);
        use governance::op::{AdminOperation, EditToleranceArgs};
        self.gov_client().execute_immediate(
            &self.admin,
            &AdminOperation::EditOracleTolerance(EditToleranceArgs {
                key: PriceKey::Token(asset),
                tolerance: preset.tolerance_bps,
            }),
        );
    }

    pub fn configure_market_oracle(&self, asset: &Address, oracle: &AssetOracle) {
        use governance::op::{AdminOperation, ConfigureAssetOracleArgs};
        self.gov_client().execute_immediate(
            &self.admin,
            &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
                key: PriceKey::Token(asset.clone()),
                oracle: oracle.clone(),
            }),
        );
    }

    pub fn set_safe_price(&self, asset_name: &str, price_wad: i128) {
        let asset = self.resolve_market(asset_name).asset.clone();
        self.mock_reflector_client()
            .set_twap_price(&asset, &price_wad);
    }

    /// Seeds the sanity band directly, bypassing governance.
    ///
    /// Setup only. Since F-3 the governed `set_sanity_band` may only tighten,
    /// so a test that needs to *start* from a band wider than the market's
    /// `tight_single_source_band` default cannot get there through the
    /// governed path.
    pub fn seed_sanity_band(&self, asset_name: &str, min_wad: i128, max_wad: i128) {
        let key = PriceKey::Token(self.resolve_asset(asset_name));
        let mut oracle = self.price_agg_client().oracle(&key).unwrap();
        oracle.min_sanity_price_wad = min_wad;
        oracle.max_sanity_price_wad = max_wad;
        self.price_agg_client().seed_oracle(&key, &oracle);
    }

    pub fn set_oracle_single_spot(&self, asset_name: &str) {
        let asset = self.resolve_asset(asset_name);
        let price_wad = self.resolve_market(asset_name).price_wad;
        let key = PriceKey::Token(asset.clone());
        let mut oracle = self.price_agg_client().oracle(&key).unwrap();
        oracle.sources = single(
            &self.env,
            with_read_mode(&oracle.sources.get_unchecked(0), OracleReadMode::Spot),
        );
        if price_wad > 0 {
            let (min_wad, max_wad) = tight_single_source_band(price_wad);
            oracle.min_sanity_price_wad = min_wad;
            oracle.max_sanity_price_wad = max_wad;
        }
        self.price_agg_client().seed_oracle(&key, &oracle);
    }

    pub fn set_oracle_primary_anchor(&self, asset_name: &str) {
        let asset = self.resolve_asset(asset_name);
        let key = PriceKey::Token(asset.clone());
        let mut oracle = self.price_agg_client().oracle(&key).unwrap();
        let first = oracle.sources.get_unchecked(0);
        let mut sources = Vec::new(&self.env);
        sources.push_back(with_read_mode(&first, OracleReadMode::Twap(3)));
        sources.push_back(with_read_mode(&first, OracleReadMode::Spot));
        oracle.sources = sources;

        oracle.min_sanity_price_wad = DEFAULT_MIN_SANITY_PRICE_WAD;
        oracle.max_sanity_price_wad = DEFAULT_MAX_SANITY_PRICE_WAD;
        self.price_agg_client().seed_oracle(&key, &oracle);
    }

    pub fn enable_dual_source_oracle(&self, asset_name: &str) {
        self.set_oracle_primary_anchor(asset_name);
    }

    pub fn set_dual_oracle_dex_anchor(&self, asset_name: &str, dex_oracle: Address) {
        let asset = self.resolve_asset(asset_name);
        let key = PriceKey::Token(asset.clone());
        let mut oracle = self.price_agg_client().oracle(&key).unwrap();

        let mut sources = Vec::new(&self.env);
        sources.push_back(with_read_mode(
            &oracle.sources.get_unchecked(0),
            OracleReadMode::Twap(3),
        ));
        sources.push_back(PriceSource::Feed(FeedSource {
            provider: ProviderRef::Reflector(ReflectorFeedRef {
                contract: dex_oracle,
                asset: OracleAssetRef::Stellar(asset.clone()),
                read_mode: OracleReadMode::Spot,
            }),
            decimals: 14,
            max_stale_seconds: oracle.max_price_stale_seconds,
        }));
        oracle.sources = sources;
        oracle.min_sanity_price_wad = DEFAULT_MIN_SANITY_PRICE_WAD;
        oracle.max_sanity_price_wad = DEFAULT_MAX_SANITY_PRICE_WAD;
        self.price_agg_client().seed_oracle(&key, &oracle);
    }
}

fn single(env: &soroban_sdk::Env, source: PriceSource) -> Vec<PriceSource> {
    let mut sources = Vec::new(env);
    sources.push_back(source);
    sources
}

fn with_read_mode(source: &PriceSource, read_mode: OracleReadMode) -> PriceSource {
    match source {
        PriceSource::Feed(feed) => match &feed.provider {
            ProviderRef::Reflector(reflector) => PriceSource::Feed(FeedSource {
                provider: ProviderRef::Reflector(ReflectorFeedRef {
                    read_mode,
                    ..reflector.clone()
                }),
                ..feed.clone()
            }),
            _ => source.clone(),
        },
        _ => source.clone(),
    }
}
