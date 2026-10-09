//! Specula oracle MC-1, re-derived at HEAD: a `ConfigureAssetOracle` that is
//! Ready when the ORACLE role narrows a sanity band stays executable, by any
//! address and unsigned, and `set_oracle` overwrites the narrowed band with the
//! band the operation carried at proposal time. `RelaxSpokeAssetFlags` carries
//! `expected_epoch` for the spoke-flag ratchet; the band ratchet has no such
//! binding.
//!
//! The first two tests pin the behaviour at HEAD (they fail once an epoch guard
//! lands, which is the intended signal). The third pins the operational
//! mitigation: cancelling the pending operation keeps the narrowing.

use controller::types::{AssetOracle, PriceKey};
use governance_interface::{AdminOperation, ConfigureAssetOracleArgs, OperationState};
use soroban_sdk::testutils::Ledger as _;
use soroban_sdk::{BytesN, Env, IntoVal, Symbol, Val, Vec};
use test_harness::errors::GenericError;
use test_harness::{
    assert_contract_error, errors, eth_preset, tolerance_band, usd, usdc_preset, LendingTest,
    ALICE, DEFAULT_TOLERANCE,
};

/// `governance::constants::TIMELOCK_OPERATION_GRACE_LEDGERS`.
const GRACE: u32 = 120_960;
const SET_ORACLE: &str = "set_oracle";
/// A routine, non-band change for the "old" operation to carry.
const ROUTINE_TOLERANCE_BPS: u32 = 400;

fn salt(env: &Env, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(env, &[byte; 32])
}

fn flatten<T, C>(
    result: Result<Result<T, C>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<(), soroban_sdk::Error> {
    match result {
        Ok(_) => Ok(()),
        Err(Ok(err)) => Err(err),
        Err(Err(invoke)) => panic!("expected contract error, got {invoke:?}"),
    }
}

fn advance(t: &LendingTest, n: u32) {
    t.env.ledger().with_mut(|l| l.sequence_number += n);
}

/// Advances `n` ledgers, then re-pushes each listed market's current price.
/// The mock feeds live in temporary storage, which the test ledger expires
/// across a timelock delay; the push keeps the band untouched.
fn advance_with_live_feeds(t: &mut LendingTest, n: u32, markets: &[&str]) {
    advance(t, n);
    for name in markets {
        let price_wad = t.resolve_market(name).price_wad;
        t.set_price_keeping_sanity_band(name, price_wad);
    }
}

fn usdc_key(t: &LendingTest) -> PriceKey {
    PriceKey::Token(t.resolve_asset("USDC"))
}

/// Band wide enough that a later narrowing can exclude the live $1 price while
/// staying at least `MIN_SANITY_BAND_BPS` wide: [0.95, 1.05].
fn wide_band() -> (i128, i128) {
    (usd(1) * 95 / 100, usd(1) * 105 / 100)
}

/// Narrowing inside `wide_band` that excludes the live $1 price: [1.005, 1.05].
fn narrowed_band() -> (i128, i128) {
    (usd(1) * 1005 / 1000, usd(1) * 105 / 100)
}

fn live_band(t: &LendingTest) -> (i128, i128) {
    let oracle = t.market_oracle_config(&t.resolve_asset("USDC"));
    (oracle.min_sanity_price_wad, oracle.max_sanity_price_wad)
}

/// The live USDC oracle config with only the band and tolerance replaced, so
/// the proposal keeps the sources and read mode the market was listed with.
fn usdc_config(t: &LendingTest, band: (i128, i128), tolerance_bps: u32) -> AssetOracle {
    let mut cfg = t.market_oracle_config(&t.resolve_asset("USDC"));
    cfg.min_sanity_price_wad = band.0;
    cfg.max_sanity_price_wad = band.1;
    cfg.tolerance = tolerance_band(&t.env, tolerance_bps);
    cfg
}

/// The call tuple the timelock scheduled for a `ConfigureAssetOracle`.
fn set_oracle_args(t: &LendingTest, resolved: &AssetOracle) -> Vec<Val> {
    soroban_sdk::vec![
        &t.env,
        usdc_key(t).into_val(&t.env),
        resolved.clone().into_val(&t.env),
    ]
}

/// Proposes `ConfigureAssetOracle` as the owner and returns the operation id
/// and the resolved oracle the schedule hashed.
fn propose_oracle(t: &LendingTest, cfg: &AssetOracle, salt_byte: u8) -> (BytesN<32>, AssetOracle) {
    let gov = t.gov_iface_client();
    let resolved = gov.resolve_asset_oracle(&usdc_key(t), cfg);
    let id = gov.propose(
        &t.admin(),
        &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
            key: usdc_key(t),
            oracle: cfg.clone(),
        }),
        &salt(&t.env, salt_byte),
    );
    (id, resolved)
}

