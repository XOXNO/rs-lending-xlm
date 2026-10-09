//! RV break pass: DeFindex adapter mapping lifecycle (dust, stale amounts,
//! donation front-runs). Every assertion encodes the SAFE expectation: a
//! failure would mean the attack works.

extern crate std;

use defindex_strategy::{DataKey, DeFindexStrategyError, Strategy, StrategyClient};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Error, IntoVal, InvokeError, Val, Vec, I256};
use test_harness::{
    eth_preset, hub_asset, usdc_preset, LendingTest, ALICE, BOB, HARNESS_HUB, HARNESS_SPOKE,
};

const UNIT: i128 = 10_000_000;
const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;
const DAY: u64 = 60 * 60 * 24;

fn flatten<T>(
    result: Result<Result<T, Error>, Result<DeFindexStrategyError, InvokeError>>,
) -> Result<T, Error> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(err),
        Err(Ok(err)) => Err(Error::from(&err)),
        Err(Err(invoke)) => panic!("expected contract error, got InvokeError: {invoke:?}"),
    }
}

struct Fx {
    t: LendingTest,
    strategy: Address,
    vault: Address,
    asset: Address,
}

impl Fx {
    fn new() -> Self {
        let mut t = LendingTest::new()
            .with_market(usdc_preset())
            .with_market(eth_preset())
            .build();
        t.supply(ALICE, "USDC", 10_000.0);
        t.supply(BOB, "ETH", 100.0);
        t.borrow(BOB, "USDC", 4_000.0);

        let asset = t.resolve_asset("USDC");
        let init_args: Vec<Val> = vec![
            &t.env,
            t.controller.clone().into_val(&t.env),
            HARNESS_HUB.into_val(&t.env),
            HARNESS_SPOKE.into_val(&t.env),
        ];
        let strategy = t.env.register(Strategy, (asset.clone(), init_args));
        let vault = Address::generate(&t.env);
        t.resolve_market("USDC")
            .token_admin
            .mint(&vault, &(100_000 * UNIT));
        Self {
            t,
            strategy,
            vault,
            asset,
        }
    }

    fn client(&self) -> StrategyClient<'_> {
        StrategyClient::new(&self.t.env, &self.strategy)
    }

    fn usdc(&self, of: &Address) -> i128 {
        token::Client::new(&self.t.env, &self.asset).balance(of)
    }

    fn stored_id(&self) -> u64 {
        let env = &self.t.env;
        env.as_contract(&self.strategy, || {
            env.storage()
                .persistent()
                .get(&DataKey::VaultAccount(self.vault.clone()))
                .unwrap_or(0)
        })
    }

    fn scaled_shares(&self, account_id: u64) -> i128 {
        let (supply, _) = self.t.ctrl_client().get_account_positions(&account_id);
        supply
            .get(hub_asset(self.asset.clone()))
            .map(|p| p.scaled_amount)
            .unwrap_or(0)
    }

    fn supply_index(&self) -> i128 {
        self.t
            .ctrl_client()
            .get_market_index(&hub_asset(self.asset.clone()))
            .supply_index
    }
}

/// D-2: withdrawing `balance - 1` leaves exactly one displayable unit, and the
/// follow-up `withdraw(1)` is a terminal close that pays floor(value), so the
/// vault loses at most one base unit (1e-7 USDC) and the account never strands.
#[test]
fn d2_partial_withdraw_by_one_unit_leaves_one_unit_and_then_closes() {
    let mut fx = Fx::new();
    let sink = Address::generate(&fx.t.env);
    fx.client().deposit(&(1_000 * UNIT), &fx.vault);
    fx.t.advance_time(37 * DAY);

    let balance = fx.client().balance(&fx.vault);
    let index = fx.supply_index();
    assert!(index > RAY, "need a non-unit index so rounding matters");

    let remaining = fx.client().withdraw(&(balance - 1), &fx.vault, &sink);
    let account_id = fx.stored_id();
    assert_eq!(remaining, 1, "leftover must display as exactly one unit");
    assert_eq!(fx.client().balance(&fx.vault), 1);
    assert!(
        fx.t.account_exists(account_id),
        "account must survive the partial"
    );
    assert_eq!(fx.usdc(&sink), balance - 1);

    let leftover_shares = fx.scaled_shares(account_id);
    // Exact value of the dust in RAY-asset units (27 dec): shares * index / RAY.
    let env = &fx.t.env;
    let leftover_value_ray = I256::from_i128(env, leftover_shares)
        .mul(&I256::from_i128(env, index))
        .div(&I256::from_i128(env, RAY))
        .to_i128()
        .expect("fits");
    let floor_units = leftover_value_ray / 100_000_000_000_000_000_000; // 1e20 per 7-dec unit
    std::println!(
        "D-2: index={index} leftover_shares={leftover_shares} leftover_value_ray={leftover_value_ray} floor_units={floor_units}"
    );
    assert!(floor_units == 0 || floor_units == 1);

    // Terminal close: amount == balance (1) -> controller withdraw-all.
    let left = fx.client().withdraw(&1, &fx.vault, &sink);
    assert_eq!(left, 0);
    assert_eq!(
        fx.stored_id(),
        0,
        "mapping must be cleared on terminal close"
    );
    assert!(!fx.t.account_exists(account_id), "account must be removed");
    let paid_total = fx.usdc(&sink);
    assert!(
        paid_total == balance - 1 || paid_total == balance,
        "terminal close pays floor: got {paid_total}, balance was {balance}"
    );
    assert_eq!(paid_total - (balance - 1), floor_units);
    assert_eq!(fx.usdc(&fx.strategy), 0, "nothing strands on the adapter");
}

