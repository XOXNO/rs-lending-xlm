use std::collections::BTreeMap;
use std::sync::OnceLock;

use common::types::{
    AccountPositionRaw, DebtPositionRaw, HubAssetKey, LiquidationEstimate, PoolKey, PoolStateRaw,
    PositionMode, SeizeMode,
};
use governance::op::{AdminOperation, CreatePoolArgs, SpokeAssetArgs, SpokeLiquidationCurveArgs};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use serde_json::Value;
use soroban_sdk::testutils::Events;
use soroban_sdk::xdr::{ContractEventBody, ScVal};
use soroban_sdk::{vec, Env, Error, Map as SorobanMap, TryFromVal, Vec as SorobanVec};

use crate::context::LendingTest;
use crate::helpers::HARNESS_HUB;
use crate::ops::internal::{burn_prefund, map_try_ok_unit, map_try_ok_value};
use crate::presets::{AssetConfigPreset, MarketParamsPreset, MarketPreset};
use crate::reference::{
    exact_totals, plan_liquidation_exact, snapshot_exact, ExactBook, ExactPlan, ExactTotals,
    RefCurve,
};

pub const MAINNET_FIXTURE_JSON: &str = include_str!("../fixtures/mainnet-2026-09-28.json");

const MOCK_REFLECTOR_PRICE_UNIT: i128 = 10_000;
const MARKET_LIQUIDITY_USD: f64 = 10_000_000.0;

#[derive(Clone, Debug)]
pub struct MainnetListing {
    pub asset: String,
    pub hub_id: u32,
    pub can_be_collateral: bool,
    pub can_be_borrowed: bool,
    pub ltv: u32,
    pub liquidation_threshold: u32,
    pub liquidation_bonus: u32,
    pub liquidation_fees: u32,
    pub supply_cap: i128,
    pub borrow_cap: i128,
}

#[derive(Clone)]
pub struct MainnetMarket {
    pub name: String,
    pub hub_id: u32,
    pub decimals: u32,
    pub price_wad: i128,
    pub min_sanity_price_wad: i128,
    pub max_sanity_price_wad: i128,
    pub params: MarketParamsPreset,
    pub is_flashloanable: bool,
    pub flashloan_fee: u32,
}

#[derive(Clone, Debug)]
pub struct MainnetSpokeConfig {
    pub config_id: u32,
    pub onchain_id: u32,
    pub name: String,
    pub curve: RefCurve,
    pub listings: Vec<MainnetListing>,
}

fn fixture() -> &'static Value {
    static FIXTURE: OnceLock<Value> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        serde_json::from_str(MAINNET_FIXTURE_JSON).expect("mainnet fixture is valid JSON")
    })
}

fn int(v: &Value) -> i128 {
    match v {
        Value::String(s) => s.parse().expect("fixture integer string"),
        Value::Number(n) => i128::from(n.as_i64().expect("fixture integer")),
        _ => panic!("fixture value {v} is not an integer"),
    }
}

fn small(v: &Value) -> u32 {
    u32::try_from(int(v)).expect("fixture value fits u32")
}

fn flag(v: &Value) -> bool {
    v.as_bool().expect("fixture boolean")
}

pub fn mainnet_onchain_spoke_ids() -> Vec<u32> {
    let mut ids: Vec<u32> = fixture()["spokes"]
        .as_object()
        .expect("fixture spokes")
        .values()
        .map(|s| small(&s["onchain_id"]))
        .collect();
    ids.sort_unstable();
    ids
}

pub fn mainnet_spoke(onchain_id: u32) -> MainnetSpokeConfig {
    let (config_id, spoke) = fixture()["spokes"]
        .as_object()
        .expect("fixture spokes")
        .iter()
        .find(|(_, s)| small(&s["onchain_id"]) == onchain_id)
        .unwrap_or_else(|| panic!("no mainnet spoke with on-chain id {onchain_id}"));
    let curve = &spoke["liquidation_curve"];
    MainnetSpokeConfig {
        config_id: config_id.parse().expect("config spoke id"),
        onchain_id,
        name: spoke["name"].as_str().expect("spoke name").to_string(),
        curve: RefCurve {
            target_hf_wad: int(&curve["target_hf_wad"]),
            hf_for_max_bonus_wad: int(&curve["hf_for_max_bonus_wad"]),
            bonus_factor_bps: int(&curve["liquidation_bonus_factor_bps"]),
        },
        listings: spoke["assets"]
            .as_object()
            .expect("spoke assets")
            .iter()
            .map(|(asset, l)| MainnetListing {
                asset: asset.clone(),
                hub_id: small(&l["hub_id"]),
                can_be_collateral: flag(&l["can_be_collateral"]),
                can_be_borrowed: flag(&l["can_be_borrowed"]),
                ltv: small(&l["ltv"]),
                liquidation_threshold: small(&l["liquidation_threshold"]),
                liquidation_bonus: small(&l["liquidation_bonus"]),
                liquidation_fees: small(&l["liquidation_fees"]),
                supply_cap: int(&l["supply_cap"]),
                borrow_cap: int(&l["borrow_cap"]),
            })
            .collect(),
    }
}

