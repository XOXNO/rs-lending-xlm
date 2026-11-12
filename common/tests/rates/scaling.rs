use super::*;
use crate::constants::{RAY, RAY_DECIMALS};
use crate::math::test_support::{ceil_of, floor_of, half_up_of, Sampler};
use crate::rates::test_support::{
    assert_exact, knife_edge_shares, sample_index, unmetered_env, KNIFE_EDGE_INDEX,
};
use crate::validation::max_cap_for_decimals;
use soroban_sdk::Env;

#[test]
fn test_scaled_to_original() {
    let env = Env::default();
    let scaled = Ray::from(100 * RAY);
    let index = Ray::from(RAY * 105 / 100);
    let result = scaled_to_original(&env, scaled, index);
    let expected = 105 * RAY;
    assert!((result.raw() - expected).abs() <= 1);
}

#[test]
fn calculate_scaled_cap_floors_and_saturates() {
    let env = Env::default();
    let index = Ray::from(RAY + RAY / 2);
    let cap = 10i128;
    let scaled = calculate_scaled_cap(&env, cap, 0, index);
    let expected = Ray::from_asset(&env, cap, 0).div_floor(&env, index);
    assert_eq!(scaled, expected);

    // Overflow path: ray form fits i128, but × RAY / 1 saturates instead of panicking.
    // 1e11 asset units @ 0 decimals → 1e38 ray (fits); 1e38 * RAY overflows mul_div.
    let large_cap = 100_000_000_000i128;
    let tiny_index = Ray::from(1);
    let saturated = calculate_scaled_cap(&env, large_cap, 0, tiny_index);
    assert_eq!(saturated.raw(), i128::MAX);
}

#[test]
fn test_resolve_withdrawal_partial_uses_ceil_burn() {
    let env = Env::default();
    let index = Ray::from(RAY + RAY / 3);
    let pos_scaled = Ray::from(10 * RAY);
    let amount = 3;
    let (burn, gross) = resolve_withdrawal(&env, amount, pos_scaled, index, 0);
    let expected_burn = calculate_scaled_supply_ceil(&env, amount, 0, index);
    assert_eq!(burn, expected_burn);
    assert_eq!(gross, amount);
    assert_eq!(burn, Ray::from_asset(&env, amount, 0).div_ceil(&env, index));
}

#[test]
fn test_resolve_withdrawal_full_close_pays_floor() {
    let env = Env::default();
    let index = Ray::ONE;
    let pos_scaled = Ray::from(RAY + RAY * 6 / 10);
    let half_up = unscale_supply(&env, pos_scaled, index, 0);
    let floor = unscale_supply_floor(&env, pos_scaled, index, 0);
    assert_eq!(half_up, 2);
    assert_eq!(floor, 1);
    let (burn, gross) = resolve_withdrawal(&env, half_up, pos_scaled, index, 0);
    assert_eq!(burn, pos_scaled);
    assert_eq!(gross, floor);
}

#[test]
fn test_calculate_scaled_borrow_ceils() {
    let env = Env::default();
    let index = Ray::from(RAY + RAY / 2);
    let amount = 1;
    let ceil = calculate_scaled_borrow(&env, amount, 0, index);
    let half_up = Ray::from_asset(&env, amount, 0).div(&env, index);
    assert!(ceil >= half_up);
    assert_eq!(ceil, Ray::from_asset(&env, amount, 0).div_ceil(&env, index));
}

#[test]
fn test_unscale_borrow_ceil_matches_pool_semantics() {
    let env = Env::default();
    let index = Ray::ONE;
    let scaled = Ray::from(RAY + RAY * 4 / 10);
    assert_eq!(unscale_borrow(&env, scaled, index, 0), 1);
    assert_eq!(unscale_borrow_ceil(&env, scaled, index, 0), 2);
}

