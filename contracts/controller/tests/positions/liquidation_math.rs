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
        // Credit-mode fields; these fixtures exercise the asset-unit pair.
        scaled_amount: amount,
        bonus_scaled: 0,
        liquidation_fees: 0,
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
        // Credit-mode fields; these fixtures exercise the asset-unit pair.
        scaled_amount: amount,
        bonus_scaled: 0,
        liquidation_fees: 0,
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

/// A full close in the solvent-toxic band pays the liquidator a positive net.
///
/// The seizure clamps to the collateral that exists, so the bonus is not fully
/// realised; the fee follows the realised excess and so stays below it.
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

    assert_eq!(
        entry.amount,
        stroops(900),
        "seizure clamps to the collateral"
    );
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

    assert_eq!(
        entry.amount,
        stroops(1_090),
        "seizure is repayment plus bonus"
    );
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

// --- small-position liquidation profitability -----------------------------
//
// ChainSecurity Mar-2026 note 8.4 derives the position value below which a
// liquidation loses money to rounding: `V < L_round / (b * (1 - f))`, where
// `L_round` is the summed rounding loss, `b` the bonus and `f` the protocol's
// cut of it. Their count was 2 debt-leg sites plus 2 collateral-leg sites.
//
// Ours is not that count. The debt leg's asset-unit ceiling
// (`unscale_borrow_ceil` in `calculate_repayment_amounts`) is priced back into
// `RepayEntry::usd_wad`, which is what `calculate_seized_collateral` multiplies
// by `(1 + bonus)` — the liquidator is credited for every unit it ceils, so the
// debt leg costs it nothing at asset-unit granularity. Both surviving sites are
// on the collateral leg, per seized position:
//
//   1. `capped_ray.to_asset_floor(decimals)` on a partial seizure  -> <= 1 unit
//   2. the dust fee bump, `protocol_fee_ray > 0 && fee_asset == 0` -> <= 1 unit
//
// So `L_round = 2 units of collateral` per leg, and because seizure is pro-rata
// across every collateral the account holds, it scales with the leg count.
//
// See docs/reference/numeric-bounds.md §6.

/// The load-bearing half of that claim, pinned on its own: a full close pays
/// `ceil(debt)` asset units, and `RepayEntry::usd_wad` is the price of what was
/// actually transferred, not of the exact debt. `calculate_seized_collateral`
/// then sizes the seizure from `repay_usd * (1 + bonus)`, so the ceiling comes
/// back to the liquidator with the bonus on top instead of being a loss.
///
/// This is what makes our rounding-site count 0 + 2 rather than ChainSecurity's
/// 2 + 2 for Aave.
#[test]
fn the_debt_legs_asset_unit_ceiling_is_priced_into_the_repayment_credit() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    let asset = Address::generate(&env);
    let hub_asset = HubAssetKey {
        hub_id: 0,
        asset: asset.clone(),
    };

    // A zero-decimal debt worth exactly 1.5 tokens: the exact value is 1.5 WAD,
    // the closeable amount is 2 units.
    let mut borrow_positions = Map::new(&env);
    borrow_positions.set(
        hub_asset.clone(),
        DebtPositionRaw {
            scaled_amount: RAY * 3 / 2,
        },
    );
    let account = Account {
        owner: Address::generate(&env),
        spoke_id: 1,
        mode: PositionMode::Normal,
        supply_positions: Map::new(&env),
        borrow_positions,
    };

    env.as_contract(&contract, || {
        let mut prices = soroban_sdk::Map::new(&env);
        prices.set(
            asset.clone(),
            PriceFeedRaw {
                price_wad: WAD,
                asset_decimals: 0,
                timestamp: 0,
            },
        );
        let mut cache = Cache::new_view(&env);
        cache.set_prices(prices);
        cache.put_market_index(&hub_asset, &index_raw());

        let payments = vec![&env, (hub_asset.clone(), 2i128)];
        let mut refunds = Vec::new(&env);
        let (total, repaid) =
            calculate_repayment_amounts(&env, &payments, &account, &mut refunds, &mut cache);

        assert_eq!(refunds.len(), 0, "2 units is exactly the closeable amount");
        assert_eq!(repaid.get_unchecked(0).amount, 2);
        assert_eq!(
            total.raw(),
            2 * WAD,
            "the credit is the price of the ceiling, not of the 1.5-token debt",
        );
        assert!(
            total.raw() > 3 * WAD / 2,
            "and it strictly exceeds the exact debt value",
        );
    });
}

