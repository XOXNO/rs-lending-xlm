use super::*;
use common::types::{PositionMode, SpokeAssetConfig};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, vec, Address, Env, Vec};

#[contract]
struct TestContract;

fn setup() -> (Env, Address) {
    let env = Env::default();
    let contract = env.register(TestContract, ());
    (env, contract)
}

fn dummy_address(env: &Env) -> Address {
    Address::generate(env)
}

#[test]
fn event_position_mode_eq_and_from() {
    assert_eq!(EventPositionMode::None, EventPositionMode::None);
    assert_ne!(EventPositionMode::Long, EventPositionMode::Short);
    assert_eq!(
        EventPositionMode::from(PositionMode::Normal),
        EventPositionMode::None
    );
    assert_eq!(
        EventPositionMode::from(PositionMode::Multiply),
        EventPositionMode::Multiply
    );
    assert_eq!(
        EventPositionMode::from(PositionMode::Long),
        EventPositionMode::Long
    );
    assert_eq!(
        EventPositionMode::from(PositionMode::Short),
        EventPositionMode::Short
    );
}

#[test]
fn event_account_attributes_from_account_meta_spoke() {
    let env = Env::default();
    let owner = dummy_address(&env);
    let meta = AccountMeta {
        owner: owner.clone(),
        spoke_id: 3,
        mode: PositionMode::Long,
    };
    let attrs = EventAccountAttributes::from(&meta);
    assert_eq!(attrs.0, owner);
    assert_eq!(attrs.1, 3);
    assert_eq!(attrs.2, EventPositionMode::Long);
}

#[test]
fn all_event_helpers_publish_one_event() {
    use soroban_sdk::testutils::Events as _;

    let (env, contract) = setup();
    env.as_contract(&contract, || {
        let asset = dummy_address(&env);
        let caller = dummy_address(&env);

        CreateMarketEvent {
            hub_id: 1,
            base_asset: asset.clone(),
            max_borrow_rate: 0,
            base_borrow_rate: 0,
            slope1: 0,
            slope2: 0,
            slope3: 0,
            mid_utilization: 0,
            optimal_utilization: 0,
            max_utilization: 0,
            reserve_factor: 0,
            market_address: asset.clone(),
        }
        .publish(&env);

        UpdateMarketParamsEvent {
            asset: asset.clone(),
            max_borrow_rate: 0,
            base_borrow_rate: 0,
            slope1: 0,
            slope2: 0,
            slope3: 0,
            mid_utilization: 0,
            optimal_utilization: 0,
            max_utilization: 0,
            reserve_factor: 0,
        }
        .publish(&env);

        let mut deposits = Vec::new(&env);
        deposits.push_back(EventDepositDelta(
            PositionAction::Supply,
            1,
            asset.clone(),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ));
        UpdatePositionBatchEvent {
            account_id: 1,
            account_attributes: EventAccountAttributes(caller.clone(), 0, EventPositionMode::None),
            deposits,
            borrows: Vec::new(&env),
        }
        .publish(&env);

        FlashLoanEvent {
            hub_id: 1,
            asset: asset.clone(),
            receiver: caller.clone(),
            caller: caller.clone(),
            amount: 0,
            fee: 0,
        }
        .publish(&env);

        LiquidationEvent {
            liquidator: caller.clone(),
            account_id: 1,
            repaid_usd_wad: 0,
            bonus_bps: 0,
        }
        .publish(&env);

        UpdateSpokeEvent {
            spoke: EventSpoke {
                spoke_id: 1,
                is_deprecated: false,
                liquidation_target_hf_wad: 1_020_000_000_000_000_000,
                hf_for_max_bonus_wad: 510_000_000_000_000_000,
                liquidation_bonus_factor_bps: 10_000,
            },
        }
        .publish(&env);

        UpdateSpokeAssetEvent {
            asset: asset.clone(),
            config: SpokeAssetConfig {
                is_collateralizable: true,
                is_borrowable: true,
                paused: false,
                frozen: false,
                loan_to_value: 9000,
                liquidation_threshold: 9500,
                liquidation_bonus: 200,
                liquidation_fees: 0,
                supply_cap: 0,
                borrow_cap: 0,
            },
            spoke_id: 1,
            hub_id: 1,
        }
        .publish(&env);

        RemoveSpokeAssetEvent {
            asset: asset.clone(),
            spoke_id: 1,
            hub_id: 1,
        }
        .publish(&env);

        CleanBadDebtEvent {
            account_id: 1,
            total_borrow_usd_wad: 0,
            total_collateral_usd_wad: 0,
        }
        .publish(&env);

        InitialMultiplyPaymentEvent {
            token: asset.clone(),
            amount: 0,
            usd_value_wad: 0,
            account_id: 1,
        }
        .publish(&env);

        let _ignored: Vec<Address> = vec![&env];
    });

    assert_eq!(env.events().all().events().len(), 10);
}

