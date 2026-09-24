//! Policy: a collateral stamped with `liquidation_threshold == 0` must stay liquidatable.
//!
//! Failure shape guarded against: seizure requires a non-zero per-asset threshold, so a zeroed
//! collateral can never be seized and the debt can never be cleared.
//!
//! Seizure here is pro-rata over the account's whole collateral set and reads no per-asset
//! threshold, so a zero-threshold account liquidates at a zero bonus. These tests cover both
//! seize modes, the mixed-collateral case, and both bad-debt routes.
//!
//! `policy_configuration_cannot_stamp_a_zero_liquidation_threshold` pins reachability:
//! `validate_risk_bounds` requires `threshold > ltv` over `u32`, so no configuration entry point
//! writes a zero, and `apply_gated_liquidation_params` only copies a validated config value onto
//! a position. The fixtures write the position map directly; the tests check that no lock-out
//! exists, not a live configuration.
//!
//! The pool is a stub that reproduces the pool's scaled arithmetic
//! (`common::rates::resolve_repay` / `resolve_withdrawal`) and nothing else; the controller crate
//! does not depend on the pool contract. Cash-side behaviour lives in `tests/test-harness`.

use crate::constants::{
    DEFAULT_HF_FOR_MAX_BONUS_WAD, DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
    DEFAULT_LIQUIDATION_TARGET_HF_WAD, RAY, WAD,
};
use crate::{storage, Controller, ControllerClient};
use common::math::fp::Ray;
use common::rates::{resolve_repay, resolve_withdrawal};
use common::types::{
    AccountMeta, AccountPositionRaw, DebtPositionRaw, HubAssetKey, MarketIndexRaw, PoolAction,
    PoolPositionMutation, PoolSeizeEntry, PoolWithdrawEntry, PositionLimits, PositionMode,
    PriceFeedRaw, PriceKey, ScaledPositionRaw, SeizeMode, SpokeAssetConfig, SpokeConfig,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{contract, contractimpl, symbol_short, token, Address, Env, Map, Vec};

/// Every token in these fixtures is 7-decimal and priced at exactly one dollar, and every index
/// sits at one RAY, so `N * RAY` scaled units are `N` tokens are `N` dollars. That keeps the
/// fixtures readable in dollars while the code under test still runs its full
/// scaled -> asset -> USD conversion chain.
const DECIMALS: u32 = 7;
const ONE_TOKEN: i128 = 10_000_000;
const SPOKE: u32 = 1;
const HUB: u32 = 1;
const VICTIM: u64 = 1;

// --- stub externals ------------------------------------------------------

#[contract]
struct StubPool;

#[contractimpl]
impl StubPool {
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
            let scaled = Ray::from(action.position.scaled_amount);
            let (burned, excess) =
                resolve_repay(&env, action.amount, scaled, Ray::from(RAY), DECIMALS);
            out.push_back(PoolPositionMutation {
                position: ScaledPositionRaw {
                    scaled_amount: scaled.checked_sub(&env, burned).raw(),
                },
                market_index: unit_index(),
                actual_amount: action.amount - excess,
                asset_decimals: DECIMALS,
            });
        }
        out
    }

    pub fn withdraw(
        env: Env,
        _receiver: Address,
        _is_liquidation: bool,
        entries: Vec<PoolWithdrawEntry>,
    ) -> Vec<PoolPositionMutation> {
        let mut out = Vec::new(&env);
        for entry in entries.iter() {
            let scaled = Ray::from(entry.action.position.scaled_amount);
            let (burned, actual) =
                resolve_withdrawal(&env, entry.action.amount, scaled, Ray::from(RAY), DECIMALS);
            out.push_back(PoolPositionMutation {
                position: ScaledPositionRaw {
                    scaled_amount: scaled.checked_sub(&env, burned).raw(),
                },
                market_index: unit_index(),
                actual_amount: actual,
                asset_decimals: DECIMALS,
            });
        }
        out
    }

    /// Counts seized entries (at least one per call) so the bad-debt tests can prove the
    /// socialization leg reached the pool.
    pub fn seize_positions(env: Env, entries: Vec<PoolSeizeEntry>) {
        let key = symbol_short!("seizes");
        let seen: u32 = env.storage().instance().get(&key).unwrap_or(0);
        env.storage()
            .instance()
            .set(&key, &(seen + entries.len().max(1)));
    }

    pub fn seize_calls(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&symbol_short!("seizes"))
            .unwrap_or(0)
    }
}

