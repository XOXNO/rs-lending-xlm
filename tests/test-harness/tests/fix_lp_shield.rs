//! Fix family `lp_shield`: the guardian's per-account valuation exclusion for
//! a supply leg whose feed is in an outage (threat model DoS.1, second review
//! pass "The documented risk worth escalating").
//!
//! An indebted borrower adds a dust leg of a collateral whose feed it can make
//! unusable, and every valuation of its account halts: liquidation, cleanup
//! and withdrawal. On mainnet the leg is an Aquarius LP the borrower takes
//! below `min_pool_value_wad`. Here a single-source WBTC feed is made stale.
//! The aggregator refuses both cases on the same strict read, so the
//! controller cannot tell them apart. `controller/audit_supply_stale_shield.rs`
//! pins that default. The patch keeps it: without an exclusion the strict
//! read still fails closed.
//!
//! The patch adds an exclusion of one leg of one named account. The guardian
//! sets it through governance's immediate path, and only while the asset's
//! strict price read fails with a feed outage (stale, no price, LP floor,
//! source disagreement), never for a sanity-band failure the oracle role can
//! cause. The leg then counts as zero collateral in that account and is never
//! priced or seized. Other holders of the listing are untouched. Bad-debt
//! cleanup keeps the strict read. Clearing the exclusion takes a timelocked
//! `RestoreAccountAssetValuation` that names the exclusion's ledger stamp and
//! executes only once the feed is usable again.
//!
//! The skeptic review's attack tests are kept below. Those whose attack the
//! repair closes are flipped and say which change closed them. On the PR #196
//! governance the restore is an ordinary proposal: it carries the proposer's
//! operation guard, so revoking that proposer voids it (last test).

use controller::types::{PriceKey, SeizeMode};
use governance_interface::{
    AdminOperation, RelaxSpokeAssetFlagsArgs, RestoreAccountAssetValuationArgs, RoleArgs,
};
use soroban_sdk::testutils::{
    Address as _, ContractEvents, Events, Ledger as _, MockAuth, MockAuthInvoke,
};
use soroban_sdk::xdr::{ContractEventBody, ScErrorType, ScVal};
use soroban_sdk::{vec, Address, BytesN, IntoVal, Symbol, Val, Vec};
use test_harness::{
    amount_raw, assert_contract_error, build_aggregator_swap, errors, hub_asset, usd, usd_cents,
    HubAssetKey, LendingTest, ALICE, BOB, CAROL, HARNESS_SPOKE, LIQUIDATOR,
};

use common::errors::SpokeError;

const FEED_USABLE: u32 = SpokeError::SpokeAssetFeedUsable as u32;
const EXCLUSION_UNCHANGED: u32 = SpokeError::SpokeAssetExclusionUnchanged as u32;
const FEED_UNUSABLE: u32 = SpokeError::SpokeAssetFeedUnusable as u32;
const FAILURE_INELIGIBLE: u32 = SpokeError::SpokeAssetFeedFailureIneligible as u32;
const DUST_USD_WAD: i128 = 5 * 1_000_000_000_000_000_000;

fn wbtc(t: &LendingTest) -> HubAssetKey {
    hub_asset(t.resolve_asset("WBTC"))
}

fn flatten<T, C>(
    r: Result<Result<T, C>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
) -> Result<(), soroban_sdk::Error> {
    match r {
        Ok(_) => Ok(()),
        Err(Ok(e)) => Err(e),
        Err(Err(e)) => panic!("unexpected InvokeError {e:?}"),
    }
}

/// `caller` asks governance's immediate path to exclude `user`'s WBTC leg.
fn exclude(t: &LendingTest, caller: &Address, user: &str) -> Result<(), soroban_sdk::Error> {
    let account_id = t.resolve_account_id(user);
    flatten(
        t.gov_client()
            .try_exclude_account_asset_valuation(caller, &account_id, &wbtc(t)),
    )
}

/// The ledger stamp of `user`'s WBTC exclusion, read through the view.
fn stamp(t: &LendingTest, user: &str) -> Option<u32> {
    t.ctrl_client()
        .get_account_valuation_exclusions(&t.resolve_account_id(user))
        .get(wbtc(t))
}

fn epoch(t: &LendingTest) -> u64 {
    t.ctrl_client()
        .get_spoke_asset_flags_epoch(&HARNESS_SPOKE, &wbtc(t))
}

/// The `excluded` field of the last `["account", "valuation_exclusion"]`
/// event of the last invocation.
fn last_valuation_event(t: &LendingTest) -> Option<bool> {
    let events: ContractEvents = t.env.events().all();
    events.events().iter().rev().find_map(|event| {
        let ContractEventBody::V0(body) = &event.body;
        let topical = matches!(
            (body.topics.first(), body.topics.get(1)),
            (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b)))
                if a.0.to_string() == "account" && b.0.to_string() == "valuation_exclusion"
        );
        if !topical {
            return None;
        }
        let ScVal::Map(Some(fields)) = &body.data else {
            return None;
        };
        fields
            .iter()
            .find_map(|entry| match (&entry.key, &entry.val) {
                (ScVal::Symbol(k), ScVal::Bool(v)) if k.0.to_string() == "excluded" => Some(*v),
                _ => None,
            })
    })
}

