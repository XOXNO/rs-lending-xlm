//! Bad-debt netting, supplier exit timing, and the same-ledger kink round trip.
//!
//! Two questions the original A4-econ tests did not answer:
//!
//! 1. Is the "supplier exits ahead of the write-down" behaviour an asymmetric
//!    capability, or is the remaining supplier equally free to leave? If the
//!    stayer can exit on the same terms at the same moment, the loss lands on
//!    whoever chose to stay — which is exactly the pro-rata-over-current-
//!    suppliers rule ADR-0012 and INV-IDX-03 specify, not a privileged path.
//! 2. `same_market_residual_is_not_netted_against_socialized_debt` uses
//!    USDC collateral against ETH debt, where offsetting is impossible without
//!    selling one asset for the other. The only configuration where netting is
//!    actually available is a residual collateral leg in the *same* market as
//!    the socialized debt. This exercises that case.

use flash_loan_receiver::{FlashLoanMode, FlashLoanRequest};
use soroban_sdk::xdr::ToXdr;
use test_harness::{
    assert_contract_error, days, errors, usd_cents, usdc_preset, LendingTest, ALICE, BOB, CAROL,
    DAVE, LIQUIDATOR,
};

fn setup() -> LendingTest {
    LendingTest::new().standard_two_asset_dust_disabled()
}

/// The "victim" of the dodge has the same exit available at the same moment.
/// No capability the dodger holds is denied to the stayer.
#[test]
fn stayer_has_the_same_exit_as_the_dodger() {
    let mut t = setup();
    t.supply(BOB, "ETH", 75.0);
    t.supply(CAROL, "ETH", 25.0);
    t.supply(ALICE, "USDC", 10.0);
    t.borrow(ALICE, "ETH", 0.003);

    let carol_supplied = t.supply_balance(CAROL, "ETH");
    let carol_wallet_before = t.token_balance(CAROL, "ETH");

    t.set_price("USDC", usd_cents(10));
    t.assert_liquidatable(ALICE);

    // Bob dodges exactly as A4-econ describes.
    t.withdraw_all(BOB, "ETH");

    // Carol, the alleged victim, is still free to do the identical thing.
    // A full exit is refused only because she would be the last supplier while
    // debt is still open (`require_solvent_withdraw_state`) — a rule that binds
    // whoever leaves last, not a privilege Bob held. She takes all but a token.
    t.withdraw(CAROL, "ETH", 24.9);
    let carol_recovered = t.token_balance(CAROL, "ETH") - carol_wallet_before;

    assert!(
        carol_recovered >= 24.9,
        "the stayer exits on the same terms: supplied={:.9} recovered={:.9}",
        carol_supplied,
        carol_recovered
    );

    std::println!(
        "V3 symmetry: carol_supplied={:.9} carol_recovered={:.9} loss={:.9}",
        carol_supplied,
        carol_recovered,
        carol_supplied - carol_recovered
    );
}