pub fn mainnet_market(name: &str) -> MainnetMarket {
    let m = &fixture()["markets"][name];
    assert!(m.is_object(), "no mainnet market named {name}");
    let p = &m["market_params"];
    MainnetMarket {
        name: name.to_string(),
        hub_id: small(&m["hub_id"]),
        decimals: small(&m["decimals"]),
        price_wad: int(&m["price_wad"]),
        min_sanity_price_wad: int(&m["min_sanity_price_wad"]),
        max_sanity_price_wad: int(&m["max_sanity_price_wad"]),
        params: MarketParamsPreset {
            max_borrow_rate: int(&p["max_borrow_rate"]),
            base_borrow_rate: int(&p["base_borrow_rate"]),
            slope1: int(&p["slope1"]),
            slope2: int(&p["slope2"]),
            slope3: int(&p["slope3"]),
            mid_utilization: int(&p["mid_utilization"]),
            optimal_utilization: int(&p["optimal_utilization"]),
            max_utilization: int(&p["max_utilization"]),
            reserve_factor: small(&p["reserve_factor"]),
        },
        is_flashloanable: flag(&p["is_flashloanable"]),
        flashloan_fee: small(&p["flashloan_fee"]),
    }
}

fn reflector_price(price_wad: i128) -> i128 {
    price_wad - price_wad % MOCK_REFLECTOR_PRICE_UNIT
}

pub struct MainnetSpoke {
    pub t: LendingTest,
    pub spoke_id: u32,
    pub config: MainnetSpokeConfig,
}

impl MainnetSpoke {
    pub fn build(onchain_id: u32) -> Self {
        let config = mainnet_spoke(onchain_id);
        let markets: Vec<MainnetMarket> = config
            .listings
            .iter()
            .map(|l| mainnet_market(&l.asset))
            .collect();

        let mut builder = LendingTest::new();
        for (listing, market) in config.listings.iter().zip(&markets) {
            let price_wad = reflector_price(market.price_wad);
            builder = builder.with_market(MarketPreset {
                name: Box::leak(market.name.clone().into_boxed_str()),
                decimals: market.decimals,
                price_wad,
                initial_liquidity: if listing.can_be_borrowed {
                    MARKET_LIQUIDITY_USD * 1e18 / price_wad as f64
                } else {
                    0.0
                },
                config: AssetConfigPreset {
                    loan_to_value: listing.ltv,
                    liquidation_threshold: listing.liquidation_threshold,
                    liquidation_bonus: listing.liquidation_bonus,
                    liquidation_fees: listing.liquidation_fees,
                    is_collateralizable: listing.can_be_collateral,
                    is_borrowable: listing.can_be_borrowed,
                    is_flashloanable: market.is_flashloanable,
                    flashloan_fee: market.flashloan_fee,
                },
                params: market.params.clone(),
            });
        }
        let t = builder.build();

        let hub_ids: Vec<u32> = fixture()["hubs"]
            .as_object()
            .expect("fixture hubs")
            .values()
            .map(|h| small(&h["onchain_id"]))
            .filter(|id| *id != HARNESS_HUB)
            .collect();
        for hub_id in hub_ids {
            assert_eq!(t.create_hub(), hub_id, "harness hub ids follow mainnet");
        }

        let admin = t.admin.clone();
        let gov = t.gov_client();
        for market in markets.iter().filter(|m| m.hub_id != HARNESS_HUB) {
            let asset = t.resolve_asset(&market.name);
            let mut params = market.params.to_market_params(&asset, market.decimals);
            params.is_flashloanable = market.is_flashloanable;
            params.flashloan_fee = market.flashloan_fee;
            gov.execute_immediate(
                &admin,
                &AdminOperation::CreateLiquidityPool(CreatePoolArgs {
                    hub_id: market.hub_id,
                    asset,
                    params,
                }),
            );
        }

        let spoke_val = gov.execute_immediate(&admin, &AdminOperation::AddSpoke);
        let spoke_id = u32::try_from_val(&t.env, &spoke_val).expect("AddSpoke returns an id");
        for listing in &config.listings {
            gov.execute_immediate(
                &admin,
                &AdminOperation::AddAssetToSpoke(SpokeAssetArgs {
                    hub_id: listing.hub_id,
                    asset: t.resolve_asset(&listing.asset),
                    spoke_id,
                    can_collateral: listing.can_be_collateral,
                    can_borrow: listing.can_be_borrowed,
                    paused: false,
                    frozen: false,
                    no_seize: false,
                    ltv: listing.ltv,
                    threshold: listing.liquidation_threshold,
                    bonus: listing.liquidation_bonus,
                    liquidation_fees: listing.liquidation_fees,
                    supply_cap: listing.supply_cap,
                    borrow_cap: listing.borrow_cap,
                }),
            );
        }
        gov.execute_immediate(
            &admin,
            &AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs {
                spoke_id,
                target_hf_wad: config.curve.target_hf_wad,
                hf_for_max_bonus_wad: config.curve.hf_for_max_bonus_wad,
                liquidation_bonus_factor_bps: config.curve.bonus_factor_bps as u32,
            }),
        );

