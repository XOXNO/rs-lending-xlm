use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};

use common::types::{
    AccountPositionRaw, DebtPositionRaw, HubAssetKey, LiquidationEstimate, MarketIndexView,
    PaymentTuple, SeizeMode,
};
use controller::constants::WAD;
use governance::op::{AdminOperation, SpokeAssetArgs, SpokeLiquidationCurveArgs};
use serde_json::{json, Map, Value};
use soroban_sdk::testutils::Events;
use soroban_sdk::xdr::{ScErrorType, ScVal};
use soroban_sdk::{
    token, vec as soroban_vec, Address, Error, Map as SorobanMap, Vec as SorobanVec,
};
use test_harness::reference::{exact_totals, snapshot_exact};

use crate::liqvid_listing_params::{setup_with_threshold, Params};
use crate::shared::data_for_topic;

const LIQ: &str = "LIQ";
const LIQ_MARKET: &str = "LIQVID";
const USDC: &str = "USDC";
const USDC_UNIT: i128 = 10_000_000;
const BPS: i128 = 10_000;
const LIQUIDATOR: &str = "wu-liquidator";
const MIN_CASES: usize = 200;
const OFFERS: [(i128, i128, &str); 4] =
    [(1, 10, "1/10"), (1, 3, "1/3"), (1, 1, "1/1"), (3, 2, "3/2")];
const NAV_PATH_PER_MILLE: [i128; 21] = [
    960, 943, 930, 910, 890, 870, 849, 800, 750, 700, 650, 600, 560, 530, 510, 495, 470, 420, 350,
    250, 150,
];
const UNIT_CENTS: [i128; 5] = [100, 700, 2_500, 10_000, 50_000];
const UNITS: [i128; 7] = [2, 3, 5, 10, 20, 35, 50];
const DEBT_PER_MILLE: [i128; 3] = [0, 600, 1_000];
const LIQVID: Listing = Listing {
    ltv: 5_000,
    lt: 5_300,
    bonus: 500,
    curve: (WAD * 106 / 100, WAD * 90 / 100, 598),
};
const LOW_LT: Listing = Listing {
    ltv: 1_950,
    lt: 2_600,
    bonus: 300,
    curve: (WAD * 110 / 100, WAD * 80 / 100, 1_500),
};
const EDGE_EPS_PPB: [i128; 3] = [-240, 0, 3_000];
const EDGE_BORROW_PCT: [i128; 4] = [90, 80, 70, 60];

#[derive(Clone, Copy)]
struct Listing {
    ltv: u32,
    lt: u32,
    bonus: u32,
    curve: (i128, i128, u32),
}

struct Cal {
    p: Params,
    listing: Listing,
    group: String,
    liquidator: Address,
}

struct Recorder {
    out: Option<BufWriter<File>>,
    cases: usize,
    ok: usize,
}

impl Recorder {
    fn write(&mut self, case: Value) {
        self.cases += 1;
        if let Some(out) = self.out.as_mut() {
            writeln!(out, "{case}").expect("write calibration case");
        }
    }
}

fn s(x: i128) -> Value {
    Value::String(x.to_string())
}

fn error_code(e: &Error) -> Value {
    if e.is_type(ScErrorType::Contract) {
        json!(e.get_code())
    } else {
        Value::String(format!("{e:?}"))
    }
}

fn scval_i128(v: &ScVal) -> i128 {
    match v {
        ScVal::I128(p) => (i128::from(p.hi) << 64) | i128::from(p.lo),
        ScVal::U64(x) => i128::from(*x),
        ScVal::U32(x) => i128::from(*x),
        other => panic!("unexpected event value {other:?}"),
    }
}

pub(crate) fn event_fields(data: &ScVal) -> BTreeMap<String, i128> {
    let ScVal::Map(Some(entries)) = data else {
        panic!("event data is not a map: {data:?}");
    };
    entries
        .iter()
        .filter_map(|e| match (&e.key, &e.val) {
            (_, ScVal::Address(_)) => None,
            (ScVal::Symbol(k), v) => Some((k.0.to_string(), scval_i128(v))),
            _ => None,
        })
        .collect()
}

