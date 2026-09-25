//! Experiment: a 0-decimal, collateral-only market (Liqvid-shaped: one raw unit
//! is one deal share worth $1..$1,000) in its own hub and spoke, borrowing
//! 7-decimal USDC from the base hub. Requires `MIN_ASSET_DECIMALS = 0`.
//!
//! Tests prefixed `zdc_`. Tables print with `--nocapture`.

use common::types::{HubAssetKey, InterestRateModel, LiquidationEstimate, SeizeMode};
use common::validation::max_cap_for_decimals;
use controller::constants::{RAY, WAD};
use governance::op::{AdminOperation, CreatePoolArgs, SpokeAssetArgs, UpgradePoolParamsArgs};
use soroban_sdk::{token, vec, Address, Error, TryFromVal, Vec};
use test_harness::{
    errors, usd, usd_cents, usdc_preset, xlm_preset, AssetConfigPreset, LendingTest, MarketPreset,
    DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, HARNESS_HUB,
};

const LIQ: &str = "LIQVID";
const LTV: u32 = 6_000;
const LT: u32 = 7_000;
const BONUS: u32 = 500;
const LIQ_CAP_UNITS: i128 = 10_000_000;
const USDC_UNIT: i128 = 10_000_000;
const USDC_CAP_RAW: i128 = 1_000_000_000 * USDC_UNIT;

struct Zdc {
    t: LendingTest,
    hub: u32,
    spoke: u32,
    liq: Address,
    usdc: Address,
}

fn setup(nav_wad: i128) -> Zdc {
    let mut t = LendingTest::new()
        .with_market(MarketPreset {
            initial_liquidity: 50_000_000.0,
            ..usdc_preset()
        })
        .with_market(xlm_preset())
        .with_market(MarketPreset {
            name: LIQ,
            decimals: 0,
            price_wad: nav_wad,
            initial_liquidity: 0.0,
            config: AssetConfigPreset {
                is_collateralizable: false,
                is_borrowable: false,
                is_flashloanable: false,
                flashloan_fee: 0,
                liquidation_fees: 0,
                ..DEFAULT_ASSET_CONFIG
            },
            params: DEFAULT_MARKET_PARAMS,
        })
        .build();

    let admin = t.admin();
    let liq = t.resolve_asset(LIQ);
    let usdc = t.resolve_asset("USDC");
    let hub = t.create_hub();
    let gov = t.gov_client();
    gov.execute_immediate(
        &admin,
        &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
            hub_id: hub,
            asset: liq.clone(),
            params: DEFAULT_MARKET_PARAMS.to_market_params(&liq, 0),
        }),
    );
    let spoke_val = gov.execute_immediate(&admin, &AdminOperation::AddSpoke);
    let spoke = u32::try_from_val(&t.env, &spoke_val).unwrap();
    gov.execute_immediate(
        &admin,
        &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
            hub_id: hub,
            asset: liq.clone(),
            spoke_id: spoke,
            can_collateral: true,
            can_borrow: false,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: LTV,
            threshold: LT,
            bonus: BONUS,
            liquidation_fees: 0,
            supply_cap: LIQ_CAP_UNITS,
            borrow_cap: 0,
        }),
    );
    gov.execute_immediate(
        &admin,
        &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
            hub_id: HARNESS_HUB,
            asset: usdc.clone(),
            spoke_id: spoke,
            can_collateral: false,
            can_borrow: true,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: 7_500,
            threshold: 8_000,
            bonus: 500,
            liquidation_fees: 0,
            supply_cap: USDC_CAP_RAW,
            borrow_cap: USDC_CAP_RAW,
        }),
    );
    t.get_or_create_user("liquidator");
    Zdc {
        t,
        hub,
        spoke,
        liq,
        usdc,
    }
}

impl Zdc {
    fn liq_key(&self) -> HubAssetKey {
        HubAssetKey {
            hub_id: self.hub,
            asset: self.liq.clone(),
        }
    }

    fn usdc_key(&self) -> HubAssetKey {
        HubAssetKey {
            hub_id: HARNESS_HUB,
            asset: self.usdc.clone(),
        }
    }

    fn user(&mut self, name: &str) -> Address {
        self.t.get_or_create_user(name)
    }

    fn liq_balance(&self, who: &Address) -> i128 {
        token::Client::new(&self.t.env, &self.liq).balance(who)
    }

    fn usdc_balance(&self, who: &Address) -> i128 {
        token::Client::new(&self.t.env, &self.usdc).balance(who)
    }