/// A listed collateral as `configs/mainnet` configures it, priced at its
/// oracle's `max_sanity_price_wad` — the highest price the feed will accept, and
/// therefore the coarsest its asset unit can get.
struct ListedCollateral {
    label: &'static str,
    decimals: u32,
    price_wad: i128,
    bonus_bps: i128,
    fees_bps: u32,
}

const LISTED_COLLATERALS: [ListedCollateral; 7] = [
    ListedCollateral {
        label: "SolvBTC (spoke 1)",
        decimals: 8,
        price_wad: 120_000 * WAD,
        bonus_bps: 900,
        fees_bps: 1_200,
    },
    ListedCollateral {
        label: "xSolvBTCSolvBTC_LP (spoke 6)",
        decimals: 7,
        price_wad: 12_000 * WAD,
        bonus_bps: 1_000,
        fees_bps: 100,
    },
    ListedCollateral {
        label: "SPIKOUKTBL (spoke 3)",
        decimals: 5,
        price_wad: 1_480_358_160_000_000_000,
        bonus_bps: 600,
        fees_bps: 1_200,
    },
    ListedCollateral {
        label: "XAUM (spoke 8)",
        decimals: 9,
        price_wad: 6_000 * WAD,
        bonus_bps: 800,
        fees_bps: 1_000,
    },
    ListedCollateral {
        label: "XLM (spoke 1)",
        decimals: 7,
        price_wad: WAD,
        bonus_bps: 900,
        fees_bps: 1_200,
    },
    ListedCollateral {
        label: "USDC (spoke 5, lowest bonus listed)",
        decimals: 7,
        price_wad: 1_050_000_000_000_000_000,
        bonus_bps: 200,
        fees_bps: 1_000,
    },
    ListedCollateral {
        label: "USST (spoke 5)",
        decimals: 18,
        price_wad: 1_089_700_000_000_000_000,
        bonus_bps: 500,
        fees_bps: 1_000,
    },
];

/// Basis-points denominator, bound locally so this section adds no imports.
const BPS_DENOM: i128 = crate::constants::BPS;

/// One asset unit of this collateral, valued in WAD USD.
fn unit_value_usd_wad(c: &ListedCollateral) -> i128 {
    c.price_wad / 10i128.pow(c.decimals)
}

/// `L_round` for one seized leg: the seizure floor plus the dust fee bump.
fn rounding_loss_usd_wad(c: &ListedCollateral) -> i128 {
    2 * unit_value_usd_wad(c)
}

/// `V* = L_round / (b * (1 - f))` in WAD USD: the repayment below which the
/// bonus cannot cover the rounding, for `legs` seized collateral positions.
fn unprofitable_below_usd_wad(c: &ListedCollateral, legs: i128) -> i128 {
    let numerator = rounding_loss_usd_wad(c) * legs * BPS_DENOM * BPS_DENOM;
    let denominator = c.bonus_bps * (BPS_DENOM - i128::from(c.fees_bps));
    numerator / denominator + 1
}

