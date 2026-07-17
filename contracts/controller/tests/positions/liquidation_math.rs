use super::*;
use crate::constants::{
    DEFAULT_HF_FOR_MAX_BONUS_WAD, DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
    DEFAULT_LIQUIDATION_TARGET_HF_WAD,
};
use common::constants::RAY;
use common::types::{
    DebtPositionRaw, MarketIndexRaw, MarketOracleConfig, OracleAssetRef, OraclePriceFluctuation,
    OracleReadMode, OracleSourceConfig, OracleSourceConfigOption, OracleStrategy, PositionMode,
    PriceFeedRaw, ReflectorBase, ReflectorSourceConfig,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{vec, Address};

/// Curve values that `add_spoke` stamps at creation.
fn default_spoke_config() -> SpokeConfig {
    SpokeConfig {
        is_deprecated: false,
        liquidation_target_hf_wad: DEFAULT_LIQUIDATION_TARGET_HF_WAD,
        hf_for_max_bonus_wad: DEFAULT_HF_FOR_MAX_BONUS_WAD,
        liquidation_bonus_factor_bps: DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
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

// Pins the literal so a drifted constant cannot hide behind tests that only
// reference the symbol.
#[test]
fn bad_debt_threshold_is_five_usd_wad() {
    assert_eq!(BAD_DEBT_USD_THRESHOLD, 5_000_000_000_000_000_000);
}

fn feed_raw() -> PriceFeedRaw {
    PriceFeedRaw {
        price_wad: WAD,
        asset_decimals: 7,
        timestamp: 0,
    }
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

// A consistent plan validates, including the `protocol_fee == amount`
// boundary the fee cap must admit.
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

fn empty_account(env: &Env) -> Account {
    Account {
        owner: Address::generate(env),
        spoke_id: 1,
        mode: PositionMode::Normal,
        supply_positions: Map::new(env),
        borrow_positions: Map::new(env),
    }
}

// Zero collateral must short-circuit to a zero proportion instead of
// dividing by the empty collateral total.
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

// Positive collateral divides through: $50 weighted of $100 total is 0.5.
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

/// One debt position of 500 tokens (7 decimals) at $1 under unit indexes,
/// priced through a real single-source Reflector config.
fn repayment_fixture(env: &Env) -> (Address, HubAssetKey, Account, MarketOracleConfig) {
    use mock_oracle::{
        MockReflectorOracle, MockReflectorOracleClient, ReflectorAsset as MockAsset,
    };

    let contract = env.register(crate::Controller, (Address::generate(env),));
    let oracle_id = env.register(MockReflectorOracle, ());
    let asset = Address::generate(env);
    MockReflectorOracleClient::new(env, &oracle_id)
        .set_price(&MockAsset::Stellar(asset.clone()), &WAD);

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

    let config = single_usd_oracle_config(oracle_id, asset);

    (contract, hub_asset, account, config)
}

/// Single-source spot Reflector config quoting `asset` in USD.
fn single_usd_oracle_config(oracle_id: Address, asset: Address) -> MarketOracleConfig {
    MarketOracleConfig {
        asset_decimals: 7,
        max_price_stale_seconds: 900,
        tolerance: OraclePriceFluctuation {
            upper_ratio_bps: 10_500,
            lower_ratio_bps: 9_500,
        },
        strategy: OracleStrategy::Single,
        primary: OracleSourceConfig::Reflector(ReflectorSourceConfig {
            contract: oracle_id,
            asset: OracleAssetRef::Stellar(asset),
            read_mode: OracleReadMode::Spot,
            decimals: 14,
            resolution_seconds: 300,
            base: ReflectorBase::Usd,
        }),
        anchor: OracleSourceConfigOption::None,
        min_sanity_price_wad: 0,
        max_sanity_price_wad: i128::MAX,
    }
}

// A payment exactly equal to the closable debt produces no refund entry.
#[test]
fn repayment_at_exact_debt_produces_no_refund() {
    let env = Env::default();
    let (contract, hub_asset, account, config) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        crate::storage::set_asset_oracle(&env, &hub_asset.asset, &config);
        let mut cache = Cache::new_view(&env);
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

// Over-repayment caps the leg at the actual debt and refunds exactly the
// excess.
#[test]
fn repayment_above_debt_refunds_exact_excess() {
    let env = Env::default();
    let (contract, hub_asset, account, config) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        crate::storage::set_asset_oracle(&env, &hub_asset.asset, &config);
        let mut cache = Cache::new_view(&env);
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
fn bad_debt_socialization_requires_debt_exceeding_collateral_under_threshold() {
    let collateral = Wad::from(BAD_DEBT_USD_THRESHOLD);
    assert!(is_socializable_bad_debt(
        collateral + Wad::from(1),
        collateral
    ));
    assert!(!is_socializable_bad_debt(collateral, collateral));
    assert!(!is_socializable_bad_debt(
        Wad::from(BAD_DEBT_USD_THRESHOLD + 2 * WAD),
        Wad::from(BAD_DEBT_USD_THRESHOLD + WAD)
    ));
}

/// Snapshot for curve tests: 100 USD debt and collateral, a 0.5 collateral-mix
/// proportion, and caller-supplied health factor / weighted collateral.
fn curve_snap(hf_raw: i128, weighted_raw: i128) -> LiquidationSnapshot {
    LiquidationSnapshot {
        total_debt: Wad::from(100 * WAD),
        total_collateral: Wad::from(100 * WAD),
        weighted_coll: Wad::from(weighted_raw),
        proportion_seized: Wad::from(WAD / 2),
        hf: Wad::from(hf_raw),
    }
}

// The default curve ramps linearly over the (target, hf_for_max_bonus) span:
// scale = min((target - hf) / (target - hf_for_max_bonus), 1).
#[test]
fn default_curve_bonus_matches_reference_scale() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(crate::constants::DEFAULT_LIQUIDATION_TARGET_HF_WAD);
    let knee = Wad::from(crate::constants::DEFAULT_HF_FOR_MAX_BONUS_WAD);

    for hf_raw in [
        100_000_000_000_000_000i128,   // 0.10 -> scale capped at 1
        450_000_000_000_000_000i128,   // 0.45 -> scale capped at 1
        800_000_000_000_000_000i128,   // 0.80 == hf_for_max_bonus -> scale exactly 1
        900_000_000_000_000_000i128,   // 0.90
        1_050_000_000_000_000_000i128, // 1.05 (below target)
    ] {
        let hf = Wad::from(hf_raw);
        let got = calculate_linear_bonus_with_target(&env, hf, base, max, &curve, target);

        // Independent reference: scale = min((target - hf) / (target - knee), 1).
        let scale = (target - hf).div(&env, target - knee).min(Wad::ONE);
        let increment = Wad::from((max - base).raw()).mul(&env, scale).raw();
        let want = Bps::from(base.raw() + increment);

        assert_eq!(got.raw(), want.raw(), "hf={hf_raw}");
    }
}

// hf >= target yields the base bonus unchanged.
#[test]
fn bonus_at_or_above_target_is_base() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let base = Bps::from(400i128);
    let max = Bps::from(1_000i128);
    let target = Wad::from(1_020_000_000_000_000_000i128);

    let got = calculate_linear_bonus_with_target(&env, target, base, max, &curve, target);
    assert_eq!(got.raw(), base.raw());
}

// A non-default bonus factor scales the increment; 2.0x doubles it exactly.
#[test]
fn bonus_factor_scales_increment() {
    let env = Env::default();
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(1_020_000_000_000_000_000i128);
    let hf = Wad::from(900_000_000_000_000_000i128);

    let default_curve = LiquidationCurve::from_config(&default_spoke_config());
    let default_bonus =
        calculate_linear_bonus_with_target(&env, hf, base, max, &default_curve, target);

    let double_factor = SpokeConfig {
        liquidation_bonus_factor_bps: 20_000,
        ..default_spoke_config()
    };
    let curve_2x = LiquidationCurve::from_config(&double_factor);
    let scaled_bonus = calculate_linear_bonus_with_target(&env, hf, base, max, &curve_2x, target);

    let inc_default = default_bonus.raw() - base.raw();
    let inc_scaled = scaled_bonus.raw() - base.raw();
    assert!(inc_default > 0);
    assert_eq!(inc_scaled, inc_default * 2);
}

// A bonus factor above BPS (100%) can push the realized bonus past `max` —
// this is why `common::validation::validate_liquidation_curve` (enforced by
// the `set_spoke_liquidation_curve` governance op) caps the configurable
// factor at BPS. At the cap itself, the realized bonus never exceeds `max`,
// for any severity between the curve's target and its max-bonus floor.
#[test]
fn bonus_factor_above_bps_can_exceed_max_uncapped() {
    let env = Env::default();
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(1_020_000_000_000_000_000i128);
    let hf = Wad::from(510_000_000_000_000_000i128); // == hf_for_max_bonus -> scale 1

    let over_cap = SpokeConfig {
        liquidation_bonus_factor_bps: 20_000, // 200%, above the enforced BPS ceiling
        ..default_spoke_config()
    };
    let curve = LiquidationCurve::from_config(&over_cap);
    let got = calculate_linear_bonus_with_target(&env, hf, base, max, &curve, target);

    assert!(
        got.raw() > max.raw(),
        "expected an over-cap factor to breach max, got {} vs max {}",
        got.raw(),
        max.raw()
    );
}

// At the enforced ceiling (bonus_factor_bps == BPS, i.e. the default and the
// max the governance op now allows), the realized bonus never exceeds `max`
// across the full HF range from target down to hf_for_max_bonus.
#[test]
fn bonus_factor_at_bps_ceiling_never_exceeds_max() {
    let env = Env::default();
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(1_020_000_000_000_000_000i128);
    let curve = LiquidationCurve::from_config(&default_spoke_config()); // factor == BPS

    for hf_raw in [
        1_019_000_000_000_000_000i128, // just below target
        900_000_000_000_000_000i128,
        700_000_000_000_000_000i128,
        510_000_000_000_000_000i128, // == hf_for_max_bonus -> scale saturates at 1
        100_000_000_000_000_000i128, // below hf_for_max_bonus -> scale still 1
    ] {
        let hf = Wad::from(hf_raw);
        let got = calculate_linear_bonus_with_target(&env, hf, base, max, &curve, target);
        assert!(
            got.raw() <= max.raw(),
            "hf={hf_raw} produced bonus {} exceeding max {}",
            got.raw(),
            max.raw()
        );
    }
}

// A custom target HF changes the estimated close amount vs the 1.02 default.
#[test]
fn custom_target_changes_estimate() {
    let env = Env::default();
    let snap = curve_snap(950_000_000_000_000_000, 95 * WAD); // hf = 0.95, weighted = 95
    let bounds = BonusBounds {
        base: Bps::from(200i128),
        max: Bps::from(1_000i128),
    };

    let default_curve = LiquidationCurve::from_config(&default_spoke_config());
    let (ideal_default, _) = estimate_liquidation_amount(&env, &snap, bounds, &default_curve);

    let custom = SpokeConfig {
        liquidation_target_hf_wad: 1_300_000_000_000_000_000, // 1.30 target
        hf_for_max_bonus_wad: 650_000_000_000_000_000,        // target / 2
        ..default_spoke_config()
    };
    let custom_curve = LiquidationCurve::from_config(&custom);
    let (ideal_custom, _) = estimate_liquidation_amount(&env, &snap, bounds, &custom_curve);

    assert!(ideal_default.raw() > 0);
    assert_ne!(ideal_default.raw(), ideal_custom.raw());
}

#[test]
fn post_liquidation_hf_saturates_when_debt_fully_repaid() {
    let env = Env::default();
    let snap = curve_snap(900_000_000_000_000_000, 90 * WAD);
    let hf = calculate_post_liquidation_hf(&env, &snap, snap.total_debt, Bps::from(0i128));
    assert_eq!(hf.raw(), i128::MAX);
}

#[test]
fn post_liquidation_hf_does_not_decrease_for_partial_zero_bonus_repay() {
    let env = Env::default();
    let snap = curve_snap(900_000_000_000_000_000, 90 * WAD);
    let hf = calculate_post_liquidation_hf(&env, &snap, Wad::from(10 * WAD), Bps::from(0i128));
    assert!(hf >= snap.hf);
}

/// One supply position of 1000 tokens (7 decimals) at $1 under unit indexes,
/// with the given position-stamped liquidation fee.
fn seize_fixture(env: &Env, fees_bps: u32) -> (Address, HubAssetKey, Account, MarketOracleConfig) {
    use mock_oracle::{
        MockReflectorOracle, MockReflectorOracleClient, ReflectorAsset as MockAsset,
    };

    let contract = env.register(crate::Controller, (Address::generate(env),));
    let oracle_id = env.register(MockReflectorOracle, ());
    let asset = Address::generate(env);
    MockReflectorOracleClient::new(env, &oracle_id)
        .set_price(&MockAsset::Stellar(asset.clone()), &WAD);

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

    let config = single_usd_oracle_config(oracle_id, asset);
    (contract, hub_asset, account, config)
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
    let (contract, hub_asset, account, config) = seize_fixture(env, fees_bps);
    env.as_contract(&contract, || {
        crate::storage::set_asset_oracle(env, &hub_asset.asset, &config);
        let mut cache = Cache::new_view(env);
        cache.put_market_index(&hub_asset, &index_raw());
        let plan = plan_for_seizure(env, repay_usd_raw, bonus_bps);
        calculate_seized_collateral(env, &account, Wad::from(1_000 * WAD), &plan, &mut cache)
    })
}

// A partial seizure floors the token conversion (half-up is reserved for the
// exact full-position close) and a zero-fee position pays zero protocol fee.
#[test]
fn partial_seizure_floors_amount_and_zero_fee_stays_zero() {
    let env = Env::default();
    // 100 tokens plus half a stroop of USD at $1; floor -> 1_000_000_000.
    let seized = run_seizure(&env, 0, 100 * WAD + 50_000_000_000, 0);
    assert_eq!(seized.len(), 1);
    let entry = seized.get_unchecked(0);
    assert_eq!(entry.amount, 1_000_000_000);
    assert_eq!(entry.protocol_fee, 0);
}

// A positive fee that floors to zero stroops is bumped to the one-unit
// minimum.
#[test]
fn dust_protocol_fee_rounds_up_to_one_unit() {
    let env = Env::default();
    // 1 stroop repaid at 50% bonus: seizure 1.5 stroops, bonus leg 0.5
    // stroops, 100% fee on it floors to 0 -> minimum fee of 1 unit.
    let seized = run_seizure(&env, 10_000, WAD / 10_000_000, 5_000);
    assert_eq!(seized.len(), 1);
    let entry = seized.get_unchecked(0);
    assert_eq!(entry.amount, 1);
    assert_eq!(entry.protocol_fee, 1);
}

// A fee that converts to whole units is passed through exactly, not clamped
// to the one-unit minimum.
#[test]
fn whole_unit_protocol_fee_is_exact() {
    let env = Env::default();
    // 100 tokens repaid at 50% bonus: seizure 150, bonus leg 50, 10% fee = 5
    // tokens exactly.
    let seized = run_seizure(&env, 1_000, 100 * WAD, 5_000);
    assert_eq!(seized.len(), 1);
    let entry = seized.get_unchecked(0);
    assert_eq!(entry.amount, 1_500_000_000);
    assert_eq!(entry.protocol_fee, 50_000_000);
}

fn stroops(tokens: i128) -> i128 {
    tokens * 10_000_000
}

// Zero excess is a no-op: no refund entries, no leg mutation.
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

// Excess equal to the tail leg's USD removes the whole leg instead of
// leaving a zero-amount split residue.
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

// Excess larger than everything refunds every leg and returns cleanly with
// the shortfall unconsumed.
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

// Excess spanning legs: the tail leg refunds fully and reduces the running
// excess; the boundary leg splits pro-rata.
#[test]
fn process_excess_payment_spans_legs_with_pro_rata_split() {
    let env = Env::default();
    let mut repaid = Vec::new(&env);
    repaid.push_back(repay_entry(&env, stroops(100), 100 * WAD));
    repaid.push_back(repay_entry(&env, stroops(40), 40 * WAD));
    let mut refunds = Vec::new(&env);

    process_excess_payment(&env, &mut repaid, &mut refunds, Wad::from(60 * WAD));

    // Tail leg ($40) fully refunded; remaining $20 splits the $100 leg 20%.
    assert_eq!(refunds.len(), 2);
    assert_eq!(refunds.get_unchecked(0).amount, stroops(40));
    assert_eq!(refunds.get_unchecked(1).amount, stroops(20));
    assert_eq!(repaid.len(), 1);
    let kept = repaid.get_unchecked(0);
    assert_eq!(kept.amount, stroops(80));
    assert_eq!(kept.usd_wad, 80 * WAD);
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

// The post-liquidation HF must weight the seized side by 1 + bonus.
#[test]
fn post_liquidation_hf_applies_bonus_on_seized_weight() {
    let env = Env::default();
    // W=100, D=100, p=1, repay 10 at 10% bonus: seized weighted = 11,
    // HF = 89/90.
    let s = snap(
        100 * WAD,
        120 * WAD,
        100 * WAD,
        WAD,
        900_000_000_000_000_000,
    );
    let hf = calculate_post_liquidation_hf(&env, &s, Wad::from(10 * WAD), Bps::from(1_000i128));
    let expected = Wad::from(89 * WAD).div(&env, Wad::from(90 * WAD));
    assert_eq!(hf.raw(), expected.raw());
}

// The effective threshold ceils and the derived max floors: at exactly 50%
// the bound is exactly 100% (10000 bps); any drifted rounding constant moves
// it off this value.
#[test]
fn max_bonus_for_threshold_is_exact_at_half() {
    let env = Env::default();
    assert_eq!(
        max_bonus_for_threshold(&env, Wad::from(WAD / 2)).raw(),
        10_000
    );
}

// When every partial repayment would reduce HF, even a zero-bonus estimate
// escalates to a full close, so a debt-covering payment leaves no refund.
#[test]
fn normalize_repayment_plan_requires_full_close_when_partials_ratchet() {
    let env = Env::default();
    let (contract, hub_asset, account, config) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        crate::storage::set_asset_oracle(&env, &hub_asset.asset, &config);
        let mut cache = Cache::new_view(&env);
        cache.put_market_index(&hub_asset, &index_raw());

        // p = 1, HF = 0.4: even a zero bonus removes weighted collateral
        // faster than debt, so no partial is HF-safe and the guard escalates
        // to a full close -- the whole $500 payment is consumed, no refund.
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

// A solvent-toxic account (collateral covers debt, but 0 <= hf/p - 1 < base)
// rejects partial payments outright: only a full close is accepted.
#[test]
#[should_panic(expected = "Error(Contract, #135)")]
fn normalize_rejects_partial_on_solvent_toxic_account() {
    let env = Env::default();
    let (contract, hub_asset, account, config) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        crate::storage::set_asset_oracle(&env, &hub_asset.asset, &config);
        let mut cache = Cache::new_view(&env);
        cache.put_market_index(&hub_asset, &index_raw());

        // p = 0.9, HF = 0.93: cap = 333 bps sits in [0, base 500).
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

        // $100 of the $500 debt: below the full-close ideal -> rejected.
        let payments = vec![&env, (hub_asset.clone(), 100_0000000i128)];
        normalize_repayment_plan(&env, &account, &payments, &s, bounds, &curve, &mut cache);
    });
}

// The same solvent-toxic account accepts a payment covering the full debt.
#[test]
fn normalize_accepts_full_close_on_solvent_toxic_account() {
    let env = Env::default();
    let (contract, hub_asset, account, config) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        crate::storage::set_asset_oracle(&env, &hub_asset.asset, &config);
        let mut cache = Cache::new_view(&env);
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

// Insolvent accounts (negative HF-neutral cap: collateral below debt) keep the
// partial path: forcing a full close would guarantee the liquidator a loss
// and freeze liquidation.
#[test]
fn normalize_allows_partial_on_insolvent_account() {
    let env = Env::default();
    let (contract, hub_asset, account, config) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        crate::storage::set_asset_oracle(&env, &hub_asset.asset, &config);
        let mut cache = Cache::new_view(&env);
        cache.put_market_index(&hub_asset, &index_raw());

        // p = 1, HF = 0.4: cap is negative, the account is insolvent.
        let s = snap(500 * WAD, 100 * WAD, 40 * WAD, WAD, 400_000_000_000_000_000);
        let bounds = BonusBounds {
            base: Bps::from(0i128),
            max: Bps::from(0i128),
        };
        let curve = LiquidationCurve::from_config(&default_spoke_config());

        let payments = vec![&env, (hub_asset.clone(), 100_0000000i128)];
        let plan =
            normalize_repayment_plan(&env, &account, &payments, &s, bounds, &curve, &mut cache);
        assert_eq!(plan.repay_usd.raw(), 100 * WAD, "partial accepted");
        assert_eq!(plan.bonus.raw(), 0);
    });
}

// The HF-preserving cap returns `None` on each of the two independent
// no-cap conditions (`proportion <= 0` OR `hf >= WAD`) and a finite floored
// cap in the toxic band. The two `None` cases must hold independently: an
// account with seizable collateral but hf >= 1 needs no cap, and a
// zero-proportion account must short-circuit before the `hf/p` division.
#[test]
fn max_hf_preserving_bonus_none_on_each_no_cap_condition() {
    // proportion > 0 but hf >= WAD (healthy): no cap.
    let healthy = snap(50 * WAD, 200 * WAD, 100 * WAD, WAD / 2, 2 * WAD);
    assert_eq!(max_hf_preserving_bonus_bps(&healthy), None);

    // hf < WAD but zero seizable proportion: no cap (also guards the
    // `hf * BPS / proportion` division against a zero divisor).
    let no_seizable = snap(90 * WAD, 100 * WAD, 0, 0, WAD / 2);
    assert_eq!(max_hf_preserving_bonus_bps(&no_seizable), None);

    // Toxic band (proportion 0.45, hf 0.5): finite cap hf/p - 1 = 1111 bps.
    let toxic = snap(90 * WAD, 100 * WAD, 45 * WAD, 45 * WAD / 100, WAD / 2);
    assert_eq!(max_hf_preserving_bonus_bps(&toxic), Some(1_111));
}

// A deeply unhealthy low-threshold position (collateral $100, debt $90,
// threshold 0.45 -> weighted $45, HF 0.5): the curve asks for the max bonus
// (12222 bps) but that seizure rate would ratchet HF on partials, so the
// guard caps the bonus at the largest HF-neutral value, hf/p - 1 = 1111 bps.
// At that bonus the near-full ideal leaves sub-floor dust, so the estimate
// closes the whole debt -- and the $90 * 1.1111 seizure stays inside the
// $100 collateral, leaving no socializable residue.
#[test]
fn estimate_toxic_band_caps_bonus_to_hf_neutral() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    let s = snap(90 * WAD, 100 * WAD, 45 * WAD, 45 * WAD / 100, WAD / 2);
    let max = max_bonus_for_threshold(&env, s.proportion_seized);
    let bounds = BonusBounds {
        base: Bps::from(500i128),
        max,
    };

    let (d, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert_eq!(bonus.raw(), 1_111, "bonus capped at hf/p - 1, not the max");
    assert_eq!(d.raw(), s.total_debt.raw(), "dust guard closes the debt");
    let seizure = d.mul(&env, Wad::ONE + bonus.to_wad(&env));
    assert!(
        seizure <= s.total_collateral,
        "capped seizure fits in collateral"
    );
}

// When even the base bonus would shrink HF (hf/p - 1 below base), partials
// cannot help the account, so the estimate requires a full close at base.
#[test]
fn estimate_full_close_when_base_bonus_ratchets() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    // p = 0.9, HF = 0.9: hf/p - 1 = 0 < base 500.
    let s = snap(
        100 * WAD,
        100 * WAD,
        90 * WAD,
        90 * WAD / 100,
        90 * WAD / 100,
    );
    let bounds = BonusBounds {
        base: Bps::from(500i128),
        max: max_bonus_for_threshold(&env, s.proportion_seized),
    };

    let (d, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert_eq!(bonus.raw(), 500, "full close pays the base bonus");
    assert_eq!(d.raw(), s.total_debt.raw(), "unsafe partials force full close");
}

// Outside the toxic band the guard is inert: the HF-scaled bonus applies
// unchanged.
#[test]
fn estimate_safe_region_keeps_scaled_bonus() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    // p = 0.5, HF = 0.95: hf/p - 1 = 9000 bps, scaled bonus is
    // 500 + 9500 * (1.10 - 0.95)/(1.10 - 0.80) = 5250 bps -- under the cap.
    let s = snap(100 * WAD, 200 * WAD, 95 * WAD, WAD / 2, 95 * WAD / 100);
    let bounds = BonusBounds {
        base: Bps::from(500i128),
        max: max_bonus_for_threshold(&env, s.proportion_seized),
    };

    let (_d, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert_eq!(bonus.raw(), 5_250, "scaled bonus kept in the safe region");
}

// The guard invariant, swept: for any estimated (partial) liquidation, a
// repayment at or below the ideal never leaves the account less healthy.
#[test]
fn partial_liquidations_never_reduce_hf() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let collateral = 100 * WAD;

    for p_pct in [30i128, 45, 60, 80, 92] {
        for hf_pct in (10..100).step_by(8) {
            let weighted = collateral * p_pct / 100;
            // hf = weighted / debt  =>  debt = weighted / hf.
            let debt = weighted * 100 / hf_pct as i128;
            let s = snap(
                debt,
                collateral,
                weighted,
                p_pct * WAD / 100,
                hf_pct as i128 * WAD / 100,
            );
            let bounds = BonusBounds {
                base: Bps::from(500i128),
                max: max_bonus_for_threshold(&env, s.proportion_seized),
            };

            let (ideal, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
            // A full-close estimate carries no partial to check.
            if ideal.raw() >= s.total_debt.raw() {
                continue;
            }
            for repay in [Wad::from(ideal.raw() / 2), ideal] {
                let post = calculate_post_liquidation_hf(&env, &s, repay, bonus);
                assert!(
                    // Half-up rounding in the seizure path may cost 1 ulp.
                    post.raw() + 10 >= s.hf.raw(),
                    "partial at p={p_pct}% hf={hf_pct}% repay={} reduced HF: {} -> {}",
                    repay.raw(),
                    s.hf.raw(),
                    post.raw()
                );
            }
        }
    }
}

// The dust guard escalates a sub-floor debt remainder to a full close. A
// high-threshold position (D=$100, C=$104, threshold 0.95 -> weighted $98.8,
// HF 0.988) repays at ~the base bonus but is collateral-capped at
// C/(1+bonus) ≈ $99, leaving < $5 of dust, so the estimate closes it fully.
#[test]
fn estimate_escalates_sub_floor_debt_dust_to_full_close() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    let s = snap(
        100 * WAD,
        104 * WAD,
        988 * WAD / 10,
        95 * WAD / 100,
        988 * WAD / 1000,
    );
    let bounds = BonusBounds {
        base: Bps::from(500i128),
        max: max_bonus_for_threshold(&env, s.proportion_seized),
    };

    let (d, _bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert_eq!(
        d.raw(),
        s.total_debt.raw(),
        "sub-floor dust escalated to a full close"
    );
}

// An above-floor remainder is left untouched: a moderately unhealthy
// low-threshold position (D=$50, C=$100, threshold 0.45 -> weighted $45,
// HF 0.9) repays a partial toward the target and keeps a >$5 debt remainder.
#[test]
fn estimate_leaves_above_floor_debt_remainder_unescalated() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    let s = snap(
        50 * WAD,
        100 * WAD,
        45 * WAD,
        45 * WAD / 100,
        90 * WAD / 100,
    );
    let bounds = BonusBounds {
        base: Bps::from(500i128),
        max: max_bonus_for_threshold(&env, s.proportion_seized),
    };

    let (d, _bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert!(d < s.total_debt, "partial repayment");
    assert!(
        s.total_debt - d >= Wad::from(BAD_DEBT_USD_THRESHOLD),
        "remainder stays above the socialization floor"
    );
}

// A mildly-unhealthy position restores HF to target with a partial repayment
// bounded by the interpolation, not the collateral cap. Pins the exact ideal
// against an independent reference so a broken denom guard or numerator sign
// (which would return `None` -> the collateral fallback, or invert the sign)
// is caught.
#[test]
fn estimate_target_reachable_returns_interpolated_partial() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    // Zero bonus keeps `1 + bonus == 1`, so `d_max == total_collateral` and the
    // interpolation is the binding constraint (collateral $200 >> repayment).
    let s = snap(100 * WAD, 200 * WAD, 95 * WAD, 475 * WAD / 1000, 95 * WAD / 100);
    let bounds = BonusBounds {
        base: Bps::from(0i128),
        max: Bps::from(0i128),
    };

    let (d, _bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);

    // Independent reference: with zero bonus the restore-to-target root is
    // `(target*D - W) / (target - p)`, clamped by collateral and total debt.
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);
    let target_debt = target.mul(&env, s.total_debt);
    let numerator = target_debt - s.weighted_coll;
    let denominator = target - s.proportion_seized;
    let expected = numerator
        .div(&env, denominator)
        .min(s.total_collateral)
        .min(s.total_debt);

    assert!(d < s.total_debt, "target-reachable partial, not a full close");
    assert_eq!(d.raw(), expected.raw());
}

// When collateral already covers the target debt (`target_debt <= weighted`),
// the estimate returns the collateral-capped maximum, not an interpolation
// over a non-positive numerator. At the exact `target_debt == weighted`
// boundary the `<=` admits the collateral-cover branch.
#[test]
fn estimate_collateral_covers_target_returns_collateral_cap() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    // target_hf * D = 1.10 * 100 = 110 == weighted_coll, the branch boundary.
    let s = snap(100 * WAD, 120 * WAD, 110 * WAD, 85 * WAD / 100, 102 * WAD / 100);
    let bounds = BonusBounds {
        base: Bps::from(0i128),
        max: Bps::from(0i128),
    };

    let (d, _bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    // d_max = collateral / 1 = 120, capped at total debt 100.
    assert_eq!(d.raw(), s.total_debt.raw());
}

// The fallback (target unreachable because the bonus makes the seizure too
// large) closes `collateral / (1 + bonus)`. A flat 50% bonus with a high
// collateral-mix proportion forces `proportion*(1+bonus) >= target`, so the
// closed-form returns `None` and the fallback binds.
#[test]
fn estimate_fallback_divides_collateral_by_one_plus_bonus() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    // base == max pins the bonus at exactly 50%. Under the HF-preservation
    // guard the unreachable-target fallback only fires when the guard is
    // inert (hf >= 1: below 1 the capped bonus keeps the target denominator
    // positive), so pin hf above the target: p*(1+b) = 0.74*1.5 = 1.11 > 1.10
    // makes the target unreachable and the fallback divides the collateral.
    let s = snap(150 * WAD, 150 * WAD, 50 * WAD, 74 * WAD / 100, 12 * WAD / 10);
    let bounds = BonusBounds {
        base: Bps::from(5_000i128),
        max: Bps::from(5_000i128),
    };

    let (d, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert_eq!(bonus.raw(), 5_000);
    // 150 / (1 + 0.5) = 100; a `-` in place of `+` would give 150 / 0.5 = 300,
    // clamped to the $150 debt.
    assert_eq!(d.raw(), 100 * WAD);
}

// The dust guard escalates a sub-floor remainder but leaves an *exactly* $5
// remainder alone (`remaining < $5`, strict). The collateral cap is set so the
// natural ideal leaves precisely `BAD_DEBT_USD_THRESHOLD` of debt; a `<=`
// would wrongly escalate this to a full close.
#[test]
fn estimate_leaves_exactly_five_dollar_remainder_unescalated() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());

    // Zero bonus, safe region (p = 0.5 <= HF = 0.53): the target-HF root is
    // exactly (1.10*100 - 53) / (1.10 - 0.5) = $95, so remaining is exactly
    // $100 - $95 = $5.
    let s = snap(100 * WAD, 106 * WAD, 53 * WAD, WAD / 2, 53 * WAD / 100);
    let bounds = BonusBounds {
        base: Bps::from(0i128),
        max: Bps::from(0i128),
    };

    let (d, _bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
    assert_eq!(
        d.raw(),
        s.total_debt.raw() - BAD_DEBT_USD_THRESHOLD,
        "an exactly-$5 remainder is left as a partial, not escalated"
    );
}

