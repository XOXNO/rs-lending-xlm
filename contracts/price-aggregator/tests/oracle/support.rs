//! Shared fixtures for the `compose`, `price`, `status`, `config`, `storage`,
//! and `providers` unit-test trees: mock RedStone feed registration,
//! single/dual/quoted/TWAP oracle configs, a contract-frame helper, and six
//! Reflector stubs — one that reports no last price, one that reports a fixed
//! price, one that reports a two-sample TWAP history, one that reports an empty
//! TWAP window, one whose price is large enough to overflow a quoted reprice,
//! and one whose reads revert.
//!
//! Included once, as `crate::test_support`, via `#[path]` on `lib.rs`. Those
//! test trees are siblings, not ancestor/descendant, so none can own this file
//! directly without the others reloading the same source a second time; each
//! instead `use`s these items from the shared crate-root module.

use common::errors::OracleError;
use common::oracle::providers::reflector::{ReflectorAsset, ReflectorOracle, ReflectorPriceData};
use mock_redstone::{MockRedStonePriceFeed, MockRedStonePriceFeedClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, panic_with_error, Address, Env, Symbol, Vec};

use crate::PriceAggregator;

/// Price scale every Reflector fixture declares, matched by the stubs below.
const REFLECTOR_DECIMALS: u32 = 14;

/// One WAD unit expressed at [`REFLECTOR_DECIMALS`], so a spot read of it
/// normalizes to exactly `WAD`.
const REFLECTOR_ONE_RAW: i128 = 100_000_000_000_000;

/// Age of the newer [`TwapReflector`] sample, in seconds before the ledger clock.
pub(crate) const TWAP_NEWER_AGE_SECS: u64 = 100;

/// Age of the older [`TwapReflector`] sample, in seconds before the ledger clock.
/// A TWAP observation dates itself to this one, not the newer sample.
pub(crate) const TWAP_OLDER_AGE_SECS: u64 = 200;

/// Samples [`TwapReflector`] reports, raw at [`REFLECTOR_DECIMALS`]. Distinct
/// values, so their mean is neither of them.
const TWAP_SAMPLES_RAW: [i128; 2] = [REFLECTOR_ONE_RAW, 3 * REFLECTOR_ONE_RAW];

/// Price [`HugeReflector`] reports, raw at [`REFLECTOR_DECIMALS`]. Normalizes to
/// `1e29` WAD — well inside `i128`, so it survives the read — but squaring it
/// through a quoted reprice needs `1e40`, which does not fit.
const REFLECTOR_HUGE_RAW: i128 = 10i128.pow(25);

/// Runs `body` in a contract frame, which persistent storage and every lookup
/// built on it (stored oracle configs, quote resolution) require.
pub(crate) fn in_contract<T>(env: &Env, body: impl FnOnce() -> T) -> T {
    let id = env.register(PriceAggregator, (Address::generate(env),));
    env.as_contract(&id, body)
}

pub(crate) fn register_redstone_feed(env: &Env) -> (Address, MockRedStonePriceFeedClient<'_>) {
    let id = env.register(MockRedStonePriceFeed, ());
    (id.clone(), MockRedStonePriceFeedClient::new(env, &id))
}

/// Reflector-shaped stub that always reports no last price. A genuinely
/// registered contract is required for the unreadable-leg cases: an
/// unregistered address traps with a host `InvalidAction` error, not the
/// provider's own `NoLastPrice`.
#[contract]
pub(crate) struct EmptyReflector;

#[contractimpl]
impl ReflectorOracle for EmptyReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        14
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(_env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        None
    }

    fn prices(_env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        None
    }
}

/// Reflector-shaped stub that answers a TWAP window with a history object
/// holding no samples, as against [`EmptyReflector`]'s "no history at all".
/// The two are distinct branches of the same rejection, and only this one
/// exercises the emptiness check on a history the provider did return.
#[contract]
pub(crate) struct EmptyWindowReflector;

#[contractimpl]
impl ReflectorOracle for EmptyWindowReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(_env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        None
    }

    fn prices(env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        Some(Vec::new(&env))
    }
}

/// Reflector-shaped stub that always reports one unit stamped at the current
/// ledger time. Spot-only: `prices` reports no history, since no fixture reads
/// TWAP from it.
#[contract]
pub(crate) struct PricedReflector;

#[contractimpl]
impl ReflectorOracle for PricedReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        Some(ReflectorPriceData {
            price: REFLECTOR_ONE_RAW,
            timestamp: env.ledger().timestamp(),
        })
    }

    fn prices(_env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        None
    }
}

/// Reflector-shaped stub reporting a price that normalizes cleanly on its own
/// and only overflows once it is multiplied by a second leg. Nothing in the
/// read path rejects it: it is positive, its scale is under the normalizer's
/// cap, and the upscale to WAD fits. Used to reach the reprice multiplication
/// in [`crate::providers::reflector`], which is why the sanity band a fixture
/// pairs with it has to be widened — the band would otherwise reject the quote
/// leg before the reprice runs.
#[contract]
pub(crate) struct HugeReflector;

#[contractimpl]
impl ReflectorOracle for HugeReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        Some(ReflectorPriceData {
            price: REFLECTOR_HUGE_RAW,
            timestamp: env.ledger().timestamp(),
        })
    }

    fn prices(_env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        None
    }
}

/// Reflector-shaped stub reporting a two-sample TWAP history regardless of how
/// many records the caller asks for. The samples carry distinct prices, so the
/// mean is a value neither of them holds and cannot be mistaken for one sample
/// echoed back, and distinct timestamps, so which one an observation dates
/// itself to is observable. Spot reads report nothing: no fixture reads
/// `lastprice` from it.
#[contract]
pub(crate) struct TwapReflector;

#[contractimpl]
impl ReflectorOracle for TwapReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(_env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        None
    }

    fn prices(env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        let now = env.ledger().timestamp();
        let history = Vec::from_array(
            &env,
            [
                ReflectorPriceData {
                    price: TWAP_SAMPLES_RAW[0],
                    timestamp: now.saturating_sub(TWAP_NEWER_AGE_SECS),
                },
                ReflectorPriceData {
                    price: TWAP_SAMPLES_RAW[1],
                    timestamp: now.saturating_sub(TWAP_OLDER_AGE_SECS),
                },
            ],
        );
        Some(history)
    }
}

/// Reflector-shaped stub whose price reads revert. Stands in for a Reflector
/// contract that is paused, archived, or upgraded to an incompatible interface:
/// the oracle config naming it is perfectly valid, and only the runtime call
/// fails. Reverting with a contract error (rather than trapping) is what a real
/// SEP-40 contract does, so callers see `Error(Contract, #216)`.
#[contract]
pub(crate) struct RevertingReflector;

#[contractimpl]
impl ReflectorOracle for RevertingReflector {
    fn base(env: Env) -> ReflectorAsset {
        ReflectorAsset::Other(Symbol::new(&env, "USD"))
    }

    fn decimals(_env: Env) -> u32 {
        REFLECTOR_DECIMALS
    }

    fn resolution(_env: Env) -> u32 {
        300
    }

    fn lastprice(env: Env, _asset: ReflectorAsset) -> Option<ReflectorPriceData> {
        panic_with_error!(&env, OracleError::OracleNotConfigured)
    }

    fn prices(env: Env, _asset: ReflectorAsset, _records: u32) -> Option<Vec<ReflectorPriceData>> {
        panic_with_error!(&env, OracleError::OracleNotConfigured)
    }
}