/// The closed form, evaluated against the configured floor rather than a
/// liquidation. `MinBorrowCollateralUsd` gates every borrow, and
/// `BAD_DEBT_USD_THRESHOLD` promotes anything smaller to a full close or to
/// permissionless socialization, so the floor is the smallest position a
/// liquidator is ever asked to clear at a profit.
#[test]
fn the_min_borrow_collateral_floor_clears_the_unprofitability_threshold_for_every_listed_pair() {
    let floor = crate::constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD;

    assert_eq!(
        crate::constants::BAD_DEBT_USD_THRESHOLD,
        floor,
        "the dust-socialization gate and the borrow floor are the same number",
    );

    for c in LISTED_COLLATERALS.iter() {
        // Worst case: the account holds the maximum number of supply positions
        // and every one of them is this asset, so every leg pays L_round.
        let legs = i128::from(crate::constants::POSITION_LIMIT_MAX);
        let threshold = unprofitable_below_usd_wad(c, legs);

        assert!(
            threshold < floor,
            "{}: unprofitable below {} wad, floor is only {} wad",
            c.label,
            threshold,
            floor,
        );
        // Not merely above it -- comfortably above it. The tightest listed pair
        // is SolvBTC at 8 decimals and $120k, and it still leaves 30x.
        assert!(
            threshold * 30 < floor,
            "{}: margin over the floor fell below 30x ({} wad vs {} wad)",
            c.label,
            threshold,
            floor,
        );
    }
}

/// The same claim, run through `calculate_seized_collateral` instead of the
/// closed form: at a floor-sized repayment the liquidator walks away with more
/// than it paid, and its shortfall against the ideal `repay * b * (1 - f)` never
/// exceeds `L_round`.
#[test]
fn a_floor_sized_liquidation_pays_the_liquidator_for_every_listed_collateral() {
    let env = Env::default();
    let floor = crate::constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD;

    for c in LISTED_COLLATERALS.iter() {
        // Ample collateral: the seizure must not clamp, or the floor site and
        // the fee bump never fire.
        let collateral_units = Wad::from(10_000 * WAD)
            .div(&env, Wad::from(c.price_wad))
            .to_token(c.decimals);

        let leg = LegSpec {
            amount: collateral_units,
            decimals: c.decimals,
            price_wad: c.price_wad,
            bonus_bps: c.bonus_bps as u32,
            fees_bps: c.fees_bps,
            threshold_bps: 7_000,
        };

        let seized = single_leg(&env, leg, floor, c.bonus_bps);
        assert_eq!(seized.len(), 1, "{}: the leg must be seized", c.label);
        let entry = seized.get_unchecked(0);

        let net_units = entry.amount - entry.protocol_fee;
        let net_usd = Wad::from_token(net_units, c.decimals)
            .mul_floor(&env, Wad::from(c.price_wad))
            .raw();

        let profit = net_usd - floor;
        assert!(
            profit > 0,
            "{}: liquidator net {} wad against a {} wad repayment",
            c.label,
            net_usd,
            floor,
        );

        // `repay * b * (1 - f)`, the profit with no rounding at all.
        let ideal =
            floor * c.bonus_bps / BPS_DENOM * (BPS_DENOM - i128::from(c.fees_bps)) / BPS_DENOM;
        assert!(
            ideal - profit <= rounding_loss_usd_wad(c),
            "{}: rounding cost {} wad exceeds the two-unit bound {} wad",
            c.label,
            ideal - profit,
            rounding_loss_usd_wad(c),
        );
    }
}

/// The realised numbers behind the table in docs/reference/numeric-bounds.md §6,
/// pinned so the documentation cannot drift away from the code. Same fixtures as
/// the test above, in the same order as `LISTED_COLLATERALS`.
#[test]
fn floor_sized_liquidation_profits_match_the_documented_table() {
    let env = Env::default();
    let floor = crate::constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD;

    // (seized units, protocol fee units, liquidator profit in WAD USD)
    let expected: [(i128, i128, i128); 7] = [
        (4_541, 45, 395_200_000_000_000_000),
        (4_583, 4, 494_800_000_000_000_000),
        (358_021, 2_431, 264_005_581_144_000_000),
        (900_000, 6_666, 360_004_000_000_000_000),
        (54_500_000, 540_000, 396_000_000_000_000_000),
        (48_571_428, 95_238, 89_999_950_000_000_000),
        (
            4_817_839_772_414_425_989,
            22_942_094_154_354_409,
            225_000_000_000_000_000,
        ),
    ];

    for (c, want) in LISTED_COLLATERALS.iter().zip(expected.iter()) {
        let collateral_units = Wad::from(10_000 * WAD)
            .div(&env, Wad::from(c.price_wad))
            .to_token(c.decimals);
        let leg = LegSpec {
            amount: collateral_units,
            decimals: c.decimals,
            price_wad: c.price_wad,
            bonus_bps: c.bonus_bps as u32,
            fees_bps: c.fees_bps,
            threshold_bps: 7_000,
        };
        let entry = single_leg(&env, leg, floor, c.bonus_bps).get_unchecked(0);
        let profit = Wad::from_token(entry.amount - entry.protocol_fee, c.decimals)
            .mul_floor(&env, Wad::from(c.price_wad))
            .raw()
            - floor;

        assert_eq!(
            (entry.amount, entry.protocol_fee, profit),
            *want,
            "{}",
            c.label
        );
    }
}

