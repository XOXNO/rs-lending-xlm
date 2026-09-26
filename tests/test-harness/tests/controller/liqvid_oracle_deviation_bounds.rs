//! Oracle-deviation bounds on the Liqvid hub (audit item U-1).
//!
//! A single-source NAV feed can report any price `p` in its sanity band
//! `[min, max]`. The true NAV `P` is in the same band, so `P / p <= u = max / min`.
//! The tests keep 0-decimal shares that borrow hub USDC and pin these bounds:
//!
//! - I1: from accounts healthy at true NAV, no sequence of borrow, Transfer or
//!   Credit liquidation, and borrow against credited shares at in-band prices
//!   reaches USDC lenders. The supply index does not move, every liquidation
//!   pays for the debt it retires, and every open debt stays backed at the
//!   band floor.
//! - I2: one liquidation at `p` that pays `R` and retires `repaid` costs the
//!   borrower, at true NAV, at most `min(E, R * (1 + b) * P / p - repaid + F)`.
//!   `E` is the equity at true NAV, `b` the curve bonus at the reported HF, and
//!   `F` one share at true NAV on a full close, else zero.
//! - I3: the one-unit sale and the full close by one unit keep I1 and I2.
//!
//! Tests prefixed `lqv_oracle_bound_`.

use common::types::{AssetOracle, HubAssetKey, PriceKey, SeizeMode};
use controller::constants::{
    BPS, DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD, MIN_WHOLE_UNIT_COLLATERAL, WAD,
};
use governance::op::{
    AdminOperation, ConfigureAssetOracleArgs, CreatePoolArgs, SpokeAssetArgs,
    SpokeLiquidationCurveArgs,
};
use soroban_sdk::{token, vec, Address, TryFromVal};
use test_harness::rwa_gated_token::RwaGatedTokenClient;
use test_harness::{
    usd_cents, usdc_preset, xlm_preset, AssetConfigPreset, LendingTest, MarketPreset,
    DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, HARNESS_HUB,
};

const LIQ: &str = "LIQVID";
const USDC_UNIT: i128 = 10_000_000;
const WAD_PER_USDC_RAW: i128 = WAD / USDC_UNIT;
const FEED_STEP_WAD: i128 = 10_000;
const THOUSAND_DOLLARS: i128 = 100_000;
const ONE_DOLLAR: i128 = 100;

#[derive(Clone, Copy)]
struct Curve {
    target_bps: i128,
    knee_bps: i128,
    factor_bps: u32,
}

#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    ltv: u32,
    threshold: u32,
    bonus: u32,
    curve: Curve,
    band_min_bps: i128,
    band_max_bps: i128,
}

const DEFAULT_CURVE: Curve = Curve {
    target_bps: 11_000,
    knee_bps: 8_000,
    factor_bps: 10_000,
};

/// The widest band a single source admits: `(max - min) / (max + min) = 10%`.
const CAP_BAND: Case = Case {
    name: "LT 60% default curve, band 11/9",
    ltv: 5_000,
    threshold: 6_000,
    bonus: 500,
    curve: DEFAULT_CURVE,
    band_min_bps: 9_000,
    band_max_bps: 11_000,
};

const TESTNET_BAND: Case = Case {
    name: "LT 60% default curve, band [0.95, 1.05]",
    band_min_bps: 9_500,
    band_max_bps: 10_500,
    ..CAP_BAND
};

const LISTING_PARAMS: Case = Case {
    name: "LT 53% curve 1.06/0.90/598, band [0.849, 1.03]",
    ltv: 5_000,
    threshold: 5_300,
    bonus: 500,
    curve: Curve {
        target_bps: 10_600,
        knee_bps: 9_000,
        factor_bps: 598,
    },
    band_min_bps: 8_490,
    band_max_bps: 10_300,
};

const CASES: [Case; 3] = [CAP_BAND, TESTNET_BAND, LISTING_PARAMS];

