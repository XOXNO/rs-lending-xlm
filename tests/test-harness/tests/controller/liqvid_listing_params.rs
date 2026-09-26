//! Liqvid listing parameters for a daily-NAV deal share with 0 decimals
//! (`docs/reference/runbooks/liqvid-listing-params.md`): LTV 50%, LT 53%,
//! base bonus 5%, a spoke curve with target HF 1.06, the full bonus at
//! HF 0.90 and factor 598 bps, and one Xoxno NAV feed whose sanity band is
//! [0.849, 1.03] x the reference NAV with a 93,600 s staleness budget. The
//! share sits behind the Asterizm gate (`RwaGatedToken`) and borrows USDC.
//!
//! Tests prefixed `lqv_params_`.

use common::types::{AssetOracle, HubAssetKey, PriceKey, PriceSource, SeizeMode};
use controller::constants::{RAY, WAD};
use governance::op::{
    AdminOperation, ConfigureAssetOracleArgs, CreatePoolArgs, SpokeAssetArgs,
    SpokeLiquidationCurveArgs,
};
use soroban_sdk::{token, vec, Address, Error, String, TryFromVal};
use test_harness::errors::OracleError;
use test_harness::mock_redstone::MockRedStonePriceFeedClient;
use test_harness::oracle::redstone::register_redstone_adapter;
use test_harness::rwa_gated_token::RwaGatedTokenClient;
use test_harness::{
    errors, usd_cents, usdc_preset, xoxno_single_config, AssetConfigPreset, LendingTest,
    MarketPreset, DEFAULT_ASSET_CONFIG, DEFAULT_MARKET_PARAMS, DEFAULT_TOLERANCE, HARNESS_HUB,
};

const LIQ: &str = "LIQVID";
const NAV_FEED: &str = "LIQVID1039";
const USDC_UNIT: i128 = 10_000_000;
const USDC_CAP_RAW: i128 = 1_000_000_000 * USDC_UNIT;

const LTV: u32 = 5_000;
const LT: u32 = 5_300;
const PREVIOUS_LT: u32 = 6_000;
const BASE_BONUS: u32 = 500;
const TARGET_HF: i128 = WAD * 106 / 100;
const FULL_BONUS_HF: i128 = WAD * 90 / 100;
const BONUS_FACTOR: u32 = 598;

const BAND_FLOOR_PER_MILLE: i128 = 849;
const BAND_CEILING_PER_MILLE: i128 = 1_030;
const NAV_MAX_STALE_SECONDS: u64 = 93_600;
const MAX_COLLATERAL_USD: i128 = 1_000_000;

const BONUS_AT_HF_ONE: i128 = 688;
const MAX_BONUS: i128 = 1_000;

struct Params {
    t: LendingTest,
    spoke: u32,
    hub: u32,
    liq: Address,
    usdc: Address,
    pool: Address,
    adapter: Address,
    nav_ref: i128,
}

/// Supply cap in whole shares for `MAX_COLLATERAL_USD` at the band ceiling.
fn supply_cap_units(nav_ref: i128) -> i128 {
    MAX_COLLATERAL_USD * WAD * 1_000 / (nav_ref * BAND_CEILING_PER_MILLE)
}

/// The share's spoke listing with liquidation threshold `threshold`.
fn liq_listing(
    hub: u32,
    liq: &Address,
    spoke: u32,
    nav_ref: i128,
    threshold: u32,
) -> SpokeAssetArgs {
    SpokeAssetArgs {
        hub_id: hub,
        asset: liq.clone(),
        spoke_id: spoke,
        can_collateral: true,
        can_borrow: false,
        paused: false,
        frozen: false,
        no_seize: false,
        ltv: LTV,
        threshold,
        bonus: BASE_BONUS,
        liquidation_fees: 0,
        supply_cap: supply_cap_units(nav_ref),
        borrow_cap: 0,
    }
}

/// Lists the share at `nav_cents` per share with the listing parameters and
/// opens the gate with only the pool allowlisted.
fn setup(nav_cents: i128) -> Params {
    setup_with_threshold(nav_cents, LT)
}