    fn supply(&mut self, name: &str, account_id: u64, units: i128) -> u64 {
        let who = self.user(name);
        self.t.resolve_market(LIQ).token_admin.mint(&who, &units);
        self.t.ctrl_client().supply(
            &who,
            &account_id,
            &self.spoke,
            &vec![&self.t.env, (self.liq_key(), units)],
        )
    }

    fn borrow(&mut self, name: &str, account_id: u64, usdc_raw: i128) {
        let who = self.user(name);
        self.t.ctrl_client().borrow(
            &who,
            &account_id,
            &vec![&self.t.env, (self.usdc_key(), usdc_raw)],
            &None,
        );
    }

    fn try_withdraw(&mut self, name: &str, account_id: u64, units: i128) -> Result<i128, Error> {
        let who = self.user(name);
        let before = self.liq_balance(&who);
        match self.t.ctrl_client().try_withdraw(
            &who,
            &account_id,
            &vec![&self.t.env, (self.liq_key(), units)],
            &None,
        ) {
            Ok(Ok(_)) => Ok(self.liq_balance(&who) - before),
            Ok(Err(e)) => Err(e.into()),
            Err(e) => Err(e.expect("contract error")),
        }
    }

    fn repay_all(&mut self, name: &str, account_id: u64) {
        let who = self.user(name);
        let owed = self.debt_raw(account_id) + 10;
        self.t.resolve_market("USDC").token_admin.mint(&who, &owed);
        self.t.ctrl_client().repay(
            &who,
            &account_id,
            &vec![&self.t.env, (self.usdc_key(), owed)],
        );
    }

    fn collateral_units(&self, account_id: u64) -> i128 {
        self.t
            .ctrl_client()
            .get_collateral_amount(&account_id, &self.liq_key())
    }

    fn collateral_scaled(&self, account_id: u64) -> i128 {
        let (supplies, _) = self.t.ctrl_client().get_account_positions(&account_id);
        supplies
            .get(self.liq_key())
            .map(|p| p.scaled_amount)
            .unwrap_or(0)
    }

    fn debt_raw(&self, account_id: u64) -> i128 {
        self.t
            .ctrl_client()
            .get_borrow_amount(&account_id, &self.usdc_key())
    }

    fn hf(&self, account_id: u64) -> i128 {
        self.t.ctrl_client().get_health_factor(&account_id)
    }

    fn exists(&self, account_id: u64) -> bool {
        self.t.ctrl_client().account_exists(&account_id)
    }

    fn estimate(&self, account_id: u64, usdc_raw: i128, mode: SeizeMode) -> LiquidationEstimate {
        self.t.ctrl_client().get_liquidation_estimate(
            &account_id,
            &vec![&self.t.env, (self.usdc_key(), usdc_raw)],
            &mode,
        )
    }

    fn liquidate(
        &mut self,
        account_id: u64,
        usdc_raw: i128,
        mode: SeizeMode,
    ) -> Result<LiqOutcome, Error> {
        let liquidator = self.user("liquidator");
        self.t
            .resolve_market("USDC")
            .token_admin
            .mint(&liquidator, &usdc_raw);
        let usdc_before = self.usdc_balance(&liquidator);
        let liq_before = self.liq_balance(&liquidator);
        let units_before = self.collateral_units(account_id);
        let debt_before = self.debt_raw(account_id);
        let res = self.t.ctrl_client().try_liquidate(
            &liquidator,
            &account_id,
            &vec![&self.t.env, (self.usdc_key(), usdc_raw)],
            &mode,
        );
        let receiver = match res {
            Ok(Ok(id)) => id,
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(e.expect("contract error")),
        };
        let exists = self.exists(account_id);
        Ok(LiqOutcome {
            paid_usdc_raw: usdc_before - self.usdc_balance(&liquidator),
            got_units: self.liq_balance(&liquidator) - liq_before,
            receiver,
            units_before,
            units_after: if exists {
                self.collateral_units(account_id)
            } else {
                0
            },
            debt_before,
            debt_after: if exists { self.debt_raw(account_id) } else { 0 },
            account_exists: exists,
        })
    }

    fn supply_index(&self, key: HubAssetKey) -> i128 {
        self.t
            .ctrl_client()
            .get_market_indexes_detailed(&Vec::from_array(&self.t.env, [key]))
            .get(0)
            .unwrap()
            .supply_index
    }

    fn hub_liq_cash(&self) -> i128 {
        self.t.pool_state_on_hub(self.hub, LIQ).cash
    }
}

#[derive(Debug)]
struct LiqOutcome {
    paid_usdc_raw: i128,
    got_units: i128,
    receiver: u64,
    units_before: i128,
    units_after: i128,
    debt_before: i128,
    debt_after: i128,
    account_exists: bool,
}