/// Same-market residual: Alice keeps an ETH deposit leg *and* an ETH debt leg.
/// Netting is arithmetically available here. Measure whether it happens.
#[test]
fn same_market_residual_is_not_netted_against_socialized_debt() {
    let mut t = setup();

    t.supply(BOB, "ETH", 100.0);

    // Alice's collateral is mostly USDC, plus a small ETH leg in the very
    // market she borrows from.
    t.supply(ALICE, "USDC", 1000.0);
    t.supply(ALICE, "ETH", 0.010);
    t.borrow(ALICE, "ETH", 0.300);

    let alice_id = t.resolve_account_id(ALICE);
    let bob_before = t.supply_balance(BOB, "ETH");
    let alice_eth_collateral = t.supply_balance(ALICE, "ETH");
    let alice_debt = t.borrow_balance(ALICE, "ETH");
    let eth_rev_before = t.snapshot_revenue("ETH");

    t.set_price("USDC", usd_cents(1));
    t.assert_liquidatable(ALICE);

    t.force_socialize_bad_debt_by_id(alice_id);

    let bob_loss = bob_before - t.supply_balance(BOB, "ETH");
    let eth_rev_gain = t.snapshot_revenue("ETH") - eth_rev_before;

    std::println!(
        "V3 same-market: alice_eth_collateral={:.9} alice_eth_debt={:.9} \
         bob_loss={:.9} eth_revenue_gain_raw={} netted={:.9}",
        alice_eth_collateral,
        alice_debt,
        bob_loss,
        eth_rev_gain,
        alice_debt - bob_loss
    );

    assert!(
        bob_loss > 0.0,
        "ETH suppliers must absorb the socialized debt, loss={bob_loss:.9}"
    );
    assert!(
        eth_rev_gain > 0,
        "same-market residual should land in ETH revenue, gain={eth_rev_gain}"
    );

    // THE CLAIM, asserted rather than printed. `alice_debt - bob_loss` is how
    // much of the socialized debt Alice's own same-market collateral absorbed.
    //
    //   netting     => Bob absorbs only (debt - collateral), so this is
    //                  ~= alice_eth_collateral.
    //   no netting  => Bob absorbs the whole debt, so this is ~= 0 and the
    //                  collateral went to revenue instead.
    //
    // Both of the bounds above hold either way, so without this the test would
    // pass unchanged if netting were introduced tomorrow.
    let offset_taken = alice_debt - bob_loss;
    assert!(
        offset_taken < alice_eth_collateral * 0.10,
        "the same-market residual is NOT netted against the socialized debt: \
         of {alice_eth_collateral:.9} ETH available to offset, only \
         {offset_taken:.9} reduced the write-down. If this assertion starts \
         failing, netting has been introduced and finding F-10 is fixed."
    );
}

/// Quantifies how much of a market a large supplier can actually pull out.
/// `require_utilization_below_max` is checked after the burn, so the exit
/// ceiling is `f <= 1 - u / max_utilization` of total supply (max_utilization
/// is 95% in the preset). Confirms the closed form by probing each side of it
/// on a fresh fixture.
#[test]
fn withdrawal_ceiling_tracks_one_minus_utilization_over_max() {
    // 100 ETH of real supply: Bob 80, Carol 20. Dave drives utilization.
    fn fixture(target_u: u32) -> LendingTest {
        let mut t = setup();
        t.supply(BOB, "ETH", 80.0);
        t.supply(CAROL, "ETH", 20.0);
        let borrow = 100.0 * f64::from(target_u) / 100.0;
        t.supply(DAVE, "USDC", borrow * 2000.0 * 2.0);
        t.borrow(DAVE, "ETH", borrow);
        t
    }

    for target_u in [10u32, 50, 80, 90] {
        let u = f64::from(target_u) / 100.0;
        // Closed form, in ETH of the 100 supplied, clamped to Bob's 80.
        let predicted = ((1.0 - u / 0.95) * 100.0).clamp(0.0, 80.0);
        let below = (predicted - 0.5).max(0.0);
        let above = predicted + 0.5;

        let ok_below = fixture(target_u).try_withdraw(BOB, "ETH", below).is_ok();
        let res_above = fixture(target_u).try_withdraw(BOB, "ETH", above);
        let ok_above = res_above.is_ok();

        std::println!(
            "V3 exit ceiling: utilization={}%  predicted_max={:.2} ETH \
             ({:.1}% of Bob\'s 80)  withdraw({:.2})={}  withdraw({:.2})={}",
            target_u,
            predicted,
            predicted / 80.0 * 100.0,
            below,
            if ok_below { "OK" } else { "REVERT" },
            above,
            if ok_above { "OK" } else { "REVERT" }
        );

        assert!(
            ok_below,
            "u={}%: withdrawal just below the ceiling must succeed",
            target_u
        );
        if predicted < 79.9 {
            // Pin the REASON, not merely that it failed. A bare !is_ok() is also
            // satisfied by insufficient collateral, a fixture break, or a panic —
            // any of which would let a ceiling that moved for the wrong reason
            // survive this test.
            assert_contract_error(res_above, errors::UTILIZATION_ABOVE_MAX);
        }
    }
}