// `get_account_bonus_params` sums each supply leg's USD value into
// `total_collateral` and weights the per-leg bonus by its share. A single
// $1000 leg at 500 bps yields base == 500 (weight 1.0); the collateral
// accumulator must add (not subtract) and the zero-collateral early return
// must fire on equality only.
#[test]
fn account_bonus_params_accumulates_collateral_and_weights_bonus() {
    let env = Env::default();
    let (contract, hub_asset, account, config) = seize_fixture(&env, 0);
    env.as_contract(&contract, || {
        crate::storage::set_asset_oracle(&env, &hub_asset.asset, &config);
        let mut cache = Cache::new_view(&env);
        cache.put_market_index(&hub_asset, &index_raw());

        // 0.5 collateral-mix proportion -> max bonus 10000 bps, so base is not
        // clamped below the leg's 500 bps.
        let bounds = get_account_bonus_params(
            &env,
            &mut cache,
            account.spoke_id,
            &account.supply_positions,
            Wad::from(WAD / 2),
        );

        assert_eq!(bounds.max.raw(), 10_000);
        assert_eq!(bounds.base.raw(), 500);
    });
}

// ---------------------------------------------------------------------------
// Bonus + seizure invariants
// ---------------------------------------------------------------------------

// The bonus is monotone in health factor: a lower HF never yields a smaller
// bonus.
#[test]
fn bonus_monotone_decreasing_in_hf() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let base = Bps::from(500i128);
    let max = Bps::from(2_500i128);
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);

    let mut prev = i128::MAX;
    for pct in (10..=102).step_by(2) {
        let hf = Wad::from(WAD * pct / 100);
        let b = calculate_linear_bonus_with_target(&env, hf, base, max, &curve, target).raw();
        assert!(
            b <= prev,
            "bonus must not increase as HF rises: hf={pct}% bonus={b} prev={prev}"
        );
        prev = b;
    }
}