        for market in &markets {
            t.seed_sanity_band(
                &market.name,
                market.min_sanity_price_wad,
                market.max_sanity_price_wad,
            );
        }

        Self {
            t,
            spoke_id,
            config,
        }
    }

    pub fn listing(&self, asset: &str) -> &MainnetListing {
        self.config
            .listings
            .iter()
            .find(|l| l.asset == asset)
            .unwrap_or_else(|| panic!("{asset} is not listed on spoke {}", self.config.name))
    }

    pub fn hub_asset(&self, asset: &str) -> HubAssetKey {
        HubAssetKey {
            hub_id: self.listing(asset).hub_id,
            asset: self.t.resolve_asset(asset),
        }
    }

    pub fn price(&self, asset: &str) -> i128 {
        self.t.resolve_market(asset).price_wad
    }

    pub fn set_price(&mut self, asset: &str, price_wad: i128) {
        self.t
            .set_price_keeping_sanity_band(asset, reflector_price(price_wad));
    }

    pub fn move_price_bps(&mut self, asset: &str, delta_bps: i128) -> i128 {
        let price = self.price(asset) * (10_000 + delta_bps) / 10_000;
        self.set_price(asset, price);
        self.price(asset)
    }

    pub fn price_in_band(&self, asset: &str, price_wad: i128) -> bool {
        let m = mainnet_market(asset);
        (m.min_sanity_price_wad..=m.max_sanity_price_wad).contains(&reflector_price(price_wad))
    }

    pub fn account_id(&self, user: &str) -> u64 {
        self.t.resolve_account_id(user)
    }

    pub fn supply(&mut self, user: &str, asset: &str, amount: i128) -> u64 {
        let addr = self.t.get_or_create_user(user);
        self.t
            .resolve_market(asset)
            .token_admin
            .mint(&addr, &amount);
        let account_id = self.t.default_account_id_or_zero(user);
        let returned = self.t.ctrl_client().supply(
            &addr,
            &account_id,
            &self.spoke_id,
            &vec![&self.t.env, (self.hub_asset(asset), amount)],
        );
        if account_id == 0 {
            self.t
                .register_account(user, returned, self.spoke_id, PositionMode::Normal);
        }
        returned
    }

    pub fn try_borrow(&mut self, user: &str, asset: &str, amount: i128) -> Result<(), Error> {
        let addr = self.t.get_or_create_user(user);
        let account_id = self.account_id(user);
        map_try_ok_unit(self.t.ctrl_client().try_borrow(
            &addr,
            &account_id,
            &vec![&self.t.env, (self.hub_asset(asset), amount)],
            &None,
        ))
    }

    pub fn usd_to_raw(&self, asset: &str, usd_wad: i128) -> i128 {
        let m = self.t.resolve_market(asset);
        (BigInt::from(usd_wad) * BigInt::from(10).pow(m.decimals) / BigInt::from(m.price_wad))
            .to_i128()
            .expect("token amount fits i128")
    }

    pub fn open_max_ltv(
        &mut self,
        user: &str,
        collateral: &str,
        collateral_amount: i128,
        debt: &str,
    ) -> i128 {
        let account_id = self.supply(user, collateral, collateral_amount);
        let ltv_usd = self.t.ctrl_client().get_ltv_collateral_usd(&account_id);
        let mut amount = self.usd_to_raw(debt, ltv_usd);
        for _ in 0..8 {
            if self.try_borrow(user, debt, amount).is_ok() {
                return amount;
            }
            amount -= 1;
        }
        panic!("no borrow of {debt} near max LTV succeeded for {user}");
    }

    pub fn health_factor_raw(&self, user: &str) -> i128 {
        self.t.health_factor_raw(user)
    }

    pub fn debt_raw(&self, user: &str, asset: &str) -> i128 {
        let book = snapshot_exact(&self.t, self.account_id(user));
        let id = book.id_of(&self.hub_asset(asset));
        book.debt
            .iter()
            .find(|d| d.asset_id == id)
            .map_or(0, |d| d.balance_ceil())
    }

    fn payments(&self, payments: &[(&str, i128)]) -> SorobanVec<(HubAssetKey, i128)> {
        let mut out = SorobanVec::new(&self.t.env);
        for (asset, amount) in payments {
            out.push_back((self.hub_asset(asset), *amount));
        }
        out
    }

    pub fn estimate(
        &self,
        user: &str,
        payments: &[(&str, i128)],
        mode: SeizeMode,
    ) -> LiquidationEstimate {
        self.t.ctrl_client().get_liquidation_estimate(
            &self.account_id(user),
            &self.payments(payments),
            &mode,
        )
    }

    pub fn reference_plan(&self, user: &str, payments: &[(&str, i128)]) -> (ExactBook, ExactPlan) {
        let book = snapshot_exact(&self.t, self.account_id(user));
        let ids: Vec<(u32, i128)> = payments
            .iter()
            .map(|(asset, amount)| (book.id_of(&self.hub_asset(asset)), *amount))
            .collect();
        let plan = plan_liquidation_exact(&book, &ids, &self.config.curve);
        (book, plan)
    }

    pub fn try_liquidate(
        &mut self,
        liquidator: &str,
        user: &str,
        payments: &[(&str, i128)],
        mode: SeizeMode,
    ) -> Result<u64, Error> {
        let addr = self.t.get_or_create_user(liquidator);
        for (asset, amount) in payments {
            self.t.resolve_market(asset).token_admin.mint(&addr, amount);
        }
        let res = map_try_ok_value(self.t.ctrl_client().try_liquidate(
            &addr,
            &self.account_id(user),
            &self.payments(payments),
            &mode,
        ));
        if res.is_err() {
            for (asset, amount) in payments {
                burn_prefund(&self.t.env, &self.t.resolve_asset(asset), &addr, *amount);
            }
        }
        res
    }

    pub fn liquidate_transfer(
        &mut self,
        liquidator: &str,
        user: &str,
        payments: &[(&str, i128)],
    ) -> u64 {
        self.try_liquidate(liquidator, user, payments, SeizeMode::Transfer)
            .expect("transfer liquidation")
    }

    pub fn liquidate_credit(
        &mut self,
        liquidator: &str,
        user: &str,
        payments: &[(&str, i128)],
        receiver_account: u64,
    ) -> u64 {
        self.try_liquidate(
            liquidator,
            user,
            payments,
            SeizeMode::Credit(receiver_account),
        )
        .expect("credit liquidation")
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SeizedLeg {
    pub asset: &'static str,
    pub gross: i128,
    pub to_liquidator: i128,
    pub fee: i128,
}

#[derive(Clone, Copy, Debug)]
pub enum VectorOutcome {
    Open {
        hf_wad: i128,
    },
    DebtFree,
    BadDebt {
        socialized: &'static [(&'static str, i128)],
    },
}