impl Cal {
    fn new(unit_cents: i128, listing: Listing, group: String) -> Self {
        let mut p = setup_with_threshold(unit_cents, 5_300);
        let admin = p.t.admin();
        let gov = p.t.gov_client();
        gov.execute_immediate(
            &admin,
            &AdminOperation::EditAssetInSpoke(SpokeAssetArgs {
                hub_id: p.hub,
                asset: p.liq.clone(),
                spoke_id: p.spoke,
                can_collateral: true,
                can_borrow: false,
                paused: false,
                frozen: false,
                no_seize: false,
                ltv: listing.ltv,
                threshold: listing.lt,
                bonus: listing.bonus,
                liquidation_fees: 0,
                supply_cap: 10_000_000,
                borrow_cap: 0,
            }),
        );
        gov.execute_immediate(
            &admin,
            &AdminOperation::SetSpokeLiquidationCurve(SpokeLiquidationCurveArgs {
                spoke_id: p.spoke,
                target_hf_wad: listing.curve.0,
                hf_for_max_bonus_wad: listing.curve.1,
                liquidation_bonus_factor_bps: listing.curve.2,
            }),
        );
        let liquidator = p.user(LIQUIDATOR);
        p.allow(&liquidator);
        Cal {
            p,
            listing,
            group,
            liquidator,
        }
    }

    fn key(&self, name: &str) -> HubAssetKey {
        if name == LIQ {
            self.p.liq_key()
        } else {
            self.p.usdc_key()
        }
    }

