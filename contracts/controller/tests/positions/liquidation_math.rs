use super::*;
use crate::constants::{
    DEFAULT_HF_FOR_MAX_BONUS_WAD, DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
    DEFAULT_LIQUIDATION_TARGET_HF_WAD, WAD,
};
use common::constants::RAY;
use common::types::SpokeConfig;
use common::types::{DebtPositionRaw, MarketIndexRaw, PositionMode, PriceFeedRaw};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address};

fn default_spoke_config() -> SpokeConfig {
    SpokeConfig {
        is_deprecated: false,
        liquidation_target_hf_wad: DEFAULT_LIQUIDATION_TARGET_HF_WAD,
        hf_for_max_bonus_wad: DEFAULT_HF_FOR_MAX_BONUS_WAD,
        liquidation_bonus_factor_bps: DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
    }
}

fn feed_raw() -> PriceFeedRaw {
    PriceFeedRaw {
        price_wad: WAD,
        asset_decimals: 7,
        timestamp: 0,
    }
}

fn single_price(env: &Env, asset: &Address) -> soroban_sdk::Map<Address, PriceFeedRaw> {
    let mut prices = soroban_sdk::Map::new(env);
    prices.set(asset.clone(), feed_raw());
    prices
}

fn index_raw() -> MarketIndexRaw {
    MarketIndexRaw {
        borrow_index: RAY,
        supply_index: RAY,
    }
}

fn hub_key(env: &Env) -> HubAssetKey {
    HubAssetKey {
        hub_id: 0,
        asset: Address::generate(env),
    }
}

fn repay_entry(env: &Env, amount: i128, usd_wad: i128) -> RepayEntry {
    RepayEntry {
        hub_asset: hub_key(env),
        amount,
        usd_wad,
        feed: feed_raw(),
        market_index: index_raw(),
    }
}

fn seize_entry(env: &Env, amount: i128, protocol_fee: i128) -> SeizeEntry {
    SeizeEntry {
        hub_asset: hub_key(env),
        amount,
        protocol_fee,
        feed: feed_raw(),
        market_index: index_raw(),
    }
}

fn plan_with(env: &Env, repay_usd: i128, seized: Vec<SeizeEntry>) -> LiquidationPlan {
    let mut repaid = Vec::new(env);
    repaid.push_back(repay_entry(env, 3, 3 * WAD));
    repaid.push_back(repay_entry(env, 2, 2 * WAD));
    LiquidationPlan {
        repayment: NormalizedRepaymentPlan {
            repaid,
            refunds: Vec::new(env),
            repay_usd: Wad::from(repay_usd),
            bonus: Bps::from(0i128),
        },
        seized,
    }
}

fn empty_account(env: &Env) -> Account {
    Account {
        owner: Address::generate(env),
        spoke_id: 1,
        mode: PositionMode::Normal,
        supply_positions: Map::new(env),
        borrow_positions: Map::new(env),
    }
}

fn repayment_fixture(env: &Env) -> (Address, HubAssetKey, Account) {
    let contract = env.register(crate::Controller, (Address::generate(env),));
    let asset = Address::generate(env);

    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };
    let mut borrow_positions = Map::new(env);
    borrow_positions.set(
        hub_asset.clone(),
        DebtPositionRaw {
            scaled_amount: Ray::from_asset(500_0000000, 7).raw(),
        },
    );
    let account = Account {
        owner: Address::generate(env),
        spoke_id: 1,
        mode: PositionMode::Normal,
        supply_positions: Map::new(env),
        borrow_positions,
    };

    (contract, hub_asset, account)
}

fn seize_fixture(env: &Env, fees_bps: u32) -> (Address, HubAssetKey, Account) {
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
            scaled_amount: Ray::from_asset(10_000_000_000, 7).raw(),
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

fn plan_for_seizure(env: &Env, repay_usd_raw: i128, bonus_bps: i128) -> NormalizedRepaymentPlan {
    NormalizedRepaymentPlan {
        repaid: Vec::new(env),
        refunds: Vec::new(env),
        repay_usd: Wad::from(repay_usd_raw),
        bonus: Bps::from(bonus_bps),
    }
}

fn run_seizure(env: &Env, fees_bps: u32, repay_usd_raw: i128, bonus_bps: i128) -> Vec<SeizeEntry> {
    let (contract, hub_asset, account) = seize_fixture(env, fees_bps);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(env);
        cache.set_prices(single_price(env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());
        let plan = plan_for_seizure(env, repay_usd_raw, bonus_bps);
        calculate_seized_collateral(env, &account, Wad::from(1_000 * WAD), &plan, &mut cache)
    })
}

fn stroops(tokens: i128) -> i128 {
    tokens * 10_000_000
}

fn snap(
    debt: i128,
    collateral: i128,
    weighted: i128,
    proportion: i128,
    hf: i128,
) -> LiquidationSnapshot {
    LiquidationSnapshot {
        total_debt: Wad::from(debt),
        total_collateral: Wad::from(collateral),
        weighted_coll: Wad::from(weighted),
        proportion_seized: Wad::from(proportion),
        hf: Wad::from(hf),
    }
}

#[test]
fn debt_close_amount_uses_pool_full_close_ceiling() {
    let env = Env::default();
    let position = DebtPosition {
        scaled_amount: Ray::from(RAY + RAY * 4 / 10),
    };

    assert_eq!(position.scaled_amount.mul(&env, Ray::ONE).to_asset(0), 1);
    assert_eq!(debt_close_amount(&env, &position, Ray::ONE, 0), 2);
}

