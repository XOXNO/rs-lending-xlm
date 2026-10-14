//! Credit-mode seizure arithmetic.
//!
//! These cover the part of `SeizeMode::Credit` that has no pool in it: the scaled split, its
//! conservation identity, and the scaled fields `calculate_seized_collateral` derives. The
//! end-to-end behaviour — cash-starved markets, pool totals, receiving-account rules, events —
//! lives in `tests/test-harness/tests/controller/liquidation_seize_modes.rs`, which needs a
//! real pool.

use super::*;
use crate::positions::liquidation::math::{calculate_seized_collateral, NormalizedRepaymentPlan};
use crate::Controller;
use common::constants::{BPS, POSITION_LIMIT_MAX, RAY};
use common::math::fp::Bps;
use common::types::{
    AccountPositionRaw, HubAssetKey, MarketIndexRaw, PositionMode, PriceFeedRaw, SeizeEntry,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Map, Vec};

const WAD: i128 = common::constants::WAD;

fn feed_raw(asset_decimals: u32) -> PriceFeedRaw {
    PriceFeedRaw {
        price_wad: WAD,
        asset_decimals,
        timestamp: 0,
    }
}

fn index_raw(supply_index: i128) -> MarketIndexRaw {
    MarketIndexRaw {
        borrow_index: RAY,
        supply_index,
    }
}

/// A `SeizeEntry` carrying only the credit-mode fields; the asset-unit pair is irrelevant here.
fn scaled_entry(
    env: &Env,
    scaled_amount: i128,
    bonus_scaled: i128,
    liquidation_fees: u32,
) -> SeizeEntry {
    SeizeEntry {
        hub_asset: HubAssetKey {
            hub_id: 0,
            asset: Address::generate(env),
        },
        amount: 1,
        protocol_fee: 0,
        scaled_amount,
        bonus_scaled,
        liquidation_fees,
        feed: feed_raw(7),
        market_index: index_raw(RAY),
    }
}

fn split(env: &Env, seized: i128, bonus: i128, fees: u32) -> (i128, i128) {
    let (fee, liquidator) =
        math::split_seized_shares(env, Ray::from(seized), Ray::from(bonus), fees);
    (fee.raw(), liquidator.raw())
}

// --- conservation --------------------------------------------------------
//
// `S == fee + liquidator` exactly. `split_seized_shares` asserts the identity;
// these tests pin the arithmetic that feeds it, including rounding boundaries.

#[test]
fn split_conserves_shares_across_the_decimal_range() {
    let env = Env::default();
    // Magnitudes spanning a 6-decimal stablecoin unit through an 18-decimal
    // token's whole supply, at RAY scale, plus the degenerate small values.
    let magnitudes = [
        1i128,
        2,
        3,
        999,
        1_000,
        RAY / 1_000_000,
        RAY,
        1_000 * RAY,
        1_000_000_000 * RAY,
    ];
    let fee_rates = [0u32, 1, 7, 100, 500, 2_500, 9_999];

    for seized in magnitudes {
        // The bonus base is bounded by the seizure; walk its whole range.
        for bonus in [0, 1, seized / 3, seized / 2, seized - 1, seized] {
            if bonus < 0 || bonus > seized {
                continue;
            }
            for fees in fee_rates {
                let (fee, liquidator) = split(&env, seized, bonus, fees);
                assert_eq!(
                    fee + liquidator,
                    seized,
                    "conservation broken: S={seized} bonus={bonus} fees={fees}"
                );
                assert!(fee >= 0 && liquidator >= 0, "a leg went negative");
                assert!(fee <= bonus.max(0), "fee exceeded its own base");
            }
        }
    }
}

#[test]
fn one_scaled_unit_seizure_conserves() {
    let env = Env::default();
    // The smallest seizure. Any rate above zero rounds the fee up to the single
    // unit, which leaves the liquidator nothing and still conserves.
    let (fee, liquidator) = split(&env, 1, 1, 1);
    assert_eq!((fee, liquidator), (1, 0));
    assert_eq!(fee + liquidator, 1);

    // With no bonus realised there is no fee base at all, so the unit is the
    // liquidator's.
    let (fee, liquidator) = split(&env, 1, 0, 9_999);
    assert_eq!((fee, liquidator), (0, 1));
    assert_eq!(fee + liquidator, 1);
}