#[contract]
struct StubOracle;

#[contractimpl]
impl StubOracle {
    pub fn prices(env: Env, keys: Vec<PriceKey>) -> Map<PriceKey, PriceFeedRaw> {
        let mut out = Map::new(&env);
        for key in keys.iter() {
            out.set(
                key,
                PriceFeedRaw {
                    price_wad: WAD,
                    asset_decimals: DECIMALS,
                    timestamp: 0,
                },
            );
        }
        out
    }
}

fn unit_index() -> MarketIndexRaw {
    MarketIndexRaw {
        borrow_index: RAY,
        supply_index: RAY,
    }
}

// --- fixture -------------------------------------------------------------

struct Fixture {
    env: Env,
    controller: Address,
    pool: Address,
    liquidator: Address,
    /// Collateral whose stamped `liquidation_threshold` is zero.
    zeroed: HubAssetKey,
    /// Collateral stamped with an ordinary threshold; present only in the mixed fixture.
    normal: HubAssetKey,
    debt: HubAssetKey,
}

impl Fixture {
    fn client(&self) -> ControllerClient<'_> {
        ControllerClient::new(&self.env, &self.controller)
    }

    fn supply_of(&self, account_id: u64, key: &HubAssetKey) -> Option<AccountPositionRaw> {
        self.client()
            .get_account_positions(&account_id)
            .0
            .get(key.clone())
    }

    fn debt_scaled(&self, account_id: u64) -> i128 {
        self.client()
            .get_account_positions(&account_id)
            .1
            .get(self.debt.clone())
            .map_or(0, |p| p.scaled_amount)
    }

    fn seize_calls(&self) -> u32 {
        StubPoolClient::new(&self.env, &self.pool).seize_calls()
    }
}

fn listed_asset(threshold: u32) -> SpokeAssetConfig {
    SpokeAssetConfig {
        is_collateralizable: true,
        is_borrowable: true,
        paused: false,
        frozen: false,
        no_seize: false,
        loan_to_value: 7_500,
        liquidation_threshold: threshold,
        liquidation_bonus: 500,
        liquidation_fees: 100,
        supply_cap: 0,
        borrow_cap: 0,
    }
}

fn stamped(scaled_amount: i128, liquidation_threshold: u32) -> AccountPositionRaw {
    AccountPositionRaw {
        scaled_amount,
        liquidation_threshold,
        liquidation_bonus: 500,
        loan_to_value: 7_500,
        liquidation_fees: 100,
    }
}