/// `unscale_supply` and `unscale_borrow` equal `scaled.mul(index).to_asset(decimals)`: a
/// half-up multiply, then a half-up asset rescale.
#[test]
fn unscale_supply_and_borrow_match_inline_half_up_mul_to_asset() {
    let env = Env::default();
    let decimals = 7u32;
    let cases = [
        (Ray::from(RAY), Ray::ONE),
        (Ray::from(RAY + RAY / 2), Ray::ONE),
        (Ray::from(RAY + RAY * 4 / 10), Ray::ONE),
        (Ray::from(RAY + RAY * 6 / 10), Ray::ONE),
        (Ray::from(10 * RAY + RAY / 3), Ray::from(RAY + RAY / 7)),
        (Ray::from(RAY * 3 / 2), Ray::from(RAY * 11 / 10)),
    ];

    for (scaled, index) in cases {
        let inline = scaled.mul(&env, index).to_asset(&env, decimals);
        assert_eq!(
            unscale_supply(&env, scaled, index, decimals),
            inline,
            "unscale_supply must equal scaled.mul(index).to_asset (half-up)"
        );
        assert_eq!(
            unscale_borrow(&env, scaled, index, decimals),
            inline,
            "unscale_borrow must equal scaled.mul(index).to_asset (half-up)"
        );
        let floor = unscale_supply_floor(&env, scaled, index, decimals);
        let ceil = unscale_borrow_ceil(&env, scaled, index, decimals);
        assert!(floor <= inline && inline <= ceil);
    }
}

#[test]
fn unscale_half_up_exact_half_residue_rounds_away_from_zero() {
    let env = Env::default();
    // At 0 decimals and index 1, `scaled = 1.5 RAY` is 1.5 tokens: half-up gives 2.
    let scaled_half = Ray::from(RAY + RAY / 2);
    assert_eq!(unscale_supply(&env, scaled_half, Ray::ONE, 0), 2);
    assert_eq!(unscale_borrow(&env, scaled_half, Ray::ONE, 0), 2);
    assert_eq!(unscale_supply_floor(&env, scaled_half, Ray::ONE, 0), 1);
    assert_eq!(unscale_borrow_ceil(&env, scaled_half, Ray::ONE, 0), 2);

    // Just below half: 1.4 → 1 half-up, 2 ceil.
    let scaled_below = Ray::from(RAY + RAY * 4 / 10);
    assert_eq!(unscale_supply(&env, scaled_below, Ray::ONE, 0), 1);
    assert_eq!(unscale_borrow(&env, scaled_below, Ray::ONE, 0), 1);

    // Just above half: 1.6 → 2 half-up, 1 floor.
    let scaled_above = Ray::from(RAY + RAY * 6 / 10);
    assert_eq!(unscale_supply(&env, scaled_above, Ray::ONE, 0), 2);
    assert_eq!(unscale_borrow(&env, scaled_above, Ray::ONE, 0), 2);
}

/// 100 tokens + 0.6 stroops at 7 decimals: floor = 1e9, half-up = 1e9 + 1.
fn seven_dec_supply_with_six_tenth_stroop() -> Ray {
    Ray::from(100 * RAY + 6 * 10i128.pow(19))
}

#[test]
fn resolve_net_settle_closes_both_when_conservative_values_match() {
    let env = Env::default();
    let decimals = 7u32;
    let index = Ray::ONE;
    let supply = seven_dec_supply_with_six_tenth_stroop();
    let debt = Ray::from(100 * RAY);

    assert_eq!(
        unscale_supply_floor(&env, supply, index, decimals),
        1_000_000_000
    );
    assert_eq!(unscale_supply(&env, supply, index, decimals), 1_000_000_001);
    assert_eq!(
        unscale_borrow_ceil(&env, debt, index, decimals),
        1_000_000_000
    );

    // Floor supply equals ceiled debt (1e9), so both sides close although half-up
    // supply is 1e9 + 1.
    let (burn_s, burn_d, settled) =
        resolve_net_settle(&env, i128::MAX, supply, debt, index, index, decimals);
    assert_eq!(settled, 1_000_000_000);
    assert_eq!(burn_s, supply);
    assert_eq!(burn_d, debt);
}