fn usdc_raw(dollars: i128) -> i128 {
    dollars * USDC_UNIT
}

fn nav(dollars: i128) -> i128 {
    usd(dollars)
}

/// Opens `units` of collateral at `nav_usd` and borrows `borrow_bps` of its value.
fn open(z: &mut Zdc, name: &str, units: i128, nav_usd: i128, borrow_bps: i128) -> u64 {
    let id = z.supply(name, 0, units);
    let borrow = usdc_raw(units * nav_usd) * borrow_bps / 10_000;
    z.borrow(name, id, borrow);
    id
}

#[test]
fn zdc_listing_admits_zero_decimals_and_supply_index_stays_ray() {
    let mut z = setup(nav(1_000));
    let id = open(&mut z, "alice", 100, 1_000, 5_000);
    assert_eq!(z.collateral_units(id), 100);
    assert_eq!(z.collateral_scaled(id), 100 * RAY);
    z.t.advance_and_sync(365 * 86_400);
    assert_eq!(
        z.supply_index(z.liq_key()),
        RAY,
        "a collateral-only market has no borrows, so its supply index never moves"
    );
    assert_eq!(z.collateral_units(id), 100);
}

#[test]
fn zdc_round_trip_supply_borrow_withdraw_repay_is_exact_at_nav_1000() {
    let mut z = setup(nav(1_000));
    let id = open(&mut z, "alice", 100, 1_000, 5_500);
    assert_eq!(z.hub_liq_cash(), 100);

    assert_eq!(z.try_withdraw("alice", id, 5), Ok(5));
    assert_eq!(z.collateral_units(id), 95);

    let err = z.try_withdraw("alice", id, 10).unwrap_err();
    assert_eq!(
        err,
        Error::from_contract_error(errors::INSUFFICIENT_COLLATERAL),
        "withdrawing 10 of 95 units at $55k debt must breach LTV"
    );

    z.t.advance_and_sync(30 * 86_400);
    z.repay_all("alice", id);
    assert_eq!(z.debt_raw(id), 0);
    assert_eq!(z.try_withdraw("alice", id, 95), Ok(95));
    let alice = z.user("alice");
    assert_eq!(z.liq_balance(&alice), 100, "every unit comes back");
    assert_eq!(z.hub_liq_cash(), 0, "no unit is stranded in the pool");
}

#[test]
fn zdc_one_and_two_unit_positions_round_trip() {
    let mut z = setup(nav(1_000));
    let one = z.supply("alice", 0, 1);
    let two = z.supply("bob", 0, 2);
    assert_eq!(z.collateral_units(one), 1);
    assert_eq!(z.collateral_units(two), 2);
    assert_eq!(z.try_withdraw("alice", one, 1), Ok(1));
    assert_eq!(z.try_withdraw("bob", two, 1), Ok(1));
    assert_eq!(z.try_withdraw("bob", two, 1), Ok(1));
    assert_eq!(z.hub_liq_cash(), 0);
}
/// Opens N units at 58% LTV, drops NAV 20% (HF ~0.966), liquidates in
/// `Transfer` mode with a debt-sized payment and prints the outcome.
fn sweep(nav_cents: i128, sizes: &[i128]) -> std::vec::Vec<(i128, Option<LiqOutcome>)> {
    let mut rows = std::vec::Vec::new();
    let new_cents = nav_cents * 8 / 10;
    std::println!(
        "\nNAV ${:.2}/unit, drop to ${:.2}",
        nav_cents as f64 / 100.0,
        new_cents as f64 / 100.0
    );
    std::println!(
        "{:>7} | {:>12} | {:>12} | {:>6} | {:>10} | {:>12} | {:>7} | {:>7}",
        "units",
        "debt $",
        "paid $",
        "got u",
        "got $",
        "liq pnl $",
        "u left",
        "exists"
    );
    for &n in sizes {
        let mut z = setup(usd_cents(nav_cents));
        let id = z.supply("alice", 0, n);
        z.borrow(
            "alice",
            id,
            n * nav_cents * USDC_UNIT / 100 * 5_800 / 10_000,
        );
        z.t.set_price(LIQ, usd_cents(new_cents));
        assert!(z.hf(id) < WAD);
        let debt = z.debt_raw(id);
        let out = match z.liquidate(id, debt + usdc_raw(1), SeizeMode::Transfer) {
            Ok(out) => out,
            Err(e) => {
                assert_eq!(e, Error::from_contract_error(errors::INVALID_PAYMENTS));
                std::println!("{n:>7} | reverted: no whole unit is seizable");
                rows.push((n, None));
                continue;
            }
        };
        let got_usd = out.got_units as f64 * new_cents as f64 / 100.0;
        let paid_usd = out.paid_usdc_raw as f64 / USDC_UNIT as f64;
        std::println!(
            "{:>7} | {:>12.2} | {:>12.2} | {:>6} | {:>10.2} | {:>12.2} | {:>7} | {:>7}",
            n,
            debt as f64 / USDC_UNIT as f64,
            paid_usd,
            out.got_units,
            got_usd,
            got_usd - paid_usd,
            out.units_after,
            out.account_exists
        );
        rows.push((n, Some(out)));
    }
    rows
}

