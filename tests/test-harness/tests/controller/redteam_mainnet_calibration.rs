use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};

use common::math::fp_core::mul_div_floor;
use common::types::{
    AccountPositionRaw, DebtPositionRaw, HubAssetKey, LiquidationEstimate, PaymentTuple, PoolKey,
    PoolStateRaw, SeizeMode,
};
use controller::constants::{RAY, WAD};
use serde_json::{json, Map, Value};
use soroban_sdk::testutils::Events;
use soroban_sdk::xdr::ScVal;
use soroban_sdk::{vec as soroban_vec, Map as SorobanMap, Vec as SorobanVec};
use test_harness::mainnet::MainnetSpoke;
use test_harness::reference::{exact_totals, snapshot_exact, ExactBook, ExactPlan};
use test_harness::LIQUIDATOR;

use crate::redteam_mainnet_smoke::assert_estimate_matches;
use crate::shared::data_for_topic;

const ACCOUNTS: usize = 12;
const CASES_PER_CHECKPOINT: usize = 2;
const MAX_STEPS: usize = 40;
const INSOLVENT_CHECKPOINTS: usize = 3;
const STEP_SECS: u64 = 6 * 3_600;
const WARMUP_SECS: u64 = 45 * 86_400;
const MIN_CASES: usize = 300;
const OFFERS: [(i128, i128, &str); 4] =
    [(1, 10, "1/10"), (1, 3, "1/3"), (1, 1, "1/1"), (3, 2, "3/2")];