/// Makes the WBTC feed one hour old: the single-source strict read refuses it.
fn make_wbtc_stale(t: &LendingTest) {
    let now = t.env.ledger().timestamp();
    t.mock_reflector_client()
        .set_price_at(&t.resolve_asset("WBTC"), &usd(60_000), &(now - 3_600));
}

/// ALICE borrows 3 ETH ($6,000) against 10,000 USDC, then plants
/// `wbtc_amount` WBTC while its feed is stale. Supply needs no price, so the
/// leg is accepted (the DoS.1 setup of `audit_supply_stale_shield.rs`).
fn shielded(wbtc_amount: f64) -> LendingTest {
    let mut t = LendingTest::new()
        .three_asset_usdc_eth_wbtc()
        .with_dust_disabled_all_markets()
        .build();
    t.set_oracle_single_spot("WBTC");
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.advance_time(5_000);
    make_wbtc_stale(&t);
    t.supply(ALICE, "WBTC", wbtc_amount);
    t
}

fn salt(t: &LendingTest, byte: u8) -> BytesN<32> {
    BytesN::<32>::from_array(&t.env, &[byte; 32])
}

fn restore_args(t: &LendingTest, user: &str, excluded_at: u32) -> RestoreAccountAssetValuationArgs {
    RestoreAccountAssetValuationArgs {
        account_id: t.resolve_account_id(user),
        hub_asset: wbtc(t),
        excluded_at,
    }
}

fn propose_restore(
    t: &LendingTest,
    user: &str,
    excluded_at: u32,
    byte: u8,
) -> Result<(), soroban_sdk::Error> {
    let op = AdminOperation::RestoreAccountAssetValuation(restore_args(t, user, excluded_at));
    flatten(
        t.gov_iface_client()
            .try_propose(&t.admin(), &op, &salt(t, byte)),
    )
}

/// Permissionless execution of a ready operation against the controller.
fn execute_ctrl_op(
    t: &LendingTest,
    function: &str,
    call: Vec<Val>,
    byte: u8,
) -> Result<(), soroban_sdk::Error> {
    flatten(t.gov_iface_client().try_execute(
        &None,
        &t.controller,
        &Symbol::new(&t.env, function),
        &call,
        &salt(t, 0),
        &salt(t, byte),
    ))
}

/// Permissionless execution of a ready restore; the encoding is
/// `op::resolve_op`'s: account id, hub asset, exclusion stamp.
fn execute_restore(
    t: &LendingTest,
    user: &str,
    excluded_at: u32,
    byte: u8,
) -> Result<(), soroban_sdk::Error> {
    let args = restore_args(t, user, excluded_at);
    let call: Vec<Val> = vec![
        &t.env,
        args.account_id.into_val(&t.env),
        args.hub_asset.into_val(&t.env),
        args.excluded_at.into_val(&t.env),
    ];
    execute_ctrl_op(t, "restore_account_asset_valuation", call, byte)
}

/// Moves past a timelock delay, then re-pushes every feed so the mock oracles
/// still hold a current price.
fn advance_ledgers(t: &LendingTest, n: u32) {
    t.env.ledger().with_mut(|l| l.sequence_number += n);
    t.refresh_oracle_prices();
}

fn ctrl_liquidation_bonus_bps(t: &LendingTest, user: &str, debt_asset: &str, amount: f64) -> i128 {
    let decimals = t.resolve_market(debt_asset).decimals;
    let payments = vec![
        &t.env,
        (
            hub_asset(t.resolve_asset(debt_asset)),
            amount_raw(amount, decimals),
        ),
    ];
    t.ctrl_client()
        .get_liquidation_estimate(&t.resolve_account_id(user), &payments, &SeizeMode::Transfer)
        .bonus_rate_bps
}

/// DoS.1 reproduction and the fix. The stale dust leg halts every valuation.
/// A stranger cannot exclude it. The guardian can, while the feed is stale.
/// The account then values its USDC alone, so it is liquidatable on that leg.
/// The liquidation seizes USDC and never touches the excluded WBTC, which also
/// cannot be withdrawn from the unhealthy account.
#[test]
fn guardian_exclusion_makes_the_shielded_account_liquidatable_on_its_other_legs() {
    let mut t = shielded(0.001);
    t.set_price("USDC", usd_cents(50));

    // The default the patch keeps: the strict read of the stale leg fails closed.
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0),
        errors::PRICE_FEED_STALE,
    );
    assert!(t.try_health_factor(ALICE).is_none());

    // Only GUARDIAN_ROLE reaches the controller; the role check runs first.
    let carol = t.get_or_create_user(CAROL);
    assert_contract_error(exclude(&t, &carol, ALICE), errors::UNAUTHORIZED);

    let e0 = epoch(&t);
    assert_eq!(stamp(&t, ALICE), None);
    exclude(&t, &t.admin(), ALICE).expect("the guardian excludes a leg whose feed is in an outage");
    assert_eq!(
        last_valuation_event(&t),
        Some(true),
        "the exclusion is announced"
    );
    assert_eq!(
        stamp(&t, ALICE),
        Some(t.env.ledger().sequence()),
        "the view reports the exclusion with its ledger stamp"
    );
    assert_eq!(epoch(&t), e0, "the listing's flags epoch is not touched");
    assert_contract_error(exclude(&t, &t.admin(), ALICE), EXCLUSION_UNCHANGED);

    // USDC alone: 10,000 * $0.50 * 0.80 / $6,000 = 0.67 (plus accrual).
    let hf = t.health_factor(ALICE);
    assert!(
        hf > 0.6 && hf < 0.7,
        "health factor values USDC alone: {hf}"
    );
    assert!(t.can_be_liquidated(ALICE));

    // The excluded leg cannot leave an unhealthy account.
    assert_contract_error(
        t.try_withdraw(ALICE, "WBTC", 0.0005),
        errors::INSUFFICIENT_COLLATERAL,
    );

    t.get_or_create_user(LIQUIDATOR);
    let wbtc_held = t.supply_balance_raw(ALICE, "WBTC");
    let usdc_held = t.supply_balance_raw(ALICE, "USDC");
    let debt = t.borrow_balance_raw(ALICE, "ETH");
    let liquidator_wbtc = t.token_balance_raw(LIQUIDATOR, "WBTC");
    let liquidator_usdc = t.token_balance_raw(LIQUIDATOR, "USDC");

    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    assert!(t.borrow_balance_raw(ALICE, "ETH") < debt, "debt repaid");
    assert!(
        t.supply_balance_raw(ALICE, "USDC") < usdc_held,
        "USDC seized"
    );
    assert!(t.token_balance_raw(LIQUIDATOR, "USDC") > liquidator_usdc);
    assert_eq!(
        t.supply_balance_raw(ALICE, "WBTC"),
        wbtc_held,
        "the excluded leg is never seized"
    );
    assert_eq!(t.token_balance_raw(LIQUIDATOR, "WBTC"), liquidator_wbtc);
}