#[test]
fn zdc_transfer_liquidation_sweep_nav_1000() {
    let rows = sweep(100_000, &[10, 20, 50, 100, 1_000]);
    for (n, out) in rows.iter().filter_map(|(n, o)| o.as_ref().map(|o| (n, o))) {
        assert!(out.got_units > 0, "{n} units: every settled call seizes");
        assert_eq!(
            out.units_before,
            out.units_after + out.got_units,
            "{n} units: collateral conserved (fees 0)"
        );
    }
}

#[test]
fn zdc_transfer_liquidation_sweep_nav_1() {
    let rows = sweep(100, &[10, 20, 50, 100, 1_000, 10_000]);
    for (n, out) in rows.iter().filter_map(|(n, o)| o.as_ref().map(|o| (n, o))) {
        assert_eq!(out.units_before, out.units_after + out.got_units, "{n}");
    }
}

/// A band quote (D <= C < D * (1 + bonus)) closes the whole debt in one call:
/// the last fractional unit rounds up instead of stranding an unliquidatable unit.
#[test]
fn zdc_band_full_close_leaves_no_unliquidatable_unit() {
    let mut z = setup(nav(1_000));
    let id = open(&mut z, "alice", 10, 1_000, 5_800);
    z.t.set_price(LIQ, nav(600));
    assert!(z.hf(id) < WAD);
    let debt = z.debt_raw(id);
    let out = z.liquidate(id, debt + 1, SeizeMode::Transfer).unwrap();
    assert_eq!(out.debt_after, 0, "{out:?}");
    assert_eq!(out.got_units, 10);
    assert!(out.paid_usdc_raw >= debt);
}

/// On a band quote, an offer below the whole debt still pays for every unit
/// it receives: only a call that repays all debt rounds a unit up.
#[test]
fn zdc_band_underpaid_offer_cannot_round_a_unit_up() {
    let mut z = setup(nav(1_000));
    let id = open(&mut z, "alice", 10, 1_000, 5_800);
    z.t.set_price(LIQ, nav(600));
    let bonus = z
        .estimate(id, usdc_raw(1), SeizeMode::Transfer)
        .bonus_rate_bps;
    let charged_fairly = |out: &LiqOutcome| {
        out.paid_usdc_raw >= out.got_units * 600 * USDC_UNIT * 10_000 / (10_000 + bonus)
    };
    match z.liquidate(id, usdc_raw(1), SeizeMode::Transfer) {
        Ok(out) => assert!(charged_fairly(&out), "{out:?}"),
        Err(e) => assert_eq!(e, Error::from_contract_error(errors::INVALID_PAYMENTS)),
    }
    assert_eq!(z.collateral_units(id), 10, "a $1 offer takes no unit");
    let out = z
        .liquidate(id, usdc_raw(2_000), SeizeMode::Transfer)
        .unwrap();
    assert!(charged_fairly(&out), "{out:?}");
}

/// A partial liquidation charges only for the whole units it seizes:
/// `paid * (1 + bonus) == units * NAV`, rounded up by at most one raw unit.
#[test]
fn zdc_partial_liquidation_charges_only_for_whole_units_seized() {
    let mut z = setup(nav(1_000));
    let id = open(&mut z, "alice", 10, 1_000, 5_800);
    z.t.set_price(LIQ, nav(800));
    let debt = z.debt_raw(id);
    let bonus = z.estimate(id, debt, SeizeMode::Transfer).bonus_rate_bps;
    let out = z.liquidate(id, debt, SeizeMode::Transfer).unwrap();
    assert!(out.got_units > 0 && out.units_after > 0, "{out:?}");
    assert!(out.debt_after < out.debt_before);
    let fair = out.got_units * 800 * USDC_UNIT * 10_000 / (10_000 + bonus);
    assert!(
        out.paid_usdc_raw >= fair && out.paid_usdc_raw <= fair + 1,
        "paid {} for {} units, fair {fair}",
        out.paid_usdc_raw,
        out.got_units
    );
}

