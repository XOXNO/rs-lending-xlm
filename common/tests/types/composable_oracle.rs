use super::*;
use soroban_sdk::{testutils::Address as _, Env, String, Symbol};

fn multi_feed(env: &Env, contract: &Address, xoxno: bool) -> FeedSource {
    let feed = MultiFeedRef {
        contract: contract.clone(),
        feed_id: String::from_str(env, "BTC"),
        nature: FeedNature::Fundamental,
    };
    FeedSource {
        provider: if xoxno {
            ProviderRef::Xoxno(feed)
        } else {
            ProviderRef::RedStone(feed)
        },
        decimals: 8,
        max_stale_seconds: 3_600,
    }
}

#[test]
fn provider_variants_expose_their_contract_and_nature() {
    let env = Env::default();
    let contract = Address::generate(&env);
    for xoxno in [false, true] {
        let feed = multi_feed(&env, &contract, xoxno);
        assert_eq!(feed.provider.contract(), &contract);
        assert_eq!(feed.provider.nature(), FeedNature::Fundamental);
        assert!(!feed.provider.is_smoothed());
    }
}

#[test]
fn reflector_twap_is_smoothed() {
    let env = Env::default();
    let provider = ProviderRef::Reflector(ReflectorFeedRef {
        contract: Address::generate(&env),
        asset: OracleAssetRef::Symbol(Symbol::new(&env, "BTC")),
        read_mode: OracleReadMode::Twap(3),
    });
    assert!(provider.is_smoothed());
    assert_eq!(provider.nature(), FeedNature::Market);
}

#[test]
fn asset_oracle_identifies_aquarius_lp() {
    let env = Env::default();
    let source = PriceSource::AquariusLp(AquariusLpSource {
        pool: Address::generate(&env),
        plane: Address::generate(&env),
        token_a: Address::generate(&env),
        token_b: Address::generate(&env),
        key_a: PriceKey::Ref(Symbol::new(&env, "A")),
        key_b: PriceKey::Ref(Symbol::new(&env, "B")),
        reserve_a_decimals: 7,
        reserve_b_decimals: 7,
        min_pool_value_wad: 1,
    });
    let oracle = AssetOracle {
        asset_decimals: 7,
        max_price_stale_seconds: 3_600,
        sources: Vec::from_array(&env, [source]),
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_500,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: 1,
        max_sanity_price_wad: i128::MAX / 2,
    };
    assert!(oracle.has_aquarius_lp_source());
    assert!(!oracle.is_dual());
}