#[derive(Clone, Copy, Debug)]
pub struct LiquidationVector {
    pub name: &'static str,
    pub spoke: u32,
    pub open_prices: &'static [(&'static str, i128)],
    pub collateral: &'static [(&'static str, i128)],
    pub debt: &'static [(&'static str, i128)],
    pub prices: &'static [(&'static str, i128)],
    pub hf_wad: i128,
    pub bonus_bps: i128,
    pub repay_usd_wad: i128,
    pub repaid: &'static [(&'static str, i128)],
    pub seized: &'static [SeizedLeg],
    pub post_collateral_usd_wad: i128,
    pub post_weighted_usd_wad: i128,
    pub post_debt_usd_wad: i128,
    pub outcome: VectorOutcome,
}

#[derive(Clone, Debug)]
pub struct LiquidationObservation {
    pub receiver_id: u64,
    pub bonus_bps: i128,
    pub repaid_usd_wad: i128,
    pub liquidator_delta: BTreeMap<String, i128>,
    pub shares_burned: BTreeMap<String, i128>,
    pub revenue_delta: BTreeMap<String, i128>,
    pub borrowed_burned: BTreeMap<String, i128>,
    pub receiver_shares: BTreeMap<String, i128>,
    pub post: Option<ExactTotals>,
    pub debt_free: bool,
    pub bad_debt_event: Option<(i128, i128)>,
}