/// The finding. Nothing bounds an asset's *unit* value. `MIN_ASSET_DECIMALS` is
/// 3 and `validate_sanity_bounds` accepts a price up to
/// `MAX_REASONABLE_PRICE_WAD` ($1e9 per whole token), so one base unit may be
/// worth $1,000,000: two hundred thousand times the entire borrow floor. At that
/// granularity a floor-sized liquidation seizes *nothing*. The whole seizure
/// floors to zero units and the leg is dropped, while the repayment still
/// settles and the debt is still burned.
///
/// This is a listing-admission constraint, not a code defect: no asset in
/// `configs/mainnet` is within four orders of magnitude of it. The condition
/// governance must check before listing a collateral is
/// `unit_value <= MinBorrowCollateralUsd * b * (1 - f) / (2 * legs)`.
#[test]
fn an_expensive_low_decimal_collateral_makes_a_floor_sized_liquidation_seize_nothing() {
    let env = Env::default();
    let floor = crate::constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD;

    let hostile = ListedCollateral {
        label: "3-decimal asset at the maximum reasonable price",
        decimals: crate::constants::MIN_ASSET_DECIMALS,
        price_wad: crate::constants::MAX_REASONABLE_PRICE_WAD,
        bonus_bps: 900,
        fees_bps: 1_200,
    };

    // Both halves of the configuration are governance-admissible today.
    assert_eq!(hostile.decimals, crate::constants::MIN_ASSET_DECIMALS);
    assert_eq!(
        hostile.price_wad,
        crate::constants::MAX_REASONABLE_PRICE_WAD
    );

    // One base unit is worth $1,000,000: 200,000x the entire borrow floor.
    assert_eq!(unit_value_usd_wad(&hostile), 1_000_000 * WAD);
    assert!(
        unprofitable_below_usd_wad(&hostile, 1) > floor,
        "the closed form must already flag this pair",
    );

    let leg = LegSpec {
        amount: 10,
        decimals: hostile.decimals,
        price_wad: hostile.price_wad,
        bonus_bps: hostile.bonus_bps as u32,
        fees_bps: hostile.fees_bps,
        threshold_bps: 7_000,
    };

    let seized = single_leg(&env, leg, floor, hostile.bonus_bps);
    assert_eq!(
        seized.len(),
        0,
        "a $5.45 seizure of a $1,000,000-per-unit asset rounds to zero units",
    );
}