#[test]
fn liquidation_plan_validate_accepts_consistent_plan() {
    let env = Env::default();
    let mut seized = Vec::new(&env);
    seized.push_back(seize_entry(&env, 10, 10));
    seized.push_back(seize_entry(&env, 1, 0));
    plan_with(&env, 5 * WAD, seized).validate(&env);
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn liquidation_plan_validate_rejects_repay_sum_mismatch() {
    let env = Env::default();
    let mut seized = Vec::new(&env);
    seized.push_back(seize_entry(&env, 10, 0));
    plan_with(&env, 5 * WAD + 1, seized).validate(&env);
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn liquidation_plan_validate_rejects_zero_seize_amount() {
    let env = Env::default();
    let mut seized = Vec::new(&env);
    seized.push_back(seize_entry(&env, 0, 0));
    plan_with(&env, 5 * WAD, seized).validate(&env);
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn liquidation_plan_validate_rejects_negative_protocol_fee() {
    let env = Env::default();
    let mut seized = Vec::new(&env);
    seized.push_back(seize_entry(&env, 5, -1));
    plan_with(&env, 5 * WAD, seized).validate(&env);
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn liquidation_plan_validate_rejects_fee_above_amount() {
    let env = Env::default();
    let mut seized = Vec::new(&env);
    seized.push_back(seize_entry(&env, 5, 6));
    plan_with(&env, 5 * WAD, seized).validate(&env);
}

#[test]
fn seizure_proportion_is_zero_for_zero_collateral() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let account = empty_account(&env);
        let mut cache = Cache::new_view(&env);
        let (proportion, bounds) =
            calculate_seizure_proportions(&env, &account, Wad::ZERO, Wad::ZERO, &mut cache);
        assert_eq!(proportion.raw(), 0);
        assert_eq!(bounds.base.raw(), 0);
    });
}

#[test]
fn seizure_proportion_divides_weighted_by_total() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let account = empty_account(&env);
        let mut cache = Cache::new_view(&env);
        let (proportion, _) = calculate_seizure_proportions(
            &env,
            &account,
            Wad::from(100 * WAD),
            Wad::from(50 * WAD),
            &mut cache,
        );
        assert_eq!(proportion.raw(), WAD / 2);
    });
}

#[test]
fn repayment_at_exact_debt_produces_no_refund() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let payments = vec![&env, (hub_asset.clone(), 500_0000000i128)];
        let mut refunds = Vec::new(&env);
        let (total, repaid) =
            calculate_repayment_amounts(&env, &payments, &account, &mut refunds, &mut cache);

        assert_eq!(refunds.len(), 0, "exact repayment must not create a refund");
        assert_eq!(repaid.len(), 1);
        assert_eq!(repaid.get_unchecked(0).amount, 500_0000000);
        assert_eq!(total.raw(), 500 * WAD);
    });
}

#[test]
fn repayment_above_debt_refunds_exact_excess() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let payments = vec![&env, (hub_asset.clone(), 500_0000005i128)];
        let mut refunds = Vec::new(&env);
        let (total, repaid) =
            calculate_repayment_amounts(&env, &payments, &account, &mut refunds, &mut cache);

        assert_eq!(refunds.len(), 1);
        let refund = refunds.get_unchecked(0);
        assert_eq!(refund.asset, hub_asset.asset);
        assert_eq!(refund.amount, 5, "refund must be payment minus debt");
        assert_eq!(repaid.get_unchecked(0).amount, 500_0000000);
        assert_eq!(total.raw(), 500 * WAD);
    });
}

#[test]
fn partial_seizure_floors_amount_and_zero_fee_stays_zero() {
    let env = Env::default();

    let seized = run_seizure(&env, 0, 100 * WAD + 50_000_000_000, 0);
    assert_eq!(seized.len(), 1);
    let entry = seized.get_unchecked(0);
    assert_eq!(entry.amount, 1_000_000_000);
    assert_eq!(entry.protocol_fee, 0);
}

#[test]
fn dust_protocol_fee_rounds_up_to_one_unit() {
    let env = Env::default();

    let seized = run_seizure(&env, 10_000, WAD / 10_000_000, 5_000);
    assert_eq!(seized.len(), 1);
    let entry = seized.get_unchecked(0);
    assert_eq!(entry.amount, 1);
    assert_eq!(entry.protocol_fee, 1);
}

#[test]
fn whole_unit_protocol_fee_is_exact() {
    let env = Env::default();

    let seized = run_seizure(&env, 1_000, 100 * WAD, 5_000);
    assert_eq!(seized.len(), 1);
    let entry = seized.get_unchecked(0);
    assert_eq!(entry.amount, 1_500_000_000);
    assert_eq!(entry.protocol_fee, 50_000_000);
}

#[test]
fn process_excess_payment_zero_excess_is_noop() {
    let env = Env::default();
    let mut repaid = Vec::new(&env);
    repaid.push_back(repay_entry(&env, stroops(100), 100 * WAD));
    let mut refunds = Vec::new(&env);

    process_excess_payment(&env, &mut repaid, &mut refunds, Wad::ZERO);

    assert_eq!(refunds.len(), 0);
    assert_eq!(repaid.len(), 1);
    assert_eq!(repaid.get_unchecked(0).amount, stroops(100));
}

#[test]
fn process_excess_payment_boundary_leg_is_removed() {
    let env = Env::default();
    let mut repaid = Vec::new(&env);
    repaid.push_back(repay_entry(&env, stroops(10), 10 * WAD));
    repaid.push_back(repay_entry(&env, stroops(5), 5 * WAD));
    let mut refunds = Vec::new(&env);

    process_excess_payment(&env, &mut repaid, &mut refunds, Wad::from(5 * WAD));

    assert_eq!(repaid.len(), 1, "the exactly-consumed leg must be removed");
    assert_eq!(repaid.get_unchecked(0).amount, stroops(10));
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds.get_unchecked(0).amount, stroops(5));
}

#[test]
fn process_excess_payment_survives_exhausting_all_legs() {
    let env = Env::default();
    let mut repaid = Vec::new(&env);
    repaid.push_back(repay_entry(&env, stroops(10), 5 * WAD));
    let mut refunds = Vec::new(&env);

    process_excess_payment(&env, &mut repaid, &mut refunds, Wad::from(8 * WAD));

    assert_eq!(repaid.len(), 0);
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds.get_unchecked(0).amount, stroops(10));
}

#[test]
fn process_excess_payment_spans_legs_with_pro_rata_split() {
    let env = Env::default();
    let mut repaid = Vec::new(&env);
    repaid.push_back(repay_entry(&env, stroops(100), 100 * WAD));
    repaid.push_back(repay_entry(&env, stroops(40), 40 * WAD));
    let mut refunds = Vec::new(&env);

    process_excess_payment(&env, &mut repaid, &mut refunds, Wad::from(60 * WAD));

    assert_eq!(refunds.len(), 2);
    assert_eq!(refunds.get_unchecked(0).amount, stroops(40));
    assert_eq!(refunds.get_unchecked(1).amount, stroops(20));
    assert_eq!(repaid.len(), 1);
    let kept = repaid.get_unchecked(0);
    assert_eq!(kept.amount, stroops(80));
    assert_eq!(kept.usd_wad, 80 * WAD);
}