#[test]
fn resolve_net_settle_does_not_use_half_up_to_close_supply() {
    let env = Env::default();
    let decimals = 7u32;
    let index = Ray::ONE;
    let supply = seven_dec_supply_with_six_tenth_stroop();
    // 1 extra RAY unit ceils to one stroop above the supply floor.
    let debt = Ray::from(100 * RAY + 1);

    assert_eq!(
        unscale_borrow_ceil(&env, debt, index, decimals),
        1_000_000_001
    );

    let (burn_s, burn_d, settled) =
        resolve_net_settle(&env, i128::MAX, supply, debt, index, index, decimals);
    assert_eq!(settled, 1_000_000_000);
    assert_eq!(burn_s, supply, "payable supply is exhausted");
    assert_eq!(
        burn_d,
        calculate_scaled_borrow_floor(&env, settled, decimals, index)
    );
    assert!(burn_d < debt, "one stroop of ceiled debt remains unpaid");
}

#[test]
fn resolve_net_settle_partial_keeps_directed_rounding() {
    let env = Env::default();
    let supply_index = Ray::from(RAY + RAY / 3);
    let borrow_index = Ray::from(RAY + RAY / 2);
    let supply = Ray::from(10 * RAY);
    let debt = Ray::from(10 * RAY);
    let amount = 3;

    let (burn_s, burn_d, settled) =
        resolve_net_settle(&env, amount, supply, debt, supply_index, borrow_index, 0);
    assert_eq!(settled, amount);
    assert_eq!(
        burn_s,
        calculate_scaled_supply_ceil(&env, amount, 0, supply_index)
    );
    assert_eq!(
        burn_d,
        calculate_scaled_borrow_floor(&env, amount, 0, borrow_index)
    );
}

// --- what the supply-index floor buys (`docs/reference/formulas.md#numeric-limits`) --
//
// `SUPPLY_INDEX_FLOOR_RAW` (RAY / 1_000) is the value
// `apply_bad_debt_to_supply_index` clamps to. It keeps the share conversions
// well defined — `calculate_scaled_supply` divides by the index — and bounds
// share inflation to 1_000x, which in turn costs three decades of deposit
// headroom in a fully written-down market.

#[test]
fn supply_index_floor_bounds_share_inflation_to_one_thousand_x() {
    use crate::constants::SUPPLY_INDEX_FLOOR_RAW;

    let env = Env::default();
    let floor = Ray::from(SUPPLY_INDEX_FLOOR_RAW);
    let decimals = 7u32;
    // 1,000 whole tokens at 7 decimals.
    let amount = 10_000_000_000i128;

    let at_one = calculate_scaled_supply(&env, amount, decimals, Ray::ONE);
    let at_floor = calculate_scaled_supply(&env, amount, decimals, floor);

    assert_eq!(at_floor.raw(), at_one.raw() * 1_000);
    // The value the shares represent is unchanged; only the share count moved.
    assert_eq!(unscale_supply(&env, at_floor, floor, decimals), amount);
}

#[test]
fn supply_index_floor_costs_three_decades_of_deposit_headroom() {
    use crate::constants::SUPPLY_INDEX_FLOOR_RAW;
    use crate::validation::max_cap_for_decimals;

    let env = Env::default();
    let floor = Ray::from(SUPPLY_INDEX_FLOOR_RAW);
    let decimals = 7u32;

    // At index 1.0 the ceiling is the balance ceiling itself.
    let ceiling = max_cap_for_decimals(decimals);
    assert!(calculate_scaled_supply(&env, ceiling, decimals, Ray::ONE).raw() > 0);

    // At the floor the scaled form is 1_000x larger, so the largest deposit a
    // written-down market can still take is the ceiling divided by 1_000.
    let at_floor_ceiling = ceiling / 1_000;
    assert!(calculate_scaled_supply(&env, at_floor_ceiling, decimals, floor).raw() > 0);
    assert_eq!(at_floor_ceiling / 10i128.pow(decimals), 170_141_183);
}

#[test]
#[should_panic(expected = "Error(Contract, #33)")]
fn supply_index_floor_makes_a_ceiling_deposit_overflow_its_scaled_form() {
    use crate::constants::SUPPLY_INDEX_FLOOR_RAW;
    use crate::validation::max_cap_for_decimals;

    let env = Env::default();

    let _ = calculate_scaled_supply(
        &env,
        max_cap_for_decimals(7),
        7,
        Ray::from(SUPPLY_INDEX_FLOOR_RAW),
    );
}

