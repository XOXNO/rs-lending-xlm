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