    fn name(&self, key: &HubAssetKey) -> &'static str {
        if *key == self.p.liq_key() {
            LIQ
        } else {
            USDC
        }
    }

    fn set_nav(&mut self, nav: i128) {
        self.p.post_nav(nav);
        let floor = self.p.nav_ref * 849 / 1_000;
        let ceiling = self.p.nav_ref * 1_030 / 1_000;
        if nav < floor || nav > ceiling {
            self.p.nav_ref = nav;
            self.p
                .configure_oracle(self.p.nav_oracle())
                .expect("band around the new NAV");
        }
    }

    fn advance(&mut self, secs: u64) {
        let nav = self.price(LIQ);
        self.p.t.advance_and_sync(secs);
        self.p.t.ctrl_client().update_indexes(
            &self.p.t.keeper,
            &soroban_vec![&self.p.t.env, self.p.liq_key()],
        );
        self.p.post_nav(nav);
    }

    fn open(&mut self, user: &str, units: i128, debt_raw: i128) -> Option<u64> {
        let id = self.p.try_supply(user, units).expect("supply");
        self.p.try_borrow(user, id, debt_raw).ok().map(|()| id)
    }

    fn view(&self, name: &str) -> MarketIndexView {
        self.p
            .t
            .ctrl_client()
            .get_market_indexes_detailed(&soroban_vec![&self.p.t.env, self.key(name)])
            .get(0)
            .expect("market index view")
    }

    fn price(&self, name: &str) -> i128 {
        self.view(name).price_wad
    }

    fn exists(&self, id: u64) -> bool {
        self.p.t.ctrl_client().account_exists(&id)
    }

    fn liquidatable(&self, id: u64) -> bool {
        self.exists(id) && self.p.debt_raw(id) > 0 && self.p.t.ctrl_client().is_liquidatable(&id)
    }

    fn total_debt(&self, id: u64) -> i128 {
        self.p.t.ctrl_client().get_total_borrow_usd(&id)
    }

    fn estimate(&self, id: u64, offer: i128, mode: &SeizeMode) -> Option<LiquidationEstimate> {
        self.p
            .t
            .ctrl_client()
            .try_get_liquidation_estimate(
                &id,
                &soroban_vec![&self.p.t.env, (self.p.usdc_key(), offer)],
                mode,
            )
            .ok()
            .and_then(Result::ok)
    }

    fn pool_json(&self, name: &str) -> Value {
        let market = if name == LIQ { LIQ_MARKET } else { USDC };
        let st = self
            .p
            .t
            .pool_client(market)
            .get_sync_data(&self.key(name))
            .state;
        json!({
            "supplied": s(st.supplied),
            "borrowed": s(st.borrowed),
            "revenue": s(st.revenue),
            "cash": s(st.cash),
            "supply_index": s(st.supply_index),
            "borrow_index": s(st.borrow_index),
        })
    }

    fn pools_json(&self) -> Value {
        json!({ LIQ: self.pool_json(LIQ), USDC: self.pool_json(USDC) })
    }

    fn markets_json(&self) -> Value {
        let mut out = Map::new();
        for (name, decimals, listing) in [
            (LIQ, 0u32, self.listing),
            (
                USDC,
                7,
                Listing {
                    ltv: 7_500,
                    lt: 8_000,
                    bonus: 500,
                    curve: self.listing.curve,
                },
            ),
        ] {
            let v = self.view(name);
            out.insert(
                name.to_string(),
                json!({
                    "decimals": decimals,
                    "price_wad": s(v.price_wad),
                    "supply_index": s(v.supply_index),
                    "borrow_index": s(v.borrow_index),
                    "listing": {"ltv": listing.ltv, "lt": listing.lt, "bonus": listing.bonus, "fee": 0},
                }),
            );
        }
        Value::Object(out)
    }

    fn positions_json(&self, id: u64) -> Value {
        type Positions = (
            SorobanMap<HubAssetKey, AccountPositionRaw>,
            SorobanMap<HubAssetKey, DebtPositionRaw>,
        );
        let got: Option<Positions> = self
            .p
            .t
            .ctrl_client()
            .try_get_account_positions(&id)
            .ok()
            .and_then(Result::ok);
        match got {
            None => Value::Null,
            Some((supply, borrow)) => json!({
                "supply": supply.iter().map(|(k, p)| json!({
                    "asset": self.name(&k),
                    "scaled": s(p.scaled_amount),
                    "ltv": p.loan_to_value,
                    "lt": p.liquidation_threshold,
                    "bonus": p.liquidation_bonus,
                    "fee": p.liquidation_fees,
                })).collect::<Vec<_>>(),
                "borrow": borrow.iter().map(|(k, p)| json!({
                    "asset": self.name(&k),
                    "scaled": s(p.scaled_amount),
                })).collect::<Vec<_>>(),
            }),
        }
    }

    fn totals_json(&self, id: u64) -> Value {
        let book = snapshot_exact(&self.p.t, id);
        let t = exact_totals(&book.collateral, &book.debt);
        json!({
            "C": s(t.total_collateral),
            "W": s(t.weighted_collateral),
            "D": s(t.total_debt),
            "HF": s(t.health_factor),
            "p": s(t.proportion_seized),
            "base": s(t.base_bonus_bps),
            "max": s(t.max_bonus_bps),
        })
    }

    fn estimate_json(&self, e: &LiquidationEstimate) -> Value {
        let pairs = |v: &SorobanVec<PaymentTuple>| {
            v.iter()
                .map(|p| {
                    let name = if p.asset == self.p.liq { LIQ } else { USDC };
                    json!([name, s(p.amount)])
                })
                .collect::<Vec<_>>()
        };
        json!({
            "bonus_bps": s(e.bonus_rate_bps),
            "max_payment_wad": s(e.max_payment_wad),
            "seized": pairs(&e.seized_collaterals),
            "protocol_fees": pairs(&e.protocol_fees),
            "refunds": pairs(&e.refunds),
        })
    }

    fn balances(&self) -> BTreeMap<&'static str, i128> {
        let who = &self.liquidator;
        BTreeMap::from([
            (
                LIQ,
                token::Client::new(&self.p.t.env, &self.p.liq).balance(who),
            ),
            (
                USDC,
                token::Client::new(&self.p.t.env, &self.p.usdc).balance(who),
            ),
        ])
    }

    #[allow(clippy::too_many_arguments)]
    fn run_case(
        &mut self,
        rec: &mut Recorder,
        step: usize,
        user: &str,
        id: u64,
        offer_raw: i128,
        offer: &str,
        credit: bool,
    ) -> bool {
        let mode = if credit {
            SeizeMode::Credit(0)
        } else {
            SeizeMode::Transfer
        };
        let estimate = self.estimate(id, offer_raw, &mode);
        let markets = self.markets_json();
        let positions = self.positions_json(id);
        let totals = self.totals_json(id);
        let pools_before = self.pools_json();
        let before = self.balances();

        let who = self.liquidator.clone();
        self.p
            .t
            .resolve_market(USDC)
            .token_admin
            .mint(&who, &offer_raw);
        let result = self.p.t.ctrl_client().try_liquidate(
            &who,
            &id,
            &soroban_vec![&self.p.t.env, (self.p.usdc_key(), offer_raw)],
            &mode,
        );
        let events = self.p.t.env.events().all();
        let outcome = match result {
            Err(e) => json!({"ok": false, "error": error_code(&e.expect("contract error"))}),
            Ok(Err(e)) => json!({"ok": false, "error": format!("{e:?}")}),
            Ok(Ok(receiver_id)) => {
                rec.ok += 1;
                let liq = data_for_topic(&events, "position", "liquidation");
                assert_eq!(liq.len(), 1, "one liquidation event");
                let liq = event_fields(&liq[0]);
                let cleanup_event = data_for_topic(&events, "debt", "bad_debt")
                    .first()
                    .map(event_fields);
                let after = self.balances();
                let delta: Map<String, Value> = after
                    .iter()
                    .map(|(a, x)| (a.to_string(), s(x - before[a])))
                    .collect();
                let (cleanup, post) = match (&cleanup_event, self.exists(id)) {
                    (Some(ev), _) => (
                        "bad_debt_cleanup",
                        json!({"C": s(ev["total_collateral_usd_wad"]), "D": s(ev["total_borrow_usd_wad"])}),
                    ),
                    (None, false) => ("account_removed", json!({"C": s(0), "D": s(0)})),
                    (None, true) => {
                        let label = if self.p.debt_raw(id) == 0 {
                            "debt_free"
                        } else {
                            "none"
                        };
                        (label, self.totals_json(id))
                    }
                };
                json!({
                    "ok": true,
                    "receiver_id": receiver_id,
                    "event": {"repaid_usd_wad": s(liq["repaid_usd_wad"]), "bonus_bps": s(liq["bonus_bps"])},
                    "liquidator_delta": Value::Object(delta),
                    "account_after": self.positions_json(id),
                    "receiver_after": if receiver_id == 0 { Value::Null } else { self.positions_json(receiver_id) },
                    "pools_after": self.pools_json(),
                    "cleanup": cleanup,
                    "post": post,
                })
            }
        };
        let ok = outcome["ok"] == Value::Bool(true);
        let (h, k, f) = self.listing.curve;
        rec.write(json!({
            "case": rec.cases,
            "scenario": self.group,
            "spoke": self.p.spoke,
            "step": step,
            "user": user,
            "account_id": id,
            "offer": offer,
            "mode": if credit { "Credit" } else { "Transfer" },
            "curve": {"H": s(h), "K": s(k), "f": s(i128::from(f))},
            "markets": markets,
            "positions": positions,
            "pools_before": pools_before,
            "payments": [[USDC, s(offer_raw)]],
            "mirror": {"totals": totals},
            "estimate": estimate.as_ref().map_or(Value::Null, |e| self.estimate_json(e)),
            "result": outcome,
        }));
        ok
    }

    fn offer_case(
        &mut self,
        rec: &mut Recorder,
        step: usize,
        user: &str,
        id: u64,
        combo: usize,
    ) -> bool {
        let (num, den, label) = OFFERS[(combo / 2) % OFFERS.len()];
        let offer_raw = (self.p.debt_raw(id) * num / den).max(1);
        self.run_case(rec, step, user, id, offer_raw, label, combo % 2 == 1)
    }
}