/// The exclusion cannot hide collateral that a liquidation should seize.
/// The controller admits it only while the asset's strict read fails, so a
/// guardian cannot zero a priced leg, healthy or not, and the liquidation
/// seizes WBTC pro rata. The borrower cannot set it either: governance owns the
/// controller. An account that holds no WBTC supply leg cannot be named.
#[test]
fn exclusion_is_refused_while_the_feed_is_usable_so_priced_collateral_stays_seizable() {
    let mut t = LendingTest::new()
        .three_asset_usdc_eth_wbtc()
        .with_dust_disabled_all_markets()
        .build();
    t.set_oracle_single_spot("WBTC");
    t.supply(ALICE, "USDC", 5_000.0);
    t.supply(ALICE, "WBTC", 0.1);
    t.borrow(ALICE, "ETH", 3.0);
    t.supply(BOB, "USDC", 1_000.0);

    assert_contract_error(exclude(&t, &t.admin(), ALICE), FEED_USABLE);
    assert_contract_error(
        exclude(&t, &t.admin(), BOB),
        errors::COLLATERAL_POSITION_NOT_FOUND,
    );

    // Underwater on priced legs: (2,500 + 3,000) * 0.80 / 6,000 = 0.73.
    t.set_price("USDC", usd_cents(50));
    t.set_price("WBTC", usd(30_000));
    assert!(t.can_be_liquidated(ALICE));
    assert_contract_error(exclude(&t, &t.admin(), ALICE), FEED_USABLE);
    assert_eq!(stamp(&t, ALICE), None, "a refused exclusion stores nothing");

    // The borrower signs a direct controller call: the owner gate refuses it
    // before the body runs, so the failure is the host's auth error.
    let alice = t.get_or_create_user(ALICE);
    let key = wbtc(&t);
    let account_id = t.resolve_account_id(ALICE);
    let args: Vec<Val> = vec![
        &t.env,
        account_id.into_val(&t.env),
        key.clone().into_val(&t.env),
    ];
    let controller = t.controller.clone();
    t.env.mock_auths(&[MockAuth {
        address: &alice,
        invoke: &MockAuthInvoke {
            contract: &controller,
            fn_name: "exclude_account_asset_valuation",
            args,
            sub_invokes: &[],
        },
    }]);
    let direct = t
        .ctrl_client()
        .try_exclude_account_asset_valuation(&account_id, &key);
    t.env.mock_all_auths_allowing_non_root_auth();
    match direct {
        Err(Ok(e)) => assert!(
            !e.is_type(ScErrorType::Contract),
            "the owner gate refuses the borrower: {e:?}"
        ),
        other => panic!("a borrower-signed exclusion must fail at the owner gate: {other:?}"),
    }

    t.get_or_create_user(LIQUIDATOR);
    let wbtc_held = t.supply_balance_raw(ALICE, "WBTC");
    let liquidator_wbtc = t.token_balance_raw(LIQUIDATOR, "WBTC");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert!(
        t.supply_balance_raw(ALICE, "WBTC") < wbtc_held,
        "priced WBTC is seized pro rata"
    );
    assert!(t.token_balance_raw(LIQUIDATOR, "WBTC") > liquidator_wbtc);
}