type AccountSides = (
    SorobanMap<HubAssetKey, AccountPositionRaw>,
    SorobanMap<HubAssetKey, DebtPositionRaw>,
);

fn scval_i128(v: &ScVal) -> i128 {
    match v {
        ScVal::I128(p) => (i128::from(p.hi) << 64) | i128::from(p.lo),
        ScVal::U64(x) => i128::from(*x),
        ScVal::U32(x) => i128::from(*x),
        other => panic!("unexpected event value {other:?}"),
    }
}

fn topic_events(env: &Env, first: &str, second: &str) -> Vec<BTreeMap<String, i128>> {
    env.events()
        .all()
        .events()
        .iter()
        .filter_map(|event| {
            let ContractEventBody::V0(body) = &event.body;
            let matches = matches!(
                (body.topics.first(), body.topics.get(1)),
                (Some(ScVal::Symbol(a)), Some(ScVal::Symbol(b)))
                    if a.0.to_string() == first && b.0.to_string() == second
            );
            if !matches {
                return None;
            }
            let ScVal::Map(Some(entries)) = &body.data else {
                panic!("{first}/{second} event data is not a map");
            };
            Some(
                entries
                    .iter()
                    .filter_map(|e| match (&e.key, &e.val) {
                        (_, ScVal::Address(_)) => None,
                        (ScVal::Symbol(k), v) => Some((k.0.to_string(), scval_i128(v))),
                        _ => None,
                    })
                    .collect(),
            )
        })
        .collect()
}

impl MainnetSpoke {
    pub fn share_unit(&self, asset: &str) -> i128 {
        10i128.pow(27 - self.t.resolve_market(asset).decimals)
    }

    pub fn set_prices(&mut self, prices: &[(&str, i128)]) {
        for (asset, price) in prices {
            self.set_price(asset, *price);
        }
    }

    pub fn borrow(&mut self, user: &str, asset: &str, amount: i128) {
        self.try_borrow(user, asset, amount)
            .unwrap_or_else(|e| panic!("{user} borrow of {amount} {asset} failed: {e:?}"));
    }

    pub fn open_position(
        &mut self,
        user: &str,
        collateral: &[(&str, i128)],
        debt: &[(&str, i128)],
    ) -> u64 {
        for (asset, amount) in collateral {
            self.supply(user, asset, *amount);
        }
        for (asset, amount) in debt {
            self.borrow(user, asset, *amount);
        }
        self.account_id(user)
    }

    pub fn open_vector(v: &LiquidationVector, user: &str) -> Self {
        let mut m = Self::build(v.spoke);
        m.set_prices(v.open_prices);
        m.open_position(user, v.collateral, v.debt);
        m.set_prices(v.prices);
        m
    }

    fn asset_of(&self, key: &HubAssetKey) -> String {
        self.config
            .listings
            .iter()
            .find(|l| self.hub_asset(&l.asset) == *key)
            .map(|l| l.asset.clone())
            .expect("position asset is listed on the spoke")
    }

    fn sides(&self, account_id: u64) -> Option<AccountSides> {
        self.t
            .ctrl_client()
            .try_get_account_positions(&account_id)
            .ok()
            .and_then(Result::ok)
    }

