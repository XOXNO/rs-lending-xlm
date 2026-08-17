use super::*;
use crate::storage;
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
fn event_account_attributes_from_account_owner_spoke_mode() {
    // `From<&AccountMeta>` is gone: `AccountMeta` no longer carries an owner, since
    // ownership now resolves through the position NFT. `From<&Account>` is the
    // surviving source of `EventAccountAttributes` and pins the same tuple shape
    // this test originally pinned against `AccountMeta`.
    let env = Env::default();
    let owner = dummy_address(&env);
    let account = Account {
        owner: owner.clone(),
        spoke_id: 3,
        mode: PositionMode::Long,
        supply_positions: soroban_sdk::Map::new(&env),
        borrow_positions: soroban_sdk::Map::new(&env),
    };
    let attrs = EventAccountAttributes::from(&account);
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
                no_seize: false,
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

// ===========================================================================
// Liquidation events: gross vs net.
//
// TOB-AAVE-4 was exactly this shape — `LiquidationCall` documented
// `liquidatedCollateral` as the collateral the liquidator receives while the
// implementation emitted the gross seizure with the protocol fee still in it,
// so every off-chain consumer under-counted the fee. The tests below pin our
// answer against quantities the controller does not compute: the tokens a pool
// actually pays out, and the fee shares the controller hands the pool.
//
// The pool is stood up as a recorder here rather than the real contract — the
// controller crate does not depend on `pool`. Its withdraw leg reuses the
// pool's own `common::rates::resolve_withdrawal` and mirrors
// `contracts/pool/src/ops/withdraw.rs::withhold_liquidation_fee`, which is the
// only pool behaviour these assertions rest on. End-to-end coverage against the
// real pool lives in `tests/test-harness/tests/controller/`.
// ===========================================================================

extern crate std;

use common::constants::RAY;
use common::math::fp::Ray;
use common::rates::resolve_withdrawal;
use common::types::{
    AccountMeta, AccountPositionRaw, DebtPositionRaw, HubAssetKey, HubConfig, MarketIndexRaw,
    PoolAction, PoolPositionMutation, PoolSeizeEntry, PoolWithdrawEntry, PriceFeedRaw, PriceKey,
    ScaledPositionRaw, SeizeMode, SpokeConfig, SpokeUsageRaw,
};
use soroban_sdk::testutils::Events as _;
use soroban_sdk::{contractimpl, symbol_short, token, Map, Symbol, TryFromVal, Val};

const DECIMALS: u32 = 7;
/// One whole token in 7-decimal native units.
const UNIT: i128 = 10_000_000;
const COLLATERAL_TOKENS: i128 = 100_000;
const DEBT_TOKENS: i128 = 70_000;
/// 20% of the bonus portion of a seizure, so the protocol fee is unmistakably
/// non-zero and large enough that a gross/net confusion cannot hide in dust.
const LIQUIDATION_FEES_BPS: u32 = 2_000;

const WITHDRAW_LOG: Symbol = symbol_short!("wlog");
const SEIZE_LOG: Symbol = symbol_short!("slog");
const FEEDS: Symbol = symbol_short!("feeds");
const SHORTFALL: Symbol = symbol_short!("short");

// --- mocks ---------------------------------------------------------------

#[contract]
struct MockToken;

#[contractimpl]
impl MockToken {
    pub fn mint(env: Env, to: Address, amount: i128) {
        let current: i128 = env.storage().instance().get(&to).unwrap_or(0);
        env.storage().instance().set(&to, &(current + amount));
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage().instance().get(&id).unwrap_or(0)
    }

    /// Delivers `bps` less than is sent, so the recipient's measured receipt
    /// falls short of the requested amount.
    pub fn set_shortfall(env: Env, bps: i128) {
        env.storage().instance().set(&SHORTFALL, &bps);
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        let shortfall: i128 = env.storage().instance().get(&SHORTFALL).unwrap_or(0);
        let delivered = amount - amount * shortfall / 10_000;
        let sender: i128 = env.storage().instance().get(&from).unwrap_or(0);
        let recipient: i128 = env.storage().instance().get(&to).unwrap_or(0);
        assert!(sender >= amount, "mock token: insufficient balance");
        env.storage().instance().set(&from, &(sender - amount));
        env.storage().instance().set(&to, &(recipient + delivered));
    }
}

#[contract]
struct MockOracle;

#[contractimpl]
impl MockOracle {
    pub fn seed(env: Env, feeds: Map<Address, PriceFeedRaw>) {
        env.storage().instance().set(&FEEDS, &feeds);
    }

    pub fn prices(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw> {
        let feeds: Map<Address, PriceFeedRaw> =
            env.storage().instance().get(&FEEDS).expect("feeds seeded");
        let mut out = Map::new(&env);
        for key in keys.iter() {
            let PriceKey::Token(asset) = key.clone() else {
                continue;
            };
            if let Some(feed) = feeds.get(asset) {
                out.set(key, feed);
            }
        }
        out
    }
}

#[contract]
struct MockPool;

#[contractimpl]
impl MockPool {
    /// Indexes are pinned at 1.0 RAY: interest accrual is orthogonal to the
    /// gross/net question and a unit index keeps shares and asset units
    /// directly comparable in the assertions.
    pub fn get_bulk_indexes(env: Env, hub_assets: Vec<HubAssetKey>) -> Vec<MarketIndexRaw> {
        let mut out = Vec::new(&env);
        for _ in hub_assets.iter() {
            out.push_back(unit_index());
        }
        out
    }

    pub fn repay(env: Env, _payer: Address, actions: Vec<PoolAction>) -> Vec<PoolPositionMutation> {
        let mut out = Vec::new(&env);
        for action in actions.iter() {
            let position = Ray::from(action.position.scaled_amount);
            let burned = Ray::from_asset(action.amount, DECIMALS).min(position);
            out.push_back(PoolPositionMutation {
                position: ScaledPositionRaw {
                    scaled_amount: position.checked_sub(&env, burned).raw(),
                },
                market_index: unit_index(),
                actual_amount: action.amount,
                asset_decimals: DECIMALS,
            });
        }
        out
    }

    /// `SeizeMode::Transfer`'s leg. Burns shares for the requested gross amount
    /// through the pool's own `resolve_withdrawal`, withholds `protocol_fee`
    /// from that gross, and pays the remainder out — the rule
    /// `withhold_liquidation_fee` implements. Records `(gross, fee, paid)` so a
    /// test can compare the emitted event against what left the pool.
    pub fn withdraw(
        env: Env,
        receiver: Address,
        is_liquidation: bool,
        entries: Vec<PoolWithdrawEntry>,
    ) -> Vec<PoolPositionMutation> {
        let mut log: Vec<(i128, i128, i128)> = env
            .storage()
            .instance()
            .get(&WITHDRAW_LOG)
            .unwrap_or(Vec::new(&env));
        let mut out = Vec::new(&env);
        for entry in entries.iter() {
            let position = Ray::from(entry.action.position.scaled_amount);
            let (burned, gross) = resolve_withdrawal(
                &env,
                entry.action.amount,
                position,
                Ray::from(RAY),
                DECIMALS,
            );
            let fee = if is_liquidation {
                entry.protocol_fee
            } else {
                0
            };
            let paid = gross - fee;
            token::Client::new(&env, &entry.action.hub_asset.asset).transfer(
                &env.current_contract_address(),
                &receiver,
                &paid,
            );
            log.push_back((gross, fee, paid));
            out.push_back(PoolPositionMutation {
                position: ScaledPositionRaw {
                    scaled_amount: position.checked_sub(&env, burned).raw(),
                },
                market_index: unit_index(),
                actual_amount: gross,
                asset_decimals: DECIMALS,
            });
        }
        env.storage().instance().set(&WITHDRAW_LOG, &log);
        out
    }

    /// `SeizeMode::Credit`'s only pool interaction: the shares reclassified as
    /// revenue. No cash moves.
    pub fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>) {
        let mut log: Vec<i128> = env
            .storage()
            .instance()
            .get(&SEIZE_LOG)
            .unwrap_or(Vec::new(&env));
        for entry in entries.iter() {
            log.push_back(entry.position.scaled_amount);
        }
        env.storage().instance().set(&SEIZE_LOG, &log);
    }

    pub fn withdraw_log(env: Env) -> Vec<(i128, i128, i128)> {
        env.storage()
            .instance()
            .get(&WITHDRAW_LOG)
            .unwrap_or(Vec::new(&env))
    }

    pub fn seize_log(env: Env) -> Vec<i128> {
        env.storage()
            .instance()
            .get(&SEIZE_LOG)
            .unwrap_or(Vec::new(&env))
    }
}

fn unit_index() -> MarketIndexRaw {
    MarketIndexRaw {
        borrow_index: RAY,
        supply_index: RAY,
    }
}

// --- fixture -------------------------------------------------------------

struct Liquidation {
    env: Env,
    controller: Address,
    pool: Address,
    collateral: Address,
    liquidator: Address,
    account_id: u64,
    debt_key: HubAssetKey,
}

/// An account whose health factor sits below one with a single collateral and a
/// single debt asset, sized so the seizure clamps below the position (leaving a
/// real bonus portion for the protocol fee to bite on) and so the residual debt
/// stays clear of the bad-debt dust gate.
fn unhealthy_account() -> Liquidation {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let controller = env.register(crate::Controller, (admin,));
    let pool = env.register(MockPool, ());
    let oracle = env.register(MockOracle, ());
    let collateral = env.register(MockToken, ());
    let debt = env.register(MockToken, ());

    let liquidator = Address::generate(&env);
    let owner = Address::generate(&env);

    let collateral_key = HubAssetKey {
        hub_id: 1,
        asset: collateral.clone(),
    };
    let debt_key = HubAssetKey {
        hub_id: 1,
        asset: debt.clone(),
    };

    let feed = PriceFeedRaw {
        price_wad: common::constants::WAD,
        asset_decimals: DECIMALS,
        timestamp: 0,
    };
    let mut feeds = Map::new(&env);
    feeds.set(collateral.clone(), feed.clone());
    feeds.set(debt.clone(), feed);
    MockOracleClient::new(&env, &oracle).seed(&feeds);

    // The liquidator funds the repayment; the pool holds the collateral it
    // would pay out in transfer mode.
    MockTokenClient::new(&env, &debt).mint(&liquidator, &(DEBT_TOKENS * UNIT));
    MockTokenClient::new(&env, &collateral).mint(&pool, &(COLLATERAL_TOKENS * UNIT));

    let nft = env.register(
        position_nft::PositionNft,
        (
            controller.clone(),
            soroban_sdk::String::from_str(&env, "uri"),
            soroban_sdk::String::from_str(&env, "Position"),
            soroban_sdk::String::from_str(&env, "POS"),
        ),
    );
    let account_id = u64::from(position_nft::PositionNftClient::new(&env, &nft).mint(&owner));

    env.as_contract(&controller, || {
        storage::set_position_nft(&env, &nft);
        storage::set_pool(&env, &pool);
        storage::set_price_aggregator(&env, &oracle);
        storage::set_hub(&env, 1, &HubConfig { is_active: true });
        storage::set_spoke(
            &env,
            1,
            &SpokeConfig {
                is_deprecated: false,
                liquidation_target_hf_wad: 1_020_000_000_000_000_000,
                hf_for_max_bonus_wad: 510_000_000_000_000_000,
                liquidation_bonus_factor_bps: 10_000,
            },
        );
        storage::set_spoke_asset(&env, 1, &collateral_key, &collateral_config());
        storage::set_spoke_asset(&env, 1, &debt_key, &collateral_config());

        storage::set_account_meta(
            &env,
            account_id,
            &AccountMeta {
                spoke_id: 1,
                mode: PositionMode::Normal,
            },
        );
        let mut supply = Map::new(&env);
        supply.set(
            collateral_key.clone(),
            AccountPositionRaw {
                scaled_amount: COLLATERAL_TOKENS * RAY,
                liquidation_threshold: 5_000,
                liquidation_bonus: 1_000,
                loan_to_value: 4_000,
                liquidation_fees: LIQUIDATION_FEES_BPS,
            },
        );
        storage::set_supply_positions(&env, account_id, &supply);
        let mut borrows = Map::new(&env);
        borrows.set(
            debt_key.clone(),
            DebtPositionRaw {
                scaled_amount: DEBT_TOKENS * RAY,
            },
        );
        storage::set_debt_positions(&env, account_id, &borrows);

        storage::set_spoke_usage(
            &env,
            1,
            &collateral_key,
            &SpokeUsageRaw {
                supplied_scaled_ray: COLLATERAL_TOKENS * RAY,
                borrowed_scaled_ray: 0,
            },
        );
        storage::set_spoke_usage(
            &env,
            1,
            &debt_key,
            &SpokeUsageRaw {
                supplied_scaled_ray: 0,
                borrowed_scaled_ray: DEBT_TOKENS * RAY,
            },
        );
        account_id
    });

    Liquidation {
        env,
        controller,
        pool,
        collateral,
        liquidator,
        account_id,
        debt_key,
    }
}

fn collateral_config() -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused: false,
        frozen: false,
        no_seize: false,
        loan_to_value: 4_000,
        liquidation_threshold: 5_000,
        liquidation_bonus: 1_000,
        liquidation_fees: LIQUIDATION_FEES_BPS,
        supply_cap: 0,
        borrow_cap: 0,
    }
}