/// Seeds one account holding `zeroed_scaled` of a zero-threshold collateral, optionally
/// `normal_scaled` of an ordinary 80% collateral, and `debt_scaled` of debt.
///
/// The supply map is written straight to storage: `validate_risk_bounds` rejects a zero
/// threshold at every configuration entry point, so no supported call sequence produces this
/// state.
fn seed(zeroed_scaled: i128, normal_scaled: i128, debt_scaled: i128) -> Fixture {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let controller = env.register(Controller, (admin.clone(),));
    ControllerClient::new(&env, &controller).unpause();

    let pool = env.register(StubPool, ());
    let oracle = env.register(StubOracle, ());

    let token_for = || {
        env.register_stellar_asset_contract_v2(admin.clone())
            .address()
    };
    let zeroed = HubAssetKey {
        hub_id: HUB,
        asset: token_for(),
    };
    let normal = HubAssetKey {
        hub_id: HUB,
        asset: token_for(),
    };
    let debt = HubAssetKey {
        hub_id: HUB,
        asset: token_for(),
    };

    let owner = Address::generate(&env);
    let liquidator = Address::generate(&env);
    // Funds the repayment leg; the seizure legs move no tokens through this stub pool.
    token::StellarAssetClient::new(&env, &debt.asset).mint(&liquidator, &(1_000 * ONE_TOKEN));

    let nft = env.register(
        position_nft::PositionNft,
        (
            controller.clone(),
            soroban_sdk::String::from_str(&env, "uri"),
            soroban_sdk::String::from_str(&env, "Position"),
            soroban_sdk::String::from_str(&env, "POS"),
        ),
    );
    // Mint the victim's NFT first so a `Credit(0)` receiver account gets a different id.
    let victim_id = u64::from(position_nft::PositionNftClient::new(&env, &nft).mint(&owner));
    assert_eq!(
        victim_id, VICTIM,
        "sanity: the victim is the first token minted"
    );

    env.as_contract(&controller, || {
        storage::set_position_nft(&env, &nft);
        storage::set_pool(&env, &pool);
        storage::set_price_aggregator(&env, &oracle);
        storage::set_position_limits(
            &env,
            &PositionLimits {
                max_supply_positions: 8,
                max_borrow_positions: 8,
            },
        );
        storage::set_spoke(
            &env,
            SPOKE,
            &SpokeConfig {
                is_deprecated: false,
                liquidation_target_hf_wad: DEFAULT_LIQUIDATION_TARGET_HF_WAD,
                hf_for_max_bonus_wad: DEFAULT_HF_FOR_MAX_BONUS_WAD,
                liquidation_bonus_factor_bps: DEFAULT_LIQUIDATION_BONUS_FACTOR_BPS,
            },
        );
        // The listings carry ordinary thresholds; only the stamped positions are zeroed. The
        // receiver-side tuple assertion depends on this.
        storage::set_spoke_asset(&env, SPOKE, &zeroed, &listed_asset(8_000));
        storage::set_spoke_asset(&env, SPOKE, &normal, &listed_asset(8_000));
        storage::set_spoke_asset(&env, SPOKE, &debt, &listed_asset(8_000));

        storage::set_account_meta(
            &env,
            VICTIM,
            &AccountMeta {
                spoke_id: SPOKE,
                mode: PositionMode::Normal,
            },
        );
        let mut supply = Map::new(&env);
        supply.set(zeroed.clone(), stamped(zeroed_scaled, 0));
        if normal_scaled > 0 {
            supply.set(normal.clone(), stamped(normal_scaled, 8_000));
        }
        storage::set_supply_positions(&env, VICTIM, &supply);
        let mut debts = Map::new(&env);
        debts.set(
            debt.clone(),
            DebtPositionRaw {
                scaled_amount: debt_scaled,
            },
        );
        storage::set_debt_positions(&env, VICTIM, &debts);
    });

    Fixture {
        env,
        controller,
        pool,
        liquidator,
        zeroed,
        normal,
        debt,
    }
}

fn payment(fx: &Fixture, tokens: i128) -> Vec<(HubAssetKey, i128)> {
    soroban_sdk::vec![&fx.env, (fx.debt.clone(), tokens * ONE_TOKEN)]
}

// --- the account is liquidatable at all ----------------------------------

#[test]
fn policy_zero_threshold_account_is_liquidatable_not_locked() {
    // $100 of zero-threshold collateral against $90 of debt. Weighted collateral is zero, so
    // the health factor is zero; the estimate must still price the liquidation.
    let fx = seed(100 * RAY, 0, 90 * RAY);
    let client = fx.client();

    assert!(client.is_liquidatable(&VICTIM));
    assert_eq!(
        client.get_health_factor(&VICTIM),
        0,
        "zero weighted collateral means a zero health factor"
    );

    let estimate =
        client.get_liquidation_estimate(&VICTIM, &payment(&fx, 30), &SeizeMode::Transfer);
    assert_eq!(
        estimate.bonus_rate_bps, 0,
        "a zero-threshold account supports no bonus at all"
    );
    assert!(
        estimate.max_payment_wad > 0,
        "the estimate must still size a repayment, not collapse to nothing"
    );
    assert_eq!(
        estimate.seized_collaterals.len(),
        1,
        "the zero-threshold collateral must appear in the seizure plan"
    );
}

#[test]
fn policy_zero_threshold_collateral_liquidates_end_to_end_in_transfer_mode() {
    let fx = seed(100 * RAY, 0, 90 * RAY);
    let client = fx.client();

    let receiver = client.liquidate(
        &fx.liquidator,
        &VICTIM,
        &payment(&fx, 30),
        &SeizeMode::Transfer,
    );

    assert_eq!(receiver, 0, "transfer mode credits no account");
    assert_eq!(
        fx.debt_scaled(VICTIM),
        60 * RAY,
        "$30 of the $90 debt must be gone"
    );
    let left = fx
        .supply_of(VICTIM, &fx.zeroed)
        .expect("a partial seizure must leave a position behind");
    assert_eq!(
        left.scaled_amount,
        70 * RAY,
        "$30 of collateral seized at a zero bonus, no more and no less"
    );
    assert_eq!(
        left.liquidation_threshold, 0,
        "a liquidation seizure must not restamp the surviving position"
    );
}