#[test]
fn normalize_repayment_plan_requires_full_close_when_partials_ratchet() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let s = snap(500 * WAD, 100 * WAD, 40 * WAD, WAD, 400_000_000_000_000_000);
        let bounds = BonusBounds {
            base: Bps::from(0i128),
            max: Bps::from(0i128),
        };
        let curve = LiquidationCurve::from_config(&default_spoke_config());

        let payments = vec![&env, (hub_asset.clone(), 500_0000000i128)];
        let plan =
            normalize_repayment_plan(&env, &account, &payments, &s, bounds, &curve, &mut cache);

        assert_eq!(plan.repay_usd.raw(), 500 * WAD);
        assert_eq!(plan.bonus.raw(), 0);
        assert_eq!(plan.refunds.len(), 0);
        assert_eq!(plan.repaid.len(), 1);
        assert_eq!(plan.repaid.get_unchecked(0).amount, 500_0000000);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #135)")]
fn normalize_rejects_partial_on_solvent_toxic_account() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let s = snap(
            500 * WAD,
            520 * WAD,
            468 * WAD,
            9 * WAD / 10,
            93 * WAD / 100,
        );
        let bounds = BonusBounds {
            base: Bps::from(500i128),
            max: max_bonus_for_threshold(&env, s.proportion_seized),
        };
        let curve = LiquidationCurve::from_config(&default_spoke_config());

        let payments = vec![&env, (hub_asset.clone(), 100_0000000i128)];
        normalize_repayment_plan(&env, &account, &payments, &s, bounds, &curve, &mut cache);
    });
}

#[test]
fn normalize_accepts_full_close_on_solvent_toxic_account() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let s = snap(
            500 * WAD,
            520 * WAD,
            468 * WAD,
            9 * WAD / 10,
            93 * WAD / 100,
        );
        let bounds = BonusBounds {
            base: Bps::from(500i128),
            max: max_bonus_for_threshold(&env, s.proportion_seized),
        };
        let curve = LiquidationCurve::from_config(&default_spoke_config());

        let payments = vec![&env, (hub_asset.clone(), 500_0000000i128)];
        let plan =
            normalize_repayment_plan(&env, &account, &payments, &s, bounds, &curve, &mut cache);
        assert_eq!(plan.repay_usd.raw(), 500 * WAD);
        assert_eq!(plan.bonus.raw(), 500, "full close pays the base bonus");
    });
}

#[test]
fn normalize_accepts_partial_when_cap_equals_base() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let s = snap(
            500 * WAD,
            525 * WAD,
            420 * WAD,
            8 * WAD / 10,
            84 * WAD / 100,
        );
        assert_eq!(
            max_hf_preserving_bonus_bps(&s),
            Some(500),
            "cap must equal the base bonus for this boundary"
        );
        let bounds = BonusBounds {
            base: Bps::from(500i128),
            max: max_bonus_for_threshold(&env, s.proportion_seized),
        };
        let curve = LiquidationCurve::from_config(&default_spoke_config());

        let payments = vec![&env, (hub_asset.clone(), 100_0000000i128)];
        let plan =
            normalize_repayment_plan(&env, &account, &payments, &s, bounds, &curve, &mut cache);
        assert_eq!(plan.repay_usd.raw(), 100 * WAD, "boundary partial accepted");
        assert_eq!(plan.bonus.raw(), 500);
    });
}

fn insolvent_snap() -> LiquidationSnapshot {
    snap(
        120 * WAD,
        100 * WAD,
        80 * WAD,
        800_000_000_000_000_000,
        666_666_666_666_666_666,
    )
}

#[test]
fn partial_liquidation_of_insolvent_account_is_permitted() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let s = insolvent_snap();
        assert!(
            max_hf_preserving_bonus_bps(&s).is_some_and(|cap| cap < 0),
            "fixture must be insolvent so the cap is negative"
        );
        let bounds = BonusBounds {
            base: Bps::from(500i128),
            max: max_bonus_for_threshold(&env, s.proportion_seized),
        };
        let curve = LiquidationCurve::from_config(&default_spoke_config());

        let payments = vec![&env, (hub_asset.clone(), 100_0000000i128)];
        let plan =
            normalize_repayment_plan(&env, &account, &payments, &s, bounds, &curve, &mut cache);
        assert_eq!(plan.repay_usd.raw(), 100 * WAD, "partial accepted");
        assert_eq!(
            plan.bonus.raw(),
            500,
            "insolvent partial pays the base bonus"
        );
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #135)")]
fn normalize_rejects_underfunded_partial_when_cap_is_below_base_but_solvent() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let s = snap(
            500 * WAD,
            510 * WAD,
            408 * WAD,
            8 * WAD / 10,
            816 * WAD / 1000,
        );
        let cap = max_hf_preserving_bonus_bps(&s).expect("cap exists");
        assert!(
            (0..500).contains(&cap),
            "fixture must sit in the solvent 0 <= cap < base band, got {cap}"
        );
        let bounds = BonusBounds {
            base: Bps::from(500i128),
            max: max_bonus_for_threshold(&env, s.proportion_seized),
        };
        let curve = LiquidationCurve::from_config(&default_spoke_config());

        let payments = vec![&env, (hub_asset.clone(), 100_0000000i128)];
        normalize_repayment_plan(&env, &account, &payments, &s, bounds, &curve, &mut cache);
    });
}

#[test]
fn account_bonus_params_weights_bonus_by_collateral_share() {
    let env = Env::default();
    let (contract, hub_asset, account) = seize_fixture(&env, 0);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let bounds = get_account_bonus_params(
            &env,
            &mut cache,
            &account.supply_positions,
            Wad::from(1_000 * WAD),
            Wad::from(WAD / 2),
        );

        assert_eq!(bounds.max.raw(), 10_000);
        assert_eq!(bounds.base.raw(), 500);
    });
}

#[test]
fn account_bonus_params_zero_collateral_yields_zero_base() {
    let env = Env::default();
    let (contract, hub_asset, account) = seize_fixture(&env, 0);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let bounds = get_account_bonus_params(
            &env,
            &mut cache,
            &account.supply_positions,
            Wad::ZERO,
            Wad::from(WAD / 2),
        );

        assert_eq!(bounds.max.raw(), 10_000);
        assert_eq!(bounds.base.raw(), 0);
    });
}