#[test]
fn fee_rounds_up_not_half_up() {
    let env = Env::default();
    // 10_000 units of bonus at 1 bps is exactly 1; at 1 bps on 10_001 it is
    // 1.0001, which must become 2, not 1.
    assert_eq!(split(&env, 1_000_000, 10_000, 1).0, 1);
    assert_eq!(split(&env, 1_000_000, 10_001, 1).0, 2);
    // Well below the half-way point, where half-up would have floored to zero.
    assert_eq!(split(&env, 1_000_000, 1, 1).0, 1);
}

#[test]
fn zero_fee_rate_gives_the_liquidator_the_whole_seizure() {
    let env = Env::default();
    let (fee, liquidator) = split(&env, 7 * RAY, 3 * RAY, 0);
    assert_eq!(fee, 0);
    assert_eq!(liquidator, 7 * RAY);
}

#[test]
fn zero_bonus_gives_the_liquidator_the_whole_seizure() {
    let env = Env::default();
    // A seizure clamped at or below the repayment share realises no excess, so
    // there is nothing for the protocol to take a cut of.
    let (fee, liquidator) = split(&env, 5 * RAY, 0, 9_999);
    assert_eq!(fee, 0);
    assert_eq!(liquidator, 5 * RAY);
}

#[test]
fn fee_never_exceeds_the_seizure_even_when_the_bonus_is_the_whole_seizure() {
    let env = Env::default();
    let seized = 3 * RAY;
    let (fee, liquidator) = split(&env, seized, seized, BPS as u32 - 1);
    assert!(fee <= seized);
    assert_eq!(fee + liquidator, seized);
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn split_rejects_a_bonus_base_above_the_seizure() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        split(&env, RAY, RAY + 1, 100);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn split_rejects_a_fee_rate_at_or_above_one_hundred_percent() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        // `validate_liquidation_fees` rejects BPS at configuration time; the
        // split re-checks because the rate is read from a stamped position.
        split(&env, RAY, RAY, BPS as u32);
    });
}

// --- under-delivery ------------------------------------------------------

#[test]
fn under_delivery_scaling_keeps_the_bonus_bounded_and_the_split_exact() {
    let env = Env::default();
    // A debt token that delivered 37% of what was sent.
    let received = Wad::from(37 * WAD);
    let planned = Wad::from(100 * WAD);

    for (seized, bonus, fees) in [
        (1_000_000i128, 250_000i128, 100u32),
        (3, 3, 9_999),
        (RAY, RAY / 20, 500),
        (7, 1, 1),
    ] {
        let entries = soroban_sdk::vec![&env, scaled_entry(&env, seized, bonus, fees)];
        let scaled = math::scale_seizures_to_received(&env, &entries, received, planned);
        let out = scaled.get(0).unwrap();

        assert!(
            out.bonus_scaled <= out.scaled_amount,
            "scaling must not lift the fee base above the seizure"
        );
        // The split is re-derived after scaling, so conservation holds against
        // the *scaled* total rather than being carried across the scaling step.
        let (fee, liquidator) = split(&env, out.scaled_amount, out.bonus_scaled, fees);
        assert_eq!(fee + liquidator, out.scaled_amount);
        assert!(
            out.scaled_amount <= seized,
            "scaling must shrink, never grow"
        );
    }
}

#[test]
fn full_delivery_leaves_the_scaled_fields_untouched() {
    let env = Env::default();
    let entries = soroban_sdk::vec![&env, scaled_entry(&env, 12_345, 678, 250)];
    let out = math::scale_seizures_to_received(&env, &entries, Wad::from(WAD), Wad::from(WAD));
    let entry = out.get(0).unwrap();
    assert_eq!(entry.scaled_amount, 12_345);
    assert_eq!(entry.bonus_scaled, 678);
    assert_eq!(entry.liquidation_fees, 250);
}

// --- scaled fields off the plan -----------------------------------------

fn seize_fixture(env: &Env, scaled_amount: i128, fees_bps: u32) -> (Address, HubAssetKey, Account) {
    let contract = env.register(crate::Controller, (Address::generate(env),));
    let asset = Address::generate(env);
    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };

    let mut supply_positions = Map::new(env);
    supply_positions.set(
        hub_asset.clone(),
        AccountPositionRaw {
            scaled_amount,
            liquidation_threshold: 8_000,
            liquidation_bonus: 500,
            loan_to_value: 7_500,
            liquidation_fees: fees_bps,
        },
    );

    let account = Account {
        owner: Address::generate(env),
        spoke_id: 1,
        mode: PositionMode::Normal,
        supply_positions,
        borrow_positions: Map::new(env),
    };
    (contract, hub_asset, account)
}