/// While excluded, WBTC grants no borrowing power. It can leave the account
/// while USDC alone keeps the account solvent, even with the feed stale. A
/// recovered feed does not restore it. Only a timelocked
/// `RestoreAccountAssetValuation` does, and only for the exclusion it names:
/// a restore with another stamp is refused at proposal, and one prepared for
/// an earlier exclusion cannot clear a later one. Once restored, WBTC counts
/// again and is seized pro rata again.
#[test]
fn clearing_the_exclusion_takes_the_timelock_and_restores_valuation_and_seizure() {
    let mut t = shielded(0.05);
    let admin = t.admin();
    exclude(&t, &admin, ALICE).expect("guardian exclusion while the feed is stale");
    let s1 = stamp(&t, ALICE).expect("excluded");
    let hf_excluded = t.health_factor_raw(ALICE);

    // A lender supplies ETH so the probe meets the collateral gate, not the
    // utilization ceiling. LTV collateral is 7,500 USDC; a 1 ETH borrow needs
    // $8,000 plus accrual. WBTC would add $2,250 of LTV collateral if it counted.
    t.supply(BOB, "ETH", 100.0);
    assert_contract_error(
        t.try_borrow(ALICE, "ETH", 1.0),
        errors::INSUFFICIENT_COLLATERAL,
    );
    t.try_withdraw(ALICE, "WBTC", 0.01)
        .expect("the excluded leg can leave while the rest keeps the account solvent");

    // A recovered feed does not undo the exclusion.
    t.set_price("WBTC", usd(60_000));
    assert_eq!(t.health_factor_raw(ALICE), hf_excluded);

    assert_contract_error(propose_restore(&t, ALICE, s1 + 1, 1), EXCLUSION_UNCHANGED);
    propose_restore(&t, ALICE, s1, 2)
        .expect("any proposer may queue a restore of the live exclusion");
    propose_restore(&t, ALICE, s1, 3).expect("a second copy of the same restore");
    let delay = t.gov_iface_client().get_min_delay();
    advance_ledgers(&t, delay);
    execute_restore(&t, ALICE, s1, 2).expect("the timelocked restore executes");
    assert_eq!(last_valuation_event(&t), Some(false));
    assert_eq!(stamp(&t, ALICE), None);
    assert!(
        t.health_factor_raw(ALICE) > hf_excluded,
        "WBTC counts again"
    );
    // The executed restore cannot run again.
    assert_contract_error(
        execute_restore(&t, ALICE, s1, 2),
        errors::TIMELOCK_UNEXPECTED_STATE,
    );

    // Underwater again: (3,000 + 2,400) * 0.80 / 6,000 = 0.72.
    t.set_price("USDC", usd_cents(30));
    assert!(t.can_be_liquidated(ALICE));
    t.get_or_create_user(LIQUIDATOR);
    let wbtc_held = t.supply_balance_raw(ALICE, "WBTC");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
    assert!(
        t.supply_balance_raw(ALICE, "WBTC") < wbtc_held,
        "the restored leg is seized pro rata again"
    );

    // A second outage, a second exclusion: the queued copy of the first restore
    // names the old stamp and cannot clear the new exclusion.
    make_wbtc_stale(&t);
    exclude(&t, &admin, ALICE).expect("second exclusion");
    let s2 = stamp(&t, ALICE).expect("excluded again");
    assert!(s2 > s1);
    t.set_price("WBTC", usd(60_000));
    assert_contract_error(execute_restore(&t, ALICE, s1, 3), EXCLUSION_UNCHANGED);
    assert_eq!(stamp(&t, ALICE), Some(s2));
}

/// Cleanup writes off debt, so it never runs against an excluded leg's
/// zero. Standalone cleanup and owner force-socialization keep the strict read
/// and fail closed while the feed is stale. A liquidation that takes every
/// valued leg leaves the account with dust valued collateral and open debt.
/// It does not run the residual cleanup that would reclassify the excluded
/// leg as revenue. Once the feed recovers, cleanup values the leg and finds
/// collateral above the dust cap.
#[test]
fn bad_debt_cleanup_never_writes_off_debt_against_an_excluded_leg() {
    let mut t = shielded(0.05);
    t.set_price("USDC", usd_cents(30));
    exclude(&t, &t.admin(), ALICE).expect("guardian exclusion while the feed is stale");
    let id = t.resolve_account_id(ALICE);

    assert_contract_error(t.try_clean_bad_debt_by_id(id), errors::PRICE_FEED_STALE);
    assert_contract_error(
        t.try_force_socialize_bad_debt_by_id(id),
        errors::PRICE_FEED_STALE,
    );

    let wbtc_held = t.supply_balance_raw(ALICE, "WBTC");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 3.0);

    assert!(
        t.account_exists(id),
        "no residual cleanup while the excluded leg is held"
    );
    assert_eq!(t.supply_balance_raw(ALICE, "WBTC"), wbtc_held);
    assert!(
        t.borrow_balance_raw(ALICE, "ETH") > 0,
        "the residual debt stays open"
    );
    assert!(
        t.total_collateral_raw(ALICE) <= DUST_USD_WAD,
        "the valued collateral is at or below the cleanup dust cap"
    );
    assert_contract_error(
        t.try_withdraw(ALICE, "WBTC", 0.01),
        errors::INSUFFICIENT_COLLATERAL,
    );

    t.set_price("WBTC", usd(60_000));
    assert_contract_error(
        t.try_clean_bad_debt_by_id(id),
        errors::CANNOT_CLEAN_BAD_DEBT,
    );
}

// ---------------------------------------------------------------------------
// Skeptic attack tests (lp_shield review), kept after the repair. Each one
// tries to bypass or abuse the exclusion. Those the repair closes are flipped
// and name the change that closed them.
// ---------------------------------------------------------------------------