#[test]
fn policy_zero_threshold_collateral_liquidates_end_to_end_in_credit_mode() {
    let fx = seed(100 * RAY, 0, 90 * RAY);
    let client = fx.client();

    let receiver = client.liquidate(
        &fx.liquidator,
        &VICTIM,
        &payment(&fx, 30),
        &SeizeMode::Credit(0),
    );

    assert!(receiver > VICTIM, "Credit(0) must open a fresh account");
    assert_eq!(fx.debt_scaled(VICTIM), 60 * RAY, "the debt must be reduced");
    assert_eq!(
        fx.supply_of(VICTIM, &fx.zeroed)
            .expect("the victim keeps the remainder")
            .scaled_amount,
        70 * RAY
    );

    let credited = fx
        .supply_of(receiver, &fx.zeroed)
        .expect("the seized shares must land on the receiving account");
    assert_eq!(
        credited.scaled_amount,
        30 * RAY,
        "a zero bonus leaves no fee base, so the liquidator takes the whole seizure"
    );
    assert_eq!(
        credited.liquidation_threshold, 8_000,
        "the receiver stamps the current listing, never the victim's zeroed tuple"
    );
    assert_eq!(
        fx.seize_calls(),
        0,
        "a zero bonus books no protocol fee, so credit mode touches the pool not at all"
    );
}

// --- the zero-threshold leg is not skipped -------------------------------

#[test]
fn policy_zero_threshold_collateral_is_seized_pro_rata_beside_a_normal_one() {
    // One collateral at a zero threshold, one at 80%. A per-asset threshold gate would leave
    // the zeroed leg unseizable. Pro-rata seizure must take both, in proportion to value.
    let fx = seed(50 * RAY, 50 * RAY, 60 * RAY);
    let client = fx.client();

    client.liquidate(
        &fx.liquidator,
        &VICTIM,
        &payment(&fx, 30),
        &SeizeMode::Transfer,
    );

    let zeroed_left = fx
        .supply_of(VICTIM, &fx.zeroed)
        .expect("the zero-threshold leg must survive a partial seizure")
        .scaled_amount;
    let normal_left = fx
        .supply_of(VICTIM, &fx.normal)
        .expect("the ordinary leg must survive a partial seizure")
        .scaled_amount;

    assert!(
        zeroed_left < 50 * RAY,
        "the zero-threshold collateral was not seized at all: {zeroed_left}"
    );
    assert_eq!(
        zeroed_left, normal_left,
        "equal-value legs must be seized equally; the threshold is not an input to the split"
    );
    assert_eq!(fx.debt_scaled(VICTIM), 30 * RAY, "the debt must be reduced");
}

// --- bad debt ------------------------------------------------------------

#[test]
fn policy_zero_threshold_account_is_still_socializable_as_bad_debt() {
    // $3 of collateral under $10 of debt: insolvent and at or below the $5 dust cap. The gate
    // reads only the two USD totals, so a zeroed threshold does not exempt the account from
    // cleanup.
    let fx = seed(3 * RAY, 0, 10 * RAY);
    let client = fx.client();

    client.clean_bad_debt(&Address::generate(&fx.env), &VICTIM);

    assert!(
        !client.account_exists(&VICTIM),
        "socialization must remove the account entry"
    );
    assert!(
        fx.seize_calls() > 0,
        "the residual positions must actually reach the pool's seize path"
    );
}

#[test]
fn policy_zero_threshold_liquidation_promotes_its_own_residual_to_bad_debt() {
    // $6 of collateral under $50 of debt. Repaying $6 consumes the whole collateral at a zero
    // bonus and leaves $44 of debt against nothing; the in-call bad-debt check must socialize it.
    let fx = seed(6 * RAY, 0, 50 * RAY);
    let client = fx.client();

    client.liquidate(
        &fx.liquidator,
        &VICTIM,
        &payment(&fx, 6),
        &SeizeMode::Transfer,
    );

    assert!(
        !client.account_exists(&VICTIM),
        "a fully-drained zero-threshold account must be socialized, not left behind"
    );
    assert!(fx.seize_calls() > 0, "bad-debt cleanup must reach the pool");
}

// --- reachability --------------------------------------------------------

#[test]
#[should_panic(expected = "#113")]
fn policy_configuration_cannot_stamp_a_zero_liquidation_threshold() {
    // `threshold > ltv` over `u32` rejects a zero threshold even at `ltv == 0`.
    let env = Env::default();
    common::validation::validate_risk_bounds(&env, 0, 0, 0);
}