impl Liquidation {
    /// Repays the whole debt balance; the planner caps the accepted repayment at
    /// the curve's ideal amount and refunds the rest before any transfer.
    /// Returns the receiving account id and everything the call published. The
    /// events are decoded here because `Env::events` only reports the most
    /// recent invocation, so any later client call — a balance read included —
    /// would discard them.
    fn liquidate(&self, mode: SeizeMode) -> (u64, Emitted) {
        let mut payments = Vec::new(&self.env);
        payments.push_back((self.debt_key.clone(), DEBT_TOKENS * UNIT));
        let liquidator = self.liquidator.clone();
        let account_id = self.account_id;
        let receiver_id = self.env.as_contract(&self.controller, || {
            crate::positions::liquidation::process_liquidation(
                &self.env,
                &liquidator,
                account_id,
                &payments,
                mode,
            )
        });
        (receiver_id, Emitted::capture(&self.env))
    }

    /// Makes the debt token deliver `bps` less than it is sent, the
    /// fee-on-transfer shape `transfer_amount_measured` exists to absorb.
    fn set_debt_shortfall_bps(&self, bps: i128) {
        MockTokenClient::new(&self.env, &self.debt_key.asset).set_shortfall(&bps);
    }

    fn collateral_balance(&self, who: &Address) -> i128 {
        MockTokenClient::new(&self.env, &self.collateral).balance(who)
    }