/// Executes the scheduled `set_oracle` with `executor = None` and no signature
/// at all, as any address can.
fn execute_as_stranger(
    t: &LendingTest,
    resolved: &AssetOracle,
    salt_byte: u8,
) -> Result<(), soroban_sdk::Error> {
    let gov = governance_interface::GovernanceClient::new(&t.env, &t.governance);
    t.env.set_auths(&[]);
    let result = flatten(gov.try_execute(
        &None,
        &t.price_aggregator,
        &Symbol::new(&t.env, SET_ORACLE),
        &set_oracle_args(t, resolved),
        &salt(&t.env, 0),
        &salt(&t.env, salt_byte),
    ));
    t.env.mock_all_auths_allowing_non_root_auth();
    result
}

/// Widens the live USDC band to `wide_band` through the production timelock
/// (propose, wait, execute), with no testing-only shortcut.
fn widen_band_through_timelock(t: &mut LendingTest) {
    let cfg = usdc_config(t, wide_band(), DEFAULT_TOLERANCE.tolerance_bps);
    let (id, resolved) = propose_oracle(t, &cfg, 1);
    let delay = t.gov_iface_client().get_min_delay();
    advance_with_live_feeds(t, delay, &["USDC", "ETH"]);
    assert_eq!(
        t.gov_iface_client().get_operation_state(&id),
        OperationState::Ready
    );
    t.gov_iface_client().execute(
        &Some(t.admin()),
        &t.price_aggregator,
        &Symbol::new(&t.env, SET_ORACLE),
        &set_oracle_args(t, &resolved),
        &salt(&t.env, 0),
        &salt(&t.env, 1),
    );
    assert_eq!(live_band(t), wide_band());
}

/// MC-1 as stated: a Ready owner proposal that carries the pre-incident band is
/// executed, unsigned, after the ORACLE role narrowed the band to exclude the
/// live price. The narrowing is gone and the controller lends against the
/// price it had blocked.
#[test]
fn stale_configure_asset_oracle_executed_unsigned_undoes_an_oracle_role_narrowing() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    widen_band_through_timelock(&mut t);
    t.supply(ALICE, "USDC", 10_000.0);

    // Routine owner proposal: tolerance change only, band copied from the live
    // config at proposal time. It becomes Ready and is left unexecuted.
    let routine = usdc_config(&t, wide_band(), ROUTINE_TOLERANCE_BPS);
    let (id, resolved) = propose_oracle(&t, &routine, 2);
    let delay = t.gov_iface_client().get_min_delay();
    advance_with_live_feeds(&mut t, delay, &["USDC", "ETH"]);
    assert_eq!(
        t.gov_iface_client().get_operation_state(&id),
        OperationState::Ready
    );

    // Incident: the ORACLE role narrows the band so the live $1 price is out
    // of band. Pricing fails closed and the controller stops lending.
    let (narrow_min, narrow_max) = narrowed_band();
    t.gov_iface_client()
        .set_sanity_band(&t.admin(), &usdc_key(&t), &narrow_min, &narrow_max);
    assert_eq!(live_band(&t), narrowed_band());
    assert_contract_error(
        t.try_borrow(ALICE, "ETH", 0.5),
        errors::SANITY_BOUND_VIOLATED,
    );

    // Any address, with no signature at all, drives the stale operation.
    execute_as_stranger(&t, &resolved, 2)
        .expect("HEAD: the stale ConfigureAssetOracle executes after the narrowing");
    assert_eq!(
        t.gov_iface_client().get_operation_state(&id),
        OperationState::Unset
    );

    // The narrowing is overwritten by the band the proposal carried, and the
    // controller lends against the price the ORACLE role had excluded.
    assert_eq!(
        live_band(&t),
        wide_band(),
        "HEAD: set_oracle overwrites the narrowed band"
    );
    assert_eq!(t.market_oracle_config(&t.resolve_asset("USDC")), resolved);
    t.try_borrow(ALICE, "ETH", 0.5)
        .expect("HEAD: the stale operation reopened lending on USDC collateral");
}