/// An offer whose seizure is below one whole unit charges nothing: no whole
/// unit backs the repayment, so the call reverts.
#[test]
fn zdc_sub_unit_liquidation_reverts_without_charging() {
    let mut z = setup(nav(1_000));
    let id = open(&mut z, "alice", 10, 1_000, 5_800);
    z.t.set_price(LIQ, nav(800));
    let debt = z.debt_raw(id);
    let est = z.estimate(id, usdc_raw(1), SeizeMode::Transfer);
    assert_eq!(est.seized_collaterals.len(), 0);
    assert_eq!(est.max_payment_wad, 0, "no payment is quoted");

    let err = z
        .liquidate(id, usdc_raw(1), SeizeMode::Transfer)
        .unwrap_err();
    assert_eq!(err, Error::from_contract_error(errors::INVALID_PAYMENTS));
    assert_eq!(z.collateral_units(id), 10);
    assert_eq!(z.debt_raw(id), debt);
}
/// Credit mode moves whole-unit shares, so both accounts withdraw every unit
/// and nothing is stranded in the pool.
#[test]
fn zdc_credit_mode_moves_whole_units_only() {
    let mut z = setup(nav(1_000));
    let id = open(&mut z, "alice", 10, 1_000, 5_800);
    z.t.set_price(LIQ, nav(800));
    let debt = z.debt_raw(id);
    let out = z
        .liquidate(id, debt, SeizeMode::Credit(0))
        .expect("credit liquidation");
    let receiver = out.receiver;
    let ray = RAY;
    let a_scaled = z.collateral_scaled(id);
    let r_scaled = z.collateral_scaled(receiver);
    std::println!(
        "credit: alice {} + {}/1e27 units, receiver {} + {}/1e27 units, debt left ${:.2}",
        a_scaled / ray,
        a_scaled % ray,
        r_scaled / ray,
        r_scaled % ray,
        z.debt_raw(id) as f64 / 1e7
    );
    assert_eq!(a_scaled + r_scaled, 10 * ray, "shares conserved (fees 0)");
    assert_eq!(r_scaled % ray, 0, "receiver holds whole units");
    assert_eq!(a_scaled % ray, 0, "borrower keeps whole units");

    z.repay_all("alice", id);
    let alice_units = z.collateral_units(id);
    let got = z.try_withdraw("alice", id, alice_units).unwrap();
    let liquidator_units = z.collateral_units(receiver);
    let got_r = z
        .try_withdraw("liquidator", receiver, liquidator_units)
        .unwrap();
    std::println!(
        "alice withdrew {got} (view {alice_units}), liquidator withdrew {got_r} (view {liquidator_units}); pool cash left {}",
        z.hub_liq_cash()
    );
    assert_eq!(got + got_r, 10, "every unit leaves");
    assert_eq!(z.hub_liq_cash(), 0, "nothing is stranded");
    assert!(!z.exists(id) || z.collateral_scaled(id) == 0);
}

/// F4: a 10-unit insolvent account. The collateral-backed quote seizes all 10
/// units and the residual debt is socialized in the same call.
#[test]
fn zdc_insolvent_multi_unit_liquidation_seizes_every_unit() {
    let mut z = setup(nav(1_000));
    z.t.supply("bob", "USDC", 100_000.0);
    let id = open(&mut z, "alice", 10, 1_000, 5_800);
    z.t.set_price(LIQ, nav(500));
    let idx_before = z.supply_index(z.usdc_key());
    let debt = z.debt_raw(id);
    let out = z.liquidate(id, debt, SeizeMode::Transfer).unwrap();
    std::println!("{out:?}");
    assert_eq!(out.got_units, 10);
    assert!(!out.account_exists);
    assert!(z.supply_index(z.usdc_key()) < idx_before);
}

/// An offer below the collateral-backed quote does not take the whole
/// collateral: the seizure floors to whole units and the charge follows it.
#[test]
fn zdc_insolvent_underpaid_offer_does_not_seize_all() {
    let mut z = setup(nav(1_000));
    let id = open(&mut z, "alice", 10, 1_000, 5_800);
    z.t.set_price(LIQ, nav(500));
    let bonus = z
        .estimate(id, usdc_raw(2_000), SeizeMode::Transfer)
        .bonus_rate_bps;
    let out = z
        .liquidate(id, usdc_raw(2_000), SeizeMode::Transfer)
        .unwrap();
    std::println!("{out:?}");
    assert_eq!(out.got_units, 4);
    assert_eq!(out.units_after, 6);
    let fair = 4 * 500 * USDC_UNIT * 10_000 / (10_000 + bonus);
    assert!(
        out.paid_usdc_raw >= fair && out.paid_usdc_raw <= fair + 1,
        "paid {} for 4 units, fair {fair}",
        out.paid_usdc_raw
    );
}

