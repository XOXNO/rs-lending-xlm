//! Failure paths of the Aquarius LP provider.
//!
//! `attest_*` runs once, at configuration time, and `read_*` re-checks the same
//! bindings on every price read, because a pool can be re-pointed or drained
//! after it was attested. The re-check arms had no tests: the stable variant
//! had none at all, and the constant-product variant only ever ran its happy
//! path.
use super::*;

use crate::registry;
use crate::test_support::{register_redstone_feed, MockAquariusPool, MockAquariusPoolClient};
use common::constants::WAD;
use common::types::{
    FeedNature, FeedSource, IndependencePolicy, MultiFeedRef, OracleTolerance, PriceSource,
    ProviderRef,
};
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{String, Symbol, Vec as SdkVec};

const SHARES: u128 = 10_000_000_000;
const RESERVE: u128 = 5_000_000_000;

struct Pool {
    pool: Address,
    share: Address,
    token_a: Address,
    token_b: Address,
}

/// A mock Aquarius pool whose share and reserve tokens are real SAC contracts,
/// so `token_decimals` resolves through an actual `decimals` entry point.
fn pool_fixture(env: &Env, kind: &str) -> Pool {
    let issuer = Address::generate(env);
    let token_a = env
        .register_stellar_asset_contract_v2(issuer.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(issuer.clone())
        .address();
    let share = env.register_stellar_asset_contract_v2(issuer).address();
    let pool = env.register(
        MockAquariusPool,
        (share.clone(), token_a.clone(), token_b.clone(), SHARES),
    );
    let client = MockAquariusPoolClient::new(env, &pool);
    client.set_pool_type(&Symbol::new(env, kind));
    client.set_reserves(&RESERVE, &RESERVE);
    Pool {
        pool,
        share,
        token_a,
        token_b,
    }
}

fn lp_source(p: &Pool) -> AquariusLpSource {
    AquariusLpSource {
        pool: p.pool.clone(),
        token_a: p.token_a.clone(),
        token_b: p.token_b.clone(),
        key_a: PriceKey::Token(p.token_a.clone()),
        key_b: PriceKey::Token(p.token_b.clone()),
        reserve_a_decimals: 7,
        reserve_b_decimals: 7,
        min_pool_value_wad: 1,
    }
}

fn lp_oracle(env: &Env, lp: &AquariusLpSource, asset_decimals: u32) -> AssetOracle {
    let mut sources = SdkVec::new(env);
    sources.push_back(PriceSource::AquariusStableLp(lp.clone()));
    AssetOracle {
        asset_decimals,
        max_price_stale_seconds: 43_200,
        sources,
        tolerance: OracleTolerance {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_524,
        },
        independence: IndependencePolicy::RequireDisjoint,
        min_sanity_price_wad: 1,
        max_sanity_price_wad: i128::MAX / 4,
    }
}

fn in_pa<T>(env: &Env, body: impl FnOnce() -> T) -> T {
    let id = env.register(crate::PriceAggregator, (Address::generate(env),));
    env.as_contract(&id, body)
}

// --- attest_stable ---------------------------------------------------------

#[test]
#[should_panic(expected = "Error(Contract, #234)")]
fn attest_stable_rejects_a_pool_that_does_not_report_itself_stable() {
    let env = Env::default();
    let p = pool_fixture(&env, "constant_product");
    let lp = lp_source(&p);
    let key = PriceKey::Token(p.share.clone());
    in_pa(&env, || {
        attest_stable(&env, &key, &lp_oracle(&env, &lp, 7), &lp)
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #234)")]
fn attest_stable_rejects_a_pool_with_no_shares_outstanding() {
    let env = Env::default();
    let issuer = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(issuer.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(issuer.clone())
        .address();
    let share = env.register_stellar_asset_contract_v2(issuer).address();
    // Zero shares: a pool nobody has minted against cannot have its share
    // priced, and dividing by it later would trap.
    let pool = env.register(
        MockAquariusPool,
        (share.clone(), token_a.clone(), token_b.clone(), 0u128),
    );
    let client = MockAquariusPoolClient::new(&env, &pool);
    client.set_pool_type(&Symbol::new(&env, "stable"));
    client.set_reserves(&RESERVE, &RESERVE);
    let p = Pool {
        pool,
        share: share.clone(),
        token_a,
        token_b,
    };
    let lp = lp_source(&p);
    let key = PriceKey::Token(share);
    in_pa(&env, || {
        attest_stable(&env, &key, &lp_oracle(&env, &lp, 7), &lp)
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #221)")]
fn attest_stable_rejects_a_share_decimals_claim_the_token_does_not_back() {
    let env = Env::default();
    let p = pool_fixture(&env, "stable");
    let lp = lp_source(&p);
    let key = PriceKey::Token(p.share.clone());
    // The SAC reports 7; the oracle claims 6. Believing the claim would scale
    // every share price by 10x.
    in_pa(&env, || {
        attest_stable(&env, &key, &lp_oracle(&env, &lp, 6), &lp)
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #220)")]
fn attest_stable_rejects_a_key_that_is_not_the_pools_share_token() {
    let env = Env::default();
    let p = pool_fixture(&env, "stable");
    let lp = lp_source(&p);
    // A different token entirely: bound_tokens refuses to bind it to the pool.
    let stranger = PriceKey::Token(p.token_a.clone());
    in_pa(&env, || {
        attest_stable(&env, &stranger, &lp_oracle(&env, &lp, 7), &lp)
    });
}

// --- read / read_stable re-checks ------------------------------------------

#[test]
fn read_refuses_a_pool_that_stopped_reporting_constant_product() {
    let env = Env::default();
    let p = pool_fixture(&env, "stable");
    let lp = lp_source(&p);
    let key = PriceKey::Token(p.share.clone());
    in_pa(&env, || {
        let mut session = Session::new(&env);
        // Attested as constant-product, now answering "stable": the read must
        // not fall through to the constant-product maths.
        assert_eq!(
            read(&mut session, &key, &lp, 7, 0).unwrap_err(),
            OracleError::NoLastPrice
        );
    });
}

#[test]
fn read_stable_refuses_a_pool_that_stopped_reporting_stable() {
    let env = Env::default();
    let p = pool_fixture(&env, "constant_product");
    let lp = lp_source(&p);
    let key = PriceKey::Token(p.share.clone());
    in_pa(&env, || {
        let mut session = Session::new(&env);
        assert_eq!(
            read_stable(&mut session, &key, &lp, 7, 0).unwrap_err(),
            OracleError::NoLastPrice
        );
    });
}

#[test]
fn read_refuses_a_share_decimals_claim_the_token_does_not_back() {
    let env = Env::default();
    let p = pool_fixture(&env, "constant_product");
    let lp = lp_source(&p);
    let key = PriceKey::Token(p.share.clone());
    in_pa(&env, || {
        let mut session = Session::new(&env);
        assert_eq!(
            read(&mut session, &key, &lp, 6, 0).unwrap_err(),
            OracleError::NoLastPrice
        );
    });
}

#[test]
fn read_refuses_a_key_that_is_not_the_pools_share_token() {
    let env = Env::default();
    let p = pool_fixture(&env, "constant_product");
    let lp = lp_source(&p);
    let stranger = PriceKey::Token(p.token_a.clone());
    in_pa(&env, || {
        let mut session = Session::new(&env);
        assert_eq!(
            read(&mut session, &stranger, &lp, 7, 0).unwrap_err(),
            OracleError::NoLastPrice
        );
    });
}

/// A pool whose bindings and type are correct but whose reserves were never
/// written, so `get_reserves` traps and the host call returns `None`. This is
/// the shape of a pool that answered at attest time and stopped answering
/// later.
fn mute_pool_fixture(env: &Env, kind: &str) -> Pool {
    let issuer = Address::generate(env);
    let token_a = env
        .register_stellar_asset_contract_v2(issuer.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(issuer.clone())
        .address();
    let share = env.register_stellar_asset_contract_v2(issuer).address();
    let pool = env.register(
        MockAquariusPool,
        (share.clone(), token_a.clone(), token_b.clone(), SHARES),
    );
    MockAquariusPoolClient::new(env, &pool).set_pool_type(&Symbol::new(env, kind));
    Pool {
        pool,
        share,
        token_a,
        token_b,
    }
}

const NOW: u64 = 1_000_000;

/// Registers a priceable oracle for each of the pool's two reserve tokens, so
/// `engine::resolve_nested` succeeds and the read gets far enough to ask the
/// pool for its reserves. Without this the read stops earlier, on
/// `OracleNotConfigured`, and proves nothing about the reserve path.
fn price_both_legs(env: &Env, p: &Pool) {
    env.ledger().set_timestamp(NOW);
    let (adapter, client) = register_redstone_feed(env);
    let ts_ms = NOW * 1_000;
    for (id, token) in [("A", &p.token_a), ("B", &p.token_b)] {
        client.set_price_data(&String::from_str(env, id), &WAD, &ts_ms, &ts_ms);
        let mut sources = SdkVec::new(env);
        sources.push_back(PriceSource::Feed(FeedSource {
            provider: ProviderRef::RedStone(MultiFeedRef {
                contract: adapter.clone(),
                feed_id: String::from_str(env, id),
                nature: FeedNature::Fundamental,
            }),
            decimals: 8,
            max_stale_seconds: 43_200,
        }));
        registry::store_oracle(
            env,
            &PriceKey::Token((*token).clone()),
            &AssetOracle {
                asset_decimals: 7,
                max_price_stale_seconds: 43_200,
                sources,
                tolerance: OracleTolerance {
                    upper_ratio_bps: 10_500,
                    lower_ratio_bps: 9_524,
                },
                independence: IndependencePolicy::RequireDisjoint,
                min_sanity_price_wad: WAD / 2,
                max_sanity_price_wad: 2 * WAD,
            },
        );
    }
}

#[test]
fn read_refuses_a_pool_that_stopped_answering_for_its_reserves() {
    let env = Env::default();
    let p = mute_pool_fixture(&env, "constant_product");
    let lp = lp_source(&p);
    let key = PriceKey::Token(p.share.clone());
    in_pa(&env, || {
        price_both_legs(&env, &p);
        let mut session = Session::new(&env);
        // Both legs price fine; it is the pool itself that has gone quiet, and
        // a share price cannot be derived without reserves.
        assert_eq!(
            read(&mut session, &key, &lp, 7, 0).unwrap_err(),
            OracleError::NoLastPrice
        );
    });
}

#[test]
fn read_stable_refuses_a_pool_that_stopped_answering_for_its_reserves() {
    let env = Env::default();
    let p = mute_pool_fixture(&env, "stable");
    let lp = lp_source(&p);
    let key = PriceKey::Token(p.share.clone());
    in_pa(&env, || {
        price_both_legs(&env, &p);
        let mut session = Session::new(&env);
        assert_eq!(
            read_stable(&mut session, &key, &lp, 7, 0).unwrap_err(),
            OracleError::NoLastPrice
        );
    });
}

#[test]
fn read_stable_refuses_a_share_decimals_claim_the_token_does_not_back() {
    let env = Env::default();
    let p = pool_fixture(&env, "stable");
    let lp = lp_source(&p);
    let key = PriceKey::Token(p.share.clone());
    in_pa(&env, || {
        let mut session = Session::new(&env);
        assert_eq!(
            read_stable(&mut session, &key, &lp, 6, 0).unwrap_err(),
            OracleError::NoLastPrice
        );
    });
}

#[test]
fn read_stable_refuses_a_key_that_is_not_the_pools_share_token() {
    let env = Env::default();
    let p = pool_fixture(&env, "stable");
    let lp = lp_source(&p);
    let stranger = PriceKey::Token(p.token_a.clone());
    in_pa(&env, || {
        let mut session = Session::new(&env);
        assert_eq!(
            read_stable(&mut session, &stranger, &lp, 7, 0).unwrap_err(),
            OracleError::NoLastPrice
        );
    });
}