/// SKEPTIC, flipped by R2 (was
/// `skeptic_oracle_band_narrowing_manufactures_the_exclusion_precondition`):
/// ORACLE_ROLE narrows WBTC's sanity band (immediate, tighten-only) so a
/// fresh, honest WBTC price fails the strict read. Before the repair the
/// guardian could then exclude WBTC and make an account that is healthy at the
/// live price (HF 1.47) liquidatable. The exclusion now admits only feed
/// outages on an allow-list, and a sanity-band violation is not one of them:
/// the account stays frozen, as at HEAD (threat model Elevation.10).
#[test]
fn skeptic_oracle_band_narrowing_no_longer_admits_the_exclusion() {
    // The default (production-shaped) WBTC oracle: the immediate band write
    // refuses a spot-only single source (`SpotOnlyNotProductionSafe`).
    let mut t = LendingTest::new()
        .three_asset_usdc_eth_wbtc()
        .with_dust_disabled_all_markets()
        .build();
    t.supply(ALICE, "USDC", 5_000.0);
    t.supply(ALICE, "WBTC", 0.1);
    t.borrow(ALICE, "ETH", 3.0);
    // (5,000 + 6,000) * 0.80 / 6,000 = 1.47 at live prices.
    assert!(t.health_factor(ALICE) > 1.4);
    let admin = t.admin();
    assert_contract_error(exclude(&t, &admin, ALICE), FEED_USABLE);

    // ORACLE_ROLE, immediate and tighten-only: keep the floor ($59,400 on the
    // seeded +/-1% band), lower the ceiling to $59,999 (50.17 bps half-width,
    // the minimum is 50). The live $60,000 price now violates the band.
    let wbtc_key = PriceKey::Token(t.resolve_asset("WBTC"));
    let seeded = t.price_agg_client().oracle(&wbtc_key).unwrap();
    t.gov_client().set_sanity_band(
        &admin,
        &wbtc_key,
        &seeded.min_sanity_price_wad,
        &usd(59_999),
    );
    assert!(
        t.try_health_factor(ALICE).is_none(),
        "the strict read fails"
    );

    // GUARDIAN_ROLE, immediate: refused, the failure is not a feed outage.
    assert_contract_error(exclude(&t, &admin, ALICE), FAILURE_INELIGIBLE);
    assert_eq!(stamp(&t, ALICE), None);
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0),
        errors::SANITY_BOUND_VIOLATED,
    );
}

/// SKEPTIC, flipped by the per-account design (was
/// `skeptic_decoy_shield_turns_the_exclusion_against_honest_holders`): a decoy
/// account plants a dust leg of the listing whose feed it can break (here:
/// stale WBTC). Before the repair the guardian could unblock the decoy only by
/// excluding the whole listing in the spoke, which made BOB, healthy at the
/// live price, liquidatable on his USDC at a 500 bps bonus (99.4% of his USDC
/// in one call). The exclusion now names one account: the guardian unblocks
/// the decoy alone, and BOB is frozen during the outage, as at HEAD, and
/// valued in full after it.
#[test]
fn skeptic_decoy_exclusion_leaves_honest_holders_of_the_listing_untouched() {
    let mut t = LendingTest::new()
        .three_asset_usdc_eth_wbtc()
        .with_dust_disabled_all_markets()
        .build();
    t.set_oracle_single_spot("WBTC");
    t.supply(CAROL, "ETH", 100.0);
    // BOB, honest: $18,000 WBTC + 2,000 USDC against 3 ETH.
    t.supply(BOB, "WBTC", 0.3);
    t.supply(BOB, "USDC", 2_000.0);
    t.borrow(BOB, "ETH", 3.0);
    // ALICE, the decoy.
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.advance_time(5_000);
    make_wbtc_stale(&t);
    t.supply(ALICE, "WBTC", 0.001);
    t.set_price("USDC", usd_cents(50));

    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0),
        errors::PRICE_FEED_STALE,
    );
    assert!(
        t.try_health_factor(BOB).is_none(),
        "BOB is frozen, not liquidatable"
    );

    exclude(&t, &t.admin(), ALICE).expect("the guardian unblocks the decoy");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);

    // BOB is not named: his account still reads WBTC strictly and stays frozen.
    assert_eq!(stamp(&t, BOB), None);
    assert!(
        t.try_health_factor(BOB).is_none(),
        "BOB stays frozen during the outage"
    );
    let usdc_bob = t.supply_balance_raw(BOB, "USDC");
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, BOB, "ETH", 1.0),
        errors::PRICE_FEED_STALE,
    );
    assert_eq!(t.supply_balance_raw(BOB, "USDC"), usdc_bob);

    // The feed recovers: BOB is valued with his WBTC at once, no restore needed.
    t.set_price("WBTC", usd(60_000));
    let hf_bob = t.health_factor(BOB);
    assert!(hf_bob > 2.0, "BOB valued in full after recovery: {hf_bob}");
    assert!(!t.can_be_liquidated(BOB));
    assert!(
        ctrl_liquidation_bonus_bps(&t, ALICE, "ETH", 0.5) > 0,
        "the decoy stays liquidatable on its valued legs"
    );
}

