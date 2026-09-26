//! Liqvid deal shares as collateral behind the Asterizm RWA gate
//! (`RwaGatedToken`): an issuer allowlist checked on every holder a transfer
//! touches, issuer freezes, and issuer-controlled decimals. The market sits in
//! its own hub and spoke and borrows 7-decimal USDC from the base hub.
//!
//! Tests prefixed `lqv_`.

use common::types::{AssetOracle, HubAssetKey, PriceKey, SeizeMode};
use controller::constants::{RAY, WAD};
use governance::op::{
    AdminOperation, ConfigureAssetOracleArgs, CreatePoolArgs, SpokeAssetArgs,
    SpokeLiquidationCurveArgs,
};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, vec, Address, Error, String, TryFromVal, Vec};
use test_harness::rwa_gated_token::RwaGatedTokenClient;
use test_harness::{
    errors, usd_cents, usdc_preset, xlm_preset, AssetConfigPreset, LendingTest, MarketPreset,
    DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, HARNESS_HUB,
};

const LIQ: &str = "LIQVID";
const LIQ_CAP_UNITS: i128 = 10_000_000;
const USDC_UNIT: i128 = 10_000_000;
const USDC_CAP_RAW: i128 = 1_000_000_000 * USDC_UNIT;
const ACCOUNT_NOT_ALLOWED: u32 = 7;
const ACCOUNT_FROZEN: u32 = 8;
const INVALID_ORACLE_DECIMALS: u32 = 221;
const SANITY_BAND_TOO_WIDE_FOR_SINGLE_SOURCE: u32 = 226;

/// Listing parameters: `unit_cents` prices one raw unit of the token.
#[derive(Clone, Copy)]
struct Listing {
    decimals: u32,
    unit_cents: i128,
    ltv: u32,
    threshold: u32,
    bonus: u32,
}

/// $1,000 shares, LTV 60% / LT 70% / bonus 5%.
const THOUSAND_DOLLAR_SHARES: Listing = Listing {
    decimals: 0,
    unit_cents: 100_000,
    ltv: 6_000,
    threshold: 7_000,
    bonus: 500,
};

/// The testnet LIQVID1039 listing: $1 shares, LTV 50% / LT 53% / bonus 5%.
const LISTED_ONE_DOLLAR_SHARES: Listing = Listing {
    decimals: 0,
    unit_cents: 100,
    ltv: 5_000,
    threshold: 5_300,
    bonus: 500,
};

struct Lqv {
    t: LendingTest,
    hub: u32,
    spoke: u32,
    liq: Address,
    usdc: Address,
    pool: Address,
    listing: Listing,
}

fn setup(nav_usd: i128) -> Lqv {
    setup_listing(Listing {
        unit_cents: nav_usd * 100,
        ..THOUSAND_DOLLAR_SHARES
    })
}

/// Lists the gated market and opens the gate with only the pool allowlisted.
fn setup_listing(listing: Listing) -> Lqv {
    let token_price = usd_cents(listing.unit_cents) * 10i128.pow(listing.decimals);
    let t = LendingTest::new()
        .with_market(MarketPreset {
            initial_liquidity: 50_000_000.0,
            ..usdc_preset()
        })
        .with_market(xlm_preset())
        .with_rwa_gated_market(MarketPreset {
            name: LIQ,
            decimals: listing.decimals,
            price_wad: token_price,
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
    let pool = t.resolve_market(LIQ).pool.clone();
    let hub = t.create_hub();
    let gov = t.gov_client();
    gov.execute_immediate(
        &admin,
        &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
            hub_id: hub,
            asset: liq.clone(),
            params: DEFAULT_MARKET_PARAMS.to_market_params(&liq, listing.decimals),
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
            ltv: listing.ltv,
            threshold: listing.threshold,
            bonus: listing.bonus,
            liquidation_fees: 0,
            supply_cap: LIQ_CAP_UNITS,
            borrow_cap: 0,
        }),
    );
    gov.execute_immediate(
        &admin,
        &AdminOperation::AddAssetToSpoke(usdc_listing(&usdc, spoke, false)),
    );
    let z = Lqv {
        t,
        hub,
        spoke,
        liq,
        usdc,
        pool,
        listing,
    };
    z.token().set_gate_active(&true);
    z.allow(&z.pool.clone());
    z
}

fn usdc_listing(usdc: &Address, spoke: u32, can_collateral: bool) -> SpokeAssetArgs {
    SpokeAssetArgs {
        hub_id: HARNESS_HUB,
        asset: usdc.clone(),
        spoke_id: spoke,
        can_collateral,
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
    }
}