struct Scenario {
    spoke: u32,
    collateral: &'static [(&'static str, i128)],
    debt: &'static [(&'static str, i128)],
    movers: &'static [(&'static str, i128)],
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        spoke: 1,
        collateral: &[("XLM", 1_000)],
        debt: &[("USDC", 1)],
        movers: &[("XLM", -300)],
    },
    Scenario {
        spoke: 1,
        collateral: &[("XLM", 600), ("SolvBTC", 600)],
        debt: &[("USDC", 1), ("EURC", 1)],
        movers: &[("XLM", -300), ("SolvBTC", -200)],
    },
    Scenario {
        spoke: 1,
        collateral: &[("USDC", 800), ("xSolvBTC", 400), ("XLM", 300)],
        debt: &[("SolvBTC", 1), ("EURC", 1)],
        movers: &[("xSolvBTC", -300), ("SolvBTC", 200), ("XLM", -200)],
    },
    Scenario {
        spoke: 1,
        collateral: &[("XLM", 15)],
        debt: &[("USDC", 1)],
        movers: &[("XLM", -500)],
    },
    Scenario {
        spoke: 2,
        collateral: &[("USTRY", 1_000)],
        debt: &[("XLM", 1)],
        movers: &[("XLM", 400)],
    },
    Scenario {
        spoke: 2,
        collateral: &[("USTRY", 500), ("CETES", 500)],
        debt: &[("XLM", 2), ("USDC", 1)],
        movers: &[("XLM", 400), ("CETES", -100)],
    },
    Scenario {
        spoke: 2,
        collateral: &[("CETES", 30)],
        debt: &[("XLM", 1), ("PYUSD", 1)],
        movers: &[("XLM", 500)],
    },
    Scenario {
        spoke: 3,
        collateral: &[("DEJTRSY", 1_000)],
        debt: &[("XLM", 1)],
        movers: &[("XLM", 400)],
    },
    Scenario {
        spoke: 3,
        collateral: &[("DEJTRSY", 400), ("DEJAAA", 600)],
        debt: &[("USDC", 1), ("XLM", 1)],
        movers: &[("XLM", 400), ("DEJAAA", -100)],
    },
    Scenario {
        spoke: 3,
        collateral: &[("DEJAAA", 25)],
        debt: &[("XLM", 1), ("EURC", 1)],
        movers: &[("XLM", 600)],
    },
    Scenario {
        spoke: 4,
        collateral: &[("EURC", 1_000)],
        debt: &[("USDC", 1)],
        movers: &[("EURC", -50)],
    },
    Scenario {
        spoke: 4,
        collateral: &[("USST", 500), ("USDY", 500), ("PYUSD", 300)],
        debt: &[("EURC", 1), ("USDT0", 1)],
        movers: &[("EURC", 100), ("USDY", -100), ("USST", -100)],
    },
    Scenario {
        spoke: 4,
        collateral: &[("USDY", 800)],
        debt: &[("EURC", 1), ("USDC", 1)],
        movers: &[("USDY", -100), ("EURC", 100)],
    },
    Scenario {
        spoke: 4,
        collateral: &[("USDT0", 12)],
        debt: &[("PYUSD", 1)],
        movers: &[("USDT0", -50), ("PYUSD", 50)],
    },
    Scenario {
        spoke: 5,
        collateral: &[("XLMSolvBTC_LP", 1_000)],
        debt: &[("USDC", 1)],
        movers: &[("XLMSolvBTC_LP", -300)],
    },
    Scenario {
        spoke: 5,
        collateral: &[
            ("xSolvBTCSolvBTC_LP", 600),
            ("AQUAUSDC_LP", 300),
            ("XLMAQUA_LP", 300),
        ],
        debt: &[("XLM", 1), ("EURC", 1)],
        movers: &[("XLM", 400), ("AQUAUSDC_LP", -300)],
    },
    Scenario {
        spoke: 5,
        collateral: &[("USTRYUSDC_LP", 40)],
        debt: &[("XLM", 1)],
        movers: &[("XLM", 500)],
    },
    Scenario {
        spoke: 6,
        collateral: &[("USDY", 1_000)],
        debt: &[("XLM", 1)],
        movers: &[("XLM", 300)],
    },
    Scenario {
        spoke: 6,
        collateral: &[("USDY", 500), ("USDYUSDC_LP", 500)],
        debt: &[("USDC", 1), ("XLM", 2)],
        movers: &[("XLM", 400), ("USDYUSDC_LP", -200)],
    },
    Scenario {
        spoke: 6,
        collateral: &[("USDYUSDC_LP", 20)],
        debt: &[("PYUSD", 1), ("EURC", 1)],
        movers: &[("USDYUSDC_LP", -150), ("EURC", 100)],
    },
    Scenario {
        spoke: 7,
        collateral: &[("XAUM", 300)],
        debt: &[("XLM", 1)],
        movers: &[("XLM", 300)],
    },
    Scenario {
        spoke: 7,
        collateral: &[("XAUM", 250), ("XAUMUSDC_LP", 250)],
        debt: &[("USDC", 1), ("XLM", 1)],
        movers: &[("XLM", 400), ("XAUM", -300)],
    },
    Scenario {
        spoke: 7,
        collateral: &[("XAUM", 15)],
        debt: &[("EURC", 1), ("PYUSD", 1)],
        movers: &[("XAUM", -400)],
    },
    Scenario {
        spoke: 8,
        collateral: &[("AQUA", 1_000)],
        debt: &[("XLM", 1)],
        movers: &[("XLM", 300), ("AQUA", -300)],
    },
    Scenario {
        spoke: 8,
        collateral: &[("AQUA", 400), ("XLMAQUA_LP", 300), ("AQUAUSDC_LP", 300)],
        debt: &[("USDC", 1), ("EURC", 1), ("XLM", 1)],
        movers: &[("AQUA", -400), ("XLMAQUA_LP", -300)],
    },
    Scenario {
        spoke: 8,
        collateral: &[("AQUAUSDC_LP", 12)],
        debt: &[("USDC", 1)],
        movers: &[("AQUAUSDC_LP", -400)],
    },
];

fn s(x: i128) -> Value {
    Value::String(x.to_string())
}

fn asset_name(m: &MainnetSpoke, key: &HubAssetKey) -> String {
    m.config
        .listings
        .iter()
        .find(|l| m.hub_asset(&l.asset) == *key)
        .map(|l| l.asset.clone())
        .expect("position asset is listed on the spoke")
}

fn all_keys(m: &MainnetSpoke) -> SorobanVec<HubAssetKey> {
    let mut keys = SorobanVec::new(&m.t.env);
    for l in &m.config.listings {
        keys.push_back(m.hub_asset(&l.asset));
    }
    keys
}

