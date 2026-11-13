use super::*;
use crate::constants::{THRESHOLD_UPDATE_MIN_HF_RAW, WAD};
use common::constants::RAY;
use common::math::fp::Ray;
use common::types::{DebtPositionRaw, MarketIndexRaw, PositionMode, PriceFeedRaw};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Map};

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

fn matching_config() -> AssetConfig {
    AssetConfig {
        loan_to_value: Bps::from(STAMPED_LTV),
        liquidation_threshold: Bps::from(STAMPED_LT),
        liquidation_bonus: Bps::from(STAMPED_BONUS),
        liquidation_fees: Bps::from(STAMPED_FEES),
        is_collateralizable: true,
        is_borrowable: true,
    }
}

#[test]
fn threshold_update_min_hf_is_one_point_zero_five_wad() {
    assert_eq!(THRESHOLD_UPDATE_MIN_HF_RAW, 1_050_000_000_000_000_000);
}

#[test]
fn adverse_risk_refresh_checks_exact_one_point_zero_five_boundary() {
    // 150 collateral * 70% threshold / 100 debt = 1.05. One raw
    // seven-decimal unit either side must not be hidden by a tolerance.
    for delta in [-1, 0, 1] {
        let env = Env::default();
        let contract = env.register(crate::Controller, (Address::generate(&env),));
        env.as_contract(&contract, || {
            let hub = HubAssetKey {
                hub_id: 1,
                asset: Address::generate(&env),
            };
            let mut account = debt_free_account(&env);
            account.borrow_positions.set(
                hub.clone(),
                DebtPositionRaw {
                    scaled_amount: Ray::from_asset(&env, 100_0000000, 7).raw(),
                },
            );
            let mut cache = Context::new_view(&env);
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
            position.scaled_amount = Ray::from_asset(&env, 150_0000000 + delta, 7);
            let mut adverse = matching_config();
            adverse.liquidation_threshold = Bps::from(7_000);
            let changed = refresh_supply_risk_params(
                &env,
                &mut cache,
                &account,
                &hub,
                &mut position,
                &adverse,
                RiskRefreshScope::FullTuple,
            );
            assert_eq!(changed, delta >= 0, "boundary delta {delta}");
            assert_eq!(
                position.liquidation_threshold.raw(),
                if delta >= 0 { 7_000 } else { STAMPED_LT }
            );
        });
    }
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
        let mut cache = Context::new_view(&env);
        let mut position = stamped_position();

        let changed = refresh_supply_risk_params(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            &config(7_000, STAMPED_BONUS + 400, STAMPED_FEES - 50),
            RiskRefreshScope::FullTuple,
        );

        assert!(changed, "restamp must report a risk-stamp change");
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
                scaled_amount: Ray::from_asset(&env, 100_0000000, 7).raw(),
            },
        );
        let account = Account {
            owner: Address::generate(&env),
            spoke_id: 1,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions,
        };

        let mut cache = Context::new_view(&env);
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
        let changed = refresh_supply_risk_params(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            &config(7_000, STAMPED_BONUS + 400, STAMPED_FEES - 50),
            RiskRefreshScope::FullTuple,
        );

        assert!(
            changed,
            "LTV still moves outside the gate, so stamps changed"
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
        let mut cache = Context::new_view(&env);
        let mut position = stamped_position();

        let changed = refresh_supply_risk_params(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            &config(7_000, STAMPED_BONUS + 400, STAMPED_FEES - 50),
            RiskRefreshScope::LtvOnly,
        );

        assert!(changed, "LtvOnly still restamps LTV");
        assert_eq!(position.loan_to_value.raw(), 5_000, "LTV still lands");
        assert_eq!(position.liquidation_threshold.raw(), STAMPED_LT);
        assert_eq!(position.liquidation_bonus.raw(), STAMPED_BONUS);
        assert_eq!(position.liquidation_fees.raw(), STAMPED_FEES);
    });
}

/// Listing at LTV 8_200 and threshold 8_500: a 300 BPS gap, with the
/// threshold 500 BPS above the stamped one.
fn looser_config(bonus: i128, fees: i128) -> AssetConfig {
    AssetConfig {
        loan_to_value: Bps::from(8_200i128),
        ..config(8_500, bonus, fees)
    }
}

#[test]
fn ltv_under_stored_threshold_drops_by_the_threshold_shortfall() {
    let env = Env::default();
    let listed = looser_config(STAMPED_BONUS, STAMPED_FEES);
    for (stored_threshold, expected_ltv) in [
        (9_000, 8_200),
        (8_500, 8_200),
        (8_499, 8_199),
        (8_000, 7_700),
        (301, 1),
        (300, 0),
        (299, 0),
        (0, 0),
    ] {
        assert_eq!(
            ltv_under_stored_threshold(&env, Bps::from(stored_threshold), &listed).raw(),
            expected_ltv,
            "stored threshold {stored_threshold}"
        );
    }
}