impl Lqv {
    fn token(&self) -> RwaGatedTokenClient<'_> {
        RwaGatedTokenClient::new(&self.t.env, &self.liq)
    }

    fn allow(&self, who: &Address) {
        self.token()
            .add_to_allowlist(&vec![&self.t.env, who.clone()]);
    }

    fn disallow(&self, who: &Address) {
        self.token()
            .remove_from_allowlist(&vec![&self.t.env, who.clone()]);
    }

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

    /// Creates an allowlisted holder with `units` raw units.
    fn holder(&mut self, name: &str, units: i128) -> Address {
        let who = self.user(name);
        self.allow(&who);
        self.token().mint(&who, &units);
        who
    }

    fn liq_balance(&self, who: &Address) -> i128 {
        token::Client::new(&self.t.env, &self.liq).balance(who)
    }

    /// Sets the price of one raw unit.
    fn set_unit_price(&mut self, unit_cents: i128) {
        let token_price = usd_cents(unit_cents) * 10i128.pow(self.listing.decimals);
        self.t.set_price(LIQ, token_price);
    }

    fn try_supply(&mut self, name: &str, account_id: u64, units: i128) -> Result<u64, Error> {
        let who = self.user(name);
        match self.t.ctrl_client().try_supply(
            &who,
            &account_id,
            &self.spoke,
            &vec![&self.t.env, (self.liq_key(), units)],
        ) {
            Ok(Ok(id)) => Ok(id),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(e.expect("contract error")),
        }
    }

    fn supply(&mut self, name: &str, account_id: u64, units: i128) -> u64 {
        self.try_supply(name, account_id, units)
            .unwrap_or_else(|e| panic!("{name} supply of {units} units failed: {e:?}"))
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

    fn try_borrow(&mut self, name: &str, account_id: u64, usdc_raw: i128) -> Result<(), Error> {
        let who = self.user(name);
        match self.t.ctrl_client().try_borrow(
            &who,
            &account_id,
            &vec![&self.t.env, (self.usdc_key(), usdc_raw)],
            &None,
        ) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e.into()),
            Err(e) => Err(e.expect("contract error")),
        }
    }

    fn try_withdraw(
        &mut self,
        name: &str,
        account_id: u64,
        units: i128,
        to: Option<Address>,
    ) -> Result<(), Error> {
        let who = self.user(name);
        match self.t.ctrl_client().try_withdraw(
            &who,
            &account_id,
            &vec![&self.t.env, (self.liq_key(), units)],
            &to,
        ) {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e.into()),
            Err(e) => Err(e.expect("contract error")),
        }
    }

    fn repay(&mut self, name: &str, account_id: u64, usdc_raw: i128) {
        let who = self.user(name);
        self.t
            .resolve_market("USDC")
            .token_admin
            .mint(&who, &usdc_raw);
        self.t.ctrl_client().repay(
            &who,
            &account_id,
            &vec![&self.t.env, (self.usdc_key(), usdc_raw)],
        );
    }

    fn repay_all(&mut self, name: &str, account_id: u64) {
        let owed = self.debt_raw(account_id) + 10;
        self.repay(name, account_id, owed);
    }

    fn try_liquidate(
        &mut self,
        liquidator: &str,
        account_id: u64,
        usdc_raw: i128,
        mode: SeizeMode,
    ) -> Result<u64, Error> {
        let who = self.user(liquidator);
        self.t
            .resolve_market("USDC")
            .token_admin
            .mint(&who, &usdc_raw);
        match self.t.ctrl_client().try_liquidate(
            &who,
            &account_id,
            &vec![&self.t.env, (self.usdc_key(), usdc_raw)],
            &mode,
        ) {
            Ok(Ok(receiver)) => Ok(receiver),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(e.expect("contract error")),
        }
    }

    fn usdc_balance(&self, who: &Address) -> i128 {
        token::Client::new(&self.t.env, &self.usdc).balance(who)
    }

    fn estimate_payment_raw(&self, account_id: u64, usdc_raw: i128) -> (i128, i128, i128) {
        let est = self.t.ctrl_client().get_liquidation_estimate(
            &account_id,
            &vec![&self.t.env, (self.usdc_key(), usdc_raw)],
            &SeizeMode::Transfer,
        );
        let seized = est
            .seized_collaterals
            .get(0)
            .map(|entry| entry.amount)
            .unwrap_or(0);
        let per_raw = WAD / USDC_UNIT;
        let payment_raw = (est.max_payment_wad + per_raw - 1) / per_raw;
        (payment_raw, seized, est.bonus_rate_bps)
    }

    fn units(&self, account_id: u64) -> i128 {
        if !self.t.ctrl_client().account_exists(&account_id) {
            return 0;
        }
        self.t
            .ctrl_client()
            .get_collateral_amount(&account_id, &self.liq_key())
    }

    fn scaled(&self, account_id: u64) -> i128 {
        let (supplies, _) = self.t.ctrl_client().get_account_positions(&account_id);
        supplies
            .get(self.liq_key())
            .map(|p| p.scaled_amount)
            .unwrap_or(0)
    }

    fn debt_raw(&self, account_id: u64) -> i128 {
        if !self.t.ctrl_client().account_exists(&account_id) {
            return 0;
        }
        self.t
            .ctrl_client()
            .get_borrow_amount(&account_id, &self.usdc_key())
    }

    fn hf(&self, account_id: u64) -> i128 {
        self.t.ctrl_client().get_health_factor(&account_id)
    }

    fn supply_index(&self) -> i128 {
        self.t
            .ctrl_client()
            .get_market_indexes_detailed(&Vec::from_array(&self.t.env, [self.liq_key()]))
            .get(0)
            .unwrap()
            .supply_index
    }

    fn reconfigure_oracle(&self, oracle: AssetOracle) -> Result<(), Error> {
        let admin = self.t.admin();
        match self.t.gov_client().try_execute_immediate(
            &admin,
            &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
                key: PriceKey::Token(self.liq.clone()),
                oracle,
            }),
        ) {
            Ok(_) => Ok(()),
            Err(Ok(e)) => Err(e),
            Err(Err(_)) => panic!("oracle reconfiguration hit a host error"),
        }
    }

    /// Every unit the pool holds backs exactly one unit of some account.
    fn assert_units_conserved(&self, accounts: &[u64]) {
        let owed: i128 = accounts.iter().map(|id| self.units(*id)).sum();
        assert_eq!(
            self.liq_balance(&self.pool),
            owed,
            "pool balance must equal the units accounts can withdraw"
        );
    }
}