#[test]
fn resolve_net_settle_zero_overlap_is_noop() {
    let env = Env::default();
    let decimals = 7u32;
    let dust = Ray::from(10i128.pow(19));
    assert_eq!(unscale_supply_floor(&env, dust, Ray::ONE, decimals), 0);

    let (burn_s, burn_d, settled) = resolve_net_settle(
        &env,
        i128::MAX,
        dust,
        Ray::from(RAY),
        Ray::ONE,
        Ray::ONE,
        decimals,
    );
    assert_eq!((burn_s, burn_d, settled), (Ray::ZERO, Ray::ZERO, 0));
}

// --- resolve_repay boundaries (GH-04) -------------------------------------

#[test]
fn resolve_repay_zero_amount_burns_nothing_and_refunds_nothing() {
    let env = Env::default();
    let index = Ray::from(RAY + RAY / 3);
    let pos = Ray::from(10 * RAY);
    let (burn, excess) = resolve_repay(&env, 0, pos, index, 7);
    assert_eq!(burn, Ray::ZERO);
    assert_eq!(excess, 0);
}

#[test]
fn resolve_repay_one_unit_below_ceil_debt_is_partial_with_floor_burn() {
    let env = Env::default();
    let index = Ray::from(RAY + RAY / 3);
    let pos = Ray::from(10 * RAY);
    let ceil_debt = unscale_borrow_ceil(&env, pos, index, 7);
    let (burn, excess) = resolve_repay(&env, ceil_debt - 1, pos, index, 7);
    assert_eq!(
        excess, 0,
        "one unit short of the ceiling is a partial repay"
    );
    assert_eq!(
        burn,
        calculate_scaled_borrow_floor(&env, ceil_debt - 1, 7, index),
        "partial repay burns the floor-scaled shares"
    );
    assert!(burn < pos, "a partial repay never burns the whole position");
}

#[test]
fn resolve_repay_at_ceil_debt_closes_with_zero_excess() {
    let env = Env::default();
    let index = Ray::from(RAY + RAY / 3);
    let pos = Ray::from(10 * RAY);
    let ceil_debt = unscale_borrow_ceil(&env, pos, index, 7);
    let (burn, excess) = resolve_repay(&env, ceil_debt, pos, index, 7);
    assert_eq!(burn, pos);
    assert_eq!(excess, 0);
}

#[test]
fn resolve_repay_one_unit_above_ceil_debt_refunds_exactly_one_unit() {
    let env = Env::default();
    let index = Ray::from(RAY + RAY / 3);
    let pos = Ray::from(10 * RAY);
    let ceil_debt = unscale_borrow_ceil(&env, pos, index, 7);
    let (burn, excess) = resolve_repay(&env, ceil_debt + 1, pos, index, 7);
    assert_eq!(burn, pos);
    assert_eq!(excess, 1);
}

#[test]
fn resolve_repay_at_index_ceiling_still_closes() {
    let env = Env::default();
    let index = Ray::from(crate::constants::MAX_BORROW_INDEX_RAY);
    let pos = Ray::from(RAY);
    let ceil_debt = unscale_borrow_ceil(&env, pos, index, 7);
    let (burn, excess) = resolve_repay(&env, ceil_debt, pos, index, 7);
    assert_eq!(burn, pos);
    assert_eq!(excess, 0);
}

// --- exact reference for every conversion ----------------------------------
//
// The reference states each conversion as `docs/reference/formulas.md` does:
// native units scale up to RAY exactly, and each division rounds in the one
// direction the function is named for.

/// Rounds `x * y / d`: one of the exact reference quotients.
type Rounder = fn(i128, i128, i128) -> Option<i128>;

/// Asset decimals from the smallest to the largest the protocol lists.
const DECIMALS: [u32; 6] = [0, 3, 6, 7, 9, 18];

/// RAY units per native unit of an asset with `decimals` decimals.
fn ray_unit(decimals: u32) -> i128 {
    10i128.pow(RAY_DECIMALS - decimals)
}