#[test]
fn seizure_never_exceeds_collateral() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let collateral = 100 * WAD;
    let weighted = 80 * WAD;
    let proportion = 80 * WAD / 100;
    let bounds = BonusBounds {
        base: Bps::from(500i128),
        max: max_bonus_for_threshold(&env, Wad::from(proportion)),
    };

    let mut ceilings_checked = 0;

    for hf_pct in (10..100).step_by(5) {
        let debt = weighted * 100 / hf_pct as i128;
        let s = snap(
            debt,
            collateral,
            weighted,
            proportion,
            WAD * hf_pct as i128 / 100,
        );
        let (ideal, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);

        if ideal.raw() == s.total_debt.raw() {
            continue;
        }
        let seizure = ideal.mul(&env, Wad::ONE.checked_add(&env, bonus.to_wad(&env)));
        ceilings_checked += 1;
        assert!(
            seizure.raw() <= collateral + WAD / 1_000,
            "seizure {} exceeds collateral {} at hf={hf_pct}%",
            seizure.raw(),
            collateral
        );
    }

    assert!(
        ceilings_checked > 0,
        "swept the HF range without reaching a single non-escalated estimate"
    );
}

#[test]
fn escalated_full_close_over_asks_and_is_clamped_to_collateral() {
    let env = Env::default();
    let seized = run_seizure(&env, 0, 1_200 * WAD, 500);

    assert_eq!(seized.len(), 1, "single collateral leg expected");
    let entry = seized.get_unchecked(0);
    assert_eq!(
        entry.amount,
        stroops(1_000),
        "seizure must clamp to the whole position, never above it"
    );
}

#[test]
fn seizure_is_clamped_to_position_across_requests() {
    let env = Env::default();

    for repay_usd in [100i128, 500, 900, 1_000, 1_500, 5_000] {
        for bonus_bps in [0i128, 250, 500, 1_500] {
            let seized = run_seizure(&env, 0, repay_usd * WAD, bonus_bps);
            let got = seized.get_unchecked(0).amount;

            let want_unclamped = stroops(repay_usd) * (10_000 + bonus_bps) / 10_000;
            let want = want_unclamped.min(stroops(1_000));
            assert!(
                got <= stroops(1_000),
                "seized {got} exceeds the 1000-token position (repay=${repay_usd}, bonus={bonus_bps})"
            );
            assert!(
                (got - want).abs() <= 1,
                "seized {got} != expected {want} (repay=${repay_usd}, bonus={bonus_bps})"
            );
        }
    }
}

#[test]
fn over_payment_is_split_between_repaid_and_refunds_without_loss() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let offered = stroops(700);
        let mut refunds = Vec::new(&env);
        let payments = vec![&env, (hub_asset.clone(), offered)];
        let (repaid_usd, repaid) =
            calculate_repayment_amounts(&env, &payments, &account, &mut refunds, &mut cache);

        let repaid_amount: i128 = repaid.iter().map(|e| e.amount).sum();
        let refunded: i128 = refunds.iter().map(|r| r.amount).sum();

        assert_eq!(repaid_amount, stroops(500), "clamped to outstanding debt");
        assert_eq!(refunded, stroops(200), "excess returned");
        assert_eq!(
            repaid_amount + refunded,
            offered,
            "repaid + refunded must equal what was offered"
        );
        assert_eq!(repaid_usd.raw(), 500 * WAD, "USD total tracks the clamp");
    });
}

#[test]
fn normalize_conserves_value_when_offer_exceeds_ideal() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let s = snap(
            500 * WAD,
            525 * WAD,
            420 * WAD,
            8 * WAD / 10,
            84 * WAD / 100,
        );
        let bounds = BonusBounds {
            base: Bps::from(500i128),
            max: max_bonus_for_threshold(&env, s.proportion_seized),
        };
        let curve = LiquidationCurve::from_config(&default_spoke_config());

        let offered = stroops(500);
        let payments = vec![&env, (hub_asset.clone(), offered)];
        let plan =
            normalize_repayment_plan(&env, &account, &payments, &s, bounds, &curve, &mut cache);

        let repaid_amount: i128 = plan.repaid.iter().map(|e| e.amount).sum();
        let refunded: i128 = plan.refunds.iter().map(|r| r.amount).sum();
        assert_eq!(
            repaid_amount + refunded,
            offered,
            "normalize must not absorb value: repaid {repaid_amount} + refunded {refunded} != offered {offered}"
        );
        assert!(
            plan.repay_usd.raw() <= 500 * WAD,
            "repayment never exceeds outstanding debt"
        );
        assert_eq!(
            plan.repay_usd.raw(),
            repaid_amount * WAD / stroops(1),
            "repay_usd must equal the USD value of the repaid legs at $1"
        );
    });
}

// --- scale_seizures_to_received ------------------------------------------
//
// Liquidation sizes collateral from the repayment value the plan *intended* to
// collect. When a debt token delivers less than was sent, the seizure must
// shrink to match, or the liquidator walks away with collateral they never
// paid for.

fn seize(asset: &Address, amount: i128, protocol_fee: i128) -> SeizeEntry {
    SeizeEntry {
        hub_asset: HubAssetKey {
            hub_id: 1,
            asset: asset.clone(),
        },
        amount,
        protocol_fee,
        feed: feed_raw(),
        market_index: index_raw(),
    }
}

#[test]
fn scaling_is_exact_identity_when_every_token_delivered_in_full() {
    let env = Env::default();
    let asset = Address::generate(&env);
    let seized = vec![&env, seize(&asset, 1_000, 70)];

    let out = scale_seizures_to_received(&env, &seized, Wad::from(500), Wad::from(500));

    // No rounding drift may creep in on the well-behaved path.
    assert_eq!(out.get_unchecked(0).amount, 1_000);
    assert_eq!(out.get_unchecked(0).protocol_fee, 70);
}

#[test]
fn seizure_shrinks_in_proportion_to_value_actually_received() {
    let env = Env::default();
    let asset = Address::generate(&env);
    let seized = vec![&env, seize(&asset, 1_000, 70)];

    // Only 60% of the intended repayment value arrived.
    let out = scale_seizures_to_received(&env, &seized, Wad::from(300), Wad::from(500));

    assert_eq!(out.get_unchecked(0).amount, 600);
    assert_eq!(out.get_unchecked(0).protocol_fee, 42);
}

#[test]
fn scaling_rounds_down_so_the_residue_stays_with_the_liquidated_account() {
    let env = Env::default();
    let asset = Address::generate(&env);
    // 7 * 1 / 3 == 2.333..; the liquidator must get 2, not 3.
    let seized = vec![&env, seize(&asset, 7, 7)];

    let out = scale_seizures_to_received(&env, &seized, Wad::from(1), Wad::from(3));

    assert_eq!(out.get_unchecked(0).amount, 2);
    assert_eq!(out.get_unchecked(0).protocol_fee, 2);
}