fn liqvid_walk(rec: &mut Recorder, unit_cents: i128) {
    let group = format!("liqvid-{unit_cents}c");
    let mut c = Cal::new(unit_cents, LIQVID, group.clone());
    let p0 = c.p.nav_ref;
    let mut accounts = Vec::new();
    for units in UNITS {
        for debt_per_mille in DEBT_PER_MILLE {
            if units * unit_cents < 1_000 {
                continue;
            }
            let max_raw = units * p0 / 2 / (WAD / USDC_UNIT);
            let raw = if debt_per_mille == 0 {
                5 * USDC_UNIT
            } else {
                max_raw * debt_per_mille / 1_000
            };
            if raw > max_raw {
                continue;
            }
            let user = format!("{group}-{units}-{debt_per_mille}");
            if let Some(id) = c.open(&user, units, raw) {
                accounts.push((user, id));
            }
        }
    }
    assert!(accounts.len() >= 5, "{group}: accounts opened");
    let mut combo = 0usize;
    for (step, per_mille) in NAV_PATH_PER_MILLE.iter().enumerate() {
        c.advance(86_400);
        c.set_nav(p0 * per_mille / 1_000);
        for (user, id) in &accounts {
            if !c.liquidatable(*id) {
                continue;
            }
            let ok = c.offer_case(rec, step, user, *id, combo);
            let full_transfer = (combo / 2) % OFFERS.len() == 2 && combo.is_multiple_of(2);
            combo += 1;
            if !ok && !full_transfer && c.liquidatable(*id) {
                let debt = c.p.debt_raw(*id);
                c.run_case(rec, step, user, *id, debt, "1/1", false);
            }
        }
    }
}