/// The window: the stale operation stays executable until exactly
/// `ready + GRACE`, and is refused with `TimelockOperationExpired` one ledger
/// later. Two identical operations under different salts probe both edges.
#[test]
fn stale_configure_asset_oracle_is_live_for_the_whole_grace_window() {
    let t = LendingTest::new().with_market(usdc_preset()).build();
    let live = t.market_oracle_config(&t.resolve_asset("USDC"));
    let routine = usdc_config(
        &t,
        (live.min_sanity_price_wad, live.max_sanity_price_wad),
        ROUTINE_TOLERANCE_BPS,
    );
    let (at_edge, resolved_edge) = propose_oracle(&t, &routine, 3);
    let (past_edge, resolved_past) = propose_oracle(&t, &routine, 4);
    assert_eq!(resolved_edge, resolved_past);

    advance(&t, t.gov_iface_client().get_min_delay() + GRACE);
    execute_as_stranger(&t, &resolved_edge, 3).expect("executable at ready + GRACE");
    assert_eq!(
        t.gov_iface_client().get_operation_state(&at_edge),
        OperationState::Unset
    );

    advance(&t, 1);
    assert_contract_error(
        execute_as_stranger(&t, &resolved_past, 4),
        GenericError::TimelockOperationExpired as u32,
    );
    assert_eq!(
        t.gov_iface_client().get_operation_state(&past_edge),
        OperationState::Ready,
        "an expired operation stays scheduled until cancelled"
    );
}

/// The documented mitigation (freeze-a-listing runbook, "Review pending
/// operations"): a CANCELLER cancels the pending `ConfigureAssetOracle` right
/// after the narrowing. The stranger's execute is refused and the narrowing
/// survives.
#[test]
fn cancelling_the_pending_configure_asset_oracle_keeps_the_narrowing() {
    let mut t = LendingTest::new()
        .with_market(usdc_preset())
        .with_market(eth_preset())
        .build();
    widen_band_through_timelock(&mut t);
    t.supply(ALICE, "USDC", 10_000.0);

    let routine = usdc_config(&t, wide_band(), ROUTINE_TOLERANCE_BPS);
    let (id, resolved) = propose_oracle(&t, &routine, 5);

    let (narrow_min, narrow_max) = narrowed_band();
    t.gov_iface_client()
        .set_sanity_band(&t.admin(), &usdc_key(&t), &narrow_min, &narrow_max);
    t.gov_iface_client().cancel(&t.admin(), &id);
    assert_eq!(
        t.gov_iface_client().get_operation_state(&id),
        OperationState::Unset
    );

    let delay = t.gov_iface_client().get_min_delay();
    advance_with_live_feeds(&mut t, delay, &["USDC", "ETH"]);
    assert_contract_error(
        execute_as_stranger(&t, &resolved, 5),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );
    assert_eq!(live_band(&t), narrowed_band());
    assert_contract_error(
        t.try_borrow(ALICE, "ETH", 0.5),
        errors::SANITY_BOUND_VIOLATED,
    );
}
