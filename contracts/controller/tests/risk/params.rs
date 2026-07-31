use super::*;
use crate::constants::{THRESHOLD_UPDATE_MIN_HF_RAW, WAD};
use common::constants::RAY;
use common::math::fp::Ray;
use common::types::{DebtPositionRaw, MarketIndexRaw, PositionMode, PriceFeedRaw};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

const STAMPED_LT: i128 = 8_000;
const STAMPED_LTV: i128 = 7_500;
const STAMPED_BONUS: i128 = 500;
const STAMPED_FEES: i128 = 100;

fn debt_free_account(env: &Env) -> Account {
    Account {
        owner: Address::generate(env),
        spoke_id: 1,
        mode: PositionMode::Normal,
        supply_positions: Map::new(env),
        borrow_positions: Map::new(env),
    }
}

fn stamped_position() -> AccountPosition {
    AccountPosition {
        scaled_amount: Ray::from(0),
        liquidation_threshold: Bps::from(STAMPED_LT),
        liquidation_bonus: Bps::from(STAMPED_BONUS),
        loan_to_value: Bps::from(STAMPED_LTV),
        liquidation_fees: Bps::from(STAMPED_FEES),
    }
}

fn config(lt: i128, bonus: i128, fees: i128) -> AssetConfig {
    AssetConfig {
        loan_to_value: Bps::from(5_000i128),
        liquidation_threshold: Bps::from(lt),
        liquidation_bonus: Bps::from(bonus),
        liquidation_fees: Bps::from(fees),
        is_collateralizable: true,
        is_borrowable: true,
    }
}

#[test]
fn threshold_update_min_hf_is_one_point_zero_five_wad() {
    assert_eq!(THRESHOLD_UPDATE_MIN_HF_RAW, 1_050_000_000_000_000_000);
}

#[test]
fn favors_liquidator_on_threshold_cut() {
    let position = stamped_position();
    assert!(favors_liquidator(
        &position,
        &config(STAMPED_LT - 1, STAMPED_BONUS, STAMPED_FEES)
    ));
}

#[test]
fn favors_liquidator_on_bonus_raise() {
    let position = stamped_position();
    assert!(favors_liquidator(
        &position,
        &config(STAMPED_LT, STAMPED_BONUS + 1, STAMPED_FEES)
    ));
}

#[test]
fn favors_liquidator_on_fee_cut() {
    let position = stamped_position();
    assert!(favors_liquidator(
        &position,
        &config(STAMPED_LT, STAMPED_BONUS, STAMPED_FEES - 1)
    ));

    assert!(!favors_liquidator(
        &position,
        &config(STAMPED_LT, STAMPED_BONUS, STAMPED_FEES + 1)
    ));
}

#[test]
fn favors_liquidator_false_when_every_field_is_borrower_favorable() {
    let position = stamped_position();
    assert!(!favors_liquidator(
        &position,
        &config(STAMPED_LT + 1, STAMPED_BONUS - 1, STAMPED_FEES + 1)
    ));
}

#[test]
fn favors_liquidator_false_when_tuple_is_unchanged() {
    let position = stamped_position();
    assert!(!favors_liquidator(
        &position,
        &config(STAMPED_LT, STAMPED_BONUS, STAMPED_FEES)
    ));
}

#[test]
fn refresh_writes_full_tuple_for_debt_free_account() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let account = debt_free_account(&env);
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let mut cache = Cache::new_view(&env);
        let mut position = stamped_position();

        refresh_supply_risk_params(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            &config(7_000, STAMPED_BONUS + 400, STAMPED_FEES - 50),
            RiskRefreshScope::FullTuple,
        );

        assert_eq!(position.liquidation_threshold.raw(), 7_000);
        assert_eq!(position.liquidation_bonus.raw(), STAMPED_BONUS + 400);
        assert_eq!(position.liquidation_fees.raw(), STAMPED_FEES - 50);
        assert_eq!(position.loan_to_value.raw(), 5_000, "LTV is ungated");
    });
}

#[test]
fn refresh_writes_ltv_but_holds_tuple_when_gate_rejects() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };

        let mut borrow_positions = Map::new(&env);
        borrow_positions.set(
            hub.clone(),
            DebtPositionRaw {
                scaled_amount: Ray::from_asset(100_0000000, 7).raw(),
            },
        );
        let account = Account {
            owner: Address::generate(&env),
            spoke_id: 1,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions,
        };

        let mut cache = Cache::new_view(&env);
        let mut prices = Map::new(&env);
        prices.set(
            hub.asset.clone(),
            PriceFeedRaw {
                price_wad: WAD,
                asset_decimals: 7,
                timestamp: 0,
            },
        );
        cache.set_prices(prices);
        cache.put_market_index(
            &hub,
            &MarketIndexRaw {
                borrow_index: RAY,
                supply_index: RAY,
            },
        );

        let mut position = stamped_position();
        refresh_supply_risk_params(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            &config(7_000, STAMPED_BONUS + 400, STAMPED_FEES - 50),
            RiskRefreshScope::FullTuple,
        );

        assert_eq!(
            position.loan_to_value.raw(),
            5_000,
            "LTV rides outside the gate"
        );
        assert_eq!(
            position.liquidation_threshold.raw(),
            STAMPED_LT,
            "gate held the threshold at its stamped vintage"
        );
        assert_eq!(
            position.liquidation_bonus.raw(),
            STAMPED_BONUS,
            "gate held the bonus"
        );
        assert_eq!(
            position.liquidation_fees.raw(),
            STAMPED_FEES,
            "gate held the fees"
        );
    });
}

#[test]
fn refresh_ltv_only_leaves_liquidation_tuple_stamped() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let account = debt_free_account(&env);
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let mut cache = Cache::new_view(&env);
        let mut position = stamped_position();

        refresh_supply_risk_params(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            &config(7_000, STAMPED_BONUS + 400, STAMPED_FEES - 50),
            RiskRefreshScope::LtvOnly,
        );

        assert_eq!(position.loan_to_value.raw(), 5_000, "LTV still lands");
        assert_eq!(position.liquidation_threshold.raw(), STAMPED_LT);
        assert_eq!(position.liquidation_bonus.raw(), STAMPED_BONUS);
        assert_eq!(position.liquidation_fees.raw(), STAMPED_FEES);
    });
}