/// `amount` native units as shares at `index`: scaled up to RAY exactly, then
/// divided by the index with `round`.
fn exact_shares(amount: i128, decimals: u32, index: i128, round: Rounder) -> Option<i128> {
    round(amount.checked_mul(ray_unit(decimals))?, RAY, index)
}

/// `scaled` shares as native units at `index`: multiplied by the index, then
/// scaled down from RAY, with `round` at both steps.
fn exact_tokens(scaled: i128, index: i128, decimals: u32, round: Rounder) -> Option<i128> {
    round(round(scaled, index, RAY)?, 1, ray_unit(decimals))
}

fn sample_decimals(sampler: &mut Sampler) -> u32 {
    DECIMALS[sampler.below(DECIMALS.len() as u32) as usize]
}

/// A native amount: within the asset's cap three times in four, otherwise any
/// non-negative value.
fn sample_amount(sampler: &mut Sampler, decimals: u32) -> i128 {
    if sampler.below(4) == 0 {
        sampler.non_negative()
    } else {
        sampler.at_most(max_cap_for_decimals(decimals))
    }
}

/// Shares worth an amount within the asset's cap at `index`, plus dust so the
/// floor, half-up and ceiling values differ. Any non-negative value when that
/// share count does not fit.
fn sample_position(sampler: &mut Sampler, decimals: u32, index: i128) -> i128 {
    let amount = sampler.at_most(max_cap_for_decimals(decimals) / 4);
    match exact_shares(amount, decimals, index, floor_of) {
        Some(shares) => shares.saturating_add(sampler.at_most(2 * ray_unit(decimals))),
        None => sampler.non_negative(),
    }
}

/// The amounts on each side of `boundary`, or zeros when it is not representable.
fn around(boundary: Option<i128>) -> [i128; 3] {
    boundary.map_or([0; 3], |value| {
        [(value - 1).max(0), value, value.saturating_add(1)]
    })
}

#[test]
fn exact_share_conversions_round_in_their_named_direction() {
    type Mint = fn(&Env, i128, u32, Ray) -> Ray;
    type Redeem = fn(&Env, Ray, Ray, u32) -> i128;

    let env = unmetered_env();
    let mints: [(Mint, Rounder); 4] = [
        (calculate_scaled_supply, floor_of),
        (calculate_scaled_supply_ceil, ceil_of),
        (calculate_scaled_borrow, ceil_of),
        (calculate_scaled_borrow_floor, floor_of),
    ];
    let redemptions: [(Redeem, Rounder); 4] = [
        (unscale_supply, half_up_of),
        (unscale_supply_floor, floor_of),
        (unscale_borrow, half_up_of),
        (unscale_borrow_ceil, ceil_of),
    ];

    let check = |amount: i128, scaled: i128, index: i128, decimals: u32| {
        let case = (amount, scaled, index, decimals);

        for (mint, round) in mints {
            assert_exact(
                &env,
                exact_shares(amount, decimals, index, round),
                |env| mint(env, amount, decimals, Ray::from(index)).raw(),
                case,
            );
        }
        for (redeem, round) in redemptions {
            assert_exact(
                &env,
                exact_tokens(scaled, index, decimals, round),
                |env| redeem(env, Ray::from(scaled), Ray::from(index), decimals),
                case,
            );
        }
        assert_exact(
            &env,
            half_up_of(scaled, index, RAY),
            |env| scaled_to_original(env, Ray::from(scaled), Ray::from(index)).raw(),
            case,
        );
        // The cap's rescale to RAY still overflows; only its division saturates.
        assert_exact(
            &env,
            amount
                .checked_mul(ray_unit(decimals))
                .map(|cap| floor_of(cap, RAY, index).unwrap_or(i128::MAX)),
            |env| calculate_scaled_cap(env, amount, decimals, Ray::from(index)).raw(),
            case,
        );
    };

    // Values next to a rounding boundary of the scale-down, where the
    // rounding of the multiply decides the result.
    for decimals in DECIMALS.into_iter().filter(|&decimals| decimals > 0) {
        for scaled in knife_edge_shares(ray_unit(decimals)) {
            check(0, scaled, KNIFE_EDGE_INDEX, decimals);
        }
    }
    let mut sampler = Sampler::new(51);
    for _ in 0..3_000 {
        let decimals = sample_decimals(&mut sampler);
        let index = sample_index(&mut sampler);
        let amount = sample_amount(&mut sampler, decimals);
        check(amount, sampler.non_negative(), index, decimals);
    }
}