#[test]
fn nothing_received_seizes_nothing() {
    let env = Env::default();
    let asset = Address::generate(&env);
    let seized = vec![&env, seize(&asset, 1_000, 70)];

    let out = scale_seizures_to_received(&env, &seized, Wad::ZERO, Wad::from(500));

    assert_eq!(out.get_unchecked(0).amount, 0);
    assert_eq!(out.get_unchecked(0).protocol_fee, 0);
}

#[test]
fn a_token_delivering_more_than_planned_cannot_inflate_the_seizure() {
    let env = Env::default();
    let asset = Address::generate(&env);
    let seized = vec![&env, seize(&asset, 1_000, 70)];

    let out = scale_seizures_to_received(&env, &seized, Wad::from(900), Wad::from(500));

    assert_eq!(out.get_unchecked(0).amount, 1_000);
    assert_eq!(out.get_unchecked(0).protocol_fee, 70);
}

/// Spoke-1 XLM as deployed on mainnet.
const MAINNET_XLM_BONUS_BPS: i128 = 900;
const MAINNET_XLM_FEES_BPS: u32 = 1_200;

fn seize_fixture_with_collateral(
    env: &Env,
    fees_bps: u32,
    collateral_tokens: i128,
) -> (Address, HubAssetKey, Account) {
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
            scaled_amount: Ray::from_asset(stroops(collateral_tokens), 7).raw(),
            liquidation_threshold: 7_800,
            liquidation_bonus: MAINNET_XLM_BONUS_BPS as u32,
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

/// A full close in the solvent-toxic band must not pay the liquidator less than
/// they put in. The seizure clamps to the collateral that exists, so the bonus is
/// never realised — but the fee is derived from the clamped seizure as though it
/// had been, and the difference is larger than the whole realised excess.
#[test]
fn full_close_in_the_solvent_toxic_band_pays_the_liquidator_a_positive_net() {
    let env = Env::default();

    // Collateral $1005 against debt $1000: solvent, but short of the $1090 a
    // full 9% bonus would need.
    let collateral_tokens = 1_005i128;
    let repaid_usd = 1_000 * WAD;

    let (contract, hub_asset, account) =
        seize_fixture_with_collateral(&env, MAINNET_XLM_FEES_BPS, collateral_tokens);
    let seized = env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());
        let plan = plan_for_seizure(&env, repaid_usd, MAINNET_XLM_BONUS_BPS);
        calculate_seized_collateral(
            &env,
            &account,
            Wad::from(collateral_tokens * WAD),
            &plan,
            &mut cache,
        )
    });

    let entry = seized.get_unchecked(0);
    assert_eq!(
        entry.amount,
        stroops(collateral_tokens),
        "the seizure must clamp to the collateral that actually exists"
    );

    // Everything in stroops at $1/token.
    let repaid = stroops(1_000);
    let net = entry.amount - repaid - entry.protocol_fee;
    assert!(
        net >= 0,
        "liquidator net must not be negative: seized={} repaid={} fee={} net={}",
        entry.amount,
        repaid,
        entry.protocol_fee,
        net
    );
}