/// SKEPTIC, flipped by R1 (was
/// `skeptic_restore_during_the_outage_rearms_the_shield`): any PROPOSER, not
/// only the owner, can queue a restore. Before the repair it executed during
/// a still-running outage and re-armed the shield. The restore now requires a
/// usable feed at execution, so it reverts and the exclusion holds.
#[test]
fn skeptic_restore_during_the_outage_is_refused_and_the_exclusion_holds() {
    let mut t = shielded(0.001);
    t.set_price("USDC", usd_cents(50));
    let admin = t.admin();
    exclude(&t, &admin, ALICE).expect("guardian exclusion while the feed is stale");
    assert!(t.can_be_liquidated(ALICE));

    let s = stamp(&t, ALICE).expect("excluded");
    propose_restore(&t, ALICE, s, 7).expect("restore queued during the outage");
    let delay = t.gov_iface_client().get_min_delay();
    advance_ledgers(&t, delay);
    make_wbtc_stale(&t); // the outage is still running
    assert_contract_error(execute_restore(&t, ALICE, s, 7), FEED_UNUSABLE);

    assert_eq!(stamp(&t, ALICE), Some(s), "the exclusion holds");
    t.liquidate(LIQUIDATOR, ALICE, "ETH", 1.0);
}

/// SKEPTIC (holds as documented, pins the limit of the lever): the exclusion
/// only drops supply legs. A dust DEBT leg of a borrowable asset whose feed
/// goes stale still halts the account. The exclusion cannot name it: the
/// account holds no supply leg of that asset.
#[test]
fn skeptic_debt_leg_of_the_excluded_asset_still_shields() {
    let mut t = LendingTest::new()
        .three_asset_usdc_eth_wbtc()
        .with_dust_disabled_all_markets()
        .build();
    t.set_oracle_single_spot("WBTC");
    t.supply(ALICE, "USDC", 10_000.0);
    t.borrow(ALICE, "ETH", 3.0);
    t.borrow(ALICE, "WBTC", 0.0001);
    t.advance_time(5_000);
    make_wbtc_stale(&t);
    t.set_price("USDC", usd_cents(50));

    assert_contract_error(
        exclude(&t, &t.admin(), ALICE),
        errors::COLLATERAL_POSITION_NOT_FOUND,
    );
    assert_contract_error(
        t.try_liquidate(LIQUIDATOR, ALICE, "ETH", 1.0),
        errors::PRICE_FEED_STALE,
    );
}

/// SKEPTIC (holds): sibling execution paths agree with the plan. The estimate
/// view and a `Credit(0)` liquidation both skip the excluded leg; the credit
/// receiver gets USDC shares only.
#[test]
fn skeptic_credit_mode_and_estimate_skip_the_excluded_leg() {
    let mut t = shielded(0.05);
    t.set_price("USDC", usd_cents(50));
    exclude(&t, &t.admin(), ALICE).expect("guardian exclusion");

    let decimals = t.resolve_market("ETH").decimals;
    let payments = vec![
        &t.env,
        (hub_asset(t.resolve_asset("ETH")), amount_raw(1.0, decimals)),
    ];
    let estimate = t.ctrl_client().get_liquidation_estimate(
        &t.resolve_account_id(ALICE),
        &payments,
        &SeizeMode::Credit(0),
    );
    let wbtc_addr = t.resolve_asset("WBTC");
    assert_eq!(estimate.seized_collaterals.len(), 1);
    assert!(estimate
        .seized_collaterals
        .iter()
        .all(|p| p.asset != wbtc_addr));

    let wbtc_held = t.supply_balance_raw(ALICE, "WBTC");
    let receiver = t
        .try_liquidate_with_mode(LIQUIDATOR, ALICE, "ETH", 1.0, SeizeMode::Credit(0))
        .expect("credit liquidation under exclusion");
    assert!(receiver > 0);
    assert!(t.supply_balance_raw_for(receiver, "USDC") > 0);
    assert_eq!(t.supply_balance_raw_for(receiver, "WBTC"), 0);
    assert_eq!(t.supply_balance_raw(ALICE, "WBTC"), wbtc_held);
}

/// SKEPTIC (holds): strategies that touch the excluded leg still price it and
/// fail closed (it is an extra asset), while strategies on the other legs run;
/// the keeper's final 1.05 refresh gate values the excluded leg at zero instead
/// of reverting on the stale read.
#[test]
fn skeptic_strategies_and_keeper_refresh_under_exclusion() {
    let mut t = shielded(0.05);
    t.supply(BOB, "ETH", 100.0);
    t.fund_router("ETH", 1_000.0);
    t.fund_router("USDC", 1_000_000.0);
    exclude(&t, &t.admin(), ALICE).expect("guardian exclusion");

    let wbtc_to_usdc = build_aggregator_swap(&t, "WBTC", "USDC", 0, 1);
    assert_contract_error(
        t.try_swap_collateral(ALICE, "WBTC", 0.01, "USDC", &wbtc_to_usdc),
        errors::PRICE_FEED_STALE,
    );
    let wbtc_to_eth = build_aggregator_swap(&t, "WBTC", "ETH", 0, 1);
    assert_contract_error(
        t.try_repay_debt_with_collateral(ALICE, "WBTC", 0.01, "ETH", &wbtc_to_eth, false),
        errors::PRICE_FEED_STALE,
    );

    let eth_decimals = t.resolve_market("ETH").decimals;
    let usdc_to_eth = build_aggregator_swap(&t, "USDC", "ETH", 0, amount_raw(0.05, eth_decimals));
    t.try_swap_collateral(ALICE, "USDC", 100.0, "ETH", &usdc_to_eth)
        .expect("a strategy on the valued legs does not read the excluded price");

    let id = t.resolve_account_id(ALICE);
    t.try_update_account_threshold(true, &[id])
        .expect("the 1.05 refresh gate values the excluded leg at zero");
}