    fn withdraw_log(&self) -> Vec<(i128, i128, i128)> {
        MockPoolClient::new(&self.env, &self.pool).withdraw_log()
    }

    fn seize_log(&self) -> Vec<i128> {
        MockPoolClient::new(&self.env, &self.pool).seize_log()
    }
}

// --- event decoding ------------------------------------------------------

struct Batch {
    account_id: u64,
    deposits: Vec<EventDepositDelta>,
    borrows: Vec<EventBorrowDelta>,
}

/// The liquidation-relevant events of one call, decoded.
struct Emitted {
    batches: std::vec::Vec<Batch>,
    /// `LiquidationEvent::repaid_usd_wad`.
    repaid_usd_wad: i128,
}

impl Emitted {
    fn capture(env: &Env) -> Self {
        use soroban_sdk::xdr::ContractEventBody;

        let mut batches = std::vec::Vec::new();
        let mut repaid_usd_wad = 0i128;
        for event in env.events().all().events().iter() {
            let ContractEventBody::V0(body) = &event.body;
            let topic = |index: usize| {
                body.topics
                    .get(index)
                    .and_then(|scval| Symbol::try_from_val(env, scval).ok())
            };
            let (Some(first), Some(second)) = (topic(0), topic(1)) else {
                continue;
            };
            if first != symbol_short!("position") {
                continue;
            }
            if second == Symbol::new(env, "batch_update") {
                let fields =
                    Vec::<Val>::try_from_val(env, &body.data).expect("batch payload is a vec");
                batches.push(Batch {
                    account_id: u64::try_from_val(env, &fields.get(0).unwrap())
                        .expect("account id"),
                    deposits: Vec::<EventDepositDelta>::try_from_val(env, &fields.get(2).unwrap())
                        .expect("deposit deltas"),
                    borrows: Vec::<EventBorrowDelta>::try_from_val(env, &fields.get(3).unwrap())
                        .expect("borrow deltas"),
                });
            } else if second == Symbol::new(env, "liquidation") {
                // `LiquidationEvent` takes the default map payload, keyed by
                // field name.
                let fields = Map::<Symbol, Val>::try_from_val(env, &body.data)
                    .expect("liquidation payload is a map");
                repaid_usd_wad = i128::try_from_val(
                    env,
                    &fields
                        .get(Symbol::new(env, "repaid_usd_wad"))
                        .expect("repaid_usd_wad"),
                )
                .expect("repaid usd");
            }
        }
        Self {
            batches,
            repaid_usd_wad,
        }
    }
}