/// Runs the seizure planner against a single-collateral account whose market sits at
/// `supply_index`, so the asset-value-to-share conversion is exercised at a non-unit index.
fn run_seizure(
    env: &Env,
    scaled_amount: i128,
    supply_index: i128,
    fees_bps: u32,
    repay_usd_raw: i128,
    bonus_bps: i128,
    total_collateral_wad: i128,
) -> (Vec<SeizeEntry>, i128) {
    let (contract, hub_asset, account) = seize_fixture(env, scaled_amount, fees_bps);
    let entries = env.as_contract(&contract, || {
        let mut cache = Context::new_view(env);
        let mut prices = Map::new(env);
        prices.set(hub_asset.asset.clone(), feed_raw(7));
        cache.set_prices(prices);
        cache.put_market_index(&hub_asset, &index_raw(supply_index));
        let plan = NormalizedRepaymentPlan {
            repaid: Vec::new(env),
            refunds: Vec::new(env),
            repay_usd: Wad::from(repay_usd_raw),
            bonus: Bps::from(bonus_bps),
            full_close: false,
            repays_all_debt: false,
            seize_all: false,
        };
        calculate_seized_collateral(
            env,
            &account,
            Wad::from(total_collateral_wad),
            &plan,
            &mut cache,
        )
        .0
    });
    (entries, scaled_amount)
}

#[test]
fn a_full_close_moves_the_entire_scaled_position_with_no_rounding() {
    let env = Env::default();
    // A supply index well away from 1.0, so a scaled -> asset -> scaled round
    // trip would visibly lose or invent shares.
    let supply_index = RAY + RAY / 3;
    let scaled = 10_000 * RAY;
    // Ask to seize far more value than the account holds, forcing the clamp.
    let (entries, position_scaled) = run_seizure(
        &env,
        scaled,
        supply_index,
        100,
        1_000_000 * WAD,
        500,
        1_000 * WAD,
    );

    let entry = entries
        .get(0)
        .expect("the single collateral must be seized");
    assert_eq!(
        entry.scaled_amount, position_scaled,
        "a full close is exactly the whole scaled position"
    );
    assert!(entry.bonus_scaled <= entry.scaled_amount);
}

#[test]
fn a_partial_seizure_never_credits_more_shares_than_the_position_holds() {
    let env = Env::default();
    let supply_index = RAY + RAY / 7;
    let scaled = 10_000 * RAY;
    let (entries, position_scaled) = run_seizure(
        &env,
        scaled,
        supply_index,
        100,
        100 * WAD,
        500,
        10_000 * WAD,
    );

    let entry = entries.get(0).expect("a seizure must be planned");
    assert!(entry.scaled_amount > 0);
    assert!(
        entry.scaled_amount < position_scaled,
        "a partial seizure must leave the account something"
    );
    assert!(entry.bonus_scaled <= entry.scaled_amount);

    let (fee, liquidator) = split(&env, entry.scaled_amount, entry.bonus_scaled, 100);
    assert_eq!(fee + liquidator, entry.scaled_amount);
}

#[test]
fn the_planner_stamps_the_seized_positions_own_fee_rate() {
    let env = Env::default();
    // Carrying the rate on the entry is what lets every site re-derive the same
    // split without reading account state again.
    let (entries, _) = run_seizure(&env, 10_000 * RAY, RAY, 750, 100 * WAD, 500, 10_000 * WAD);
    assert_eq!(entries.get(0).unwrap().liquidation_fees, 750);
}