fn seize_at(env: &Env, collateral_tokens: i128, repaid_usd: i128) -> SeizeEntry {
    let (contract, hub_asset, account) =
        seize_fixture_with_collateral(env, MAINNET_XLM_FEES_BPS, collateral_tokens);
    let seized = env.as_contract(&contract, || {
        let mut cache = Cache::new_view(env);
        cache.set_prices(single_price(env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());
        let plan = plan_for_seizure(env, repaid_usd, MAINNET_XLM_BONUS_BPS);
        calculate_seized_collateral(
            env,
            &account,
            Wad::from(collateral_tokens * WAD),
            &plan,
            &mut cache,
        )
    });
    seized.get_unchecked(0)
}

/// Collateral short of the debt itself: there is no excess to take a cut of, and
/// deriving a bonus from the clamped seizure would invent one.
#[test]
fn bad_debt_seizure_charges_no_fee_and_does_not_trap() {
    let env = Env::default();

    let entry = seize_at(&env, 900, 1_000 * WAD);

    assert_eq!(entry.amount, stroops(900), "seizure clamps to the collateral");
    assert_eq!(
        entry.protocol_fee, 0,
        "no realised excess means no fee, got {}",
        entry.protocol_fee
    );
}

/// The regression guard: when nothing clamps, the realised excess IS the full
/// bonus, so the fee must be bit-identical to the pre-change behaviour.
#[test]
fn unclamped_seizure_still_charges_the_full_bonus_fee() {
    let env = Env::default();

    // $2000 collateral against $1000 repaid: the 9% bonus fits with room spare.
    let entry = seize_at(&env, 2_000, 1_000 * WAD);

    assert_eq!(entry.amount, stroops(1_090), "seizure is repayment plus bonus");
    // 12% of the realised 90-token bonus.
    assert_eq!(
        entry.protocol_fee,
        (stroops(1_090) - stroops(1_000)) * i128::from(MAINNET_XLM_FEES_BPS) / 10_000,
        "the whole bonus is realised"
    );
}

// --- the protocol fee follows the realised excess -------------------------
//
// The seizure is sized as `repayment * (1 + bonus)` and then clamped to the
// collateral that exists. The fee is a cut of the excess the liquidator
// actually walked away with: `seized - repayment_share`, never a notional
// bonus reconstructed from the clamped seizure.

#[derive(Clone, Copy)]
struct LegSpec {
    /// Amount in the asset's own units, not whole tokens.
    amount: i128,
    decimals: u32,
    price_wad: i128,
    bonus_bps: u32,
    fees_bps: u32,
    threshold_bps: u32,
}

impl LegSpec {
    const fn mainnet_xlm(amount: i128) -> Self {
        LegSpec {
            amount,
            decimals: 7,
            price_wad: WAD,
            bonus_bps: MAINNET_XLM_BONUS_BPS as u32,
            fees_bps: MAINNET_XLM_FEES_BPS,
            threshold_bps: 7_800,
        }
    }

    const fn with_fees(mut self, fees_bps: u32) -> Self {
        self.fees_bps = fees_bps;
        self
    }
}

/// Build a real multi-leg account and derive `total_collateral` exactly the way
/// `calculate_account_risk_totals_body` does, so the shares the seizure computes
/// are the shares production would compute.
fn seize_legs(
    env: &Env,
    legs: &[LegSpec],
    repay_usd_raw: i128,
    plan_bonus_bps: i128,
) -> (Vec<Address>, Vec<SeizeEntry>) {
    let contract = env.register(crate::Controller, (Address::generate(env),));
    let mut prices = soroban_sdk::Map::new(env);
    let mut supply_positions = Map::new(env);
    let mut assets: Vec<Address> = Vec::new(env);
    let mut keys: Vec<HubAssetKey> = Vec::new(env);

    for leg in legs {
        let asset = Address::generate(env);
        prices.set(
            asset.clone(),
            PriceFeedRaw {
                price_wad: leg.price_wad,
                asset_decimals: leg.decimals,
                timestamp: 0,
            },
        );
        let key = HubAssetKey {
            hub_id: 0,
            asset: asset.clone(),
        };
        supply_positions.set(
            key.clone(),
            AccountPositionRaw {
                scaled_amount: Ray::from_asset(leg.amount, leg.decimals).raw(),
                liquidation_threshold: leg.threshold_bps,
                liquidation_bonus: leg.bonus_bps,
                loan_to_value: leg.threshold_bps - 500,
                liquidation_fees: leg.fees_bps,
            },
        );
        assets.push_back(asset);
        keys.push_back(key);
    }

    let account = Account {
        owner: Address::generate(env),
        spoke_id: 1,
        mode: PositionMode::Normal,
        supply_positions,
        borrow_positions: Map::new(env),
    };

    let seized = env.as_contract(&contract, || {
        let mut cache = Cache::new_view(env);
        cache.set_prices(prices.clone());
        for key in keys.iter() {
            cache.put_market_index(&key, &index_raw());
        }
        let mut total_collateral = Wad::ZERO;
        for (hub_asset, position) in iter_typed_positions(&account.supply_positions) {
            let feed = cache.cached_price(&hub_asset.asset);
            total_collateral = total_collateral.checked_add(
                env,
                risk::position_value(env, position.scaled_amount, Ray::ONE, feed.price),
            );
        }
        let plan = plan_for_seizure(env, repay_usd_raw, plan_bonus_bps);
        calculate_seized_collateral(env, &account, total_collateral, &plan, &mut cache)
    });

    (assets, seized)
}

fn leg_entry(seized: &Vec<SeizeEntry>, asset: &Address) -> SeizeEntry {
    seized
        .iter()
        .find(|e| e.hub_asset.asset == *asset)
        .expect("leg missing from the seizure")
}

fn single_leg(env: &Env, leg: LegSpec, repay_usd_raw: i128, bonus_bps: i128) -> Vec<SeizeEntry> {
    let (_, seized) = seize_legs(env, &[leg], repay_usd_raw, bonus_bps);
    seized
}

/// Collateral worth exactly the debt: the liquidator gets every stroop back and
/// not one more, so there is no excess for the protocol to take a cut of.
#[test]
fn seizure_at_exactly_the_debt_value_charges_no_protocol_fee() {
    let env = Env::default();

    let seized = single_leg(
        &env,
        LegSpec::mainnet_xlm(stroops(1_000)),
        1_000 * WAD,
        MAINNET_XLM_BONUS_BPS,
    );

    let entry = seized.get_unchecked(0);
    assert_eq!(entry.amount, stroops(1_000), "the whole position is seized");
    assert_eq!(entry.protocol_fee, 0, "zero excess means zero fee");
}

/// One stroop of collateral above the debt is one stroop of realised excess.
/// The fee is bumped off zero to a whole stroop, so it consumes the entire
/// excess -- but it must never exceed it, or the liquidator ends up under water.
#[test]
fn a_one_stroop_excess_is_charged_at_most_one_stroop_of_fee() {
    let env = Env::default();

    let seized = single_leg(
        &env,
        LegSpec::mainnet_xlm(stroops(1_000) + 1),
        1_000 * WAD,
        MAINNET_XLM_BONUS_BPS,
    );

    let entry = seized.get_unchecked(0);
    assert_eq!(entry.amount, stroops(1_000) + 1, "the position clamps");

    let realised_excess = entry.amount - stroops(1_000);
    assert_eq!(realised_excess, 1);
    assert_eq!(
        entry.protocol_fee, 1,
        "the fee bump takes the whole one-stroop excess"
    );
    assert!(
        entry.protocol_fee <= realised_excess,
        "fee {} exceeds the realised excess {}",
        entry.protocol_fee,
        realised_excess
    );
}

/// Collateral exactly equal to `repayment * (1 + bonus)` is the point where the
/// clamp stops binding. The fee here must equal the unclamped fee, because the
/// whole bonus is realised at that boundary and not one stroop more.
#[test]
fn seizure_exactly_at_the_bonus_boundary_charges_the_full_bonus_fee() {
    let env = Env::default();

    let seized = single_leg(
        &env,
        LegSpec::mainnet_xlm(stroops(1_090)),
        1_000 * WAD,
        MAINNET_XLM_BONUS_BPS,
    );

    let entry = seized.get_unchecked(0);
    assert_eq!(entry.amount, stroops(1_090), "the seizure just fits");
    assert_eq!(
        entry.protocol_fee,
        (stroops(1_090) - stroops(1_000)) * i128::from(MAINNET_XLM_FEES_BPS) / 10_000,
        "the boundary fee equals the unclamped fee"
    );
}

/// Fee rates are per collateral leg. With room to spare on both legs, each one
/// charges its own rate against its own realised bonus, never a blended rate.
#[test]
fn each_leg_charges_its_own_fee_rate_on_its_own_realised_bonus() {
    let env = Env::default();

    let (assets, seized) = seize_legs(
        &env,
        &[
            LegSpec::mainnet_xlm(stroops(1_000)).with_fees(1_200),
            LegSpec::mainnet_xlm(stroops(3_000)).with_fees(100),
        ],
        1_000 * WAD,
        MAINNET_XLM_BONUS_BPS,
    );

    // $4000 of collateral against a $1090 seizure: neither leg clamps, and the
    // seizure splits 1:3 by value.
    let cheap = leg_entry(&seized, &assets.get_unchecked(0));
    assert_eq!(cheap.amount, stroops(272) + 5_000_000);
    assert_eq!(
        cheap.protocol_fee,
        (cheap.amount - stroops(250)) * 1_200 / 10_000,
        "12% of its own 22.5-token bonus"
    );

    let dear = leg_entry(&seized, &assets.get_unchecked(1));
    assert_eq!(dear.amount, stroops(817) + 5_000_000);
    assert_eq!(
        dear.protocol_fee,
        (dear.amount - stroops(750)) * 100 / 10_000,
        "1% of its own 67.5-token bonus"
    );
}

/// Every leg's seizure is `repayment * (1 + bonus) * leg_value / total_value`,
/// so the clamp condition `seizure > position` reduces to
/// `repayment * (1 + bonus) > total_value` -- identical for every leg. A bad
/// debt close therefore clamps all legs at once and charges no fee anywhere,
/// whatever each leg's fee rate is.
#[test]
fn a_bad_debt_close_clamps_every_leg_and_charges_no_fee_on_any_of_them() {
    let env = Env::default();

    let (assets, seized) = seize_legs(
        &env,
        &[
            LegSpec::mainnet_xlm(stroops(200)).with_fees(1_200),
            LegSpec::mainnet_xlm(stroops(800)).with_fees(100),
        ],
        1_000 * WAD,
        MAINNET_XLM_BONUS_BPS,
    );

    let first = leg_entry(&seized, &assets.get_unchecked(0));
    let second = leg_entry(&seized, &assets.get_unchecked(1));
    assert_eq!(first.amount, stroops(200), "leg clamps to its position");
    assert_eq!(second.amount, stroops(800), "leg clamps to its position");
    assert_eq!(first.protocol_fee, 0, "no realised excess on a bad debt");
    assert_eq!(second.protocol_fee, 0, "no realised excess on a bad debt");
}

/// Because the clamp condition is leg-independent, a mixed clamp only exists
/// inside the rounding noise at the exact boundary. Sitting there, two legs land
/// on their whole position and the third lands one stroop short of it, and each
/// still charges its own rate on its own excess.
#[test]
fn at_the_clamp_boundary_legs_split_by_rounding_yet_keep_their_own_fee_rates() {
    let env = Env::default();

    // $1090 of collateral against a $1000 repayment at a 9% bonus: the seizure
    // is worth exactly the collateral, and the shares do not divide evenly.
    let (assets, seized) = seize_legs(
        &env,
        &[
            LegSpec::mainnet_xlm(3_333_333_333).with_fees(1_200),
            LegSpec::mainnet_xlm(3_636_363_636).with_fees(100),
            LegSpec::mainnet_xlm(3_930_303_031).with_fees(0),
        ],
        1_000 * WAD,
        MAINNET_XLM_BONUS_BPS,
    );

    let clamped_dear = leg_entry(&seized, &assets.get_unchecked(0));
    let clamped_cheap = leg_entry(&seized, &assets.get_unchecked(1));
    let unclamped = leg_entry(&seized, &assets.get_unchecked(2));

    assert_eq!(
        clamped_dear.amount, 3_333_333_333,
        "leg reaches its position"
    );
    assert_eq!(
        clamped_cheap.amount, 3_636_363_636,
        "leg reaches its position"
    );
    assert_eq!(
        unclamped.amount, 3_930_303_030,
        "the floored leg stays one stroop below its position"
    );

    assert_eq!(clamped_dear.protocol_fee, 33_027_522, "12% leg");
    assert_eq!(clamped_cheap.protocol_fee, 3_002_502, "1% leg");
    assert_eq!(unclamped.protocol_fee, 0, "0% leg pays nothing");

    // Each fee is bounded by the excess its own leg realised over its own share
    // of the repayment.
    for (entry, base) in [
        (&clamped_dear, 3_058_104_893i128),
        (&clamped_cheap, 3_336_113_427),
        (&unclamped, 3_605_782_596),
    ] {
        assert!(
            entry.protocol_fee <= entry.amount - base,
            "fee {} exceeds the leg's realised excess {}",
            entry.protocol_fee,
            entry.amount - base
        );
    }
}

/// With no bonus the repayment share IS the seizure, so there is never an excess
/// and never a fee -- whether or not the seizure clamps.
#[test]
fn a_zero_bonus_never_charges_a_protocol_fee() {
    let env = Env::default();

    let mut fits = LegSpec::mainnet_xlm(stroops(2_000));
    fits.bonus_bps = 0;
    let unclamped = single_leg(&env, fits, 1_000 * WAD, 0).get_unchecked(0);
    assert_eq!(unclamped.amount, stroops(1_000), "seizure is the repayment");
    assert_eq!(unclamped.protocol_fee, 0);

    let mut clamps = LegSpec::mainnet_xlm(stroops(500));
    clamps.bonus_bps = 0;
    let clamped = single_leg(&env, clamps, 1_000 * WAD, 0).get_unchecked(0);
    assert_eq!(clamped.amount, stroops(500), "seizure clamps");
    assert_eq!(clamped.protocol_fee, 0);
}

/// The per-account bonus ceiling is derived from the threshold:
/// `threshold * (1 + bonus) <= 100%`. At that ceiling a clamped seizure must
/// still charge only on the excess it realised, not on the 28.2% it asked for.
#[test]
fn at_the_derived_max_bonus_the_fee_follows_the_realised_excess() {
    let env = Env::default();

    let max = max_bonus_for_threshold(&env, Wad::from(7_800 * WAD / 10_000));
    assert_eq!(max.raw(), 2_820, "ceiling derived from the 78% threshold");

    // $1200 of collateral: the 28.2% bonus asks for $1282 and clamps.
    let mut leg = LegSpec::mainnet_xlm(stroops(1_200));
    leg.bonus_bps = max.raw() as u32;
    let entry = single_leg(&env, leg, 1_000 * WAD, max.raw()).get_unchecked(0);

    assert_eq!(entry.amount, stroops(1_200), "seizure clamps to collateral");
    let realised_excess = entry.amount - stroops(1_000);
    assert_eq!(
        entry.protocol_fee,
        realised_excess * i128::from(MAINNET_XLM_FEES_BPS) / 10_000,
        "12% of the 200 tokens actually realised, not of the 282 requested"
    );
    assert!(
        entry.amount - stroops(1_000) - entry.protocol_fee > 0,
        "liquidator keeps a positive net"
    );
}

/// KNOWN DEFECT, recorded as observed. When the whole bonus is worth less than
/// one unit of the asset, `protocol_fee_ray > 0 && fee_asset == 0` bumps the fee
/// to a whole unit. At one stroop of repayment the seizure floors back to the
/// repayment itself -- zero realised excess -- and the bump still charges one
/// stroop, so the liquidator nets minus one stroop.
#[test]
fn the_dust_fee_bump_charges_more_than_the_realised_excess() {
    let env = Env::default();

    // One stroop of repayment against ample collateral: seizure is 1.09 stroops.
    let repay_stroops = 1i128;
    let entry = single_leg(
        &env,
        LegSpec::mainnet_xlm(stroops(1_000)),
        WAD / 10_000_000,
        MAINNET_XLM_BONUS_BPS,
    )
    .get_unchecked(0);

    assert_eq!(entry.amount, 1, "1.09 stroops floors back to 1");
    let realised_excess = entry.amount - repay_stroops;
    assert_eq!(realised_excess, 0, "nothing above the repayment was seized");
    assert_eq!(entry.protocol_fee, 1, "the bump charges a whole stroop");
    assert_eq!(
        entry.amount - repay_stroops - entry.protocol_fee,
        -1,
        "the liquidator is one stroop out of pocket"
    );
}

/// A two-decimal asset at $0.13: the leg's units are coarse enough that both the
/// seizure and the fee floor visibly, and the fee must still land inside the
/// floored excess.
#[test]
fn a_two_decimal_asset_at_a_non_unit_price_charges_within_the_floored_excess() {
    let env = Env::default();

    // 1000 tokens at two decimals.
    let mut leg = LegSpec::mainnet_xlm(100_000);
    leg.decimals = 2;
    leg.price_wad = 130_000_000_000_000_000;
    let entry = single_leg(&env, leg, 100 * WAD, MAINNET_XLM_BONUS_BPS).get_unchecked(0);

    // $109 of seizure at $0.13 is 838.4615.. tokens, floored to 838.46.
    assert_eq!(entry.amount, 83_846);
    assert_eq!(entry.protocol_fee, 830);
    // $100 of repayment at $0.13 is 769.2307.. tokens.
    let realised_excess = entry.amount - 76_923;
    assert!(
        entry.protocol_fee <= realised_excess,
        "fee {} exceeds the realised excess {realised_excess}",
        entry.protocol_fee
    );
}

/// The same trade on an eighteen-decimal asset, the maximum the protocol allows.
/// Ray headroom above eighteen decimals is only nine digits, so this is where a
/// rescale would break first.
#[test]
fn an_eighteen_decimal_asset_at_a_non_unit_price_charges_within_the_realised_excess() {
    let env = Env::default();

    let mut leg = LegSpec::mainnet_xlm(1_000 * WAD);
    leg.decimals = 18;
    leg.price_wad = 130_000_000_000_000_000;
    let entry = single_leg(&env, leg, 100 * WAD, MAINNET_XLM_BONUS_BPS).get_unchecked(0);

    assert_eq!(entry.amount, 838_461538461538461538);
    assert_eq!(entry.protocol_fee, 8_307692307692307692);
    let realised_excess = entry.amount - 769_230769230769230769;
    assert!(
        entry.protocol_fee <= realised_excess,
        "fee {} exceeds the realised excess {realised_excess}",
        entry.protocol_fee
    );
}

/// A repayment so small against so expensive an asset that the seizure rounds to
/// zero: the leg is skipped outright rather than emitted with a zero amount,
/// which `LiquidationPlan::validate` would reject.
#[test]
fn a_leg_whose_seizure_rounds_to_zero_is_skipped_entirely() {
    let env = Env::default();

    let mut leg = LegSpec::mainnet_xlm(1);
    leg.decimals = 0;
    leg.price_wad = WAD * 1_000_000;
    let seized = single_leg(&env, leg, 100_000, MAINNET_XLM_BONUS_BPS);

    assert_eq!(seized.len(), 0, "no entry is emitted for a zero seizure");
}

/// The sibling of the above on the floor path: a leg so small that its share of
/// the seizure lands just under one stroop is dropped, while the legs beside it
/// are seized normally.
#[test]
fn a_sub_unit_leg_is_dropped_while_its_siblings_are_still_seized() {
    let env = Env::default();

    let (assets, seized) = seize_legs(
        &env,
        &[
            LegSpec::mainnet_xlm(1),
            LegSpec::mainnet_xlm(5_449_999_999),
            LegSpec::mainnet_xlm(5_450_000_000),
        ],
        1_000 * WAD,
        MAINNET_XLM_BONUS_BPS,
    );

    assert_eq!(seized.len(), 2, "the dust leg is dropped");
    assert!(
        !seized
            .iter()
            .any(|e| e.hub_asset.asset == assets.get_unchecked(0)),
        "the dust leg must not appear"
    );
    assert_eq!(
        leg_entry(&seized, &assets.get_unchecked(1)).amount,
        5_449_999_999
    );
    assert_eq!(
        leg_entry(&seized, &assets.get_unchecked(2)).amount,
        5_450_000_000
    );
}

// --- the full-close gate boundary (math.rs:160-169) -----------------------

/// `cap >= 0` is the gate's solvency test. A cap of exactly zero means any bonus
/// at all pushes the health factor down, so an underfunded partial is refused.
#[test]
#[should_panic(expected = "Error(Contract, #135)")]
fn the_full_close_gate_fires_when_the_hf_preserving_cap_is_exactly_zero() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let s = snap(500 * WAD, 500 * WAD, 400 * WAD, 8 * WAD / 10, 8 * WAD / 10);
        assert_eq!(
            max_hf_preserving_bonus_bps(&s),
            Some(0),
            "fixture must sit exactly on the cap == 0 boundary"
        );
        let bounds = BonusBounds {
            base: Bps::from(500i128),
            max: max_bonus_for_threshold(&env, s.proportion_seized),
        };
        let curve = LiquidationCurve::from_config(&default_spoke_config());

        let payments = vec![&env, (hub_asset.clone(), 100_0000000i128)];
        normalize_repayment_plan(&env, &account, &payments, &s, bounds, &curve, &mut cache);
    });
}

/// One basis point the other side of the same boundary the account is insolvent,
/// the gate stops applying, and the underfunded partial is accepted.
#[test]
fn the_full_close_gate_yields_when_the_hf_preserving_cap_is_one_bp_negative() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        let hf = 799_920_000_000_000_000i128;
        let debt = Wad::from(400 * WAD).div(&env, Wad::from(hf)).raw();
        let s = snap(debt, 500 * WAD, 400 * WAD, 8 * WAD / 10, hf);
        assert_eq!(
            max_hf_preserving_bonus_bps(&s),
            Some(-1),
            "fixture must sit one bp below the cap == 0 boundary"
        );
        let bounds = BonusBounds {
            base: Bps::from(500i128),
            max: max_bonus_for_threshold(&env, s.proportion_seized),
        };
        let curve = LiquidationCurve::from_config(&default_spoke_config());

        let payments = vec![&env, (hub_asset.clone(), 100_0000000i128)];
        let plan =
            normalize_repayment_plan(&env, &account, &payments, &s, bounds, &curve, &mut cache);

        assert_eq!(plan.repay_usd.raw(), 100 * WAD, "partial accepted");
        assert_eq!(plan.bonus.raw(), 500, "the base bonus is paid");
    });
}