/// Where the boundary actually is, for the same 900 bps / 1,200 bps curve at
/// three decimals: the closed form puts it at a $198 token price, and the
/// realised net goes negative a little later because the floor loss is usually
/// well under a full unit. Below the boundary a floor-sized close still pays.
#[test]
fn the_profitability_boundary_at_three_decimals_sits_between_198_and_237_dollars() {
    let env = Env::default();
    let floor = crate::constants::DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD;

    let at_price = |price_usd: i128| -> i128 {
        let c = ListedCollateral {
            label: "3-decimal probe",
            decimals: 3,
            price_wad: price_usd * WAD,
            bonus_bps: 900,
            fees_bps: 1_200,
        };
        let leg = LegSpec {
            amount: 1_000_000,
            decimals: c.decimals,
            price_wad: c.price_wad,
            bonus_bps: c.bonus_bps as u32,
            fees_bps: c.fees_bps,
            threshold_bps: 7_000,
        };
        let seized = single_leg(&env, leg, floor, c.bonus_bps);
        if seized.is_empty() {
            return -floor;
        }
        let entry = seized.get_unchecked(0);
        Wad::from_token(entry.amount - entry.protocol_fee, c.decimals)
            .mul_floor(&env, Wad::from(c.price_wad))
            .raw()
            - floor
    };

    // The closed-form bound `2 * unit <= repay * b * (1 - f)` breaks at $198:
    // 2 * 0.198 = 0.396 = 5 * 0.09 * 0.88.
    assert!(at_price(198) > 0, "the bound must be conservative at $198");
    // The realised net survives past it, and turns negative at $237.
    assert!(at_price(236) > 0, "still profitable one dollar below");
    assert!(at_price(237) < 0, "the realised net turns negative at $237");
}

// --- V-6: splitting a close into N partials is never more profitable --------
//
// CS-AAVE4-009 against Aave: when `proportion_seized * (1 + bonus) > HF`, each
// partial liquidation *lowers* the health factor, the bonus curve pays more at
// the lower health factor, and N slices therefore extract more collateral than
// one close of the summed repayment. Aave forbade the configuration off-chain.
// We clamp at runtime instead: `max_hf_preserving_bonus_bps` caps the bonus at
// `HF / proportion_seized - 1`, which is exactly the rate that leaves the
// health factor unchanged, so the next slice cannot be paid any better.
//
// The chain below runs the production plan path in `build_liquidation_plan`'s
// own order -- risk totals, seizure proportions, `normalize_repayment_plan`,
// `calculate_seized_collateral` -- and feeds each step's seizure and repayment
// back into the book before the next step.

/// Liquidation threshold of the single collateral leg. Because the account
/// holds exactly one collateral asset, this is also `proportion_seized`.
const SPLIT_LT_BPS: u32 = 8_000;
/// The leg's configured liquidation bonus, so `BonusBounds::base` is this.
const SPLIT_BONUS_BPS: u32 = 500;
/// Collateral in the splitting book: $1,125 against $1,000 of debt puts the
/// health factor at 0.9 and keeps it there under an HF-preserving seizure.
const SPLIT_COLLATERAL_TOKENS: i128 = 1_125;
/// Debt in the splitting book.
const SPLIT_DEBT_TOKENS: i128 = 1_000;

struct SplitBook {
    contract: Address,
    owner: Address,
    coll: HubAssetKey,
    debt: HubAssetKey,
}

fn split_book(env: &Env) -> SplitBook {
    SplitBook {
        contract: env.register(crate::Controller, (Address::generate(env),)),
        owner: Address::generate(env),
        coll: hub_key(env),
        debt: hub_key(env),
    }
}

impl SplitBook {
    fn prices(&self, env: &Env) -> soroban_sdk::Map<Address, PriceFeedRaw> {
        let mut prices = soroban_sdk::Map::new(env);
        prices.set(self.coll.asset.clone(), feed_raw());
        prices.set(self.debt.asset.clone(), feed_raw());
        prices
    }

    fn account(&self, env: &Env, coll_stroops: i128, debt_stroops: i128) -> Account {
        let mut supply_positions = Map::new(env);
        supply_positions.set(
            self.coll.clone(),
            AccountPositionRaw {
                scaled_amount: Ray::from_asset(coll_stroops, 7).raw(),
                liquidation_threshold: SPLIT_LT_BPS,
                liquidation_bonus: SPLIT_BONUS_BPS,
                loan_to_value: 7_500,
                liquidation_fees: 0,
            },
        );
        let mut borrow_positions = Map::new(env);
        borrow_positions.set(
            self.debt.clone(),
            DebtPositionRaw {
                scaled_amount: Ray::from_asset(debt_stroops, 7).raw(),
            },
        );
        Account {
            owner: self.owner.clone(),
            spoke_id: 1,
            mode: PositionMode::Normal,
            supply_positions,
            borrow_positions,
        }
    }
}