#[test]
fn refresh_keeps_ltv_under_the_held_threshold_when_gate_rejects() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let mut account = debt_free_account(&env);
        account.borrow_positions.set(
            hub.clone(),
            DebtPositionRaw {
                scaled_amount: Ray::from_asset(&env, 100_0000000, 7).raw(),
            },
        );
        let mut cache = Context::new_view(&env);
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

        // The higher bonus favors liquidators and the empty position fails
        // the 1.05 gate, so the stamped threshold stays.
        let mut position = stamped_position();
        let changed = refresh_supply_risk_params(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            &looser_config(STAMPED_BONUS + 100, STAMPED_FEES),
            RiskRefreshScope::FullTuple,
        );

        assert!(changed, "LTV moves to the listed gap under the threshold");
        assert_eq!(
            position.loan_to_value.raw(),
            STAMPED_LT - 300,
            "LTV keeps the listed 300 BPS gap under the held threshold"
        );
        assert_eq!(position.liquidation_threshold.raw(), STAMPED_LT);
        assert_eq!(position.liquidation_bonus.raw(), STAMPED_BONUS);
        assert_eq!(position.liquidation_fees.raw(), STAMPED_FEES);
    });
}

#[test]
fn refresh_ltv_only_keeps_ltv_under_the_stamped_threshold() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let account = debt_free_account(&env);
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let mut cache = Context::new_view(&env);
        let mut position = stamped_position();

        let changed = refresh_supply_risk_params(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            &looser_config(STAMPED_BONUS, STAMPED_FEES),
            RiskRefreshScope::LtvOnly,
        );

        assert!(changed);
        assert_eq!(position.loan_to_value.raw(), STAMPED_LT - 300);
        assert_eq!(position.liquidation_threshold.raw(), STAMPED_LT);
        assert_eq!(position.liquidation_bonus.raw(), STAMPED_BONUS);
        assert_eq!(position.liquidation_fees.raw(), STAMPED_FEES);
    });
}

#[test]
fn refresh_writes_the_listed_ltv_when_the_listed_threshold_lands() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let account = debt_free_account(&env);
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let mut cache = Context::new_view(&env);
        let mut position = stamped_position();

        let changed = refresh_supply_risk_params(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            &looser_config(STAMPED_BONUS + 100, STAMPED_FEES),
            RiskRefreshScope::FullTuple,
        );

        assert!(changed);
        assert_eq!(position.loan_to_value.raw(), 8_200);
        assert_eq!(position.liquidation_threshold.raw(), 8_500);
        assert_eq!(position.liquidation_bonus.raw(), STAMPED_BONUS + 100);
        assert_eq!(position.liquidation_fees.raw(), STAMPED_FEES);
    });
}

#[test]
fn refresh_returns_false_when_full_tuple_already_matches() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let account = debt_free_account(&env);
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let mut cache = Context::new_view(&env);
        let mut position = stamped_position();

        let changed = refresh_supply_risk_params(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            &matching_config(),
            RiskRefreshScope::FullTuple,
        );

        assert!(
            !changed,
            "identical listed config must be a risk-stamp noop"
        );
        assert_eq!(position.loan_to_value.raw(), STAMPED_LTV);
        assert_eq!(position.liquidation_threshold.raw(), STAMPED_LT);
        assert_eq!(position.liquidation_bonus.raw(), STAMPED_BONUS);
        assert_eq!(position.liquidation_fees.raw(), STAMPED_FEES);
    });
}

#[test]
fn refresh_returns_false_when_ltv_only_already_matches() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let account = debt_free_account(&env);
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let mut cache = Context::new_view(&env);
        let mut position = stamped_position();

        let changed = refresh_supply_risk_params(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            &matching_config(),
            RiskRefreshScope::LtvOnly,
        );

        assert!(!changed, "matching LTV is a noop under LtvOnly");
        assert_eq!(position.loan_to_value.raw(), STAMPED_LTV);
        assert_eq!(position.liquidation_threshold.raw(), STAMPED_LT);
        assert_eq!(position.liquidation_bonus.raw(), STAMPED_BONUS);
        assert_eq!(position.liquidation_fees.raw(), STAMPED_FEES);
    });
}

#[test]
fn refresh_returns_false_when_gate_holds_and_ltv_already_matches() {
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
                scaled_amount: Ray::from_asset(&env, 100_0000000, 7).raw(),
            },
        );
        let account = Account {
            owner: Address::generate(&env),
            spoke_id: 1,
            mode: PositionMode::Normal,
            supply_positions: Map::new(&env),
            borrow_positions,
        };

        let mut cache = Context::new_view(&env);
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

        // Listed LTV already equals stamp; adverse tuple is gated and held.
        let mut position = stamped_position();
        let adverse = AssetConfig {
            loan_to_value: Bps::from(STAMPED_LTV),
            liquidation_threshold: Bps::from(7_000i128),
            liquidation_bonus: Bps::from(STAMPED_BONUS + 400),
            liquidation_fees: Bps::from(STAMPED_FEES - 50),
            is_collateralizable: true,
            is_borrowable: true,
        };

        let changed = refresh_supply_risk_params(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            &adverse,
            RiskRefreshScope::FullTuple,
        );

        assert!(
            !changed,
            "gated hold with matching LTV must report unchanged stamps"
        );
        assert_eq!(position.loan_to_value.raw(), STAMPED_LTV);
        assert_eq!(position.liquidation_threshold.raw(), STAMPED_LT);
        assert_eq!(position.liquidation_bonus.raw(), STAMPED_BONUS);
        assert_eq!(position.liquidation_fees.raw(), STAMPED_FEES);
    });
}