/// D-3: `balance()` read in one tx, `withdraw(that)` in a later tx after
/// accrual, turns the intended full exit into a partial: the account survives
/// with the accrued interest and stays reachable through the mapping.
#[test]
fn d3_stale_full_amount_after_accrual_is_a_partial_that_stays_reachable() {
    let mut fx = Fx::new();
    let sink = Address::generate(&fx.t.env);
    fx.client().deposit(&(1_000 * UNIT), &fx.vault);
    fx.t.advance_time(30 * DAY);
    let stale = fx.client().balance(&fx.vault);
    fx.t.advance_time(DAY);
    let fresh = fx.client().balance(&fx.vault);
    assert!(fresh > stale, "interest must accrue between the two reads");

    let remaining = fx.client().withdraw(&stale, &fx.vault, &sink);
    let account_id = fx.stored_id();
    assert_eq!(remaining, fresh - stale);
    assert!(account_id != 0 && fx.t.account_exists(account_id));
    assert_eq!(fx.usdc(&sink), stale);

    let left = fx.client().withdraw(&remaining, &fx.vault, &sink);
    assert_eq!(left, 0);
    assert_eq!(fx.stored_id(), 0);
    let paid = fx.usdc(&sink);
    assert!(
        paid == fresh || paid == fresh - 1,
        "paid {paid}, fresh {fresh}"
    );
}

/// D-3 (donation front-run): a 1-unit third-party top-up before `withdraw(B)`
/// makes it partial; the donated unit stays withdrawable, nothing is lost.
#[test]
fn d3_donation_front_run_leaves_the_donation_withdrawable() {
    let mut fx = Fx::new();
    let sink = Address::generate(&fx.t.env);
    fx.client().deposit(&(1_000 * UNIT), &fx.vault);
    fx.t.advance_time(30 * DAY);
    let b = fx.client().balance(&fx.vault);
    let account_id = fx.stored_id();

    let attacker = Address::generate(&fx.t.env);
    fx.t.resolve_market("USDC")
        .token_admin
        .mint(&attacker, &UNIT);
    fx.t.ctrl_client().supply(
        &attacker,
        &account_id,
        &HARNESS_SPOKE,
        &vec![&fx.t.env, (hub_asset(fx.asset.clone()), 1)],
    );
    assert_eq!(fx.client().balance(&fx.vault), b + 1);

    let remaining = fx.client().withdraw(&b, &fx.vault, &sink);
    assert_eq!(remaining, 1);
    assert_eq!(
        fx.stored_id(),
        account_id,
        "mapping must not be cleared on a partial"
    );
    assert!(fx.t.account_exists(account_id));
    assert_eq!(fx.usdc(&sink), b);

    let left = fx.client().withdraw(&1, &fx.vault, &sink);
    assert_eq!(left, 0);
    assert_eq!(fx.stored_id(), 0);
    assert!(!fx.t.account_exists(account_id));
    let paid = fx.usdc(&sink);
    assert!(paid == b || paid == b + 1, "paid {paid}, b {b}");
}

/// D-2 (zero-balance dust): an account whose shares display as 0 units cannot be
/// withdrawn (InsufficientBalance) but stays mapped, and a deposit + terminal
/// close recovers it. Reached here by withdrawing all but one unit and then
/// relying on the half-up display: a 1-unit account is the smallest reachable
/// state without index socialization, so we only assert the recovery path.
#[test]
fn d2_dust_account_is_recovered_by_deposit_then_terminal_close() {
    let mut fx = Fx::new();
    let sink = Address::generate(&fx.t.env);
    fx.client().deposit(&(1_000 * UNIT), &fx.vault);
    fx.t.advance_time(37 * DAY);
    let balance = fx.client().balance(&fx.vault);
    fx.client().withdraw(&(balance - 1), &fx.vault, &sink);
    let account_id = fx.stored_id();
    assert_eq!(fx.client().balance(&fx.vault), 1);

    // Over-balance requests fail closed and change nothing.
    let err = flatten(fx.client().try_withdraw(&2, &fx.vault, &sink));
    assert_eq!(
        err.unwrap_err(),
        Error::from_contract_error(DeFindexStrategyError::InsufficientBalance as u32)
    );
    assert_eq!(fx.stored_id(), account_id);

    // Deposit tops up the SAME account (mapping reused), then a terminal close.
    let after = fx.client().deposit(&(10 * UNIT), &fx.vault);
    assert_eq!(
        fx.stored_id(),
        account_id,
        "deposit must reuse the mapped account"
    );
    assert_eq!(after, 10 * UNIT + 1);
    let left = fx.client().withdraw(&after, &fx.vault, &sink);
    assert_eq!(left, 0);
    assert_eq!(fx.stored_id(), 0);
    let paid = fx.usdc(&sink);
    assert!(
        paid == balance - 1 + 10 * UNIT || paid == balance + 10 * UNIT,
        "paid {paid}, balance {balance}"
    );
}
