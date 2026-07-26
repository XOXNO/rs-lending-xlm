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

/// Curve values that `add_spoke` stamps at creation.
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

/// Aggregator-resolved price map for a single asset (the controller reads
/// prices from this map after the one `prices()` call per flow).
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

/// One debt position of 500 tokens (7 decimals) at $1 under unit indexes.
/// Callers seed `Cache` prices via `single_price` (no live oracle).
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

/// One supply position of 1000 tokens (7 decimals) at $1 under unit indexes,
/// with the given position-stamped liquidation fee.
/// Callers seed `Cache` prices via `single_price` (no live oracle).
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

// A payment exactly equal to the closable debt produces no refund entry.
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

// Over-repayment caps the leg at the actual debt and refunds exactly the
// excess.
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

// When every partial repayment would reduce HF, even a zero-bonus estimate
// escalates to a full close, so a debt-covering payment leaves no refund.
#[test]
fn normalize_repayment_plan_requires_full_close_when_partials_ratchet() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
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
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
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

// Boundary: cap == base exactly (hf/p - 1 == base). A partial at the base
// bonus is exactly HF-neutral there, so it must be ACCEPTED -- the full-close
// gate uses a strict `cap < base`. p = 0.8, hf = 0.84 gives
// cap = 0.84/0.8 - 1 = 500 bps == base, and C/D = 1.05 keeps it solvent.
#[test]
fn normalize_accepts_partial_when_cap_equals_base() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        // debt $500, collateral $525, weighted $420: p = 0.8, hf = 0.84.
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

        // A $100 partial (below the full-close ideal) is accepted, not rejected.
        let payments = vec![&env, (hub_asset.clone(), 100_0000000i128)];
        let plan =
            normalize_repayment_plan(&env, &account, &payments, &s, bounds, &curve, &mut cache);
        assert_eq!(plan.repay_usd.raw(), 100 * WAD, "boundary partial accepted");
        assert_eq!(plan.bonus.raw(), 500);
    });
}

// Insolvent accounts (negative HF-neutral cap: collateral below debt) keep the
// partial path: forcing a full close would guarantee the liquidator a loss
// and freeze liquidation.
#[test]
fn normalize_allows_partial_on_insolvent_account() {
    let env = Env::default();
    let (contract, hub_asset, account) = repayment_fixture(&env);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
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

// `get_account_bonus_params` weights each supply leg's bonus by its share of
// the caller-supplied `total_collateral`. A single $1000 leg at 500 bps carries
// weight 1.0 and yields base == 500.
#[test]
fn account_bonus_params_weights_bonus_by_collateral_share() {
    let env = Env::default();
    let (contract, hub_asset, account) = seize_fixture(&env, 0);
    env.as_contract(&contract, || {
        let mut cache = Cache::new_view(&env);
        cache.set_prices(single_price(&env, &hub_asset.asset));
        cache.put_market_index(&hub_asset, &index_raw());

        // The fixture's sole leg is 1000 tokens at a $1.00 feed and a unit index.
        // 0.5 collateral-mix proportion -> max bonus 10000 bps, so base is not
        // clamped below the leg's 500 bps.
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

// A zero total collateral short-circuits to a zero base before any leg is
// priced, and the ceiling still reflects the seized proportion.
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

// Bonus + seizure invariants

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

    let mut ceilings_checked = 0;

    for hf_pct in (10..100).step_by(5) {
        // hf = weighted / debt  =>  debt = weighted / hf
        let debt = weighted * 100 / hf_pct as i128;
        let s = snap(
            debt,
            collateral,
            weighted,
            proportion,
            WAD * hf_pct as i128 / 100,
        );
        let (ideal, bonus) = estimate_liquidation_amount(&env, &s, bounds, &curve);
        // The dust guard may escalate to a full close whose notional seizure
        // exceeds collateral; the real per-asset seizure is capped downstream in
        // `calculate_seized_collateral`. Assert the ceiling only on the
        // non-escalated (target-HF or collateral-capped) path.
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

    // Full-close estimates are skipped above, so without this the sweep would
    // pass vacuously if every case escalated to a full close.
    assert!(
        ceilings_checked > 0,
        "swept the HF range without reaching a single non-escalated estimate"
    );
}