fn contract_error(code: u32) -> Error {
    Error::from_contract_error(code)
}

/// Opens `units` raw units and borrows `borrow_bps` of their value.
fn open(z: &mut Lqv, name: &str, units: i128, borrow_bps: i128) -> u64 {
    z.holder(name, units);
    let id = z.supply(name, 0, units);
    let value_raw = units * z.listing.unit_cents * USDC_UNIT / 100;
    z.borrow(name, id, value_raw * borrow_bps / 10_000);
    id
}

#[test]
fn lqv_gated_round_trip_is_exact_and_needs_only_the_pool_and_the_holder() {
    let mut z = setup(1_000);
    assert!(
        !z.token().is_allowlisted(&z.t.controller_address()),
        "the controller stays off the allowlist"
    );
    let id = open(&mut z, "alice", 100, 5_000);
    assert_eq!(z.units(id), 100);
    assert_eq!(z.scaled(id), 100 * RAY);
    z.assert_units_conserved(&[id]);

    z.t.advance_and_sync(180 * 86_400);
    assert_eq!(
        z.supply_index(),
        RAY,
        "a gated collateral-only market never accrues"
    );
    z.repay_all("alice", id);
    z.try_withdraw("alice", id, 100, None).unwrap();
    let alice = z.user("alice");
    assert_eq!(z.liq_balance(&alice), 100, "every share comes back");
    assert_eq!(
        z.liq_balance(&z.pool),
        0,
        "no share is stranded in the pool"
    );
}

#[test]
fn lqv_supply_from_a_holder_off_the_allowlist_reverts_with_the_token_error() {
    let mut z = setup(1_000);
    let alice = z.holder("alice", 10);
    z.disallow(&alice);
    assert_eq!(
        z.try_supply("alice", 0, 10),
        Err(contract_error(ACCOUNT_NOT_ALLOWED))
    );
    assert_eq!(z.liq_balance(&alice), 10);
    assert_eq!(z.liq_balance(&z.pool), 0);
}

#[test]
fn lqv_a_pool_off_the_allowlist_takes_no_deposit() {
    let mut z = setup(1_000);
    z.holder("alice", 10);
    z.disallow(&z.pool.clone());
    assert_eq!(
        z.try_supply("alice", 0, 10),
        Err(contract_error(ACCOUNT_NOT_ALLOWED))
    );
}

#[test]
fn lqv_withdraw_to_a_recipient_off_the_allowlist_reverts_and_keeps_the_position() {
    let mut z = setup(1_000);
    z.holder("alice", 10);
    let id = z.supply("alice", 0, 10);
    let stranger = z.user("stranger");
    assert_eq!(
        z.try_withdraw("alice", id, 4, Some(stranger.clone())),
        Err(contract_error(ACCOUNT_NOT_ALLOWED))
    );
    assert_eq!(z.units(id), 10);
    z.allow(&stranger);
    z.try_withdraw("alice", id, 4, Some(stranger.clone()))
        .unwrap();
    assert_eq!(z.liq_balance(&stranger), 4);
    z.assert_units_conserved(&[id]);
}

#[test]
fn lqv_transfer_liquidation_needs_an_allowlisted_liquidator() {
    let mut z = setup(1_000);
    let id = open(&mut z, "alice", 10, 5_800);
    z.set_unit_price(80_000);
    let debt = z.debt_raw(id);

    assert_eq!(
        z.try_liquidate("liquidator", id, debt, SeizeMode::Transfer),
        Err(contract_error(ACCOUNT_NOT_ALLOWED))
    );
    assert_eq!(z.units(id), 10, "a reverted liquidation moves nothing");
    assert_eq!(z.debt_raw(id), debt);

    let liquidator = z.user("liquidator");
    z.allow(&liquidator);
    z.try_liquidate("liquidator", id, debt, SeizeMode::Transfer)
        .unwrap();
    let seized = z.liq_balance(&liquidator);
    assert!(seized > 0);
    assert_eq!(z.units(id) + seized, 10);
    assert!(z.debt_raw(id) < debt);
    z.assert_units_conserved(&[id]);
}