    pub fn supply_shares(&self, account_id: u64) -> BTreeMap<String, i128> {
        self.sides(account_id)
            .map(|(supply, _)| {
                supply
                    .iter()
                    .map(|(key, p)| (self.asset_of(&key), p.scaled_amount))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn pool_state(&self, asset: &str) -> PoolStateRaw {
        self.t
            .pool_client(asset)
            .get_sync_data(&self.hub_asset(asset))
            .state
    }

    pub fn set_pool_cash(&self, asset: &str, cash: i128) {
        let env = &self.t.env;
        let key = PoolKey::State(self.hub_asset(asset));
        env.as_contract(&self.t.resolve_market(asset).pool, || {
            let mut state: PoolStateRaw = env
                .storage()
                .persistent()
                .get(&key)
                .expect("pool state exists");
            state.cash = cash;
            env.storage().persistent().set(&key, &state);
        });
    }

    fn pool_states(&self) -> BTreeMap<String, PoolStateRaw> {
        self.config
            .listings
            .iter()
            .map(|l| (l.asset.clone(), self.pool_state(&l.asset)))
            .collect()
    }

    fn balances(&self, user: &str) -> BTreeMap<String, i128> {
        self.config
            .listings
            .iter()
            .map(|l| (l.asset.clone(), self.t.token_balance_raw(user, &l.asset)))
            .collect()
    }

    pub fn observe_liquidation(
        &mut self,
        liquidator: &str,
        user: &str,
        payments: &[(&str, i128)],
        mode: SeizeMode,
    ) -> Result<LiquidationObservation, Error> {
        self.t.get_or_create_user(liquidator);
        let account_id = self.account_id(user);
        let shares_before = self.supply_shares(account_id);
        let pools_before = self.pool_states();
        let balances_before = self.balances(liquidator);

        let receiver_id = self.try_liquidate(liquidator, user, payments, mode)?;

        let liq = topic_events(&self.t.env, "position", "liquidation");
        assert_eq!(liq.len(), 1, "one liquidation event");
        let bad_debt_event = topic_events(&self.t.env, "debt", "bad_debt")
            .first()
            .map(|e| (e["total_borrow_usd_wad"], e["total_collateral_usd_wad"]));

        let shares_after = self.supply_shares(account_id);
        let pools_after = self.pool_states();
        let balances_after = self.balances(liquidator);
        let sides = self.sides(account_id);
        let debt_free = sides.as_ref().is_none_or(|(_, borrow)| borrow.is_empty());
        let post = match (&sides, bad_debt_event) {
            (Some(_), None) => {
                let book = snapshot_exact(&self.t, account_id);
                Some(exact_totals(&book.collateral, &book.debt))
            }
            _ => None,
        };

        let names: Vec<String> = self
            .config
            .listings
            .iter()
            .map(|l| l.asset.clone())
            .collect();
        let delta = |f: &dyn Fn(&str) -> i128| -> BTreeMap<String, i128> {
            names.iter().map(|a| (a.clone(), f(a))).collect()
        };
        Ok(LiquidationObservation {
            receiver_id,
            bonus_bps: liq[0]["bonus_bps"],
            repaid_usd_wad: liq[0]["repaid_usd_wad"],
            liquidator_delta: delta(&|a| balances_after[a] - balances_before[a]),
            shares_burned: delta(&|a| {
                shares_before.get(a).copied().unwrap_or(0)
                    - shares_after.get(a).copied().unwrap_or(0)
            }),
            revenue_delta: delta(&|a| pools_after[a].revenue - pools_before[a].revenue),
            borrowed_burned: delta(&|a| pools_before[a].borrowed - pools_after[a].borrowed),
            receiver_shares: if receiver_id == 0 {
                BTreeMap::new()
            } else {
                self.supply_shares(receiver_id)
            },
            post,
            debt_free,
            bad_debt_event,
        })
    }

    pub fn assert_transfer_vector(
        &mut self,
        liquidator: &str,
        user: &str,
        v: &LiquidationVector,
    ) -> LiquidationObservation {
        let ctx = v.name;
        assert_eq!(
            self.health_factor_raw(user),
            v.hf_wad,
            "{ctx}: health factor"
        );

        let estimate = self.estimate(user, v.debt, SeizeMode::Transfer);
        assert_eq!(
            estimate.bonus_rate_bps, v.bonus_bps,
            "{ctx}: estimate bonus"
        );
        assert_eq!(
            estimate.max_payment_wad, v.repay_usd_wad,
            "{ctx}: estimate repayment"
        );
        let planned: BTreeMap<String, (i128, i128)> = estimate
            .seized_collaterals
            .iter()
            .zip(estimate.protocol_fees.iter())
            .map(|(s, f)| {
                let asset = self
                    .config
                    .listings
                    .iter()
                    .find(|l| self.t.resolve_asset(&l.asset) == s.asset)
                    .map(|l| l.asset.clone())
                    .expect("seized asset is listed");
                (asset, (s.amount, f.amount))
            })
            .collect();
        let expected: BTreeMap<String, (i128, i128)> = v
            .seized
            .iter()
            .map(|l| (l.asset.to_string(), (l.gross, l.fee)))
            .collect();
        assert_eq!(planned, expected, "{ctx}: estimate seizure and fee");

        let obs = self
            .observe_liquidation(liquidator, user, v.debt, SeizeMode::Transfer)
            .unwrap_or_else(|e| panic!("{ctx}: transfer liquidation reverted: {e:?}"));
        assert_eq!(obs.bonus_bps, v.bonus_bps, "{ctx}: event bonus");
        assert_eq!(
            obs.repaid_usd_wad, v.repay_usd_wad,
            "{ctx}: event repayment"
        );
        for (asset, repaid) in v.repaid {
            let offered = v
                .debt
                .iter()
                .find(|(a, _)| a == asset)
                .map(|(_, x)| *x)
                .expect("repaid leg is a debt leg");
            assert_eq!(
                obs.liquidator_delta[*asset],
                offered - repaid,
                "{ctx}: {asset} repaid"
            );
        }
        for leg in v.seized {
            let unit = self.share_unit(leg.asset);
            assert_eq!(
                obs.liquidator_delta[leg.asset], leg.to_liquidator,
                "{ctx}: {} to liquidator",
                leg.asset
            );
            assert_eq!(
                obs.shares_burned[leg.asset],
                leg.gross * unit,
                "{ctx}: {} shares burned",
                leg.asset
            );
            assert_eq!(
                obs.revenue_delta[leg.asset],
                leg.fee * unit,
                "{ctx}: {} fee revenue",
                leg.asset
            );
        }

        match v.outcome {
            VectorOutcome::Open { hf_wad } => {
                let post = obs.post.expect("account keeps debt");
                assert!(!obs.debt_free, "{ctx}: debt remains");
                assert_eq!(
                    (
                        post.total_collateral,
                        post.weighted_collateral,
                        post.total_debt,
                        post.health_factor
                    ),
                    (
                        v.post_collateral_usd_wad,
                        v.post_weighted_usd_wad,
                        v.post_debt_usd_wad,
                        hf_wad
                    ),
                    "{ctx}: post C, W, D, HF"
                );
                assert_eq!(self.health_factor_raw(user), hf_wad, "{ctx}: post HF view");
            }
            VectorOutcome::DebtFree => {
                assert!(obs.debt_free, "{ctx}: account is debt free");
                assert_eq!(obs.bad_debt_event, None, "{ctx}: no cleanup");
                let post = obs.post.expect("residual collateral stays");
                assert_eq!(
                    (
                        post.total_collateral,
                        post.weighted_collateral,
                        post.total_debt
                    ),
                    (v.post_collateral_usd_wad, v.post_weighted_usd_wad, 0),
                    "{ctx}: post C, W, D"
                );
            }
            VectorOutcome::BadDebt { socialized } => {
                assert!(obs.debt_free, "{ctx}: cleanup clears the debt");
                assert_eq!(
                    obs.bad_debt_event,
                    Some((v.post_debt_usd_wad, v.post_collateral_usd_wad)),
                    "{ctx}: cleanup event D and C"
                );
                for (asset, units) in socialized {
                    let repaid = v
                        .repaid
                        .iter()
                        .find(|(a, _)| a == asset)
                        .map_or(0, |(_, x)| *x);
                    assert_eq!(
                        obs.borrowed_burned[*asset],
                        (repaid + units) * self.share_unit(asset),
                        "{ctx}: {asset} repaid plus socialized"
                    );
                }
            }
        }
        self.t.assert_spoke_usage_matches_positions();
        obs
    }
}