/// The curve bonus at reported `hf` for a single-leg account, capped so the
/// seizure keeps HF: `min(curve(hf), hf / LT - 1)`.
fn curve_bonus_bps(case: &Case, hf: i128) -> i128 {
    let lt = i128::from(case.threshold);
    let max = BPS * (BPS - lt) / lt;
    let base = i128::from(case.bonus).min(max);
    let target = case.curve.target_bps * WAD / BPS;
    let knee = case.curve.knee_bps * WAD / BPS;
    let curve = if hf >= target {
        base
    } else {
        let scale = ((target - hf) * WAD / (target - knee)).min(WAD);
        base + (max - base) * scale / WAD * i128::from(case.curve.factor_bps) / BPS
    };
    curve.min(hf_cap_bps(case, hf))
}

/// The bonus at which a single-leg seizure keeps HF: `hf / LT - 1`.
fn hf_cap_bps(case: &Case, hf: i128) -> i128 {
    hf * BPS * BPS / (i128::from(case.threshold) * WAD) - BPS
}

/// USDC raw value of `units` shares at `price_wad` per share.
fn value_raw(units: i128, price_wad: i128) -> i128 {
    units * price_wad / WAD_PER_USDC_RAW
}

struct Outcome {
    receiver: u64,
    loss: i128,
    equity: i128,
}

struct Z {
    t: LendingTest,
    hub: u32,
    spoke: u32,
    liq: Address,
    usdc: Address,
    case: Case,
    band: (i128, i128),
    reported: i128,
    nav: i128,
    usdc_supply_index: i128,
    liquidations: u32,
}

fn setup(case: Case, nav_cents: i128) -> Z {
    assert!(
        i128::from(case.threshold) * case.band_max_bps < BPS * case.band_min_bps,
        "{}: LT * max < min keeps every healthy account solvent at the band floor",
        case.name
    );
    let top = usd_cents(nav_cents) * case.band_max_bps / BPS;
    let t = LendingTest::new()
        .with_market(MarketPreset {
            initial_liquidity: 50_000_000.0,
            ..usdc_preset()
        })
        .with_market(xlm_preset())
        .with_rwa_gated_market(MarketPreset {
            name: LIQ,
            decimals: 0,
            price_wad: top,
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
            params: DEFAULT_MARKET_PARAMS.to_market_params(&liq, 0),
        }),
    );
    let spoke_val = gov.execute_immediate(&admin, &AdminOperation::AddSpoke);
    let spoke = u32::try_from_val(&t.env, &spoke_val).unwrap();
    let usdc_cap = 1_000_000_000 * USDC_UNIT;
    for (asset, hub_id, collateral, ltv, threshold, bonus, supply_cap, borrow_cap) in [
        (
            liq.clone(),
            hub,
            true,
            case.ltv,
            case.threshold,
            case.bonus,
            10_000_000,
            0,
        ),
        (
            usdc.clone(),
            HARNESS_HUB,
            false,
            7_500,
            8_000,
            500,
            usdc_cap,
            usdc_cap,
        ),
    ] {
        gov.execute_immediate(
            &admin,
            &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
                hub_id,
                asset,
                spoke_id: spoke,
                can_collateral: collateral,
                can_borrow: !collateral,
                paused: false,
                frozen: false,
                no_seize: false,
                ltv,
                threshold,
                bonus,
                liquidation_fees: 0,
                supply_cap,
                borrow_cap,
            }),
        );
    }
    gov.execute_immediate(
        &admin,
        &AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs {
            spoke_id: spoke,
            target_hf_wad: case.curve.target_bps * WAD / BPS,
            hf_for_max_bonus_wad: case.curve.knee_bps * WAD / BPS,
            liquidation_bonus_factor_bps: case.curve.factor_bps,
        }),
    );

    let mut z = Z {
        t,
        hub,
        spoke,
        liq,
        usdc,
        case,
        band: (0, top),
        reported: top,
        nav: top,
        usdc_supply_index: 0,
        liquidations: 0,
    };
    let gate = RwaGatedTokenClient::new(&z.t.env, &z.liq);
    gate.set_gate_active(&true);
    let keeper = z.t.get_or_create_user("keeper");
    gate.add_to_allowlist(&vec![&z.t.env, pool, keeper]);
    z.move_band(top);
    z.usdc_supply_index = z.usdc_supply_index_now();
    z
}

impl Z {
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

    fn usdc_supply_index_now(&self) -> i128 {
        self.t
            .ctrl_client()
            .get_market_indexes_detailed(&vec![&self.t.env, self.usdc_key()])
            .get(0)
            .unwrap()
            .supply_index
    }