/// `setup` with the share listed at liquidation threshold `threshold`.
fn setup_with_threshold(nav_cents: i128, threshold: u32) -> Params {
    let nav_ref = usd_cents(nav_cents);
    let t = LendingTest::new()
        .with_market(MarketPreset {
            initial_liquidity: 50_000_000.0,
            ..usdc_preset()
        })
        .with_rwa_gated_market(MarketPreset {
            name: LIQ,
            decimals: 0,
            price_wad: nav_ref,
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
    gov.execute_immediate(
        &admin,
        &AdminOperation::AddAssetToSpoke(liq_listing(hub, &liq, spoke, nav_ref, threshold)),
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
    gov.execute_immediate(
        &admin,
        &AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs {
            spoke_id: spoke,
            target_hf_wad: TARGET_HF,
            hf_for_max_bonus_wad: FULL_BONUS_HF,
            liquidation_bonus_factor_bps: BONUS_FACTOR,
        }),
    );

    let adapter = register_redstone_adapter(&t, &[(NAV_FEED, nav_ref)]);
    let p = Params {
        t,
        spoke,
        hub,
        liq,
        usdc,
        pool,
        adapter,
        nav_ref,
    };
    p.configure_oracle(p.nav_oracle())
        .expect("the listing oracle passes admission");
    p.token().set_gate_active(&true);
    p.allow(&p.pool.clone());
    p
}

impl Params {
    fn token(&self) -> RwaGatedTokenClient<'_> {
        RwaGatedTokenClient::new(&self.t.env, &self.liq)
    }

    fn allow(&self, who: &Address) {
        self.token()
            .add_to_allowlist(&vec![&self.t.env, who.clone()]);
    }

    fn feed_id(&self) -> String {
        String::from_str(&self.t.env, NAV_FEED)
    }

    /// The single Xoxno NAV feed, the band around `nav_ref` and the
    /// staleness budget of the listing.
    fn nav_oracle(&self) -> AssetOracle {
        let key = PriceKey::Token(self.liq.clone());
        let mut oracle = self.t.price_agg_client().oracle(&key).unwrap();
        let xoxno = xoxno_single_config(
            &self.t.env,
            &self.adapter,
            &self.feed_id(),
            self.nav_ref,
            DEFAULT_TOLERANCE.tolerance_bps,
        );
        let PriceSource::Feed(mut feed) = xoxno.sources.get(0).unwrap() else {
            panic!("a Xoxno source is a plain feed");
        };
        feed.max_stale_seconds = NAV_MAX_STALE_SECONDS;
        oracle.sources = vec![&self.t.env, PriceSource::Feed(feed)];
        oracle.max_price_stale_seconds = NAV_MAX_STALE_SECONDS;
        oracle.min_sanity_price_wad = self.nav_ref * BAND_FLOOR_PER_MILLE / 1_000;
        oracle.max_sanity_price_wad = self.nav_ref * BAND_CEILING_PER_MILLE / 1_000;
        oracle
    }

    fn configure_oracle(&self, oracle: AssetOracle) -> Result<(), Error> {
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
            Err(Err(_)) => panic!("oracle configuration hit a host error"),
        }
    }

    /// Posts a NAV per share signed now.
    fn post_nav(&self, nav_wad: i128) {
        MockRedStonePriceFeedClient::new(&self.t.env, &self.adapter)
            .set_price(&self.feed_id(), &nav_wad);
    }

    /// Posts a NAV per share signed `age_seconds` ago.
    fn post_nav_aged(&self, nav_wad: i128, age_seconds: u64) {
        let signed_ms = (self.t.env.ledger().timestamp() - age_seconds) * 1_000;
        MockRedStonePriceFeedClient::new(&self.t.env, &self.adapter).set_price_data(
            &self.feed_id(),
            &nav_wad,
            &signed_ms,
            &signed_ms,
        );
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

    fn liq_balance(&self, who: &Address) -> i128 {
        token::Client::new(&self.t.env, &self.liq).balance(who)
    }

    fn usdc_balance(&self, who: &Address) -> i128 {
        token::Client::new(&self.t.env, &self.usdc).balance(who)
    }

    fn try_supply(&mut self, name: &str, units: i128) -> Result<u64, Error> {
        let who = self.user(name);
        self.allow(&who);
        self.token().mint(&who, &units);
        match self.t.ctrl_client().try_supply(
            &who,
            &0,
            &self.spoke,
            &vec![&self.t.env, (self.liq_key(), units)],
        ) {
            Ok(Ok(id)) => Ok(id),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(e.expect("contract error")),
        }
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

    /// Supplies `units` shares at the posted `nav_wad` and borrows the LTV
    /// maximum of their value.
    fn open_at_max_ltv(&mut self, name: &str, units: i128, nav_wad: i128) -> u64 {
        let id = self
            .try_supply(name, units)
            .unwrap_or_else(|e| panic!("{name}: supply of {units} shares failed: {e:?}"));
        let max_raw = units * nav_wad * i128::from(LTV) / 10_000 / (WAD / USDC_UNIT);
        self.try_borrow(name, id, max_raw)
            .unwrap_or_else(|e| panic!("{name}: borrow of {max_raw} failed: {e:?}"));
        id
    }

    /// Liquidates with a full-debt offer. Returns the shares received and
    /// the USDC paid.
    fn try_liquidate(&mut self, liquidator: &str, account_id: u64) -> Result<(i128, i128), Error> {
        let who = self.user(liquidator);
        self.allow(&who);
        let offer = self.debt_raw(account_id);
        self.t.resolve_market("USDC").token_admin.mint(&who, &offer);
        let shares_before = self.liq_balance(&who);
        let usdc_before = self.usdc_balance(&who);
        match self.t.ctrl_client().try_liquidate(
            &who,
            &account_id,
            &vec![&self.t.env, (self.usdc_key(), offer)],
            &SeizeMode::Transfer,
        ) {
            Ok(Ok(_)) => Ok((
                self.liq_balance(&who) - shares_before,
                usdc_before - self.usdc_balance(&who),
            )),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(e.expect("contract error")),
        }
    }

    fn quoted_bonus_bps(&self, account_id: u64) -> i128 {
        self.t
            .ctrl_client()
            .get_liquidation_estimate(
                &account_id,
                &vec![&self.t.env, (self.usdc_key(), self.debt_raw(account_id))],
                &SeizeMode::Transfer,
            )
            .bonus_rate_bps
    }

    fn units(&self, account_id: u64) -> i128 {
        self.t
            .ctrl_client()
            .get_collateral_amount(&account_id, &self.liq_key())
    }

    fn debt_raw(&self, account_id: u64) -> i128 {
        self.t
            .ctrl_client()
            .get_borrow_amount(&account_id, &self.usdc_key())
    }

    fn hf(&self, account_id: u64) -> i128 {
        self.t.ctrl_client().get_health_factor(&account_id)
    }

    fn liquidatable(&self, account_id: u64) -> bool {
        self.t.ctrl_client().is_liquidatable(&account_id)
    }

    fn stored_lt(&self, account_id: u64) -> u32 {
        self.t
            .ctrl_client()
            .get_account_positions(&account_id)
            .0
            .get(self.liq_key())
            .unwrap()
            .liquidation_threshold
    }
}

fn contract_error(code: u32) -> Error {
    Error::from_contract_error(code)
}

/// NAV per share, in WAD, that puts a max-LTV account opened at `nav_ref`
/// at health factor `hf_bps`.
fn nav_for_hf(nav_ref: i128, hf_bps: i128) -> i128 {
    nav_ref * hf_bps * i128::from(LTV) / (i128::from(LT) * 10_000)
}

/// Effective liquidator bonus in bps: share value received over USDC paid.
fn effective_bonus_bps(shares: i128, nav_wad: i128, paid_raw: i128) -> i128 {
    shares * nav_wad * 10_000 / (paid_raw * (WAD / USDC_UNIT)) - 10_000
}

/// The listing's band and staleness pass oracle admission at their on-chain
/// caps; a band past the 10% single-source width and a budget past 26 hours
/// are refused. The supply cap holds at most $1M of shares at the ceiling.
#[test]
fn lqv_params_listing_oracle_config_sits_inside_the_on_chain_caps() {
    let mut p = setup(100);
    let stored =
        p.t.price_agg_client()
            .oracle(&PriceKey::Token(p.liq.clone()))
            .unwrap();
    assert_eq!(stored.max_price_stale_seconds, NAV_MAX_STALE_SECONDS);
    assert_eq!(stored.min_sanity_price_wad, usd_cents(100) * 849 / 1_000);
    assert_eq!(stored.max_sanity_price_wad, usd_cents(100) * 1_030 / 1_000);

    let too_wide = AssetOracle {
        min_sanity_price_wad: usd_cents(80),
        ..p.nav_oracle()
    };
    assert_eq!(
        p.configure_oracle(too_wide),
        Err(contract_error(
            OracleError::SanityBandTooWideForSingleSource as u32
        ))
    );
    let too_stale = AssetOracle {
        max_price_stale_seconds: NAV_MAX_STALE_SECONDS + 1,
        ..p.nav_oracle()
    };
    assert_eq!(
        p.configure_oracle(too_stale),
        Err(contract_error(OracleError::InvalidStalenessConfig as u32))
    );

    let cap = supply_cap_units(p.nav_ref);
    assert_eq!(cap, 970_873);
    assert!(cap * p.nav_ref * BAND_CEILING_PER_MILLE / 1_000 <= MAX_COLLATERAL_USD * WAD);
    p.try_supply("whale", cap)
        .expect("the cap itself is admitted");
    assert_eq!(
        p.try_supply("late", 1),
        Err(contract_error(errors::SPOKE_SUPPLY_CAP_REACHED))
    );
}

/// An account opened at max LTV becomes liquidatable after a 5.66% NAV drop,
/// which the band accepts. The liquidation pays the bonus at HF just below 1
/// and restores the target HF.
#[test]
fn lqv_params_max_ltv_account_turns_liquidatable_inside_the_band() {
    let mut p = setup(100);
    let id = p.open_at_max_ltv("alice", 10_000, p.nav_ref);
    assert_eq!(p.hf(id), TARGET_HF, "max LTV opens at HF = LT / LTV");

    p.post_nav(usd_cents(100) * 944 / 1_000);
    assert!(p.hf(id) >= WAD);
    assert!(!p.liquidatable(id), "a 5.6% drop leaves HF at or above 1");

    let nav = usd_cents(100) * 943 / 1_000;
    p.post_nav(nav);
    assert!(nav >= p.nav_ref * BAND_FLOOR_PER_MILLE / 1_000);
    assert!(
        p.hf(id) < WAD && p.liquidatable(id),
        "a 5.7% drop trips HF 1"
    );
    let bonus = p.quoted_bonus_bps(id);
    assert!(
        (BONUS_AT_HF_ONE..=BONUS_AT_HF_ONE + 3).contains(&bonus),
        "bonus just below HF 1: {bonus}"
    );

    let debt = p.debt_raw(id);
    let (shares, paid) = p.try_liquidate("liquidator", id).unwrap();
    assert!(shares > 0 && paid > 0 && paid < debt);
    assert_eq!(p.units(id) + shares, 10_000, "shares are conserved");
    assert_eq!(p.liq_balance(&p.pool), p.units(id));
    let effective = effective_bonus_bps(shares, nav, paid);
    assert!((effective - bonus).abs() <= 2, "{effective} vs {bonus}");
    let post = p.hf(id);
    assert!(
        (WAD * 1_055 / 1_000..=WAD * 1_065 / 1_000).contains(&post),
        "post HF {post}"
    );
}

/// The bonus rises from 7.2% at HF 0.99 to 8.4% at HF 0.95 and 10% at
/// HF 0.90, the band floor for an account opened at the reference NAV.
#[test]
fn lqv_params_bonus_at_hf_099_095_090_stays_in_the_target_range() {
    let mut p = setup(100);
    for (hf_bps, expected) in [(9_900i128, 719i128), (9_500, 844), (9_000, 1_000)] {
        p.post_nav(p.nav_ref);
        let name = format!("borrower-{hf_bps}");
        let id = p.open_at_max_ltv(&name, 10_000, p.nav_ref);
        let nav = nav_for_hf(p.nav_ref, hf_bps);
        assert!(
            nav >= p.nav_ref * BAND_FLOOR_PER_MILLE / 1_000,
            "HF {hf_bps} is inside the band"
        );
        p.post_nav(nav);
        let hf = p.hf(id);
        assert!(
            (hf - hf_bps * WAD / 10_000).abs() <= WAD / 10_000,
            "HF {hf} for target {hf_bps}"
        );
        let bonus = p.quoted_bonus_bps(id);
        assert!(
            (bonus - expected).abs() <= 1,
            "HF {hf_bps}: bonus {bonus}, expected {expected}"
        );
        assert!((BONUS_AT_HF_ONE..=MAX_BONUS).contains(&bonus));

        let (shares, paid) = p.try_liquidate("liquidator", id).unwrap();
        let effective = effective_bonus_bps(shares, nav, paid);
        assert!(
            (effective - bonus).abs() <= 2,
            "HF {hf_bps}: effective {effective} vs quoted {bonus}"
        );
        let post = p.hf(id);
        assert!(
            (WAD * 1_055 / 1_000..=WAD * 1_065 / 1_000).contains(&post),
            "HF {hf_bps}: post HF {post}"
        );
    }
}

/// Every NAV step from the HF-1 trigger down to the band floor liquidates a
/// max-LTV account; one step past either band edge fails closed for
/// liquidation and borrowing, and the edges themselves price.
#[test]
fn lqv_params_nav_sweep_liquidates_inside_the_band_and_fails_closed_outside() {
    let mut p = setup(100);
    let floor = p.nav_ref * BAND_FLOOR_PER_MILLE / 1_000;
    let ceiling = p.nav_ref * BAND_CEILING_PER_MILLE / 1_000;
    for per_mille in [943i128, 930, 920, 910, 900, 890, 880, 870, 860, 850, 849] {
        p.post_nav(p.nav_ref);
        let name = format!("borrower-{per_mille}");
        let id = p.open_at_max_ltv(&name, 10_000, p.nav_ref);
        let nav = p.nav_ref * per_mille / 1_000;
        p.post_nav(nav);
        assert!(p.liquidatable(id), "NAV {per_mille}/1000");
        let bonus = p.quoted_bonus_bps(id);
        assert!(
            (BONUS_AT_HF_ONE..=MAX_BONUS).contains(&bonus),
            "NAV {per_mille}/1000: bonus {bonus}"
        );
        p.try_liquidate("liquidator", id)
            .unwrap_or_else(|e| panic!("NAV {per_mille}/1000: {e:?}"));
        assert!(p.hf(id) > WAD, "NAV {per_mille}/1000: HF restored");
    }

    p.post_nav(p.nav_ref);
    let id = p.open_at_max_ltv("edge", 10_000, p.nav_ref);
    let debt = p.debt_raw(id);
    p.post_nav(floor - usd_cents(1) / 100);
    assert_eq!(
        p.try_liquidate("liquidator", id),
        Err(contract_error(errors::SANITY_BOUND_VIOLATED))
    );
    assert_eq!((p.units(id), p.debt_raw(id)), (10_000, debt));
    p.post_nav(floor);
    p.try_liquidate("liquidator", id)
        .expect("the band floor itself prices");

    p.post_nav(ceiling + usd_cents(1) / 100);
    let id = p
        .try_supply("upper", 10_000)
        .expect("supply needs no price");
    assert_eq!(
        p.try_borrow("upper", id, 100 * USDC_UNIT),
        Err(contract_error(errors::SANITY_BOUND_VIOLATED))
    );
    p.post_nav(ceiling);
    p.try_borrow("upper", id, 100 * USDC_UNIT)
        .expect("the band ceiling itself prices");
}

/// A NAV signed up to 26 hours ago still prices; one second more fails closed.
#[test]
fn lqv_params_nav_prices_until_the_26_hour_staleness_budget() {
    let mut p = setup(100);
    p.t.supply("lender", "USDC", 1_000_000.0);
    let id = p.open_at_max_ltv("alice", 10_000, p.nav_ref);
    p.t.advance_time(2 * 86_400);
    let nav = p.nav_ref * 930 / 1_000;

    p.post_nav_aged(nav, NAV_MAX_STALE_SECONDS + 1);
    assert_eq!(
        p.try_liquidate("liquidator", id),
        Err(contract_error(errors::PRICE_FEED_STALE))
    );
    assert_eq!(
        p.try_borrow("alice", id, USDC_UNIT),
        Err(contract_error(errors::PRICE_FEED_STALE))
    );

    p.post_nav_aged(nav, NAV_MAX_STALE_SECONDS);
    p.try_liquidate("liquidator", id)
        .expect("a NAV exactly 26 hours old prices");
}

/// At $1 per share the $5 minimum LTV collateral keeps accounts below 10
/// shares debt-free, so 1-5 share accounts never reach liquidation. The
/// smallest borrower, 10 shares with $5 of debt, closes in full: the residue
/// would sit below $5, and the seizure rounds up to whole shares.
#[test]
fn lqv_params_one_dollar_accounts_below_ten_shares_cannot_borrow() {
    let mut p = setup(100);
    for units in 1..=9i128 {
        let name = format!("small-{units}");
        let id = p.try_supply(&name, units).unwrap();
        assert_eq!(
            p.try_borrow(&name, id, units * USDC_UNIT / 2),
            Err(contract_error(errors::MIN_BORROW_COLLATERAL_NOT_MET)),
            "{units} shares"
        );
    }
    let id = p.open_at_max_ltv("ten", 10, p.nav_ref);
    assert_eq!(p.debt_raw(id), 5 * USDC_UNIT);

    let nav = p.nav_ref * 930 / 1_000;
    p.post_nav(nav);
    let bonus = p.quoted_bonus_bps(id);
    assert!((BONUS_AT_HF_ONE..=MAX_BONUS).contains(&bonus));
    let (shares, paid) = p.try_liquidate("liquidator", id).unwrap();
    assert_eq!(p.debt_raw(id), 0, "the $5 account closes in full");
    assert_eq!(paid, 5 * USDC_UNIT);
    let fair_shares = paid * (WAD / USDC_UNIT) * (10_000 + bonus) / 10_000;
    assert_eq!(
        shares,
        (fair_shares + nav - 1) / nav,
        "the seizure rounds up to whole shares"
    );
    assert_eq!(p.units(id), 10 - shares);
}

/// At $1,000 per share, a 1-share account cannot borrow, and each 2-5 share
/// account at max LTV sells exactly one share at `NAV / (1 + bonus)` just
/// past the HF-1 trigger and at the band floor, and ends above HF 1.
#[test]
fn lqv_params_two_to_five_thousand_dollar_shares_sell_one_share_inside_the_band() {
    let mut p = setup(100_000);
    let id = p.try_supply("single", 1).unwrap();
    assert_eq!(
        p.try_borrow("single", id, 500 * USDC_UNIT),
        Err(contract_error(errors::MIN_BORROW_COLLATERAL_NOT_MET)),
        "one share cannot carry debt"
    );

    for per_mille in [943i128, 849] {
        for units in 2..=5i128 {
            p.post_nav(p.nav_ref);
            let name = format!("holder-{per_mille}-{units}");
            let id = p.open_at_max_ltv(&name, units, p.nav_ref);
            let nav = p.nav_ref * per_mille / 1_000;
            p.post_nav(nav);
            assert!(p.liquidatable(id), "{units} shares at {per_mille}/1000");
            let bonus = p.quoted_bonus_bps(id);
            let debt = p.debt_raw(id);
            let (shares, paid) = p
                .try_liquidate("liquidator", id)
                .unwrap_or_else(|e| panic!("{units} shares at {per_mille}/1000: {e:?}"));
            assert_eq!(shares, 1, "{units} shares at {per_mille}/1000");
            assert_eq!(p.units(id), units - 1);
            let fair = nav * 10_000 / (10_000 + bonus) / (WAD / USDC_UNIT);
            assert!(
                paid >= fair && paid <= fair + fair / 100_000 + 2,
                "{units} shares at {per_mille}/1000: paid {paid}, fair {fair}"
            );
            assert!(p.debt_raw(id) < debt);
            assert!(
                p.hf(id) > WAD,
                "{units} shares at {per_mille}/1000: HF {}",
                p.hf(id)
            );
            assert_eq!(
                p.t.ctrl_client()
                    .get_account_positions(&id)
                    .0
                    .get(p.liq_key())
                    .unwrap()
                    .scaled_amount
                    % RAY,
                0,
                "the borrower keeps whole shares"
            );
        }
    }
}

/// Borrower loss in USD cents, rounded half up: seized share value at `nav_wad`
/// minus the USDC paid.
fn loss_cents(shares: i128, nav_wad: i128, paid_raw: i128) -> i128 {
    let loss_raw = shares * nav_wad / (WAD / USDC_UNIT) - paid_raw;
    (loss_raw + USDC_UNIT / 200) / (USDC_UNIT / 100)
}

/// 1,000 shares opened at max LTV. One jump to the band floor liquidates to
/// HF 1.06 and costs the borrower $16.75 when it borrowed at $1, $18.37 at
/// $1.015 and $20.07 at the $1.03 ceiling. A path of 1% NAV steps from $1
/// liquidates at 0.94 and 0.88 only, costs $8.76 in total, and leaves HF 1.022
/// at the floor.
#[test]
fn lqv_params_floor_jump_costs_more_than_one_percent_steps() {
    let mut p = setup(100);
    let floor = p.nav_ref * BAND_FLOOR_PER_MILLE / 1_000;
    for (borrow_per_mille, seized, cents) in [
        (1_000i128, 217i128, 1_675i128),
        (1_015, 238, 1_837),
        (1_030, 260, 2_007),
    ] {
        let borrow_nav = p.nav_ref * borrow_per_mille / 1_000;
        p.post_nav(borrow_nav);
        let id = p.open_at_max_ltv(&format!("jump-{borrow_per_mille}"), 1_000, borrow_nav);
        p.post_nav(floor);
        let (shares, paid) = p.try_liquidate("liquidator", id).unwrap();
        assert_eq!(
            (shares, loss_cents(shares, floor, paid)),
            (seized, cents),
            "borrowed at {borrow_per_mille}/1000"
        );
        let post = p.hf(id);
        assert!(
            (WAD * 1_055 / 1_000..=WAD * 1_065 / 1_000).contains(&post),
            "borrowed at {borrow_per_mille}/1000: HF {post} after the jump"
        );
    }

    p.post_nav(p.nav_ref);
    let id = p.open_at_max_ltv("steps", 1_000, p.nav_ref);
    let mut liquidated_at = Vec::new();
    let mut total_cents = 0;
    for per_mille in (850..=990).rev().step_by(10).chain([BAND_FLOOR_PER_MILLE]) {
        let nav = p.nav_ref * per_mille / 1_000;
        p.post_nav(nav);
        if !p.liquidatable(id) {
            continue;
        }
        let (shares, paid) = p.try_liquidate("liquidator", id).unwrap();
        liquidated_at.push(per_mille);
        total_cents += loss_cents(shares, nav, paid);
    }
    assert_eq!(liquidated_at, [940, 880]);
    assert_eq!(total_cents, 876);
    let hf = p.hf(id);
    assert!(
        (WAD * 1_020 / 1_000..=WAD * 1_025 / 1_000).contains(&hf),
        "HF {hf} at the floor after the steps"
    );
}

/// Accounts opened at max LTV under LT 6000 keep it after the listing edit to
/// LT 5300 until `update_account_threshold(has_risks = true)` refreshes them.
/// The refresh applies LT 5300 while the HF with LT 5300 is at least 1.05, at
/// a NAV of 0.9906 R or more. Below that it keeps LT 6000 and succeeds while
/// the HF with LT 6000 is at least 1.05, at 0.875 R or more. Lower, it reverts
/// with `HealthFactorTooLow` and keeps LT 6000.
#[test]
fn lqv_params_restamp_applies_lt_5300_only_above_the_gate() {
    let mut p = setup_with_threshold(100, PREVIOUS_LT);
    let cases = [
        (1_000i128, Ok(()), LT),
        (991, Ok(()), LT),
        (990, Ok(()), PREVIOUS_LT),
        (876, Ok(()), PREVIOUS_LT),
        (
            874,
            Err(contract_error(errors::HEALTH_FACTOR_TOO_LOW)),
            PREVIOUS_LT,
        ),
    ];
    let ids: Vec<u64> = cases
        .iter()
        .map(|(per_mille, _, _)| {
            p.open_at_max_ltv(&format!("restamp-{per_mille}"), 10_000, p.nav_ref)
        })
        .collect();

    let admin = p.t.admin();
    p.t.gov_client().execute_immediate(
        &admin,
        &AdminOperation::EditAssetInSpoke(liq_listing(p.hub, &p.liq, p.spoke, p.nav_ref, LT)),
    );
    for (id, (per_mille, outcome, lt)) in ids.into_iter().zip(cases) {
        assert_eq!(p.stored_lt(id), PREVIOUS_LT, "the edit does not restamp");
        p.post_nav(p.nav_ref * per_mille / 1_000);
        assert_eq!(
            (
                p.t.try_update_account_threshold(true, &[id]),
                p.stored_lt(id)
            ),
            (outcome, lt),
            "NAV {per_mille}/1000"
        );
    }
}