fn advance(m: &mut MainnetSpoke, secs: u64) {
    m.t.advance_time(secs);
    let keys = all_keys(m);
    m.t.ctrl_client().update_indexes(&m.t.keeper, &keys);
}

fn seed_pool_indexes(m: &MainnetSpoke, salt: usize) {
    let env = &m.t.env;
    for (i, l) in m.config.listings.iter().enumerate() {
        let key = PoolKey::State(m.hub_asset(&l.asset));
        let market = m.t.resolve_market(&l.asset);
        let n = (salt * 7 + i * 13) as i128;
        let supply_index = RAY + RAY / 1_000 * (17 + n % 83) + 123_456_789 * (n + 1);
        let borrow_index = supply_index + RAY / 1_000 * (5 + n % 41) + 987_654_321;
        let unit = 10i128.pow(27 - market.decimals);
        env.as_contract(&market.pool, || {
            let mut st: PoolStateRaw = env
                .storage()
                .persistent()
                .get(&key)
                .expect("pool state exists");
            st.supply_index = supply_index;
            st.borrow_index = borrow_index;
            st.supplied = mul_div_floor(env, st.cash * unit, RAY, supply_index);
            st.borrowed = mul_div_floor(env, st.supplied * 3 / 10, supply_index, borrow_index);
            env.storage().persistent().set(&key, &st);
        });
    }
}