    /// Governance moves the band to `[top * min / max, top]` and the true NAV
    /// to `top`. The single-source width check admits the band.
    fn move_band(&mut self, top: i128) {
        let top = top - top % FEED_STEP_WAD;
        let step = self.case.band_max_bps * FEED_STEP_WAD;
        let floor = (top * self.case.band_min_bps + step - 1) / step * FEED_STEP_WAD;
        self.t.set_price_keeping_sanity_band(LIQ, top);
        let key = PriceKey::Token(self.liq.clone());
        let oracle = self.t.price_agg_client().oracle(&key).unwrap();
        let admin = self.t.admin();
        if let Err(e) = self.t.gov_client().try_execute_immediate(
            &admin,
            &AdminOperation::ConfigureAssetOracle(ConfigureAssetOracleArgs {
                key,
                oracle: AssetOracle {
                    min_sanity_price_wad: floor,
                    max_sanity_price_wad: top,
                    ..oracle
                },
            }),
        ) {
            panic!("{}: band [{floor}, {top}] refused: {e:?}", self.case.name);
        }
        self.band = (floor, top);
        self.nav = top;
        self.reported = top;
    }

    fn report(&mut self, price_wad: i128) {
        let price_wad = price_wad - price_wad % FEED_STEP_WAD;
        assert!(
            (self.band.0..=self.band.1).contains(&price_wad),
            "reported prices stay in the band"
        );
        self.t.set_price_keeping_sanity_band(LIQ, price_wad);
        self.reported = price_wad;
    }

    fn exists(&self, id: u64) -> bool {
        self.t.ctrl_client().account_exists(&id)
    }

    fn units(&self, id: u64) -> i128 {
        if !self.exists(id) {
            return 0;
        }
        self.t
            .ctrl_client()
            .get_collateral_amount(&id, &self.liq_key())
    }

    fn debt(&self, id: u64) -> i128 {
        if !self.exists(id) {
            return 0;
        }
        self.t
            .ctrl_client()
            .get_borrow_amount(&id, &self.usdc_key())
    }

    fn usdc_balance(&self, who: &Address) -> i128 {
        token::Client::new(&self.t.env, &self.usdc).balance(who)
    }

    fn open(&mut self, name: &str, units: i128) -> u64 {
        let who = self.t.get_or_create_user(name);
        let gate = RwaGatedTokenClient::new(&self.t.env, &self.liq);
        gate.add_to_allowlist(&vec![&self.t.env, who.clone()]);
        gate.mint(&who, &units);
        self.t.ctrl_client().supply(
            &who,
            &0,
            &self.spoke,
            &vec![&self.t.env, (self.liq_key(), units)],
        )
    }

    /// Borrows up to the LTV limit at the reported price.
    fn borrow_max(&mut self, owner: &str, id: u64) {
        if self.units(id) < MIN_WHOLE_UNIT_COLLATERAL {
            return;
        }
        let limit = value_raw(self.units(id), self.reported) * i128::from(self.case.ltv) / BPS;
        let amount = limit - self.debt(id) - 100;
        if amount < USDC_UNIT || limit * WAD_PER_USDC_RAW < DEFAULT_MIN_BORROW_COLLATERAL_USD_WAD {
            return;
        }
        let who = self.t.get_or_create_user(owner);
        self.t.ctrl_client().borrow(
            &who,
            &id,
            &vec![&self.t.env, (self.usdc_key(), amount)],
            &None,
        );
    }

    fn liquidatable(&self, id: u64) -> bool {
        self.debt(id) > 0 && self.t.ctrl_client().get_health_factor(&id) < WAD
    }