#[test]
fn zdc_zero_decimal_market_cannot_be_listed_borrowable() {
    let z = setup(nav(1_000));
    let admin = z.t.admin();
    let err =
        z.t.gov_client()
            .try_execute_immediate(
                &admin,
                &AdminOperation::EditAssetInSpoke(SpokeAssetArgs {
                    hub_id: z.hub,
                    asset: z.liq.clone(),
                    spoke_id: z.spoke,
                    can_collateral: true,
                    can_borrow: true,
                    paused: false,
                    frozen: false,
                    no_seize: false,
                    ltv: LTV,
                    threshold: LT,
                    bonus: BONUS,
                    liquidation_fees: 0,
                    supply_cap: LIQ_CAP_UNITS,
                    borrow_cap: LIQ_CAP_UNITS,
                }),
            )
            .unwrap_err()
            .unwrap();
    assert_eq!(
        err,
        Error::from_contract_error(errors::INVALID_BORROW_PARAMS)
    );
}

/// Whole-unit seizure is pro-rata only on a single collateral leg, so a
/// sub-3-decimal leg is its account's only supply position, whichever path
/// adds the second.
#[test]
fn zdc_whole_unit_leg_is_its_accounts_only_supply_position() {
    let mut z = setup(nav(1_000));
    let admin = z.t.admin();
    z.t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::EditAssetInSpoke(SpokeAssetArgs {
            hub_id: HARNESS_HUB,
            asset: z.usdc.clone(),
            spoke_id: z.spoke,
            can_collateral: true,
            can_borrow: true,
            paused: false,
            frozen: false,
            no_seize: false,
            ltv: 7_500,
            threshold: 8_000,
            bonus: 500,
            liquidation_fees: 0,
            supply_cap: USDC_CAP_RAW,
            borrow_cap: USDC_CAP_RAW,
        }),
    );
    let limit = Error::from_contract_error(errors::POSITION_LIMIT_EXCEEDED);
    let usdc_leg = |z: &Zdc| (z.usdc_key(), usdc_raw(100));

    let liq_first = z.supply("alice", 0, 10);
    let alice = z.user("alice");
    z.t.resolve_market("USDC")
        .token_admin
        .mint(&alice, &usdc_raw(100));
    let err =
        z.t.ctrl_client()
            .try_supply(&alice, &liq_first, &z.spoke, &vec![&z.t.env, usdc_leg(&z)]);
    assert_eq!(err.unwrap_err().unwrap(), limit, "stable after LIQ");

    let bob = z.user("bob");
    z.t.resolve_market("USDC")
        .token_admin
        .mint(&bob, &usdc_raw(100));
    let usdc_first =
        z.t.ctrl_client()
            .supply(&bob, &0, &z.spoke, &vec![&z.t.env, usdc_leg(&z)]);
    z.t.resolve_market(LIQ).token_admin.mint(&bob, &10);
    let err = z.t.ctrl_client().try_supply(
        &bob,
        &usdc_first,
        &z.spoke,
        &vec![&z.t.env, (z.liq_key(), 10)],
    );
    assert_eq!(err.unwrap_err().unwrap(), limit, "LIQ after stable");

    z.t.supply("lender", "USDC", 100_000.0);
    let id = open(&mut z, "carol", 10, 1_000, 5_800);
    z.t.set_price(LIQ, nav(800));
    let liquidator = z.user("liquidator");
    z.t.resolve_market("USDC")
        .token_admin
        .mint(&liquidator, &usdc_raw(200));
    let receiver = z.t.ctrl_client().supply(
        &liquidator,
        &0,
        &z.spoke,
        &vec![&z.t.env, (z.usdc_key(), usdc_raw(100))],
    );
    let debt = z.debt_raw(id);
    let err = z
        .liquidate(id, debt, SeizeMode::Credit(receiver))
        .unwrap_err();
    assert_eq!(err, limit, "credit cannot add LIQ beside the stable");
}

