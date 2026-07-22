use super::*;
use crate::constants::THRESHOLD_UPDATE_MIN_HF_RAW;
use common::math::fp::Ray;
use common::types::PositionMode;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

fn debt_free_account(env: &Env) -> Account {
    Account {
        owner: Address::generate(env),
        spoke_id: 1,
        mode: PositionMode::Normal,
        supply_positions: Map::new(env),
        borrow_positions: Map::new(env),
    }
}

fn position_with_threshold(lt_bps: i128) -> AccountPosition {
    AccountPosition {
        scaled_amount: Ray::from(0),
        liquidation_threshold: Bps::from(lt_bps),
        liquidation_bonus: Bps::from(500i128),
        loan_to_value: Bps::from(7_500i128),
        liquidation_fees: Bps::from(100i128),
    }
}

// H-RISK-04: LT cuts on debt-free accounts always apply; the 1.05 HF floor is
// the gate constant used for debt-bearing accounts (integration PoC covers
// sticky LT under live risk totals).
#[test]
fn threshold_update_min_hf_is_one_point_zero_five_wad() {
    assert_eq!(THRESHOLD_UPDATE_MIN_HF_RAW, 1_050_000_000_000_000_000);
}

// Threshold raises are unconditional and debt-free decreases skip the HF
// gate — both must land on the position.
#[test]
fn apply_liquidation_threshold_updates_position_value() {
    let env = Env::default();
    let contract = env.register(crate::Controller, (Address::generate(&env),));
    env.as_contract(&contract, || {
        let account = debt_free_account(&env);
        let hub = HubAssetKey {
            hub_id: 0,
            asset: Address::generate(&env),
        };
        let mut cache = Cache::new_view(&env);

        let mut position = position_with_threshold(8_000);
        apply_liquidation_threshold(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            Bps::from(9_000i128),
        );
        assert_eq!(position.liquidation_threshold.raw(), 9_000);

        apply_liquidation_threshold(
            &env,
            &mut cache,
            &account,
            &hub,
            &mut position,
            Bps::from(7_000i128),
        );
        assert_eq!(position.liquidation_threshold.raw(), 7_000);
    });
}