    /// One liquidation that offers the whole debt; checks I1 and I2 on it.
    /// The receiver is zero for Transfer.
    fn liquidate(&mut self, id: u64, mode: SeizeMode) -> Outcome {
        let (p, nav, name) = (self.reported, self.nav, self.case.name);
        let hf = self.t.ctrl_client().get_health_factor(&id);
        let (n0, d0) = (self.units(id), self.debt(id));
        let bonus = curve_bonus_bps(&self.case, hf);
        let payment = vec![&self.t.env, (self.usdc_key(), d0 + 10)];
        let quoted = self
            .t
            .ctrl_client()
            .get_liquidation_estimate(&id, &payment, &SeizeMode::Transfer)
            .bonus_rate_bps;
        assert!(
            (quoted - bonus).abs() <= 1,
            "{name}: the quote applies the curve bonus at the reported HF {hf}: {quoted} vs {bonus}"
        );

        let keeper = self.t.get_or_create_user("keeper");
        self.t
            .resolve_market("USDC")
            .token_admin
            .mint(&keeper, &(d0 + 10));
        let before = self.usdc_balance(&keeper);
        let receiver = match self
            .t
            .ctrl_client()
            .try_liquidate(&keeper, &id, &payment, &mode)
        {
            Ok(Ok(receiver)) => receiver,
            other => panic!("{name}: liquidation of {n0} shares for {d0} at {p} failed: {other:?}"),
        };
        self.liquidations += 1;
        let paid = before - self.usdc_balance(&keeper);
        let (n1, d1) = (self.units(id), self.debt(id));
        let repaid = d0 - d1;

        assert!(
            paid >= repaid,
            "{name}: debt retired without payment: paid {paid}, retired {repaid}"
        );
        assert!(
            n1 > 0 || d1 == 0,
            "{name}: no debt stays without collateral"
        );

        let unit = value_raw(1, nav);
        let loss = (n0 - n1) * unit - repaid;
        let equity = n0 * unit - d0;
        let full_close = if d1 == 0 { unit } else { 0 };
        let seized_bound = paid * (BPS + bonus.max(quoted)) * nav;
        let bound = (seized_bound + BPS * p - 1) / (BPS * p) - repaid + full_close;
        assert!(
            loss <= equity,
            "{name}: borrower loss {loss} above equity {equity} at true NAV"
        );
        assert!(
            loss <= bound,
            "{name}: borrower loss {loss} above the curve bound {bound} (bonus {bonus}, paid {paid})"
        );
        Outcome {
            receiver,
            loss,
            equity,
        }
    }

    /// Liquidates every liquidatable account at the reported price until it is
    /// healthy or closed. Odd accounts liquidate in Credit mode when `credit`
    /// is set. Returns the credit receivers.
    fn liquidate_all(&mut self, accounts: &[(String, u64)], credit: bool) -> Vec<u64> {
        let mut receivers = Vec::new();
        for (i, (_, id)) in accounts.iter().enumerate() {
            for _ in 0..6 {
                if !self.liquidatable(*id) {
                    break;
                }
                let mode = if credit && i % 2 == 1 {
                    SeizeMode::Credit(0)
                } else {
                    SeizeMode::Transfer
                };
                let receiver = self.liquidate(*id, mode).receiver;
                if receiver != 0 {
                    receivers.push(receiver);
                }
            }
            assert!(
                !self.liquidatable(*id),
                "{}: account {id} stays liquidatable",
                self.case.name
            );
        }
        receivers
    }

    /// Reports the band top, then steps down to the floor and liquidates at
    /// each step.
    fn staircase(&mut self, accounts: &[(String, u64)], credit: bool) -> Vec<u64> {
        let mut receivers = Vec::new();
        let (floor, top) = self.band;
        for step in 0..4 {
            self.report(top - (top - floor) * step / 3);
            receivers.extend(self.liquidate_all(accounts, credit));
        }
        receivers
    }

    /// Governance moves the band so each account is healthy at true NAV (the
    /// band top) and at reported HF 0.98 at the band floor, then the feed
    /// reports the floor. The accounts hold equal positions.
    fn move_to_floor_hf(&mut self, ids: &[u64]) {
        let lt = i128::from(self.case.threshold);
        let (units, debt) = (self.units(ids[0]), self.debt(ids[0]));
        let floor = debt * WAD_PER_USDC_RAW * 98 / 100 * BPS / (units * lt);
        self.move_band(floor * self.case.band_max_bps / self.case.band_min_bps);
        self.report(self.band.0);
        for id in ids {
            assert!(
                self.true_hf(*id) >= WAD,
                "{}: healthy at true NAV",
                self.case.name
            );
            let hf = self.t.ctrl_client().get_health_factor(id);
            assert!(
                hf < WAD && hf > WAD * 97 / 100,
                "{}: reported HF {hf}",
                self.case.name
            );
        }
    }