/// Isolation binds the account holding the whole-unit leg, not its spoke: an
/// account without one keeps several positions beside it.
#[test]
fn zdc_accounts_without_a_whole_unit_leg_keep_several_positions() {
    let mut z = setup(nav(1_000));
    let admin = z.t.admin();
    let xlm = z.t.resolve_asset("XLM");
    for (asset, can_borrow) in [(z.usdc.clone(), true), (xlm.clone(), false)] {
        let op = if can_borrow {
            AdminOperation::EditAssetInSpoke
        } else {
            AdminOperation::AddAssetToSpoke
        };
        z.t.gov_client().execute_immediate(
            &admin,
            &op(SpokeAssetArgs {
                hub_id: HARNESS_HUB,
                asset,
                spoke_id: z.spoke,
                can_collateral: true,
                can_borrow,
                paused: false,
                frozen: false,
                no_seize: false,
                ltv: 7_500,
                threshold: 8_000,
                bonus: 500,
                liquidation_fees: 0,
                supply_cap: USDC_CAP_RAW,
                borrow_cap: if can_borrow { USDC_CAP_RAW } else { 0 },
            }),
        );
    }
    let dave = z.user("dave");
    z.t.resolve_market("USDC")
        .token_admin
        .mint(&dave, &usdc_raw(100));
    z.t.resolve_market("XLM")
        .token_admin
        .mint(&dave, &(1_000 * USDC_UNIT));
    let id = z.t.ctrl_client().supply(
        &dave,
        &0,
        &z.spoke,
        &vec![&z.t.env, (z.usdc_key(), usdc_raw(100))],
    );
    let xlm_key = HubAssetKey {
        hub_id: HARNESS_HUB,
        asset: xlm,
    };
    z.t.ctrl_client().supply(
        &dave,
        &id,
        &z.spoke,
        &vec![&z.t.env, (xlm_key, 1_000 * USDC_UNIT)],
    );
    let (supplies, _) = z.t.ctrl_client().get_account_positions(&id);
    assert_eq!(supplies.len(), 2);
}

/// Borrowing needs two whole units behind a sub-3-decimal leg: below HF 1 a
/// one-unit account quotes less than one seizable unit until the full-debt
/// promotion, while two units already floor to one.
#[test]
fn zdc_borrow_needs_two_whole_units_of_collateral() {
    let mut z = setup(nav(1_000));
    let one = z.supply("alice", 0, 1);
    let alice = z.user("alice");
    let err =
        z.t.ctrl_client()
            .try_borrow(
                &alice,
                &one,
                &vec![&z.t.env, (z.usdc_key(), usdc_raw(100))],
                &None,
            )
            .unwrap_err()
            .unwrap();
    assert_eq!(
        err,
        Error::from_contract_error(errors::MIN_BORROW_COLLATERAL_NOT_MET)
    );
    let two = open(&mut z, "bob", 2, 1_000, 5_000);
    assert!(z.debt_raw(two) > 0);
}

/// The two-unit floor holds after any debt-bearing action, so a withdraw
/// cannot leave one unit behind debt; a debt-free exit is not gated.
#[test]
fn zdc_withdraw_cannot_leave_one_unit_behind_debt() {
    let mut z = setup(nav(1_000));
    let id = open(&mut z, "alice", 2, 1_000, 1_000);
    assert_eq!(
        z.try_withdraw("alice", id, 1).unwrap_err(),
        Error::from_contract_error(errors::MIN_BORROW_COLLATERAL_NOT_MET)
    );
    z.repay_all("alice", id);
    assert_eq!(z.try_withdraw("alice", id, 1), Ok(1));
}

/// A flash-loan fee rounds up to one whole unit, so a sub-3-decimal market
/// cannot be flash-loanable.
#[test]
fn zdc_zero_decimal_market_cannot_be_flash_loanable() {
    let z = setup(nav(1_000));
    let admin = z.t.admin();
    let hub = z.t.create_hub();
    let mut params = DEFAULT_MARKET_PARAMS.to_market_params(&z.liq, 0);
    params.is_flashloanable = true;
    params.flashloan_fee = 9;
    let err =
        z.t.gov_client()
            .try_execute_immediate(
                &admin,
                &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
                    hub_id: hub,
                    asset: z.liq.clone(),
                    params,
                }),
            )
            .unwrap_err()
            .unwrap();
    assert_eq!(
        err,
        Error::from_contract_error(errors::INVALID_BORROW_PARAMS)
    );

    let listed = DEFAULT_MARKET_PARAMS.to_market_params(&z.liq, 0);
    let err =
        z.t.gov_client()
            .try_execute_immediate(
                &admin,
                &AdminOperation::UpgradeLiquidityPoolParams(UpgradePoolParamsArgs {
                    hub_asset: z.liq_key(),
                    params: InterestRateModel {
                        max_borrow_rate: listed.max_borrow_rate,
                        base_borrow_rate: listed.base_borrow_rate,
                        slope1: listed.slope1,
                        slope2: listed.slope2,
                        slope3: listed.slope3,
                        mid_utilization: listed.mid_utilization,
                        optimal_utilization: listed.optimal_utilization,
                        max_utilization: listed.max_utilization,
                        reserve_factor: listed.reserve_factor,
                        is_flashloanable: true,
                        flashloan_fee: 9,
                    },
                }),
            )
            .unwrap_err()
            .unwrap();
    assert_eq!(
        err,
        Error::from_contract_error(errors::INVALID_BORROW_PARAMS),
        "a params update cannot enable flash loans on the listed 0-decimal market"
    );
}

