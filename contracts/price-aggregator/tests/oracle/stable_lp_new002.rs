//! Regression tests for NEW-002 (Specula run 3 CR-6 and LP-4), end to end
//! through the price aggregator.
//!
//! A stable LP is listed through `set_oracle` while its pool is balanced, as
//! governance would list it. The pool then drains to one side, as it does
//! when one coin depegs, and the contract entry points are called the way
//! the controller, keepers and frontends call them.
//!
//! Mounted as a child of the registry tests so it reuses their stable-LP
//! listing fixtures.

use super::*;
use crate::test_support::MockAquariusPoolClient;
use crate::PriceAggregatorClient;

/// One token at 7 decimals, in raw units.
const TOKEN: u128 = 10_000_000;

struct ListedStableLp {
    aggregator: Address,
    pool: Address,
    key: PriceKey,
    leg_a: PriceKey,
}

/// Lists a stable LP whose reserves and share supply are `shares` tokens
/// each, so its price at listing is 2 WAD (inside the fixture's [1, 3] WAD
/// sanity band). `set_oracle` probes the pool, so it must be balanced here.
fn list_balanced_stable_lp(env: &Env, shares_tokens: u128, amp: u128) -> ListedStableLp {
    env.ledger().set_timestamp(1_000_000);
    let aggregator = env.register(PriceAggregator, (Address::generate(env),));
    let units = shares_tokens * TOKEN;
    env.as_contract(&aggregator, || {
        let (pool, share, token_a, token_b) = lp_fixture(env, "stable", units, units, units);
        MockAquariusPoolClient::new(env, &pool).set_amp(&amp);
        let (key_a, key_b) = dollar_underlyings(env, &token_a, &token_b);
        let oracle = stable_lp_oracle(env, &pool, &token_a, &token_b, key_a.clone(), key_b);
        let key = PriceKey::Token(share);
        set_oracle(env, key.clone(), oracle);
        ListedStableLp {
            aggregator: aggregator.clone(),
            pool,
            key,
            leg_a: key_a,
        }
    })
}

/// Defect 1. The listed pool drains to 10^10 : 10^7 tokens (1,000:1) at
/// amp 2. `D` is about 2.362e9 tokens and the supply is 1.2e9 shares, so the
/// fair price is about 1.968 WAD. Before the fix `prices` reverts with
/// `InvalidPrice` (`Error(Contract, #217)`): every valuation of an account
/// holding this LP, liquidation included, reverts with it.
#[test]
fn new002_drained_stable_pool_still_prices() {
    let env = Env::default();
    let lp = list_balanced_stable_lp(&env, 1_200_000_000, 2);
    MockAquariusPoolClient::new(&env, &lp.pool)
        .set_reserves(&(10_000_000_000 * TOKEN), &(10_000_000 * TOKEN));

    let client = PriceAggregatorClient::new(&env, &lp.aggregator);
    let keys = Vec::from_array(&env, [lp.key.clone()]);
    let price = client.prices(&keys).get_unchecked(lp.key).price_wad;
    let expected = 1_968_514_416_602_111_980;
    assert!(
        (price - expected).abs() <= expected / 1_000_000,
        "LP price {price}, expected about {expected}"
    );
}

/// Defect 2. The listed pool drains to 10^15 : 1,000 tokens at amp 100. The
/// first Newton step overflows `U256` and the host traps. `quotes` is
/// documented to mark an unusable price instead of panicking, so the batch
/// must still return, with the healthy key priced and the LP key either
/// priced or marked `InvalidPrice`. Before the fix the whole call reverts,
/// and with it the controller view `get_all_market_indexes_detailed`.
#[test]
fn new002_quotes_survives_an_overflowing_stable_pool() {
    let env = Env::default();
    let lp = list_balanced_stable_lp(&env, 500_000_000_000, 100);
    MockAquariusPoolClient::new(&env, &lp.pool)
        .set_reserves(&(1_000_000_000_000_000 * TOKEN), &(1_000 * TOKEN));

    let client = PriceAggregatorClient::new(&env, &lp.aggregator);
    let keys = Vec::from_array(&env, [lp.leg_a.clone(), lp.key.clone()]);
    let quotes = match client.try_quotes(&keys) {
        Ok(Ok(quotes)) => quotes,
        other => panic!("quotes reverted for the whole batch: {other:?}"),
    };
    assert!(
        quotes.get_unchecked(lp.leg_a).valid,
        "the healthy key lost its price"
    );
    let status = quotes.get_unchecked(lp.key);
    assert!(
        status.valid || status.error_code == Some(OracleError::InvalidPrice as u32),
        "LP key status {status:?}"
    );
}