fn edge_price(c: &Cal, id: u64, eps_ppb: i128, bonus: i128) -> i128 {
    c.total_debt(id) * (BPS + bonus) * 1_000_000_000 / (BPS * (1_000_000_000 + eps_ppb))
}

fn low_lt_edges(rec: &mut Recorder) -> usize {
    let mut c = Cal::new(100_000, LOW_LT, "low-lt-edge".to_string());
    let mut accounts = Vec::new();
    for pct in EDGE_BORROW_PCT {
        for eps in EDGE_EPS_PPB {
            let user = format!("edge-{pct}-{eps}");
            let raw = 2 * 1_000 * USDC_UNIT * i128::from(LOW_LT.ltv) / BPS * pct / 100;
            let id = c.open(&user, 2, raw).expect("edge account opens");
            accounts.push((user, id, eps));
        }
    }
    let mut refused_in_band = 0;
    for (n, (user, id, eps)) in accounts.into_iter().enumerate() {
        let lt = i128::from(LOW_LT.lt);
        let debt = c.total_debt(id);
        c.set_nav(debt * 7_550 / (2 * lt));
        let debt_raw = c.p.debt_raw(id);
        let bonus = c
            .estimate(id, debt_raw, &SeizeMode::Transfer)
            .expect("estimate below HF 1")
            .bonus_rate_bps;
        c.set_nav(edge_price(&c, id, eps, bonus));
        assert!(c.liquidatable(id), "{user}: liquidatable at the edge");
        let credit = n % 2 == 1;
        let mut settled = false;
        for (num, den, label) in [(1, 100, "1/100"), (1, 1, "1/1"), (3, 1, "3/1")] {
            let offer = (c.p.debt_raw(id) * num / den).max(1);
            if c.run_case(rec, 0, &user, id, offer, label, credit) {
                settled = true;
                break;
            }
        }
        if eps == 0 {
            assert!(!settled, "{user}: every offer reverts in the margin band");
            refused_in_band += 1;
            c.advance(86_400);
            let debt_raw = c.p.debt_raw(id);
            assert!(
                c.run_case(rec, 1, &user, id, debt_raw, "1/1", credit),
                "{user}: accrual ends the margin band"
            );
        } else {
            assert!(settled, "{user}: a rule-1 or rule-2 edge settles");
        }
        if c.exists(id) && c.p.debt_raw(id) > 0 {
            let debt = c.total_debt(id);
            c.set_nav(debt + 1);
            if c.liquidatable(id) {
                let offer = c.p.debt_raw(id) * 3 / 2;
                c.run_case(rec, 2, &user, id, offer, "3/2", !credit);
            }
        }
    }
    refused_in_band
}

#[test]
fn redteam_liqvid_calibration_whole_unit_corpus() {
    let out = env::var("REDTEAM_CALIB_OUT")
        .ok()
        .map(|path| BufWriter::new(File::create(path).expect("create calibration output")));
    let mut rec = Recorder {
        out,
        cases: 0,
        ok: 0,
    };
    for unit_cents in UNIT_CENTS {
        let start = rec.cases;
        liqvid_walk(&mut rec, unit_cents);
        println!("liqvid-{unit_cents}c: {} cases", rec.cases - start);
    }
    let start = rec.cases;
    let refused = low_lt_edges(&mut rec);
    println!("low-lt-edge: {} cases", rec.cases - start);
    if let Some(out) = rec.out.as_mut() {
        out.flush().expect("flush calibration output");
    }
    assert_eq!(refused, EDGE_BORROW_PCT.len());
    assert!(
        rec.cases >= MIN_CASES,
        "only {} calibration cases",
        rec.cases
    );
    assert!(
        rec.ok * 2 >= rec.cases,
        "{} of {} settled",
        rec.ok,
        rec.cases
    );
}