/// A protocol liquidation fee would round to zero or one whole unit, so a
/// sub-3-decimal listing must carry no fee.
#[test]
fn zdc_zero_decimal_listing_rejects_a_liquidation_fee() {
    let z = setup(nav(1_000));
    let admin = z.t.admin();
    let err =
        z.t.gov_client()
            .try_execute_immediate(
                &admin,
                &AdminOperation::EditAssetInSpoke(SpokeAssetArgs {
                    hub_id: z.hub,
                    asset: z.liq.clone(),
                    spoke_id: z.spoke,
                    can_collateral: true,
                    can_borrow: false,
                    paused: false,
                    frozen: false,
                    no_seize: false,
                    ltv: LTV,
                    threshold: LT,
                    bonus: BONUS,
                    liquidation_fees: 1,
                    supply_cap: LIQ_CAP_UNITS,
                    borrow_cap: 0,
                }),
            )
            .unwrap_err()
            .unwrap();
    assert_eq!(
        err,
        Error::from_contract_error(errors::INVALID_BORROW_PARAMS)
    );
}

/// A liquidator who sizes the repayment to land on whole units keeps the
/// bonus: repay ceil(k * NAV / (1 + bonus)) for the largest whole k in quote.
#[test]
fn zdc_liquidator_sized_to_whole_units_keeps_the_bonus() {
    let mut z = setup(nav(1_000));
    let id = open(&mut z, "alice", 10, 1_000, 5_800);
    z.t.set_price(LIQ, nav(800));
    let debt = z.debt_raw(id);
    let est = z.estimate(id, debt, SeizeMode::Transfer);
    let one_plus_b = 10_000 + est.bonus_rate_bps;
    let k = est.max_payment_wad * one_plus_b / 10_000 / usd(800);
    let pay = (k * 800 * USDC_UNIT * 10_000 + one_plus_b - 1) / one_plus_b + 1;
    let out = z.liquidate(id, pay, SeizeMode::Transfer).unwrap();
    let pnl = out.got_units * 800 * USDC_UNIT - out.paid_usdc_raw;
    std::println!(
        "k={k} pay=${:.2} {out:?} pnl=${:.2}",
        pay as f64 / 1e7,
        pnl as f64 / 1e7
    );
    assert_eq!(out.got_units, k);
    assert!(pnl > 0);
}

#[test]
fn zdc_large_position_near_cap() {
    let mut z = setup(nav(1_000));
    let id = z.supply("whale", 0, 8_700_000);
    assert_eq!(z.collateral_units(id), 8_700_000);
    let whale = z.user("whale");
    let over = LIQ_CAP_UNITS - 8_700_000 + 1;
    z.t.resolve_market(LIQ).token_admin.mint(&whale, &over);
    let err =
        z.t.ctrl_client()
            .try_supply(&whale, &id, &z.spoke, &vec![&z.t.env, (z.liq_key(), over)]);
    assert_eq!(
        err.unwrap_err().unwrap(),
        Error::from_contract_error(errors::SPOKE_SUPPLY_CAP_REACHED),
        "supply one unit above the 10M-unit cap is rejected"
    );
    z.borrow("whale", id, usdc_raw(40_000_000));
    z.t.set_price(LIQ, nav(6));
    assert!(z.hf(id) < WAD);
    let debt = z.debt_raw(id);
    let out = z.liquidate(id, debt, SeizeMode::Transfer).unwrap();
    std::println!("{out:?}");
    assert!(out.got_units > 0);
    assert!(out.units_before >= out.units_after + out.got_units);
}

#[test]
fn zdc_cap_domain_at_zero_decimals_is_170_billion_units() {
    let z = setup(nav(1_000));
    let admin = z.t.admin();
    let max = max_cap_for_decimals(0);
    assert_eq!(max, 170_141_183_460);
    let args = |cap: i128| SpokeAssetArgs {
        hub_id: z.hub,
        asset: z.liq.clone(),
        spoke_id: z.spoke,
        can_collateral: true,
        can_borrow: false,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: LTV,
        threshold: LT,
        bonus: BONUS,
        liquidation_fees: 0,
        supply_cap: cap,
        borrow_cap: 0,
    };
    let gov = z.t.gov_client();
    assert!(gov
        .try_execute_immediate(&admin, &AdminOperation::EditAssetInSpoke(args(max + 1)))
        .is_err());
    gov.execute_immediate(&admin, &AdminOperation::EditAssetInSpoke(args(max)));
}