    fn true_hf(&self, id: u64) -> i128 {
        value_raw(self.units(id), self.nav) * i128::from(self.case.threshold) / BPS * WAD
            / self.debt(id)
    }

    /// Opens one account per size, each borrowed to the LTV limit.
    fn open_borrowers(&mut self, sizes: &[i128]) -> Vec<(String, u64)> {
        let mut accounts = Vec::new();
        for (i, units) in sizes.iter().enumerate() {
            let name = format!("borrower{i}");
            let id = self.open(&name, *units);
            self.borrow_max(&name, id);
            accounts.push((name, id));
        }
        accounts
    }

    /// I1: no loss reached USDC lenders, and every open debt is backed at the
    /// band floor and at true NAV.
    fn assert_lenders_whole(&self, accounts: &[(String, u64)]) {
        let name = self.case.name;
        assert_eq!(
            self.usdc_supply_index_now(),
            self.usdc_supply_index,
            "{name}: the USDC supply index is unchanged"
        );
        for (_, id) in accounts {
            let (units, debt) = (self.units(*id), self.debt(*id));
            assert!(
                debt <= value_raw(units, self.band.0),
                "{name}: account {id} debt {debt} is not backed at the band floor"
            );
            assert!(
                debt <= value_raw(units, self.nav),
                "{name}: account {id} debt {debt} is not backed at true NAV"
            );
        }
    }
}

const SIZES: [i128; 7] = [2, 2, 3, 3, 5, 8, 40];

/// Borrow at the band top and jump to the floor; borrow again at the top
/// against the survivors and the credited shares and step down to the floor;
/// borrow again at the top and jump to the floor.
fn top_borrow_floor_liquidation(z: &mut Z, sizes: &[i128]) {
    let mut accounts = z.open_borrowers(sizes);
    z.report(z.band.0);
    let mut receivers = z.liquidate_all(&accounts, true);
    for round in 0..2 {
        accounts.extend(receivers.drain(..).map(|id| (String::from("keeper"), id)));
        z.report(z.band.1);
        for (owner, id) in accounts.clone() {
            z.borrow_max(&owner, id);
        }
        if round == 0 {
            receivers = z.staircase(&accounts, true);
        } else {
            z.report(z.band.0);
            z.liquidate_all(&accounts, false);
        }
    }
    z.assert_lenders_whole(&accounts);

    let (ltv, lt) = (i128::from(z.case.ltv), i128::from(z.case.threshold));
    assert_eq!(
        z.liquidations > 0,
        lt * z.band.0 < ltv * z.band.1,
        "{}: a band-top borrow is liquidatable at the floor iff LT / LTV < max / min",
        z.case.name
    );
}

/// I1 and I2 with the true NAV at the band top: the floor report undervalues
/// the shares by the full band ratio.
#[test]
fn lqv_oracle_bound_nav_at_band_top() {
    for case in CASES {
        let mut z = setup(case, THOUSAND_DOLLARS);
        top_borrow_floor_liquidation(&mut z, &SIZES);

        let mut z = setup(case, ONE_DOLLAR);
        top_borrow_floor_liquidation(&mut z, &[10_000, 25]);
    }
}

/// I1 and I2 with the true NAV at the band floor: the top report lets every
/// account borrow against an overvalued share.
#[test]
fn lqv_oracle_bound_nav_at_band_floor() {
    for case in CASES {
        let mut z = setup(case, THOUSAND_DOLLARS);
        z.nav = z.band.0;
        top_borrow_floor_liquidation(&mut z, &SIZES);
    }
}