/// Credit mode moves supply shares, not tokens, so the issuer allowlist does
/// not gate it; the receiver can exit only once the issuer admits it.
#[test]
fn lqv_credit_liquidation_moves_shares_without_a_token_transfer() {
    let mut z = setup(1_000);
    let id = open(&mut z, "alice", 10, 5_800);
    z.set_unit_price(80_000);
    let debt = z.debt_raw(id);

    let receiver = z
        .try_liquidate("liquidator", id, debt, SeizeMode::Credit(0))
        .unwrap();
    let credited = z.units(receiver);
    assert!(credited > 0);
    assert_eq!(z.units(id) + credited, 10);
    assert_eq!(z.scaled(receiver) % RAY, 0, "credit moves whole units");
    z.assert_units_conserved(&[id, receiver]);

    assert_eq!(
        z.try_withdraw("liquidator", receiver, credited, None),
        Err(contract_error(ACCOUNT_NOT_ALLOWED)),
        "the receiver cannot take shares out before the issuer admits it"
    );
    let liquidator = z.user("liquidator");
    z.allow(&liquidator);
    z.try_withdraw("liquidator", receiver, credited, None)
        .unwrap();
    assert_eq!(z.liq_balance(&liquidator), credited);
}

/// An issuer freeze of the pool stops every token movement through it:
/// deposits, withdrawals and Transfer liquidations revert. Credit liquidation
/// and USDC repayment keep working, and unfreezing restores exits.
#[test]
fn lqv_issuer_pool_freeze_blocks_token_movement_until_unfrozen() {
    let mut z = setup(1_000);
    let id = open(&mut z, "alice", 10, 3_000);
    z.holder("bob", 5);
    let liquidator = z.user("liquidator");
    z.allow(&liquidator);

    z.token().freeze(&z.pool.clone());
    assert_eq!(
        z.try_supply("bob", 0, 5),
        Err(contract_error(ACCOUNT_FROZEN))
    );
    let bob = z.user("bob");
    assert_eq!(z.liq_balance(&bob), 5, "a refused deposit credits nothing");
    assert_eq!(
        z.try_withdraw("alice", id, 1, None),
        Err(contract_error(ACCOUNT_FROZEN))
    );
    let debt_before = z.debt_raw(id);
    z.repay("alice", id, 100 * USDC_UNIT);
    assert!(
        z.debt_raw(id) < debt_before,
        "USDC repayment ignores the freeze"
    );

    z.set_unit_price(40_000);
    assert!(z.hf(id) < WAD);
    let debt = z.debt_raw(id);
    assert_eq!(
        z.try_liquidate("liquidator", id, debt, SeizeMode::Transfer),
        Err(contract_error(ACCOUNT_FROZEN))
    );
    let receiver = z
        .try_liquidate("liquidator", id, debt, SeizeMode::Credit(0))
        .expect("credit mode settles without a token transfer");
    assert!(z.units(receiver) > 0);

    z.token().unfreeze(&z.pool.clone());
    let left = z.units(receiver);
    z.try_withdraw("liquidator", receiver, left, None).unwrap();
    assert_eq!(z.liq_balance(&liquidator), left);
}

/// The issuer removes the pool from the allowlist after deposits: every exit
/// reverts until it is restored, and Credit liquidation still settles.
#[test]
fn lqv_pool_removed_from_allowlist_after_deposits_blocks_exits_only() {
    let mut z = setup(1_000);
    let id = open(&mut z, "alice", 10, 5_800);
    let liquidator = z.user("liquidator");
    z.allow(&liquidator);
    z.disallow(&z.pool.clone());

    assert_eq!(
        z.try_withdraw("alice", id, 1, None),
        Err(contract_error(ACCOUNT_NOT_ALLOWED))
    );
    z.set_unit_price(80_000);
    let debt = z.debt_raw(id);
    assert_eq!(
        z.try_liquidate("liquidator", id, debt, SeizeMode::Transfer),
        Err(contract_error(ACCOUNT_NOT_ALLOWED))
    );
    let receiver = z
        .try_liquidate("liquidator", id, debt, SeizeMode::Credit(0))
        .unwrap();
    assert!(z.units(receiver) > 0);
    z.assert_units_conserved(&[id, receiver]);
}

/// An allowlisted but frozen liquidator cannot receive shares in Transfer
/// mode; Credit mode settles.
#[test]
fn lqv_a_frozen_liquidator_can_only_liquidate_in_credit_mode() {
    let mut z = setup(1_000);
    let id = open(&mut z, "alice", 10, 5_800);
    let liquidator = z.user("liquidator");
    z.allow(&liquidator);
    z.token().freeze(&liquidator);
    z.set_unit_price(80_000);
    let debt = z.debt_raw(id);

    assert_eq!(
        z.try_liquidate("liquidator", id, debt, SeizeMode::Transfer),
        Err(contract_error(ACCOUNT_FROZEN))
    );
    let receiver = z
        .try_liquidate("liquidator", id, debt, SeizeMode::Credit(0))
        .unwrap();
    assert!(z.units(receiver) > 0);
    assert!(z.debt_raw(id) < debt);
}