impl Batch {
    /// The single delta in this batch tagged `action` for `asset`.
    fn delta_for(&self, action: PositionAction, asset: &Address) -> EventDepositDelta {
        let mut found: Option<EventDepositDelta> = None;
        for delta in self.deposits.iter() {
            if delta.0 == action && delta.2 == *asset {
                assert!(
                    found.is_none(),
                    "expected a single matching delta per batch"
                );
                found = Some(delta);
            }
        }
        found.expect("batch carries the expected delta")
    }

    /// The liquidated account's seizure delta — **gross** of the protocol fee.
    fn seize_delta(&self, asset: &Address) -> EventDepositDelta {
        self.delta_for(PositionAction::LiqSeize, asset)
    }

    /// A share-credit receiver's credit delta — **net** of the protocol fee.
    ///
    /// Deliberately a different tag from [`Self::seize_delta`]: one tag
    /// carrying both senses would let an indexer read the gross figure as
    /// liquidator proceeds and overstate them by the fee.
    fn credit_delta(&self, asset: &Address) -> EventDepositDelta {
        self.delta_for(PositionAction::LiqCredit, asset)
    }
}

// --- transfer mode -------------------------------------------------------

#[test]
fn transfer_mode_seizure_delta_is_gross_of_the_protocol_fee() {
    let t = unhealthy_account();
    let before = t.collateral_balance(&t.liquidator);

    let (receiver_id, emitted) = t.liquidate(SeizeMode::Transfer);
    assert_eq!(receiver_id, 0, "no receiving account");
    let batches = &emitted.batches;

    let received = t.collateral_balance(&t.liquidator) - before;
    let (gross, fee, paid) = t.withdraw_log().get(0).expect("one withdraw leg");
    assert!(fee > 0, "fixture must produce a non-zero protocol fee");
    assert_eq!(paid, received, "the pool paid what the liquidator received");

    assert_eq!(batches.len(), 1, "transfer mode publishes one batch");
    let delta = batches[0].seize_delta(&t.collateral);

    // The emitted amount is the whole seizure, fee included — it is NOT the
    // liquidator's proceeds. This is the TOB-AAVE-4 distinction.
    assert_eq!(delta.5, gross, "LiqSeize amount is the gross seizure");
    assert_eq!(
        delta.5 - fee,
        received,
        "the liquidator receives the emitted amount minus the protocol fee"
    );
    assert!(
        delta.5 > received,
        "gross must exceed the payout whenever a fee is charged"
    );
}