/// A4-01b claims that below `hf ~= proportion_seized` no repayment size is
/// profitable, so liquidators rationally stop, leaving a $12k-debt / $10k-
/// collateral account permanently unrecognisable because `clean_bad_debt` is
/// gated at $5 of collateral. Build exactly that account and drive it.
///
/// NOTE on the numbers: the harness pre-funds the liquidator's repayment
/// (`burn_prefund`), so the ETH spend does not appear as a wallet delta. The
/// meaningful figure is the seized USD against the $1,000 repaid per step.
#[test]
fn deep_underwater_account_still_liquidates_to_the_dust_gate() {
    let mut t = setup();
    t.supply(BOB, "ETH", 100.0);

    // Alice: $100k USDC collateral, 6 ETH = $12,000 debt.
    t.supply(ALICE, "USDC", 100_000.0);
    t.borrow(ALICE, "ETH", 6.0);
    let alice_id = t.resolve_account_id(ALICE);
    t.get_or_create_user(LIQUIDATOR);

    // Crash USDC 10x -> collateral $10,000 against $12,000 of debt.
    t.set_price("USDC", usd_cents(10));

    std::println!(
        "V3 A4-01b start: collateral_usd={:.2} debt_usd={:.2} hf={:.4} liquidatable={}",
        t.total_collateral(ALICE),
        t.total_debt(ALICE),
        t.health_factor(ALICE),
        t.can_be_liquidated(ALICE)
    );

    // The permissionless dust gate is shut at this size — A4-01b is right there.
    // Pin the specific error: a bare is_err() would let any unrelated revert
    // stand in for the gate actually refusing.
    assert_contract_error(
        t.try_clean_bad_debt_by_id(alice_id),
        errors::CANNOT_CLEAN_BAD_DEBT,
    );
    std::println!("V3 A4-01b clean_bad_debt at $10k collateral: REVERT (CannotCleanBadDebt)");

    // The load-bearing question: can a liquidator still make money here?
    let repay_usd = 0.5 * 2000.0;
    let mut steps = 0;
    for step in 1..=14 {
        if !t.account_exists(alice_id) {
            std::println!(
                "V3 A4-01b: account GONE at step {} — cleanup fired automatically",
                step
            );
            break;
        }
        let coll_before = t.total_collateral(ALICE);
        let liq_usdc_before = t.token_balance(LIQUIDATOR, "USDC");

        match t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5) {
            Ok(()) => {
                steps = step;
                let got_usd = (t.token_balance(LIQUIDATOR, "USDC") - liq_usdc_before) * 0.10;
                let coll_after = if t.account_exists(alice_id) {
                    t.total_collateral(ALICE)
                } else {
                    0.0
                };
                std::println!(
                    "V3 A4-01b step {:>2}: repaid ${:.0} -> seized ${:.2} \
                     (margin {:+.2}%)  collateral ${:.2} -> ${:.2}",
                    step,
                    repay_usd,
                    got_usd,
                    (got_usd - repay_usd) / repay_usd * 100.0,
                    coll_before,
                    coll_after
                );
            }
            Err(e) => {
                std::println!("V3 A4-01b step {:>2}: liquidate REVERTED {:?}", step, e);
                break;
            }
        }
    }

    std::println!(
        "V3 A4-01b end: account_exists={} after {} profitable partial liquidations",
        t.account_exists(alice_id),
        steps
    );
    assert!(
        steps >= 2,
        "partial liquidation must remain available below hf=0.80"
    );
}