/// At one index, shares minted for a deposit never redeem for more than the
/// deposit, and shares minted for a loan never owe less than the loan.
#[test]
fn exact_mint_then_redeem_never_favours_the_account() {
    let env = unmetered_env();
    let mut sampler = Sampler::new(52);
    for _ in 0..3_000 {
        let decimals = sample_decimals(&mut sampler);
        let index = sample_index(&mut sampler);
        let amount = sampler.at_most(max_cap_for_decimals(decimals));
        let case = (amount, index, decimals);

        if exact_shares(amount, decimals, index, floor_of).is_some() {
            let minted = calculate_scaled_supply(&env, amount, decimals, Ray::from(index));
            let claim = unscale_supply_floor(&env, minted, Ray::from(index), decimals);
            let displayed = unscale_supply(&env, minted, Ray::from(index), decimals);
            assert!(claim <= displayed && displayed <= amount, "{case:?}");
        }
        let owed = exact_shares(amount, decimals, index, ceil_of)
            .and_then(|minted| exact_tokens(minted, index, decimals, ceil_of));
        if owed.is_some() {
            let minted = calculate_scaled_borrow(&env, amount, decimals, Ray::from(index));
            let debt = unscale_borrow_ceil(&env, minted, Ray::from(index), decimals);
            assert!(debt >= amount, "{case:?}");
        }
    }
}

/// `resolve_withdrawal` as documented: a request of at least the half-up
/// balance burns the position and pays its floor value; a smaller one burns
/// ceiling shares and pays the request. Returns `(burned, paid)`.
fn exact_withdrawal(
    amount: i128,
    position: i128,
    index: i128,
    decimals: u32,
) -> Option<(i128, i128)> {
    let displayed = exact_tokens(position, index, decimals, half_up_of)?;
    let payable = exact_tokens(position, index, decimals, floor_of)?;
    if amount >= displayed {
        Some((position, payable))
    } else {
        Some((exact_shares(amount, decimals, index, ceil_of)?, amount))
    }
}

/// `resolve_repay` as documented: a payment of at least the ceiling debt
/// burns the position and returns the excess; a smaller one burns floor
/// shares. Returns `(burned, excess)`.
fn exact_repay(amount: i128, position: i128, index: i128, decimals: u32) -> Option<(i128, i128)> {
    let owed = exact_tokens(position, index, decimals, ceil_of)?;
    if amount >= owed {
        Some((position, amount - owed))
    } else {
        Some((exact_shares(amount, decimals, index, floor_of)?, 0))
    }
}

#[test]
fn exact_withdrawal_and_repay_split_at_the_documented_boundary() {
    let env = unmetered_env();
    let mut sampler = Sampler::new(53);
    for _ in 0..2_000 {
        let decimals = sample_decimals(&mut sampler);
        let index = sample_index(&mut sampler);
        let position = sample_position(&mut sampler, decimals, index);
        let displayed = exact_tokens(position, index, decimals, half_up_of);
        let owed = exact_tokens(position, index, decimals, ceil_of);

        let amounts = [
            [0, 1, sample_amount(&mut sampler, decimals)],
            around(displayed),
            around(owed),
        ];
        for amount in amounts.into_iter().flatten() {
            let case = (amount, position, index, decimals);

            let withdrawal = exact_withdrawal(amount, position, index, decimals);
            assert_exact(
                &env,
                withdrawal,
                |env| {
                    let (burned, paid) = resolve_withdrawal(
                        env,
                        amount,
                        Ray::from(position),
                        Ray::from(index),
                        decimals,
                    );
                    (burned.raw(), paid)
                },
                case,
            );
            if let Some((burned, paid)) = withdrawal {
                // Never more shares than the position holds, and never a
                // payout above the floor value of the shares burned.
                let burned_value = exact_tokens(burned, index, decimals, floor_of);
                assert!(burned <= position, "{case:?}");
                assert!(burned_value.is_some_and(|value| paid <= value), "{case:?}");
            }

            let repay = exact_repay(amount, position, index, decimals);
            assert_exact(
                &env,
                repay,
                |env| {
                    let (burned, excess) =
                        resolve_repay(env, amount, Ray::from(position), Ray::from(index), decimals);
                    (burned.raw(), excess)
                },
                case,
            );
            if let Some((burned, excess)) = repay {
                // The debt cancelled, valued rounding up, never exceeds the
                // payment kept, and a partial payment leaves a share behind.
                let burned_value = exact_tokens(burned, index, decimals, ceil_of);
                assert!(
                    burned_value.is_some_and(|value| value <= amount - excess),
                    "{case:?}"
                );
                if owed.is_some_and(|owed| amount < owed) {
                    assert!(burned < position, "{case:?}");
                }
            }
        }
    }
}