/// A frozen borrower cannot move shares, but the protocol can still liquidate
/// the position, and the borrower can still repay in USDC: seizure moves shares
/// from the pool, never from the borrower.
#[test]
fn lqv_a_frozen_borrower_is_still_liquidatable() {
    let mut z = setup(1_000);
    let id = open(&mut z, "alice", 10, 5_800);
    let alice = z.user("alice");
    z.token().freeze(&alice);
    assert_eq!(
        z.token().try_transfer(&alice, &z.pool.clone(), &0),
        Err(Ok(contract_error(ACCOUNT_FROZEN))),
        "the freeze gates even a zero transfer"
    );
    let debt_before = z.debt_raw(id);
    z.repay("alice", id, 100 * USDC_UNIT);
    assert!(z.debt_raw(id) < debt_before);

    z.set_unit_price(80_000);
    let debt = z.debt_raw(id);
    let liquidator = z.user("liquidator");
    z.allow(&liquidator);
    z.try_liquidate("liquidator", id, debt, SeizeMode::Transfer)
        .unwrap();
    assert!(z.liq_balance(&liquidator) > 0);
    assert!(z.debt_raw(id) < debt);
    z.assert_units_conserved(&[id]);
}

/// An issuer decimals relabel does not reach the protocol: governance keeps
/// resolving the listed decimals from the stored oracle, so maintenance such
/// as moving the sanity band still works, and a partial liquidation seizes
/// exactly what the view quotes.
#[test]
fn lqv_issuer_decimals_relabel_keeps_the_listed_unit_and_oracle_maintenance() {
    let mut z = setup(1_000);
    let id = open(&mut z, "alice", 10, 5_800);
    let key = PriceKey::Token(z.liq.clone());
    let oracle = z.t.price_agg_client().oracle(&key).unwrap();
    assert_eq!(oracle.asset_decimals, 0);
    z.reconfigure_oracle(oracle.clone())
        .expect("positive control: a reconfiguration before the relabel succeeds");

    z.token().set_metadata(
        &2,
        &String::from_str(&z.t.env, LIQ),
        &String::from_str(&z.t.env, LIQ),
    );
    let mut moved = oracle.clone();
    moved.min_sanity_price_wad = oracle.min_sanity_price_wad * 95 / 100;
    moved.max_sanity_price_wad = oracle.max_sanity_price_wad * 95 / 100;
    z.reconfigure_oracle(moved.clone())
        .expect("oracle maintenance after the relabel");
    let stored = z.t.price_agg_client().oracle(&key).unwrap();
    assert_eq!(
        stored.asset_decimals, 0,
        "the listed unit survives the relabel"
    );
    assert_eq!(stored.min_sanity_price_wad, moved.min_sanity_price_wad);

    let mut relabelled = stored.clone();
    relabelled.asset_decimals = 2;
    assert_eq!(
        z.t.price_agg_client().try_set_oracle(&key, &relabelled),
        Err(Ok(contract_error(INVALID_ORACLE_DECIMALS))),
        "a direct owner call cannot move the unit either"
    );

    z.set_unit_price(80_000);
    let liquidator = z.user("liquidator");
    z.allow(&liquidator);
    let offer = 1_500 * USDC_UNIT;
    let (_, quoted, _) = z.estimate_payment_raw(id, offer);
    z.try_liquidate("liquidator", id, offer, SeizeMode::Transfer)
        .unwrap();
    let seized = z.liq_balance(&liquidator);
    assert!((1..10).contains(&seized), "seized {seized}");
    assert_eq!(seized, quoted, "execution matches the view");
    assert_eq!(z.units(id), 10 - seized);
}

/// After a relabel, a second hub can list the token only in its listed unit.
#[test]
fn lqv_second_hub_listing_after_a_relabel_uses_the_listed_unit() {
    let z = setup(1_000);
    z.token().set_metadata(
        &2,
        &String::from_str(&z.t.env, LIQ),
        &String::from_str(&z.t.env, LIQ),
    );
    let admin = z.t.admin();
    let hub = z.t.create_hub();
    let create = |decimals: u32| {
        z.t.gov_client().try_execute_immediate(
            &admin,
            &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
                hub_id: hub,
                asset: z.liq.clone(),
                params: DEFAULT_MARKET_PARAMS.to_market_params(&z.liq, decimals),
            }),
        )
    };
    assert_eq!(
        create(2).map(|_| ()).map_err(|e| e.map_err(|_| ())),
        Err(Ok(contract_error(errors::INVALID_ASSET)))
    );
    assert!(create(0).is_ok(), "the listed unit is accepted");
}