/// The one band where a full close IS forced: `max_hf_preserving_bonus_bps`
/// returns a cap in `[0, base_bonus)`, i.e. `hf` in `[threshold, 1.05*threshold)`
/// = [0.80, 0.84) at the preset. Check a full close is still profitable there,
/// so the band is not a stuck state either.
#[test]
fn forced_full_close_is_profitable_for_the_liquidator() {
    let mut t = setup();
    t.supply(BOB, "ETH", 100.0);

    // collateral = 1.025 * debt  ->  hf = 1.025 * 0.80 = 0.82
    t.supply(ALICE, "USDC", 123_000.0);
    t.borrow(ALICE, "ETH", 6.0);
    let alice_id = t.resolve_account_id(ALICE);
    t.get_or_create_user(LIQUIDATOR);
    t.set_price("USDC", usd_cents(10));

    std::println!(
        "V3 full-close band: collateral_usd={:.2} debt_usd={:.2} hf={:.4}",
        t.total_collateral(ALICE),
        t.total_debt(ALICE),
        t.health_factor(ALICE)
    );

    let partial = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 0.5);
    // Pin WHY the partial is refused. Without this the test passes unchanged if
    // this band stops forcing a full close, which is half of what its own doc
    // comment claims it demonstrates.
    std::println!("V3 full-close band: partial 0.5 ETH -> {partial:?}");
    assert_contract_error(partial, errors::FULL_CLOSE_REQUIRED);

    let liq_usdc_before = t.token_balance(LIQUIDATOR, "USDC");
    let full = t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 6.0);
    let got_usd = (t.token_balance(LIQUIDATOR, "USDC") - liq_usdc_before) * 0.10;
    std::println!(
        "V3 full-close band: full 6.0 ETH (=$12000) -> {}  seized=${:.2} (margin {:+.2}%)  \
         account_exists={}",
        if full.is_ok() { "OK" } else { "REVERT" },
        got_usd,
        (got_usd - 12_000.0) / 12_000.0 * 100.0,
        t.account_exists(alice_id)
    );
    assert!(
        full.is_ok(),
        "a full close must succeed in the forced-full-close band"
    );
    assert!(
        got_usd > 12_000.0,
        "a full close must be profitable, seized=${:.2}",
        got_usd
    );
}

// ---------------------------------------------------------------------------
// F-11: does a nested unguarded pool mutation get reverted by flash's stale Cache?
// ---------------------------------------------------------------------------
//
// The claim: an owner calling `upgrade_liquidity_pool_params` inside a flash
// callback reaches `markets.rs:104` `pool_update_indexes_call`, which commits
// accrual to `PoolKey::State` with no flash guard. `flash::apply` then commits
// the `Cache` it loaded before the callback, "silently reverting" that accrual.
//
// Run the identical flash loan twice — with and without the re-entry — and
// compare the committed state. If accrual were lost, the two must diverge.
// Each run gets its own `#[test]` so each owns its `Env`.

