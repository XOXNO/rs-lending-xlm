//! Credit-mode seizure arithmetic.
//!
//! These cover the part of `SeizeMode::Credit` that has no pool in it: the scaled split, its
//! conservation identity, and the scaled fields `calculate_seized_collateral` derives. The
//! end-to-end behaviour — cash-starved markets, pool totals, receiving-account rules, events —
//! lives in `tests/test-harness/tests/controller/liquidation_seize_modes.rs`, which needs a
//! real pool.

use super::*;
use crate::positions::liquidation::math::{calculate_seized_collateral, NormalizedRepaymentPlan};
use common::constants::{BPS, RAY};
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
// `S == fee + liquidator` exactly. A share invented on this path is a supplier
// claim with nothing behind it; a share destroyed is collateral that silently
// evaporates. The production code asserts the identity itself — these pin the
// arithmetic that feeds it, including the boundaries where rounding could bite.

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
    // The smallest seizure representable. Any rate above zero rounds the fee up
    // to the single unit, leaving the liquidator nothing — which still conserves.
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
        let mut cache = Cache::new_view(env);
        let mut prices = Map::new(env);
        prices.set(hub_asset.asset.clone(), feed_raw(7));
        cache.set_prices(prices);
        cache.put_market_index(&hub_asset, &index_raw(supply_index));
        let plan = NormalizedRepaymentPlan {
            repaid: Vec::new(env),
            refunds: Vec::new(env),
            repay_usd: Wad::from(repay_usd_raw),
            bonus: Bps::from(bonus_bps),
        };
        calculate_seized_collateral(
            env,
            &account,
            Wad::from(total_collateral_wad),
            &plan,
            &mut cache,
        )
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