/// Revenue on the gated market: a claim with nothing to claim moves no token,
/// so it needs no allowlist. Seized-share revenue moves pool -> controller ->
/// accumulator, so the claim reverts, and takes a batch with it, until the
/// issuer admits both.
#[test]
fn lqv_revenue_claim_needs_the_controller_and_accumulator_allowlisted() {
    let mut z = setup(1_000);
    let admin = z.t.admin();
    let accumulator = Address::generate(&z.t.env);
    z.t.gov_client()
        .execute_immediate(&admin, &AdminOperation::SetAccumulator(accumulator.clone()));
    let claim = |z: &Lqv, assets: Vec<HubAssetKey>| match z
        .t
        .ctrl_client()
        .try_claim_revenue(&admin, &assets)
    {
        Ok(Ok(amounts)) => Ok(amounts),
        Ok(Err(e)) => Err(e.into()),
        Err(e) => Err(e.expect("contract error")),
    };
    let only_liq = vec![&z.t.env, z.liq_key()];
    assert_eq!(
        claim(&z, only_liq.clone()).unwrap().get(0),
        Some(0),
        "an empty claim moves no token"
    );

    let id = open(&mut z, "alice", 10, 5_800);
    z.set_unit_price(30_000);
    z.t.force_socialize_bad_debt_by_id(id);
    assert_eq!(z.units(id), 0);
    assert_eq!(
        z.liq_balance(&z.pool),
        10,
        "seized shares stay in the pool as revenue"
    );

    assert_eq!(
        claim(&z, only_liq.clone()),
        Err(contract_error(ACCOUNT_NOT_ALLOWED))
    );
    assert_eq!(
        claim(&z, vec![&z.t.env, z.usdc_key(), z.liq_key()]),
        Err(contract_error(ACCOUNT_NOT_ALLOWED)),
        "one gated market reverts the whole batch"
    );
    z.allow(&z.t.controller_address());
    assert_eq!(
        claim(&z, only_liq.clone()),
        Err(contract_error(ACCOUNT_NOT_ALLOWED)),
        "the accumulator must be admitted too"
    );
    z.allow(&accumulator);
    assert_eq!(claim(&z, only_liq).unwrap().get(0), Some(10));
    assert_eq!(z.liq_balance(&accumulator), 10);
}

/// NAV falls in daily steps. Each step's liquidation seizes whole shares and
/// every share stays accounted for.
#[test]
fn lqv_stepwise_nav_markdown_liquidates_in_whole_shares() {
    let mut z = setup(1_000);
    let id = open(&mut z, "alice", 50, 5_900);
    let liquidator = z.user("liquidator");
    z.allow(&liquidator);
    for unit_cents in [95_000i128, 90_000, 85_000, 80_000, 75_000] {
        z.t.advance_and_sync(86_400);
        z.set_unit_price(unit_cents);
        if z.hf(id) >= WAD {
            continue;
        }
        let debt = z.debt_raw(id);
        z.try_liquidate("liquidator", id, debt, SeizeMode::Transfer)
            .unwrap_or_else(|e| panic!("NAV ${}: liquidation reverted {e:?}", unit_cents / 100));
        assert_eq!(z.scaled(id) % RAY, 0, "the borrower keeps whole shares");
        z.assert_units_conserved(&[id]);
    }
    assert!(z.liq_balance(&liquidator) > 0);
    assert_eq!(z.units(id) + z.liq_balance(&liquidator), 50);
}

/// The smallest borrowing account (two shares) just below HF 1 sells one
/// share at `NAV / (1 + bonus)`. The one-share residue, once below HF 1 again,
/// closes in full: the liquidator repays all debt for the last share.
#[test]
fn lqv_two_share_account_sells_one_share_then_closes_in_full() {
    let mut z = setup(1_000);
    let id = open(&mut z, "alice", 2, 5_990);
    z.set_unit_price(85_000);
    assert!(z.hf(id) < WAD);
    let liquidator = z.user("liquidator");
    z.allow(&liquidator);
    let debt = z.debt_raw(id);
    z.try_liquidate("liquidator", id, debt, SeizeMode::Transfer)
        .unwrap();
    assert_eq!(z.liq_balance(&liquidator), 1);
    assert_eq!(z.units(id), 1);
    let residue_debt = z.debt_raw(id);
    assert!(residue_debt > 0 && residue_debt < debt);

    let unit_cents = residue_debt * 100 / USDC_UNIT * 13_570 / 10_000;
    z.set_unit_price(unit_cents);
    let hf = z.hf(id);
    assert!(
        hf < WAD && hf > WAD * 94 / 100,
        "the residue sits just below HF 1: {hf}"
    );
    let paid_before = z.usdc_balance(&liquidator);
    z.try_liquidate("liquidator", id, residue_debt + 10, SeizeMode::Transfer)
        .unwrap();
    assert_eq!(z.liq_balance(&liquidator), 2, "the last share is seized");
    assert_eq!(z.debt_raw(id), 0, "the debt closes");
    let paid = paid_before + residue_debt + 10 - z.usdc_balance(&liquidator);
    assert!(paid >= residue_debt, "the liquidator repays the whole debt");
    assert_eq!(z.liq_balance(&z.pool), 0);
}