/// Strengthens what `flash_loan_adversarial.rs:186`
/// (`test_flash_loan_reenter_supply_against_live_controller_rejects`) already
/// covers. That test asserts only `is_err()` via `assert_reentry_fails`, so it
/// cannot distinguish the Soroban host's re-entry prohibition from the
/// protocol's own flash guard. This one pins the exact host error, and adds the
/// owner-gated arm the existing test does not reach.
///
/// The property: **a flash-loan
/// callback cannot reach any controller entrypoint at all.**
///
/// The call stack is controller -> pool -> receiver -> **controller**, and the
/// Soroban host prohibits contract re-entry. Cross-contract calls default to
/// `ContractReentryMode::Prohibited`
/// (soroban-env-host-27.0.1/src/host/frame.rs:110,119) and any re-entry into a
/// contract already on the context stack returns
/// `Error(Context, InvalidAction)` (frame.rs:924-950).
///
/// The two cases below discriminate the mechanism. `supply` is **not**
/// owner-gated and auth mocking is left ON, so neither an authorization failure
/// nor the flash guard can explain its rejection — if the guard were what
/// fired, `supply` would return `FlashLoanOngoing`. Both return the identical
/// host error instead.
///
/// Consequence for F-11: the precondition (reaching an unguarded controller
/// entrypoint from inside a callback) cannot be constructed, independently of
/// the flash guard and of who holds the owner key.
#[test]
fn flash_callback_cannot_reach_any_controller_entrypoint() {
    let host_reentry_error = soroban_sdk::Error::from_type_and_code(
        soroban_sdk::xdr::ScErrorType::Context,
        soroban_sdk::xdr::ScErrorCode::InvalidAction,
    );

    for (mode, label) in [
        (
            FlashLoanMode::ReenterControllerSupply,
            "supply (NOT owner-gated, auth mocking ON)",
        ),
        (
            FlashLoanMode::ReenterControllerUpgradePoolParams,
            "upgrade_liquidity_pool_params (owner-gated, reaches the unguarded markets.rs path)",
        ),
    ] {
        let mut t = LendingTest::new().with_market(usdc_preset()).build();
        t.supply(ALICE, "USDC", 100_000.0);
        t.supply(CAROL, "USDC", 50_000.0);
        t.borrow(CAROL, "USDC", 20_000.0);
        t.advance_time(days(30));

        let receiver = t.deploy_adversarial_flash_loan_receiver();
        let data = FlashLoanRequest { mode }.to_xdr(&t.env);
        let amount = 10_000 * 10i128.pow(t.resolve_market("USDC").decimals);
        let config = t.get_asset_config("USDC");
        let fee =
            common::math::fp::Bps::from(config.flashloan_fee).flash_loan_fee_on(&t.env, amount);
        let asset = t.resolve_asset("USDC");
        soroban_sdk::token::StellarAssetClient::new(&t.env, &asset).mint(&receiver, &fee);

        let result = t.try_flash_loan_with_data(BOB, "USDC", amount, &receiver, &data);
        std::println!("V3 callback re-entry: {} -> {:?}", label, result);

        assert_eq!(
            result,
            Err(host_reentry_error),
            "re-entering the controller from a flash callback must be refused by the \
             host re-entry rule, not merely fail: {} returned {:?}",
            label,
            result
        );
    }

    std::println!(
        "V3 verdict: both owner-gated and non-owner-gated re-entry return the SAME host \
         error, so the reject is the Soroban re-entry prohibition -- not auth ordering and \
         not the flash guard."
    );
}

// ---------------------------------------------------------------------------
// N2: can a same-ledger borrow -> repay round trip across a utilization kink
// move the index in the attacker's favour?
// ---------------------------------------------------------------------------
//
// The claim: `ops::synced_market` (ops/mod.rs:29-33) accrues BEFORE every
// mutation, reading utilization from committed state — i.e. from before the
// caller's own borrow. After the first leg `last_timestamp == now`, so the
// second leg's `elapsed_ms()` is 0, `needs_accrual` (cache/mod.rs:137-139) is
// false and `global_sync` returns immediately. No index moves.
//
// A test that only asserts "the indexes are identical" is worthless unless the
// measurement can detect a move at all. So each case is run twice: once with
// the round trip closed in the same ledger (the attack), and once holding the
// position across real time (the positive control). If the second pair does not
// diverge, the first pair proving equal means nothing.

/// Reads the **committed** `PoolStateRaw` — not a view.
///
/// `get_market_indexes_detailed` runs `simulate_update_indexes(now)`
/// (`views.rs:160` -> `context/market_index.rs:25` -> pool `get_bulk_indexes`,
/// documented "Simulate accrued indexes ... without writing state"), so it
/// reconstructs the accrued-to-now value whether or not anything was ever
/// committed. That would let this test pass even if leg 1 stopped committing.
/// `get_sync_data` returns the raw stored state instead.
///
/// `last_timestamp` is the direct mechanism probe: it is what `mark_accrued()`
/// (`cache/mod.rs:141-143`) stamps, so asserting it is unchanged across leg 2
/// tests `elapsed_ms() == 0` itself rather than its numerical effect.
fn committed_state(t: &LendingTest, asset: &str) -> (i128, i128, u64) {
    let key = test_harness::hub_asset(t.resolve_asset(asset));
    let sync = t.pool_client(asset).get_sync_data(&key);
    (
        sync.state.supply_index,
        sync.state.borrow_index,
        sync.state.last_timestamp,
    )
}

