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
use soroban_sdk::Address;

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

    let config = MarketOracleConfig {
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
    };

    (contract, hub_asset, account, config)
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

        let payments = soroban_sdk::vec![&env, (hub_asset.clone(), 500_0000000i128)];
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

        let payments = soroban_sdk::vec![&env, (hub_asset.clone(), 500_0000005i128)];
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

// With `hf_for_max_bonus = target / 2` the curve equals `2 * gap / target`.
#[test]
fn default_curve_bonus_matches_two_gap_scale() {
    let env = Env::default();
    let curve = LiquidationCurve::from_config(&default_spoke_config());
    let base = Bps::from(500i128);
    let max = Bps::from(1_500i128);
    let target = Wad::from(1_020_000_000_000_000_000i128);

    for hf_raw in [
        100_000_000_000_000_000i128,   // 0.10 -> scale capped at 1
        450_000_000_000_000_000i128,   // 0.45
        510_000_000_000_000_000i128,   // 0.51 == target/2 -> scale exactly 1
        900_000_000_000_000_000i128,   // 0.90
        1_010_000_000_000_000_000i128, // 1.01 (just below target)
    ] {
        let hf = Wad::from(hf_raw);
        let got = calculate_linear_bonus_with_target(&env, hf, base, max, &curve, target);

        // Independent reference: scale = min(2 * (target - hf) / target, 1).
        let gap_wad = (target - hf).div(&env, target);
        let scale = gap_wad.mul(&env, Wad::from(2 * WAD)).min(Wad::ONE);
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