/// A low-threshold listing on a mainnet-shaped curve, where one share at
/// `NAV / (1 + bonus)` covers the whole debt, closes in full and takes exactly
/// one of the two shares; the liquidator's effective bonus is `NAV / debt - 1`.
#[test]
fn lqv_low_threshold_full_close_takes_one_share_of_two() {
    let mut z = setup_listing(Listing {
        ltv: 2_500,
        threshold: 3_000,
        ..THOUSAND_DOLLAR_SHARES
    });
    let admin = z.t.admin();
    z.t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs {
            spoke_id: z.spoke,
            target_hf_wad: WAD * 115 / 100,
            hf_for_max_bonus_wad: WAD * 90 / 100,
            liquidation_bonus_factor_bps: 1_500,
        }),
    );
    let id = open(&mut z, "alice", 2, 2_500);
    z.set_unit_price(80_000);
    let hf = z.hf(id);
    assert!(hf < WAD && hf > WAD * 9 / 10, "HF {hf}");
    let liquidator = z.user("liquidator");
    z.allow(&liquidator);
    let debt = z.debt_raw(id);
    let before = z.usdc_balance(&liquidator);
    z.try_liquidate("liquidator", id, debt + 10, SeizeMode::Transfer)
        .unwrap();
    assert_eq!(z.liq_balance(&liquidator), 1, "exactly one share");
    assert_eq!(z.units(id), 1, "the borrower keeps the other share");
    assert_eq!(z.debt_raw(id), 0, "the debt closes");
    let paid = before + debt + 10 - z.usdc_balance(&liquidator);
    assert!(
        paid >= debt && paid <= debt + 1,
        "paid {paid} for debt {debt}"
    );
    z.assert_units_conserved(&[id]);
}

/// On a small account the view's payment buys exactly one share, and an offer
/// below one share at `NAV / (1 + bonus)` takes nothing.
#[test]
fn lqv_small_account_offer_boundary() {
    let mut z = setup(1_000);
    let id = open(&mut z, "alice", 3, 5_990);
    z.set_unit_price(85_000);
    let debt = z.debt_raw(id);
    let (payment, seized, bonus) = z.estimate_payment_raw(id, debt);
    assert_eq!(seized, 1);
    let unit_raw = 850 * USDC_UNIT;
    let fair = unit_raw * 10_000 / (10_000 + bonus);
    assert!(
        payment >= fair && payment <= fair + fair / 100_000,
        "{payment} vs {fair}"
    );

    assert_eq!(
        z.try_liquidate("liquidator", id, fair - 1, SeizeMode::Transfer),
        Err(contract_error(errors::INVALID_PAYMENTS))
    );
    assert_eq!(z.units(id), 3);
    let liquidator = z.user("liquidator");
    z.allow(&liquidator);
    let before = z.usdc_balance(&liquidator);
    z.try_liquidate("liquidator", id, payment, SeizeMode::Transfer)
        .unwrap();
    assert_eq!(z.liq_balance(&liquidator), 1);
    let paid = before + payment - z.usdc_balance(&liquidator);
    assert!(paid >= fair && paid <= fair + 2, "paid {paid}, fair {fair}");
}

/// The one-unit sale covers every leg below 3 decimals, not only 0 decimals:
/// a two-raw-unit account at 1 and 2 decimals sells one raw unit, while a
/// 3-decimal leg keeps the plain curve quote.
#[test]
fn lqv_one_unit_sale_applies_below_three_decimals_only() {
    for decimals in [1u32, 2] {
        let mut z = setup_listing(Listing {
            decimals,
            ..THOUSAND_DOLLAR_SHARES
        });
        let id = open(&mut z, "alice", 2, 5_990);
        z.set_unit_price(85_000);
        assert!(z.hf(id) < WAD, "{decimals} decimals");
        let liquidator = z.user("liquidator");
        z.allow(&liquidator);
        let debt = z.debt_raw(id);
        z.try_liquidate("liquidator", id, debt, SeizeMode::Transfer)
            .unwrap_or_else(|e| panic!("{decimals} decimals: {e:?}"));
        assert_eq!(z.liq_balance(&liquidator), 1, "{decimals} decimals");
    }

    let mut z = setup_listing(Listing {
        decimals: 3,
        ..THOUSAND_DOLLAR_SHARES
    });
    let id = open(&mut z, "alice", 2, 5_990);
    z.set_unit_price(85_000);
    assert!(z.hf(id) < WAD);
    let debt = z.debt_raw(id);
    let (payment, _, bonus) = z.estimate_payment_raw(id, debt);
    let one_unit = 850 * USDC_UNIT * 10_000 / (10_000 + bonus);
    assert!(
        payment < one_unit,
        "a 3-decimal leg is not raised to one unit: {payment} vs {one_unit}"
    );
}