#[test]
fn create_market_event_carries_hub_id() {
    let env = Env::default();
    let asset = dummy_address(&env);
    let ev = CreateMarketEvent {
        hub_id: 2,
        base_asset: asset.clone(),
        max_borrow_rate: 0,
        base_borrow_rate: 0,
        slope1: 0,
        slope2: 0,
        slope3: 0,
        mid_utilization: 0,
        optimal_utilization: 0,
        max_utilization: 0,
        reserve_factor: 0,
        market_address: asset.clone(),
    };
    assert_eq!(ev.hub_id, 2);
}

#[test]
fn position_deltas_carry_hub_id_and_liquidation_fees() {
    let env = Env::default();
    let asset = dummy_address(&env);
    let dep = EventDepositDelta(
        PositionAction::Supply,
        4,
        asset.clone(),
        0,
        0,
        0,
        0,
        0,
        0,
        150,
    );
    let bor = EventBorrowDelta(PositionAction::Repay, 9, asset.clone(), 0, 0, 0);
    assert_eq!(dep.1, 4);
    assert_eq!(dep.9, 150);
    assert_eq!(bor.1, 9);
}

#[test]
fn flash_loan_event_carries_hub_id() {
    let env = Env::default();
    let asset = dummy_address(&env);
    let caller = dummy_address(&env);
    let ev = FlashLoanEvent {
        hub_id: 7,
        asset: asset.clone(),
        receiver: caller.clone(),
        caller,
        amount: 0,
        fee: 0,
    };
    assert_eq!(ev.hub_id, 7);
}

#[test]
fn liquidation_event_carries_liquidator_and_account() {
    let env = Env::default();
    let liquidator = dummy_address(&env);
    let ev = LiquidationEvent {
        liquidator: liquidator.clone(),
        account_id: 42,
        repaid_usd_wad: 1_500_000,
        bonus_bps: 500,
    };
    assert_eq!(ev.liquidator, liquidator);
    assert_eq!(ev.account_id, 42);
    assert_eq!(ev.repaid_usd_wad, 1_500_000);
    assert_eq!(ev.bonus_bps, 500);
}

#[test]
fn spoke_asset_events_carry_hub_id() {
    let env = Env::default();
    let asset = dummy_address(&env);
    let upd = UpdateSpokeAssetEvent {
        asset: asset.clone(),
        config: SpokeAssetConfig {
            is_collateralizable: true,
            is_borrowable: true,
            paused: false,
            frozen: false,
            loan_to_value: 9000,
            liquidation_threshold: 9500,
            liquidation_bonus: 200,
            liquidation_fees: 0,
            supply_cap: 0,
            borrow_cap: 0,
        },
        spoke_id: 1,
        hub_id: 3,
    };
    let rem = RemoveSpokeAssetEvent {
        asset,
        spoke_id: 1,
        hub_id: 3,
    };
    assert_eq!(upd.hub_id, 3);
    assert_eq!(rem.hub_id, 3);
}