#[derive(Clone, Copy)]
struct SliceOutcome {
    /// Collateral units leaving the account.
    seized: i128,
    /// Debt units the plan actually accepted.
    repaid: i128,
    /// Bonus the plan paid, in basis points.
    bonus_bps: i128,
    /// Health factor the slice started from.
    hf_wad: i128,
}

/// Liquidates a book of `coll_stroops` collateral against `debt_stroops` debt
/// with an offer of `offer_stroops`, through the production plan path.
fn liquidate_slice(
    env: &Env,
    book: &SplitBook,
    coll_stroops: i128,
    debt_stroops: i128,
    offer_stroops: i128,
) -> SliceOutcome {
    let account = book.account(env, coll_stroops, debt_stroops);
    env.as_contract(&book.contract, || {
        let mut cache = Cache::new_view(env);
        cache.set_prices(book.prices(env));
        cache.put_market_index(&book.coll, &index_raw());
        cache.put_market_index(&book.debt, &index_raw());

        let totals = risk::calculate_account_risk_totals(
            env,
            &mut cache,
            &account.supply_positions,
            &account.borrow_positions,
        );
        let (proportion_seized, bounds) = calculate_seizure_proportions(
            env,
            &account,
            totals.total_collateral,
            totals.weighted_collateral,
            &mut cache,
        );
        let s = LiquidationSnapshot {
            total_debt: totals.total_debt,
            total_collateral: totals.total_collateral,
            weighted_coll: totals.weighted_collateral,
            proportion_seized,
            hf: totals.health_factor,
        };
        let curve = LiquidationCurve::from_config(&default_spoke_config());
        let payments = vec![env, (book.debt.clone(), offer_stroops)];
        let plan =
            normalize_repayment_plan(env, &account, &payments, &s, bounds, &curve, &mut cache);
        let seized =
            calculate_seized_collateral(env, &account, totals.total_collateral, &plan, &mut cache);

        SliceOutcome {
            seized: seized.iter().map(|e| e.amount).sum(),
            repaid: plan.repaid.iter().map(|e| e.amount).sum(),
            bonus_bps: plan.bonus.raw(),
            hf_wad: totals.health_factor.raw(),
        }
    })
}

/// The splitting book sits exactly on the CS-AAVE4-009 precondition: the bonus
/// curve asks for more than the health factor can support, so without the clamp
/// every partial would erode the health factor and earn a larger bonus next
/// time. The cap still sits above the base bonus, so partials stay legal --
/// `normalize_repayment_plan`'s `FullCloseRequired` gate is not what is under
/// test here.
#[test]
fn the_splitting_book_is_where_the_curve_out_asks_the_hf_preserving_cap() {
    let env = Env::default();
    let s = snap(
        SPLIT_DEBT_TOKENS * WAD,
        SPLIT_COLLATERAL_TOKENS * WAD,
        SPLIT_COLLATERAL_TOKENS * WAD * i128::from(SPLIT_LT_BPS) / 10_000,
        WAD * i128::from(SPLIT_LT_BPS) / 10_000,
        9 * WAD / 10,
    );
    let cap = max_hf_preserving_bonus_bps(&s).expect("cap exists below one WAD");
    let max = max_bonus_for_threshold(&env, s.proportion_seized);
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let curve_bonus = crate::positions::liquidation::curve::calculate_linear_bonus_with_target(
        &env,
        s.hf,
        Bps::from(i128::from(SPLIT_BONUS_BPS)),
        max,
        &curve,
        Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD),
    );

    assert_eq!(cap, 1_250, "HF 0.9 over a 0.8 seizure proportion");
    assert!(
        cap >= i128::from(SPLIT_BONUS_BPS),
        "partials must stay legal: cap {cap} is below the base bonus"
    );
    assert!(
        curve_bonus.raw() > cap,
        "the curve must out-ask the cap or the clamp is not exercised: \
         curve={} cap={cap}",
        curve_bonus.raw()
    );
}