/// The NAV falls until each account sits at true HF 1.01, governance moves the
/// band down to the new NAV, and the feed jumps to the new floor. Where the
/// curve bonus reaches the HF-preserving cap, one liquidation takes the whole
/// equity at true NAV; the lenders still lose nothing.
#[test]
fn lqv_oracle_bound_markdown_to_hf_one_then_band_floor() {
    let mut at_cap = 0;
    for case in CASES {
        let mut z = setup(case, THOUSAND_DOLLARS);
        let mut accounts = z.open_borrowers(&SIZES);
        let (ltv, lt) = (i128::from(case.ltv), i128::from(case.threshold));
        z.move_band(z.band.1 * ltv * 101 / (lt * 100));
        for (_, id) in &accounts {
            assert!(z.true_hf(*id) >= WAD, "{}: healthy at true NAV", case.name);
        }
        z.report(z.band.0);
        for (i, (_, id)) in accounts.clone().iter().enumerate() {
            let hf = z.t.ctrl_client().get_health_factor(id);
            assert!(hf < WAD, "{}: the floor report liquidates", case.name);
            let mode = if i % 2 == 1 {
                SeizeMode::Credit(0)
            } else {
                SeizeMode::Transfer
            };
            let out = z.liquidate(*id, mode);
            if curve_bonus_bps(&case, hf) == hf_cap_bps(&case, hf) {
                at_cap += 1;
                assert_eq!(out.loss, out.equity, "{}", case.name);
                assert!(!z.exists(*id), "{}", case.name);
            }
            if out.receiver != 0 {
                accounts.push(("keeper".into(), out.receiver));
            }
        }
        z.liquidate_all(&accounts, false);
        z.assert_lenders_whole(&accounts);
    }
    assert!(at_cap > 0, "a floor report reaches the HF-preserving cap");
}

/// Opens one two-share account per seize mode, each borrowed to the LTV
/// limit, and moves the band to reported HF 0.98 at its floor.
fn whole_unit_setup(case: Case) -> (Z, [(SeizeMode, u64); 2]) {
    let mut z = setup(case, THOUSAND_DOLLARS);
    let transfer = z.open("transfer", 2);
    z.borrow_max("transfer", transfer);
    let credit = z.open("credit", 2);
    z.borrow_max("credit", credit);
    z.move_to_floor_hf(&[transfer, credit]);
    (
        z,
        [
            (SeizeMode::Transfer, transfer),
            (SeizeMode::Credit(0), credit),
        ],
    )
}

/// I3: a two-share account at reported HF 0.98 sells exactly one share at
/// `unit / (1 + b)`, where `b` is the curve bonus at the reported HF, not the
/// 5% base bonus.
#[test]
fn lqv_oracle_bound_one_unit_sale() {
    for case in CASES {
        let (mut z, accounts) = whole_unit_setup(case);
        let hf = z.t.ctrl_client().get_health_factor(&accounts[0].1);
        let bonus = curve_bonus_bps(&case, hf);
        if case.curve.factor_bps == DEFAULT_CURVE.factor_bps {
            assert!(
                bonus > 5 * i128::from(case.bonus),
                "{}: the default curve pays {bonus} bps at HF {hf}",
                case.name
            );
        }
        for (mode, id) in accounts {
            let debt = z.debt(id);
            z.liquidate(id, mode);
            assert_eq!(z.units(id), 1, "{}: exactly one share is sold", case.name);
            assert!(z.debt(id) > 0 && z.debt(id) < debt, "{}", case.name);
            assert!(!z.liquidatable(id), "{}", case.name);
        }
        z.assert_lenders_whole(&accounts.map(|(_, id)| (String::new(), id)));
    }
}

/// I3: the one-share residue of a one-unit sale, moved to reported HF 0.98,
/// closes in full. The liquidator repays the whole debt for the one share, so
/// the borrower loses exactly its equity at true NAV and the lenders lose
/// nothing.
#[test]
fn lqv_oracle_bound_full_close_by_one_unit() {
    for case in CASES {
        let (mut z, accounts) = whole_unit_setup(case);
        for (mode, id) in accounts {
            z.liquidate(id, mode);
        }
        z.move_to_floor_hf(&accounts.map(|(_, id)| id));
        for (mode, id) in accounts {
            assert_eq!(z.units(id), 1, "{}", case.name);
            let out = z.liquidate(id, mode);
            assert!(!z.exists(id), "{}: the account closes", case.name);
            assert_eq!(out.loss, out.equity, "{}", case.name);
            if out.receiver != 0 {
                assert_eq!(z.units(out.receiver), 1, "{}", case.name);
            }
        }
        z.assert_lenders_whole(&[]);
    }
}