/// The share leg stays its account's only supply position behind the gate.
#[test]
fn lqv_a_share_leg_stays_its_accounts_only_supply_position() {
    let mut z = setup(1_000);
    let admin = z.t.admin();
    z.t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::EditAssetInSpoke(usdc_listing(&z.usdc, z.spoke, true)),
    );
    z.holder("alice", 10);
    let id = z.supply("alice", 0, 10);
    let alice = z.user("alice");
    z.t.resolve_market("USDC")
        .token_admin
        .mint(&alice, &(100 * USDC_UNIT));
    let err =
        z.t.ctrl_client()
            .try_supply(
                &alice,
                &id,
                &z.spoke,
                &vec![&z.t.env, (z.usdc_key(), 100 * USDC_UNIT)],
            )
            .unwrap_err()
            .unwrap();
    assert_eq!(err, contract_error(errors::POSITION_LIMIT_EXCEEDED));
    let (supplies, _) = z.t.ctrl_client().get_account_positions(&id);
    assert_eq!(supplies.len(), 1);
}

/// The testnet listing ($1 shares, LT 53%): a NAV markdown path liquidates in
/// whole shares, and a gap into insolvency seizes every remaining share.
#[test]
fn lqv_listed_one_dollar_shares_markdown_and_insolvency() {
    let mut z = setup_listing(LISTED_ONE_DOLLAR_SHARES);
    let id = open(&mut z, "alice", 10_000, 4_900);
    let liquidator = z.user("liquidator");
    z.allow(&liquidator);
    for unit_cents in [95i128, 90, 85, 80] {
        z.t.advance_and_sync(86_400);
        z.set_unit_price(unit_cents);
        if z.hf(id) >= WAD {
            continue;
        }
        let debt = z.debt_raw(id);
        z.try_liquidate("liquidator", id, debt, SeizeMode::Transfer)
            .unwrap_or_else(|e| panic!("NAV {unit_cents} cents: {e:?}"));
        z.assert_units_conserved(&[id]);
    }
    assert!(z.liq_balance(&liquidator) > 0, "the markdown liquidated");

    z.set_unit_price(30);
    let collateral_left = z.units(id);
    assert!(collateral_left > 0);
    let debt = z.debt_raw(id);
    z.try_liquidate("liquidator", id, debt, SeizeMode::Transfer)
        .unwrap();
    assert_eq!(z.units(id), 0, "insolvency seizes every share");
    assert_eq!(z.liq_balance(&liquidator), 10_000);
}

/// A NAV move outside the sanity band fails closed: borrowing and liquidation
/// revert until governance moves the band. A single-source band keeps a
/// half-width of at most 10%, so it moves around the new NAV rather than widens.
#[test]
fn lqv_nav_outside_the_sanity_band_fails_closed_until_governance_moves_it() {
    let mut z = setup_listing(LISTED_ONE_DOLLAR_SHARES);
    let id = open(&mut z, "alice", 10_000, 4_900);
    let liquidator = z.user("liquidator");
    z.allow(&liquidator);
    z.t.seed_sanity_band(LIQ, usd_cents(95), usd_cents(105));
    z.t.set_price_keeping_sanity_band(LIQ, usd_cents(80));

    let debt = z.debt_raw(id);
    assert!(z.try_borrow("alice", id, USDC_UNIT).is_err());
    assert!(z
        .try_liquidate("liquidator", id, debt, SeizeMode::Transfer)
        .is_err());
    assert_eq!(z.units(id), 10_000);

    let key = PriceKey::Token(z.liq.clone());
    let mut moved = z.t.price_agg_client().oracle(&key).unwrap();
    let too_wide = AssetOracle {
        min_sanity_price_wad: usd_cents(50),
        ..moved.clone()
    };
    assert_eq!(
        z.reconfigure_oracle(too_wide),
        Err(contract_error(SANITY_BAND_TOO_WIDE_FOR_SINGLE_SOURCE))
    );
    moved.min_sanity_price_wad = usd_cents(74);
    moved.max_sanity_price_wad = usd_cents(86);
    z.reconfigure_oracle(moved).unwrap();
    z.try_liquidate("liquidator", id, debt, SeizeMode::Transfer)
        .unwrap();
    assert!(z.liq_balance(&liquidator) > 0);
}

/// A NAV feed that goes stale fails closed for new borrowing and liquidation.
#[test]
fn lqv_stale_nav_fails_closed() {
    let mut z = setup_listing(LISTED_ONE_DOLLAR_SHARES);
    let id = open(&mut z, "alice", 10_000, 4_000);
    let liquidator = z.user("liquidator");
    z.allow(&liquidator);
    z.t.advance_time_no_refresh(30 * 86_400);
    assert!(z.try_borrow("alice", id, USDC_UNIT).is_err());
    let debt = z.debt_raw(id);
    assert!(z
        .try_liquidate("liquidator", id, debt, SeizeMode::Transfer)
        .is_err());
    assert_eq!(z.units(id), 10_000);
}