/// SKEPTIC, flipped by the per-account design (was
/// `skeptic_restore_and_relax_queued_together_void_each_other`): the runbook
/// freezes the listing and then excludes the shielding account. Recovery needs
/// a restore and a relax. The exclusion no longer touches the listing's flags
/// epoch and binds to its own ledger stamp, so both can be queued together and
/// execute in one timelock round, in either order.
#[test]
fn skeptic_restore_and_relax_queued_together_both_execute() {
    let mut t = shielded(0.05);
    let admin = t.admin();
    t.gov_client()
        .set_spoke_asset_flags(&admin, &HARNESS_SPOKE, &wbtc(&t), &false, &true, &false);
    let e = epoch(&t);
    exclude(&t, &admin, ALICE).expect("guardian exclusion after the freeze");
    assert_eq!(epoch(&t), e, "the exclusion leaves the flags epoch");
    let s = stamp(&t, ALICE).expect("excluded");
    t.set_price("WBTC", usd(60_000)); // the feed recovers

    propose_restore(&t, ALICE, s, 21).expect("restore");
    let relax = AdminOperation::RelaxSpokeAssetFlags(RelaxSpokeAssetFlagsArgs {
        spoke_id: HARNESS_SPOKE,
        hub_asset: wbtc(&t),
        expected_epoch: e,
        paused: false,
        frozen: false,
        no_seize: false,
    });
    flatten(
        t.gov_iface_client()
            .try_propose(&admin, &relax, &salt(&t, 22)),
    )
    .expect("relax at e");

    let delay = t.gov_iface_client().get_min_delay();
    advance_ledgers(&t, delay);
    let relax_call: Vec<Val> = vec![
        &t.env,
        HARNESS_SPOKE.into_val(&t.env),
        wbtc(&t).into_val(&t.env),
        e.into_val(&t.env),
        false.into_val(&t.env),
        false.into_val(&t.env),
        false.into_val(&t.env),
    ];
    execute_ctrl_op(&t, "relax_spoke_asset_flags", relax_call, 22)
        .expect("the relax executes first");
    execute_restore(&t, ALICE, s, 21).expect("the restore still executes");
    assert_eq!(stamp(&t, ALICE), None);
    assert!(
        !t.ctrl_client()
            .get_spoke_asset(&HARNESS_SPOKE, &wbtc(&t))
            .frozen
    );
}

/// A fresh address granted `role` through the testing-only immediate path.
fn grant_fresh(t: &LendingTest, role: &str) -> Address {
    let who = Address::generate(&t.env);
    let op = AdminOperation::GrantGovRole(RoleArgs {
        account: who.clone(),
        role: Symbol::new(&t.env, role),
    });
    t.gov_client().execute_immediate(&t.admin(), &op);
    who
}

/// The restore rides the governance operation guard like every other
/// operation: a PROPOSER that is not the owner may queue it, and once the owner
/// strips that key with `revoke_role_immediate` the queued restore is void at
/// execution (`NotAuthorized`), although its stamp is live and the feed is
/// usable. The owner's own restore still executes. A guardian revoked the same
/// way can no longer exclude, while a guardian holding the role can.
#[test]
fn restore_queued_by_a_revoked_proposer_is_void_and_a_revoked_guardian_cannot_exclude() {
    let mut t = shielded(0.05);
    let admin = t.admin();
    exclude(&t, &admin, ALICE).expect("guardian exclusion while the feed is stale");
    let s = stamp(&t, ALICE).expect("excluded");
    t.set_price("WBTC", usd(60_000)); // the feed recovers

    let proposer = grant_fresh(&t, "PROPOSER");
    let op = AdminOperation::RestoreAccountAssetValuation(restore_args(&t, ALICE, s));
    flatten(
        t.gov_iface_client()
            .try_propose(&proposer, &op, &salt(&t, 31)),
    )
    .expect("a non-owner proposer queues the restore");
    t.gov_client()
        .revoke_role_immediate(&proposer, &Symbol::new(&t.env, "PROPOSER"));
    let delay = t.gov_iface_client().get_min_delay();
    advance_ledgers(&t, delay);
    assert_contract_error(execute_restore(&t, ALICE, s, 31), errors::NOT_AUTHORIZED);
    assert_eq!(stamp(&t, ALICE), Some(s), "the exclusion holds");

    propose_restore(&t, ALICE, s, 32).expect("the owner queues the restore");
    advance_ledgers(&t, delay);
    execute_restore(&t, ALICE, s, 32).expect("the owner's restore executes");
    assert_eq!(stamp(&t, ALICE), None);

    make_wbtc_stale(&t);
    let revoked = grant_fresh(&t, "GUARDIAN");
    t.gov_client()
        .revoke_role_immediate(&revoked, &Symbol::new(&t.env, "GUARDIAN"));
    assert_contract_error(exclude(&t, &revoked, ALICE), errors::UNAUTHORIZED);
    assert_eq!(stamp(&t, ALICE), None);
    let guardian = grant_fresh(&t, "GUARDIAN");
    exclude(&t, &guardian, ALICE).expect("a guardian holding the role excludes");
    assert!(stamp(&t, ALICE).is_some());
}

