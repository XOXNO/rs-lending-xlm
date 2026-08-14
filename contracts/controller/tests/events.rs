use super::*;
use common::types::{InterestRateModel, MarketParamsRaw, PositionMode, SpokeAssetConfig};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, Address, Env, Vec};

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

fn sample_rate_model() -> InterestRateModel {
    InterestRateModel {
        max_borrow_rate: 1_000_000_000_000_000_000,
        base_borrow_rate: 10_000_000_000_000_000,
        slope1: 50_000_000_000_000_000,
        slope2: 100_000_000_000_000_000,
        slope3: 200_000_000_000_000_000,
        mid_utilization: 500_000_000_000_000_000,
        optimal_utilization: 800_000_000_000_000_000,
        max_utilization: 900_000_000_000_000_000,
        reserve_factor: 1_000,
        // Intentionally non-default: must NOT appear on market events.
        is_flashloanable: true,
        flashloan_fee: 9,
    }
}

fn sample_market_params(asset: &Address) -> MarketParamsRaw {
    let model = sample_rate_model();
    MarketParamsRaw {
        max_borrow_rate: model.max_borrow_rate,
        base_borrow_rate: model.base_borrow_rate,
        slope1: model.slope1,
        slope2: model.slope2,
        slope3: model.slope3,
        mid_utilization: model.mid_utilization,
        optimal_utilization: model.optimal_utilization,
        max_utilization: model.max_utilization,
        reserve_factor: model.reserve_factor,
        is_flashloanable: model.is_flashloanable,
        flashloan_fee: model.flashloan_fee,
        asset_id: asset.clone(),
        asset_decimals: 7,
    }
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

const PUBLISHED_EVENT_COUNT: usize = 10;

#[test]
fn every_event_helper_publishes_exactly_one_event() {
    use soroban_sdk::testutils::Events as _;

    let (env, contract) = setup();
    env.as_contract(&contract, || {
        let asset = dummy_address(&env);
        let caller = dummy_address(&env);

        // Asserting after every publish pins each helper to exactly one event;
        // a single total could hide one helper emitting two and another zero.
        let mut published = 0usize;
        let assert_one_more = |published: &mut usize, label: &str| {
            *published += 1;
            assert_eq!(
                env.events().all().events().len(),
                *published,
                "{label} must publish exactly one event"
            );
        };

        CreateMarketEvent::from_params(
            1,
            asset.clone(),
            asset.clone(),
            &sample_market_params(&asset),
        )
        .publish(&env);
        assert_one_more(&mut published, "CreateMarketEvent");

        UpdateMarketParamsEvent::from((1u32, asset.clone(), &sample_rate_model())).publish(&env);
        assert_one_more(&mut published, "UpdateMarketParamsEvent");

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
        assert_one_more(&mut published, "UpdatePositionBatchEvent");

        FlashLoanEvent {
            hub_id: 1,
            asset: asset.clone(),
            receiver: caller.clone(),
            caller: caller.clone(),
            amount: 0,
            fee: 0,
        }
        .publish(&env);
        assert_one_more(&mut published, "FlashLoanEvent");

        LiquidationEvent {
            liquidator: caller.clone(),
            account_id: 1,
            repaid_usd_wad: 0,
            bonus_bps: 0,
        }
        .publish(&env);
        assert_one_more(&mut published, "LiquidationEvent");

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
        assert_one_more(&mut published, "UpdateSpokeEvent");

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
        assert_one_more(&mut published, "UpdateSpokeAssetEvent");

        RemoveSpokeAssetEvent {
            asset: asset.clone(),
            spoke_id: 1,
            hub_id: 1,
        }
        .publish(&env);
        assert_one_more(&mut published, "RemoveSpokeAssetEvent");

        CleanBadDebtEvent {
            account_id: 1,
            total_borrow_usd_wad: 0,
            total_collateral_usd_wad: 0,
        }
        .publish(&env);
        assert_one_more(&mut published, "CleanBadDebtEvent");

        InitialMultiplyPaymentEvent {
            token: asset.clone(),
            amount: 0,
            account_id: 1,
        }
        .publish(&env);
        assert_one_more(&mut published, "InitialMultiplyPaymentEvent");

        assert_eq!(
            published, PUBLISHED_EVENT_COUNT,
            "helper coverage drifted: update the exercised list and the count together"
        );
    });
}

#[test]
fn create_market_event_from_params_flattens_rate_fields() {
    let env = Env::default();
    let asset = dummy_address(&env);
    let market = dummy_address(&env);
    let params = sample_market_params(&asset);

    let ev = CreateMarketEvent::from_params(2, asset.clone(), market.clone(), &params);

    assert_eq!(ev.hub_id, 2);
    assert_eq!(ev.base_asset, asset);
    assert_eq!(ev.market_address, market);
    assert_eq!(ev.max_borrow_rate, params.max_borrow_rate);
    assert_eq!(ev.base_borrow_rate, params.base_borrow_rate);
    assert_eq!(ev.slope1, params.slope1);
    assert_eq!(ev.slope2, params.slope2);
    assert_eq!(ev.slope3, params.slope3);
    assert_eq!(ev.mid_utilization, params.mid_utilization);
    assert_eq!(ev.optimal_utilization, params.optimal_utilization);
    assert_eq!(ev.max_utilization, params.max_utilization);
    assert_eq!(ev.reserve_factor, params.reserve_factor);
    // Wire shape remains flat named fields only — no nested params struct and
    // no flash-loan / decimals fields on this event type.
}

#[test]
fn update_market_params_event_from_rate_model_is_flat() {
    let env = Env::default();
    let asset = dummy_address(&env);
    let model = sample_rate_model();

    let via_ctor = UpdateMarketParamsEvent::from_rate_model(7, asset.clone(), &model);
    let via_from = UpdateMarketParamsEvent::from((7u32, asset.clone(), &model));

    assert_eq!(via_ctor, via_from);
    assert_eq!(via_ctor.hub_id, 7);
    assert_eq!(via_ctor.asset, asset);
    assert_eq!(via_ctor.max_borrow_rate, model.max_borrow_rate);
    assert_eq!(via_ctor.base_borrow_rate, model.base_borrow_rate);
    assert_eq!(via_ctor.slope1, model.slope1);
    assert_eq!(via_ctor.slope2, model.slope2);
    assert_eq!(via_ctor.slope3, model.slope3);
    assert_eq!(via_ctor.mid_utilization, model.mid_utilization);
    assert_eq!(via_ctor.optimal_utilization, model.optimal_utilization);
    assert_eq!(via_ctor.max_utilization, model.max_utilization);
    assert_eq!(via_ctor.reserve_factor, model.reserve_factor);
    // is_flashloanable / flashloan_fee exist on the model but have no event
    // fields — mapping must drop them rather than nest the whole model.
}