/// `cross`: borrow across the optimal-utilization kink (30% -> 85%, into
/// `slope3`). `hold`: leave the position open for a day before repaying.
/// Returns the committed (supply_index, borrow_index, last_timestamp) plus the
/// borrow rate before and at the peak, so the test can prove the kink was
/// actually exercised.
fn kink_run(cross: bool, hold: bool) -> (i128, i128, u64, f64, f64) {
    let mut t = LendingTest::new().standard_two_asset_dust_disabled();
    t.supply(ALICE, "USDC", 100_000.0);
    t.supply(BOB, "ETH", 100.0);
    t.borrow(BOB, "USDC", 30_000.0); // utilization 30%, below mid kink (50%)

    t.advance_time(days(30));
    let rate_before = t.pool_borrow_rate("USDC");

    let mut rate_peak = rate_before;
    if cross {
        // 30k -> 85k of 100k supplied = 85% utilization, past optimal (80%),
        // so the marginal rate is on slope3 (150% RAY).
        t.borrow(BOB, "USDC", 55_000.0);
        rate_peak = t.pool_borrow_rate("USDC");
    }
    if hold {
        t.advance_time(days(1));
    }
    if cross {
        t.repay(BOB, "USDC", 55_000.0);
    }

    t.update_indexes_for(&["USDC"]);

    let (si, bi, ts) = committed_state(&t, "USDC");
    (si, bi, ts, rate_before, rate_peak)
}

#[test]
fn same_ledger_kink_roundtrip_cannot_move_the_index() {
    let (si_base, bi_base, ts_base, r0, _) = kink_run(false, false);
    let (si_atk, bi_atk, ts_atk, r_before, r_peak) = kink_run(true, false);

    // Positive control: same two shapes, but the position is held for a day.
    let (si_hb, bi_hb, _, _, _) = kink_run(false, true);
    let (si_ha, bi_ha, _, _, _) = kink_run(true, true);

    std::println!(
        "V3 N2 kink exercised: borrow_rate {:.6} at 30% -> {:.6} at 85% ({:.2}x)",
        r_before,
        r_peak,
        r_peak / r_before
    );
    std::println!(
        "V3 N2 same-ledger : baseline supply={} borrow={} last_ts={}",
        si_base,
        bi_base,
        ts_base
    );
    std::println!(
        "V3 N2 same-ledger : attack   supply={} borrow={} last_ts={}",
        si_atk,
        bi_atk,
        ts_atk
    );
    std::println!(
        "V3 N2 held-1-day  : baseline supply={} borrow={}",
        si_hb,
        bi_hb
    );
    std::println!(
        "V3 N2 held-1-day  : attack   supply={} borrow={}",
        si_ha,
        bi_ha
    );

    // Guard 1: the kink is actually being crossed. Without this the test could
    // silently stop exercising slope3 if a preset changed, and still pass.
    assert!(
        r_peak > r_before * 2.0,
        "the 30% -> 85% leg must cross into slope3: {:.6} -> {:.6}",
        r_before,
        r_peak
    );
    assert!(r0 > 0.0);

    // Guard 2: the measurement is sensitive. If holding 85% for a day did not
    // move the committed indexes, the equality below would be vacuous.
    assert_ne!(
        (si_ha, bi_ha),
        (si_hb, bi_hb),
        "measurement not sensitive: holding 85% utilization for a day must move \
         the committed indexes, else the same-ledger equality proves nothing"
    );

    // The claim under test, on committed state.
    assert_eq!(
        (si_atk, bi_atk),
        (si_base, bi_base),
        "a same-ledger kink round trip must not move the committed indexes"
    );
    // And the mechanism directly: leg 1 stamped `last_timestamp`, so leg 2 saw
    // `elapsed_ms() == 0` and accrued nothing.
    assert_eq!(
        ts_atk, ts_base,
        "leg 2 must not re-accrue: committed last_timestamp must match baseline"
    );

    std::println!(
        "V3 N2 verdict: holding across time DOES move the committed indexes (control), \
         yet the same-ledger round trip moves NEITHER index and leaves last_timestamp \
         untouched. Kink gaming needs real time at risk."
    );
}