// ---------------------------------------------------------------------------
// Review findings LP-1 and LP-2: the per-leg 1.05 gate that decides whether a
// liquidator-favoring listing tuple applies to a refreshed leg values the
// account's valued book, the one liquidation and the keeper's final gate
// value. The excluded leg counts as zero there and its price is never read.
// ---------------------------------------------------------------------------

/// The liquidation threshold stamped on `user`'s `asset` supply leg.
fn stamped_threshold(t: &LendingTest, user: &str, asset: &str) -> u32 {
    let (supplies, _) = t
        .ctrl_client()
        .get_account_positions(&t.resolve_account_id(user));
    supplies
        .get(hub_asset(t.resolve_asset(asset)))
        .expect("supply leg")
        .liquidation_threshold
}

/// Lowers `asset`'s listing threshold from 8,000 to 7,600 BPS, a tuple that
/// favors the liquidator over every stamp the fixture wrote.
fn lower_listing_threshold(t: &LendingTest, asset: &str) {
    t.edit_asset_config(asset, |config| config.liquidation_threshold = 7_600);
}

/// LP-1: after the excluded leg's feed recovers and before the restore, a
/// third party's top-up of the account's USDC leg cannot apply a lower listed
/// threshold that the valued book (USDC alone) does not carry at 1.05, so it
/// cannot push the account under one and liquidate it. The keeper path
/// refuses the same refresh.
#[test]
fn third_party_top_up_cannot_restamp_an_excluded_account_into_liquidation() {
    let mut t = shielded(0.05);
    exclude(&t, &t.admin(), ALICE).expect("guardian exclusion while the feed is stale");
    t.set_price("WBTC", usd(60_000)); // the feed recovers; the exclusion stands
    t.set_price("USDC", usd_cents(78));
    // USDC alone: 0.80 * 7,800 / 6,000 = 1.04 (less accrual).
    let hf = t.health_factor(ALICE);
    assert!(hf > 1.0 && hf < 1.05, "valued health factor {hf}");
    lower_listing_threshold(&t, "USDC");
    let account = t.resolve_account_id(ALICE);

    t.get_or_create_user(BOB);
    t.supply_to(BOB, account, "USDC", 0.01);

    assert_eq!(
        stamped_threshold(&t, ALICE, "USDC"),
        8_000,
        "0.76 * 7,800 / 6,000 < 1.05 on the valued book: the stamp stays"
    );
    assert!(!t.can_be_liquidated(ALICE));
    assert_contract_error(
        t.try_update_account_threshold(true, &[account]),
        errors::HEALTH_FACTOR_TOO_LOW,
    );
}

/// LP-2: while the excluded leg's feed is still dead, the owner can top up a
/// held leg whose listing holds a liquidator-favoring tuple. The tuple gate
/// values the valued book and never reads the dead price.
#[test]
fn owner_top_up_with_a_pending_tuple_runs_while_the_excluded_feed_is_dead() {
    let mut t = shielded(0.05);
    exclude(&t, &t.admin(), ALICE).expect("guardian exclusion while the feed is stale");
    lower_listing_threshold(&t, "USDC");

    t.try_supply(ALICE, "USDC", 100.0)
        .expect("the top-up does not read the excluded leg's dead feed");
    assert_eq!(
        stamped_threshold(&t, ALICE, "USDC"),
        7_600,
        "0.76 * 10,100 / 6,000 >= 1.05 on the valued book: the tuple applies"
    );
}

/// LP-2: while the feed is still dead, the owner can withdraw part of the
/// excluded leg although its listing holds a liquidator-favoring tuple.
#[test]
fn partial_withdrawal_of_the_excluded_leg_runs_while_its_feed_is_dead() {
    let mut t = shielded(0.05);
    exclude(&t, &t.admin(), ALICE).expect("guardian exclusion while the feed is stale");
    lower_listing_threshold(&t, "WBTC");
    let held = t.supply_balance_raw(ALICE, "WBTC");

    t.try_withdraw(ALICE, "WBTC", 0.01)
        .expect("a partial withdrawal of the excluded leg");
    assert!(t.supply_balance_raw(ALICE, "WBTC") < held);
    assert_eq!(stamped_threshold(&t, ALICE, "WBTC"), 7_600);
    assert!(stamp(&t, ALICE).is_some(), "the exclusion stays");
}

/// LP-2 sibling: the keeper's full-tuple refresh with pending
/// liquidator-favoring tuples runs while the excluded feed is dead.
#[test]
fn keeper_refresh_with_pending_tuples_runs_while_the_excluded_feed_is_dead() {
    let t = shielded(0.05);
    exclude(&t, &t.admin(), ALICE).expect("guardian exclusion while the feed is stale");
    lower_listing_threshold(&t, "USDC");
    lower_listing_threshold(&t, "WBTC");
    let account = t.resolve_account_id(ALICE);

    t.try_update_account_threshold(true, &[account])
        .expect("the per-leg gates value the valued book");
    assert_eq!(stamped_threshold(&t, ALICE, "USDC"), 7_600);
    assert_eq!(stamped_threshold(&t, ALICE, "WBTC"), 7_600);
}