fn pool_state(m: &MainnetSpoke, asset: &str) -> Value {
    let st =
        m.t.pool_client(asset)
            .get_sync_data(&m.hub_asset(asset))
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

fn pool_states(m: &MainnetSpoke) -> Value {
    let mut out = Map::new();
    for l in &m.config.listings {
        out.insert(l.asset.clone(), pool_state(m, &l.asset));
    }
    Value::Object(out)
}

fn balances(m: &MainnetSpoke) -> BTreeMap<String, i128> {
    m.config
        .listings
        .iter()
        .map(|l| (l.asset.clone(), m.t.token_balance_raw(LIQUIDATOR, &l.asset)))
        .collect()
}

type Positions = (
    SorobanMap<HubAssetKey, AccountPositionRaw>,
    SorobanMap<HubAssetKey, DebtPositionRaw>,
);

fn positions(m: &MainnetSpoke, account_id: u64) -> Option<Positions> {
    m.t.ctrl_client()
        .try_get_account_positions(&account_id)
        .ok()
        .and_then(Result::ok)
}

fn supply_json(m: &MainnetSpoke, side: &SorobanMap<HubAssetKey, AccountPositionRaw>) -> Value {
    Value::Array(
        side.iter()
            .map(|(key, p)| {
                json!({
                    "asset": asset_name(m, &key),
                    "scaled": s(p.scaled_amount),
                    "ltv": p.loan_to_value,
                    "lt": p.liquidation_threshold,
                    "bonus": p.liquidation_bonus,
                    "fee": p.liquidation_fees,
                })
            })
            .collect(),
    )
}

fn borrow_json(m: &MainnetSpoke, side: &SorobanMap<HubAssetKey, DebtPositionRaw>) -> Value {
    Value::Array(
        side.iter()
            .map(|(key, p)| json!({"asset": asset_name(m, &key), "scaled": s(p.scaled_amount)}))
            .collect(),
    )
}

fn positions_json(m: &MainnetSpoke, account_id: u64) -> Value {
    match positions(m, account_id) {
        None => Value::Null,
        Some((supply, borrow)) => json!({
            "supply": supply_json(m, &supply),
            "borrow": borrow_json(m, &borrow),
        }),
    }
}

fn liquidatable(m: &MainnetSpoke, account_id: u64) -> bool {
    match positions(m, account_id) {
        Some((_, borrow)) if !borrow.is_empty() => {
            let book = snapshot_exact(&m.t, account_id);
            exact_totals(&book.collateral, &book.debt).health_factor < WAD
        }
        _ => false,
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

fn event_fields(data: &ScVal) -> BTreeMap<String, i128> {
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

fn book_json(m: &MainnetSpoke, book: &ExactBook) -> Value {
    let mut markets = Map::new();
    for key in &book.keys {
        let asset = asset_name(m, key);
        let view =
            m.t.ctrl_client()
                .get_market_indexes_detailed(&soroban_vec![&m.t.env, key.clone()])
                .get(0)
                .expect("market index view");
        let l = m.listing(&asset);
        markets.insert(
            asset.clone(),
            json!({
                "hub_id": key.hub_id,
                "decimals": m.t.resolve_market(&asset).decimals,
                "price_wad": s(view.price_wad),
                "supply_index": s(view.supply_index),
                "borrow_index": s(view.borrow_index),
                "listing": {
                    "ltv": l.ltv,
                    "lt": l.liquidation_threshold,
                    "bonus": l.liquidation_bonus,
                    "fee": l.liquidation_fees,
                },
                "pool": pool_state(m, &asset),
            }),
        );
    }
    Value::Object(markets)
}

fn plan_json(m: &MainnetSpoke, book: &ExactBook, plan: &ExactPlan) -> Value {
    let name = |id: u32| asset_name(m, book.key_of(id));
    let t = &plan.totals;
    json!({
        "quote_usd": s(plan.quote_usd),
        "bonus_bps": s(plan.bonus_bps),
        "repay_usd": s(plan.repay_usd),
        "seize_all": plan.seize_all,
        "repaid": plan.repaid.iter().map(|(id, a)| json!([name(*id), s(*a)])).collect::<Vec<_>>(),
        "refunds": plan.refunds.iter().map(|(id, a)| json!([name(*id), s(*a)])).collect::<Vec<_>>(),
        "seized": plan.seized.iter().map(|x| json!({
            "asset": name(x.asset_id),
            "amount": s(x.amount),
            "protocol_fee": s(x.protocol_fee),
            "scaled": s(x.scaled_amount),
            "bonus_scaled": s(x.bonus_scaled),
            "credit_fee_scaled": s(x.credit_fee_scaled),
        })).collect::<Vec<_>>(),
        "totals": {
            "C": s(t.total_collateral),
            "W": s(t.weighted_collateral),
            "D": s(t.total_debt),
            "HF": s(t.health_factor),
            "p": s(t.proportion_seized),
            "base": s(t.base_bonus_bps),
            "max": s(t.max_bonus_bps),
        },
    })
}

fn estimate_json(m: &MainnetSpoke, e: &LiquidationEstimate) -> Value {
    let pairs = |v: &SorobanVec<PaymentTuple>| {
        v.iter()
            .map(|p| {
                let key = m
                    .config
                    .listings
                    .iter()
                    .find(|l| m.t.resolve_asset(&l.asset) == p.asset)
                    .expect("estimate asset is listed");
                json!([key.asset, s(p.amount)])
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

struct Recorder {
    out: Option<BufWriter<File>>,
    cases: usize,
}

impl Recorder {
    fn write(&mut self, case: Value) {
        self.cases += 1;
        if let Some(out) = self.out.as_mut() {
            writeln!(out, "{case}").expect("write calibration case");
        }
    }
}

fn open_account(m: &mut MainnetSpoke, user: &str, sc: &Scenario) -> u64 {
    let share = ACCOUNTS as i128 + 1;
    for (asset, usd) in sc.collateral {
        let cap = m.listing(asset).supply_cap / share;
        let amount = m.usd_to_raw(asset, usd * WAD).min(cap);
        m.supply(user, asset, amount);
    }
    let account_id = m.account_id(user);
    let ltv_usd = m.t.ctrl_client().get_ltv_collateral_usd(&account_id);
    let weights: i128 = sc.debt.iter().map(|(_, w)| w).sum();
    for (i, (asset, w)) in sc.debt.iter().enumerate() {
        let usd = if i + 1 == sc.debt.len() {
            ltv_usd - m.t.ctrl_client().get_total_borrow_usd(&account_id)
        } else {
            ltv_usd * w / weights
        };
        let cap = m.listing(asset).borrow_cap / share;
        let mut amount = m.usd_to_raw(asset, usd).min(cap);
        loop {
            assert!(amount > 0, "{user}: no {asset} borrow fits");
            if m.try_borrow(user, asset, amount).is_ok() {
                break;
            }
            amount = amount * 9_990 / 10_000;
        }
    }
    account_id
}

fn run_case(
    m: &mut MainnetSpoke,
    rec: &mut Recorder,
    scenario: usize,
    step: usize,
    user: &str,
    combo: usize,
) -> bool {
    let account_id = m.account_id(user);
    let (num, den, offer) = OFFERS[combo / 2];
    let (mode, credit) = if combo.is_multiple_of(2) {
        (SeizeMode::Transfer, false)
    } else {
        (SeizeMode::Credit(0), true)
    };

    let book = snapshot_exact(&m.t, account_id);
    let payments: Vec<(String, i128)> = book
        .debt
        .iter()
        .map(|d| {
            let asset = asset_name(m, book.key_of(d.asset_id));
            (asset, (d.balance_ceil() * num / den).max(1))
        })
        .collect();
    let pay: Vec<(&str, i128)> = payments.iter().map(|(a, x)| (a.as_str(), *x)).collect();

    let (book, plan) = m.reference_plan(user, &pay);
    let context = format!("scenario {scenario} step {step} {user} offer {offer}");
    assert_eq!(
        m.health_factor_raw(user),
        plan.totals.health_factor,
        "{context}: health factor"
    );
    let mut estimates = Vec::new();
    for (mode_i, credit_i) in [(SeizeMode::Transfer, false), (SeizeMode::Credit(0), true)] {
        let estimate = m.estimate(user, &pay, mode_i);
        assert_estimate_matches(&book, &plan, &estimate, credit_i, &context);
        estimates.push(estimate);
    }
    let estimate = &estimates[usize::from(credit)];

    let collateral = positions_json(m, account_id);
    let markets = book_json(m, &book);
    let pools_before = pool_states(m);
    let before = balances(m);

    let result = m.try_liquidate(LIQUIDATOR, user, &pay, mode);
    let events = m.t.env.events().all();

    let outcome = match result {
        Err(e) => json!({ "ok": false, "error": format!("{e:?}") }),
        Ok(receiver_id) => {
            let liq = data_for_topic(&events, "position", "liquidation");
            assert_eq!(liq.len(), 1, "{context}: one liquidation event");
            let liq = event_fields(&liq[0]);
            let cleanup_event = data_for_topic(&events, "debt", "bad_debt")
                .first()
                .map(event_fields);
            let after = balances(m);
            let delta: Map<String, Value> = after
                .iter()
                .map(|(a, x)| (a.clone(), s(x - before[a])))
                .collect();
            let account_after = positions_json(m, account_id);
            let (cleanup, post) = match (&cleanup_event, positions(m, account_id)) {
                (Some(ev), _) => (
                    "bad_debt_cleanup",
                    json!({"C": s(ev["total_collateral_usd_wad"]), "D": s(ev["total_borrow_usd_wad"])}),
                ),
                (None, None) => ("account_removed", json!({"C": s(0), "D": s(0)})),
                (None, Some((supply, borrow))) => {
                    let book = snapshot_exact(&m.t, account_id);
                    let t = exact_totals(&book.collateral, &book.debt);
                    let label = match (supply.is_empty(), borrow.is_empty()) {
                        (true, true) => "account_removed",
                        (false, true) => "debt_free",
                        _ => "none",
                    };
                    (
                        label,
                        json!({
                            "C": s(t.total_collateral),
                            "W": s(t.weighted_collateral),
                            "D": s(t.total_debt),
                            "HF": s(t.health_factor),
                        }),
                    )
                }
            };
            json!({
                "ok": true,
                "receiver_id": receiver_id,
                "event": {
                    "repaid_usd_wad": s(liq["repaid_usd_wad"]),
                    "bonus_bps": s(liq["bonus_bps"]),
                },
                "liquidator_delta": Value::Object(delta),
                "account_after": account_after,
                "receiver_after": if receiver_id == 0 { Value::Null } else { positions_json(m, receiver_id) },
                "pools_after": pool_states(m),
                "cleanup": cleanup,
                "post": post,
            })
        }
    };

    let ok = outcome["ok"] == Value::Bool(true);
    rec.write(json!({
        "case": rec.cases,
        "scenario": scenario,
        "spoke": m.config.onchain_id,
        "step": step,
        "user": user,
        "account_id": account_id,
        "offer": offer,
        "mode": if credit { "Credit" } else { "Transfer" },
        "curve": {
            "H": s(m.config.curve.target_hf_wad),
            "K": s(m.config.curve.hf_for_max_bonus_wad),
            "f": s(m.config.curve.bonus_factor_bps),
        },
        "markets": markets,
        "positions": collateral,
        "pools_before": pools_before,
        "payments": payments.iter().map(|(a, x)| json!([a, s(*x)])).collect::<Vec<_>>(),
        "mirror": plan_json(m, &book, &plan),
        "estimate": estimate_json(m, estimate),
        "result": outcome,
    }));
    ok
}

fn run_scenario(index: usize, sc: &Scenario, rec: &mut Recorder) -> usize {
    let mut m = MainnetSpoke::build(sc.spoke);
    m.t.get_or_create_user(LIQUIDATOR);
    if index % 2 == 1 {
        seed_pool_indexes(&m, index);
    }
    let users: Vec<String> = (0..ACCOUNTS).map(|k| format!("cal{index}_{k}")).collect();
    for user in &users {
        open_account(&mut m, user, sc);
    }
    advance(&mut m, WARMUP_SECS);

    let start = rec.cases;
    let mut cursor = 0usize;
    let mut combo = 0usize;
    let mut insolvent_checkpoints = 0;
    for step in 0..MAX_STEPS {
        let mut moved = false;
        for (asset, bps) in sc.movers {
            let next = m.price(asset) * (10_000 + bps) / 10_000;
            if m.price_in_band(asset, next) {
                m.move_price_bps(asset, *bps);
                moved = true;
            }
        }
        if !moved {
            break;
        }
        advance(&mut m, STEP_SECS);

        let mut taken = 0;
        let mut scanned = 0;
        let mut insolvent = false;
        while taken < CASES_PER_CHECKPOINT && scanned < users.len() {
            let user = users[cursor % users.len()].clone();
            cursor += 1;
            scanned += 1;
            let Some(account_id) = m.t.find_account_id(&user) else {
                continue;
            };
            if !liquidatable(&m, account_id) {
                continue;
            }
            let book = snapshot_exact(&m.t, account_id);
            let t = exact_totals(&book.collateral, &book.debt);
            insolvent |= t.total_collateral < t.total_debt;
            run_case(&mut m, rec, index, step, &user, combo % 8);
            combo += 1;
            taken += 1;
        }
        if insolvent {
            insolvent_checkpoints += 1;
            if insolvent_checkpoints >= INSOLVENT_CHECKPOINTS {
                break;
            }
        }
    }
    let cases = rec.cases - start;
    assert!(
        cases > 0,
        "scenario {index} on spoke {} never became liquidatable",
        sc.spoke
    );
    cases
}

#[test]
fn redteam_calibration_estimate_matches_exact_reference_across_mainnet_spokes() {
    let out = env::var("REDTEAM_CALIB_OUT")
        .ok()
        .map(|path| BufWriter::new(File::create(path).expect("create calibration output")));
    let mut rec = Recorder { out, cases: 0 };
    let mut spokes = BTreeSet::new();
    for (index, sc) in SCENARIOS.iter().enumerate() {
        let cases = run_scenario(index, sc, &mut rec);
        spokes.insert(sc.spoke);
        println!("scenario {index} spoke {}: {cases} cases", sc.spoke);
    }
    if let Some(out) = rec.out.as_mut() {
        out.flush().expect("flush calibration output");
    }
    assert_eq!(
        spokes.into_iter().collect::<Vec<_>>(),
        (1..=8).collect::<Vec<u32>>()
    );
    assert!(
        rec.cases >= MIN_CASES,
        "only {} calibration cases",
        rec.cases
    );
}