/// `resolve_net_settle` as documented. Returns
/// `(burned_supply, burned_debt, settled)`.
fn exact_net_settle(
    amount: i128,
    (supply, supply_index): (i128, i128),
    (debt, borrow_index): (i128, i128),
    decimals: u32,
) -> Option<(i128, i128, i128)> {
    let payable = exact_tokens(supply, supply_index, decimals, floor_of)?;
    let owed = exact_tokens(debt, borrow_index, decimals, ceil_of)?;
    let settled = amount.min(payable).min(owed);
    if settled <= 0 {
        return Some((0, 0, 0));
    }
    let burned_supply = if settled == payable {
        supply
    } else {
        exact_shares(settled, decimals, supply_index, ceil_of)?.min(supply)
    };
    let burned_debt = if settled == owed {
        debt
    } else {
        exact_shares(settled, decimals, borrow_index, floor_of)?.min(debt)
    };
    Some((burned_supply, burned_debt, settled))
}

#[test]
fn exact_net_settle_burns_each_side_in_its_conservative_direction() {
    let env = unmetered_env();
    let mut sampler = Sampler::new(54);
    for _ in 0..2_000 {
        let decimals = sample_decimals(&mut sampler);
        let (supply_index, borrow_index) = (sample_index(&mut sampler), sample_index(&mut sampler));
        let supply = sample_position(&mut sampler, decimals, supply_index);
        let debt = sample_position(&mut sampler, decimals, borrow_index);
        let payable = exact_tokens(supply, supply_index, decimals, floor_of);
        let owed = exact_tokens(debt, borrow_index, decimals, ceil_of);

        let amounts = [
            [-1, 0, sample_amount(&mut sampler, decimals)],
            around(payable),
            around(owed),
        ];
        for amount in amounts.into_iter().flatten() {
            let case = (amount, supply, debt, supply_index, borrow_index, decimals);
            let expected = exact_net_settle(
                amount,
                (supply, supply_index),
                (debt, borrow_index),
                decimals,
            );
            assert_exact(
                &env,
                expected,
                |env| {
                    let (burned_supply, burned_debt, settled) = resolve_net_settle(
                        env,
                        amount,
                        Ray::from(supply),
                        Ray::from(debt),
                        Ray::from(supply_index),
                        Ray::from(borrow_index),
                        decimals,
                    );
                    (burned_supply.raw(), burned_debt.raw(), settled)
                },
                case,
            );
            if let Some((burned_supply, burned_debt, settled)) = expected {
                // The supply burned is worth at least the settled amount, and
                // the debt cancelled at most that amount.
                let supply_value = exact_tokens(burned_supply, supply_index, decimals, floor_of);
                let debt_value = exact_tokens(burned_debt, borrow_index, decimals, ceil_of);
                assert!(burned_supply <= supply && burned_debt <= debt, "{case:?}");
                assert!(
                    supply_value.is_some_and(|value| value >= settled),
                    "{case:?}"
                );
                assert!(debt_value.is_some_and(|value| value <= settled), "{case:?}");
            }
        }
    }
}