#[test]
fn transfer_mode_seizure_delta_matches_the_share_burn_it_reports() {
    let t = unhealthy_account();
    let before_scaled = COLLATERAL_TOKENS * RAY;
    let (_, emitted) = t.liquidate(SeizeMode::Transfer);
    let delta = emitted.batches[0].seize_delta(&t.collateral);
    let (gross, fee, _) = t.withdraw_log().get(0).expect("one withdraw leg");

    // Field 3 is the position's post-state scaled amount. The shares burned
    // cover the gross amount, fee included: the fee is taken out of the
    // liquidated account's collateral, not out of the pool's own book.
    let burned = before_scaled - delta.3;
    let (expected_burn, expected_gross) = t.env.as_contract(&t.controller, || {
        resolve_withdrawal(
            &t.env,
            gross,
            Ray::from(before_scaled),
            Ray::from(RAY),
            DECIMALS,
        )
    });
    assert_eq!(burned, expected_burn.raw(), "shares burned cover the gross");
    assert_eq!(expected_gross, gross);
    assert!(
        Ray::from(burned).to_asset_floor(DECIMALS) >= fee,
        "the fee is carved out of shares the liquidated account gave up"
    );
}

// --- credit mode ---------------------------------------------------------

#[test]
fn credit_mode_debits_the_victim_gross_and_credits_the_receiver_net() {
    let t = unhealthy_account();
    let before = t.collateral_balance(&t.liquidator);

    let (receiver_id, emitted) = t.liquidate(SeizeMode::Credit(0));
    let batches = &emitted.batches;
    assert!(
        receiver_id != 0,
        "credit mode returns the receiving account"
    );

    assert_eq!(
        t.collateral_balance(&t.liquidator),
        before,
        "credit mode moves no collateral tokens at all"
    );
    assert!(
        t.withdraw_log().is_empty(),
        "credit mode must not route through the withdraw leg"
    );

    let fee_shares = t.seize_log().get(0).expect("one fee reclassification");
    assert!(
        fee_shares > 0,
        "fixture must produce a non-zero protocol fee"
    );

    assert_eq!(batches.len(), 2, "credit mode publishes two batches");
    assert_eq!(
        batches[0].account_id, t.account_id,
        "liquidated account's batch comes first"
    );
    assert_eq!(
        batches[1].account_id, receiver_id,
        "the receiving account's batch comes second"
    );

    let victim = batches[0].seize_delta(&t.collateral);
    let receiver = batches[1].credit_delta(&t.collateral);

    // Field 3 is each position's post-state scaled amount: the victim started
    // whole, the receiver started empty.
    let debited = COLLATERAL_TOKENS * RAY - victim.3;
    let credited = receiver.3;
    assert_eq!(
        debited,
        credited + fee_shares,
        "the shares the victim loses are the shares the receiver gains plus the \
         fee the pool reclassifies — nothing is created or destroyed"
    );

    // Same tag, two different senses: the victim's amount is the gross seizure,
    // the receiver's is what it actually received.
    assert!(
        victim.5 > receiver.5,
        "victim delta is gross, receiver delta is net of the fee"
    );
    assert_eq!(
        victim.5 - receiver.5,
        Ray::from(fee_shares).to_asset_floor(DECIMALS),
        "the gap between the two batches is exactly the protocol fee"
    );
}