// The bonus stays within `[base, max]` across the whole HF range.
#[test]
fn bonus_within_base_and_max_bounds() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let base = Bps::from(500i128);
    let max = Bps::from(2_500i128);
    let target = Wad::from(DEFAULT_LIQUIDATION_TARGET_HF_WAD);

    for pct in (5..=110).step_by(3) {
        let hf = Wad::from(WAD * pct / 100);
        let b = calculate_linear_bonus_with_target(&env, hf, base, max, &curve, target).raw();
        assert!(
            b >= base.raw() && b <= max.raw(),
            "bonus {b} out of [{}, {}] at hf={pct}%",
            base.raw(),
            max.raw()
        );
    }
}

// The estimated seizure never exceeds the account's collateral, at any
// liquidatable HF. This is the per-threshold ceiling that keeps a liquidation
// from over-seizing. Single 0.80-threshold collateral, swept from shallow to
// deep.
#[test]
fn seizure_never_exceeds_collateral() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let collateral = 100 * WAD;
    let weighted = 80 * WAD; // threshold 0.80
    let proportion = 80 * WAD / 100;
    let bounds = BonusBounds {
        base: Bps::from(500i128),
        max: max_bonus_for_threshold(&env, Wad::from(proportion)),
    };

    for hf_pct in (10..100).step_by(5) {
        // hf = weighted / debt  =>  debt = weighted / hf
        let debt = weighted * 100 / hf_pct as i128;
        let s = snap(debt, collateral, weighted, proportion, WAD * hf_pct as i128 / 100);
        let (ideal, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
        // The dust guard may escalate to a full close whose notional seizure
        // exceeds collateral; the real per-asset seizure is capped downstream in
        // `calculate_seized_collateral`. Assert the ceiling only on the
        // non-escalated (target-HF or collateral-capped) path.
        if ideal.raw() == s.total_debt.raw() {
            continue;
        }
        let seizure = ideal.mul(&env, Wad::ONE + bonus.to_wad(&env));
        assert!(
            seizure.raw() <= collateral + WAD / 1_000,
            "seizure {} exceeds collateral {} at hf={hf_pct}%",
            seizure.raw(),
            collateral
        );
    }
}