/// Four sequential partial liquidations of the same account must not extract
/// more collateral than one liquidation repaying the same total.
#[test]
fn a_chain_of_partial_liquidations_never_out_extracts_one_summed_close() {
    let env = Env::default();
    let book = split_book(&env);
    const SLICES: i128 = 4;

    let coll_0 = stroops(SPLIT_COLLATERAL_TOKENS);
    let debt_0 = stroops(SPLIT_DEBT_TOKENS);
    let slice = stroops(100);

    let mut coll = coll_0;
    let mut debt = debt_0;
    let mut chain_seized = 0i128;
    let mut chain_repaid = 0i128;

    for step in 0..SLICES {
        let out = liquidate_slice(&env, &book, coll, debt, slice);
        assert!(
            out.hf_wad < WAD,
            "step {step} started from a healthy book, so this is not a liquidation chain"
        );
        assert_eq!(out.repaid, slice, "step {step} must take the whole slice");
        assert!(out.seized > 0, "step {step} seized nothing");
        chain_seized += out.seized;
        chain_repaid += out.repaid;
        coll -= out.seized;
        debt -= out.repaid;
    }
    assert_eq!(chain_repaid, slice * SLICES);

    let single = liquidate_slice(&env, &book, coll_0, debt_0, chain_repaid);
    assert_eq!(
        single.repaid, chain_repaid,
        "the single close must take the same repayment as the chain"
    );

    // One unit of floor-rounding slack per step, nothing more.
    let tolerance = SLICES;
    assert!(
        chain_seized <= single.seized + tolerance,
        "splitting out-extracted a single close: chain={chain_seized} \
         single={} excess={}",
        single.seized,
        chain_seized - single.seized
    );
    assert!(
        chain_seized + tolerance >= single.seized,
        "the chain extracted materially less than one close, so the bound above \
         is slack rather than tight: chain={chain_seized} single={}",
        single.seized
    );
}

/// The never-recovering path: the account stays liquidatable for the whole
/// chain, and the clamp holds its health factor flat instead of letting each
/// slice ratchet it down. Eight slices, so the ratchet has room to compound.
#[test]
fn a_never_recovering_position_holds_its_health_factor_across_a_long_chain() {
    let env = Env::default();
    let book = split_book(&env);
    const SLICES: i128 = 8;

    let coll_0 = stroops(SPLIT_COLLATERAL_TOKENS);
    let debt_0 = stroops(SPLIT_DEBT_TOKENS);
    let slice = stroops(100);

    let mut coll = coll_0;
    let mut debt = debt_0;
    let mut chain_seized = 0i128;
    let mut chain_repaid = 0i128;
    let mut prev_hf = 0i128;
    let mut prev_bonus = i128::MAX;

    for step in 0..SLICES {
        let out = liquidate_slice(&env, &book, coll, debt, slice);
        assert!(
            out.hf_wad < WAD,
            "step {step}: the position must never recover, got hf={}",
            out.hf_wad
        );
        assert!(
            out.hf_wad >= prev_hf,
            "step {step}: the health factor ratcheted down, {prev_hf} -> {}",
            out.hf_wad
        );
        assert!(
            out.bonus_bps <= prev_bonus,
            "step {step}: the bonus ratcheted up, {prev_bonus} -> {}",
            out.bonus_bps
        );
        prev_hf = out.hf_wad;
        prev_bonus = out.bonus_bps;
        chain_seized += out.seized;
        chain_repaid += out.repaid;
        coll -= out.seized;
        debt -= out.repaid;
    }

    let single = liquidate_slice(&env, &book, coll_0, debt_0, chain_repaid);
    assert_eq!(single.repaid, chain_repaid);
    assert!(
        chain_seized <= single.seized + SLICES,
        "eight slices out-extracted a single close: chain={chain_seized} \
         single={} excess={}",
        single.seized,
        chain_seized - single.seized
    );
}