#[test]
fn credit_mode_receiver_batch_identifies_the_new_account() {
    let t = unhealthy_account();
    let (receiver_id, emitted) = t.liquidate(SeizeMode::Credit(0));
    let receiver = &emitted.batches[1];
    assert_eq!(receiver.account_id, receiver_id);
    assert_eq!(
        receiver.deposits.len(),
        1,
        "the receiver's batch carries only the credited collateral"
    );
}

// --- repayment leg -------------------------------------------------------

#[test]
fn liquidation_event_reports_the_delivered_repayment_not_the_planned_one() {
    let t = unhealthy_account();
    // A debt token that keeps 1% of every transfer. `transfer_amount_measured`
    // exists precisely because such tokens are in scope, and the seizure is
    // scaled down to match — the headline event must be measured too.
    t.set_debt_shortfall_bps(100);

    let (_, emitted) = t.liquidate(SeizeMode::Transfer);

    let repay_legs: Vec<EventBorrowDelta> = emitted.batches[0].borrows.clone();
    assert_eq!(repay_legs.len(), 1, "one debt leg");
    let leg = repay_legs.get(0).unwrap();
    assert_eq!(leg.0, PositionAction::LiqRepay);

    // Both assets are priced at 1.0, so USD (WAD) and 7-decimal token units
    // differ only by scale: one token unit is 10^11 WAD of value.
    let applied_usd_wad = leg.5 * (common::constants::WAD / UNIT);

    // The event must not overstate the debt retired. Valuing the measured
    // receipt re-derives the USD figure through a floor-rounded ratio, so allow
    // a wei of slack — but nothing near the token's ~1% shortfall.
    let slack = applied_usd_wad / 1_000_000;
    assert!(
        emitted.repaid_usd_wad <= applied_usd_wad + slack,
        "the event must report the delivered repayment ({}), not the planned one; \
         the batch applied {}",
        emitted.repaid_usd_wad,
        applied_usd_wad
    );
    assert!(
        emitted.repaid_usd_wad + slack >= applied_usd_wad,
        "the event must not understate the delivered repayment either: event {} vs batch {}",
        emitted.repaid_usd_wad,
        applied_usd_wad
    );
}