/// MC-2: a partial seizure whose floored token amount equals the leg's
/// half-up balance is a full close in the pool (`resolve_withdrawal` burns
/// every share and pays the floor), so the plan takes the exact held shares
/// too. 10,000 shares at index 1 + 3e-12 are worth 10,000 tokens plus 0.3 of
/// a 7-decimal unit; a seizure of 10,000 tokens plus 0.1 unit floors to the
/// half-up balance.
#[test]
fn a_partial_seizure_at_the_half_up_balance_is_a_full_close() {
    let env = Env::default();
    let supply_index = RAY + 3_000_000_000_000_000;
    let scaled = 10_000 * RAY;
    let held_wad = 10_000 * WAD + 30_000_000_000;
    let seizure_wad = 10_000 * WAD + 10_000_000_000;
    let (entries, position_scaled) =
        run_seizure(&env, scaled, supply_index, 100, seizure_wad, 0, held_wad);

    let entry = entries.get(0).expect("a seizure must be planned");
    assert_eq!(
        entry.amount, 100_000_000_000,
        "floored to the half-up balance"
    );
    assert_eq!(
        entry.scaled_amount, position_scaled,
        "the pool closes this leg, so the plan takes every share"
    );
    assert!(entry.bonus_scaled <= entry.scaled_amount);

    // One unit less and the leg stays partial in both representations.
    let (entries, _) = run_seizure(
        &env,
        scaled,
        supply_index,
        100,
        seizure_wad - 100_000_000_000,
        0,
        held_wad,
    );
    let entry = entries.get(0).expect("a seizure must be planned");
    assert_eq!(entry.amount, 99_999_999_999);
    assert!(entry.scaled_amount < position_scaled);
}

/// MC-2 boundary: the promotion follows the pool's half-up rule exactly. At a
/// remainder of exactly half a unit the half-up balance is one unit above the
/// floored request, so the pool would not close the leg and the plan must stay
/// partial; just below half a unit it closes, and the shares the promotion
/// adds over the planned amount are worth less than half a unit.
#[test]
fn mc2_promotion_fires_only_inside_the_pools_half_up_window() {
    let env = Env::default();
    let scaled = 10_000 * RAY;
    let unit_wad = 100_000_000_000;

    // f = 0.5 unit: half-up balance 10,000 tokens + 1 unit, request floors below it.
    let half_index = RAY + 5_000_000_000_000_000;
    let held_wad = 10_000 * WAD + unit_wad / 2;
    let seizure_wad = 10_000 * WAD + unit_wad * 4 / 10;
    let (entries, position_scaled) =
        run_seizure(&env, scaled, half_index, 100, seizure_wad, 0, held_wad);
    let entry = entries.get(0).expect("a seizure must be planned");
    assert_eq!(entry.amount, 100_000_000_000);
    assert!(
        entry.scaled_amount < position_scaled,
        "at f = 0.5 the pool keeps the leg open, so the plan stays partial"
    );

    // f just below 0.5 unit: the same request now reaches the half-up balance.
    let below_index = RAY + 4_999_000_000_000_000;
    let held_wad = 10_000 * WAD + unit_wad * 4_999 / 10_000;
    let seizure_wad = 10_000 * WAD + unit_wad / 10;
    let (entries, position_scaled) =
        run_seizure(&env, scaled, below_index, 100, seizure_wad, 0, held_wad);
    let entry = entries.get(0).expect("a seizure must be planned");
    assert_eq!(entry.amount, 100_000_000_000);
    assert_eq!(
        entry.scaled_amount, position_scaled,
        "promoted to a full close"
    );
    // The extra shares are the sub-half-unit remainder, valued in RAY.
    let held_ray = Ray::from(position_scaled)
        .mul(&env, Ray::from(below_index))
        .raw();
    let planned_ray = entry.amount * 100_000_000_000_000_000_000;
    assert!(
        held_ray - planned_ray < 50_000_000_000_000_000_000,
        "the promotion hands over less than half a unit: {}",
        held_ray - planned_ray
    );
}

/// MC-4 belt: an empty `Credit` receiver skips the governed position limit
/// (its slots are the liquidated account's legs), but never takes more new
/// supply slots than `POSITION_LIMIT_MAX`, the bound the per-account resource
/// budget is sized for. Six credited legs into an empty receiver are refused.
#[test]
#[should_panic(expected = "Error(Contract, #109)")]
fn an_empty_credit_receiver_never_exceeds_the_position_limit_max() {
    let env = Env::default();
    let contract = env.register(Controller, (Address::generate(&env),));
    let receiver = Account {
        owner: Address::generate(&env),
        spoke_id: 1,
        mode: PositionMode::Normal,
        supply_positions: Map::new(&env),
        borrow_positions: Map::new(&env),
    };
    let mut seized = Vec::new(&env);
    for _ in 0..=POSITION_LIMIT_MAX {
        seized.push_back(scaled_entry(&env, 10 * RAY, 0, 0));
    }
    env.as_contract(&contract, || {
        let mut cache = Context::new_view(&env);
        require_credit_position_limit(&env, &receiver, &seized, &mut cache);
    });
}
