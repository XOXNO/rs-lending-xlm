//! Seed libFuzzer corpora from existing Soroban test snapshots.
//!
//! Walks `*/test_snapshots/**/*.json`, extracts numeric fields (i128, u32, u64)
//! from `ledger_entries.contract_data` maps (MarketParams, MarketState,
//! positions, etc.), then packs them into per-target byte layouts matching
//! each fuzz target's byte layout.
//!
//! Flow seeds are fixed-width op streams. Numeric targets keep compact
//! structure-aware layouts seeded with realistic magnitudes (RAY indexes, bps
//! rates, timestamps, position amounts) before mutation.
//!
//! Usage:
//!   cargo run --release --features seed-corpus --bin seed_corpus -- --output corpus
//!
//! Snapshots that fail to parse are logged and skipped; never abort the run.

use common::constants::{BPS, MAX_BORROW_RATE_RAY, MILLISECONDS_PER_YEAR, RAY};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
// Types

/// Numeric entropy extracted from a single snapshot file.
///
/// Fields are multi-valued (one snapshot carries many market states, positions,
/// etc.); every combination a target can use is enumerated downstream.
#[derive(Debug, Default)]
struct ExtractedFields {
    // i128 values found anywhere in ledger_entries (indexes, amounts, rates).
    i128s: Vec<i128>,
    // u64 timestamps / nonces.
    u64s: Vec<u64>,
    // u32 values (asset_decimals, etc.).
    u32s: Vec<u32>,

    // Structured per-market fields, keyed when a symbol is identifiable.
    // These are far more useful for `rates_borrow` than a shapeless i128 heap.
    market_params: Vec<MarketParamsFields>,
    market_states: Vec<MarketStateFields>,
    // Position amounts found in account maps (scaled amounts).
    position_amounts: Vec<i128>,
}

#[derive(Debug, Clone, Default)]
struct MarketParamsFields {
    base_borrow_rate_ray: Option<i128>,
    slope1_ray: Option<i128>,
    slope2_ray: Option<i128>,
    slope3_ray: Option<i128>,
    mid_utilization_ray: Option<i128>,
    optimal_utilization_ray: Option<i128>,
    max_borrow_rate_ray: Option<i128>,
    reserve_factor_bps: Option<i128>,
    asset_decimals: Option<u32>,
}

#[derive(Debug, Clone, Default)]
struct MarketStateFields {
    supply_index_ray: Option<i128>,
    borrow_index_ray: Option<i128>,
    supplied_ray: Option<i128>,
    borrowed_ray: Option<i128>,
    revenue_ray: Option<i128>,
    last_timestamp: Option<u64>,
}
// Walking + parsing

fn walk_snapshots(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_dir(root, &mut out);
    out.sort();
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // Skip target dirs, node_modules, .git, etc.
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            // Recurse into nested directories; the file-name filter is applied below.
            walk_dir(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "json")
            .unwrap_or(false)
        {
            // Only pick up files under a `test_snapshots` ancestor.
            if path.components().any(|c| c.as_os_str() == "test_snapshots") {
                out.push(path);
            }
        }
    }
}

fn parse_i128_str(s: &str) -> Option<i128> {
    s.parse::<i128>().ok()
}

fn parse_u64_str(s: &str) -> Option<u64> {
    s.parse::<u64>().ok()
}

/// Extract an i128 value from a Soroban tagged-union JSON node.
/// Accepts forms: `{"i128": "123"}`, `{"i128": {"lo": "123", "hi": "0"}}`,
/// `{"i128": 123}`.
fn extract_i128(val: &Value) -> Option<i128> {
    let inner = val.get("i128")?;
    match inner {
        Value::String(s) => parse_i128_str(s),
        Value::Number(n) => n.as_i64().map(|i| i as i128),
        Value::Object(map) => {
            let lo = map.get("lo")?;
            let hi = map.get("hi")?;
            let lo_u64 = match lo {
                Value::String(s) => parse_u64_str(s)?,
                Value::Number(n) => n.as_u64()?,
                _ => return None,
            };
            let hi_i64 = match hi {
                Value::String(s) => s.parse::<i64>().ok()?,
                Value::Number(n) => n.as_i64()?,
                _ => return None,
            };
            Some(((hi_i64 as i128) << 64) | (lo_u64 as i128))
        }
        _ => None,
    }
}

fn extract_u64(val: &Value) -> Option<u64> {
    let inner = val.get("u64")?;
    match inner {
        Value::String(s) => parse_u64_str(s),
        Value::Number(n) => n.as_u64(),
        _ => None,
    }
}

fn extract_u32(val: &Value) -> Option<u32> {
    let inner = val.get("u32")?;
    match inner {
        Value::Number(n) => n.as_u64().map(|x| x as u32),
        Value::String(s) => s.parse::<u32>().ok(),
        _ => None,
    }
}

/// Walks the JSON tree and harvests every i128, u32, and u64 value.
fn harvest_numeric_rec(val: &Value, out: &mut ExtractedFields) {
    match val {
        Value::Object(map) => {
            // Grab leaf tagged-union values.
            if let Some(v) = extract_i128(val) {
                out.i128s.push(v);
            }
            if let Some(v) = extract_u64(val) {
                out.u64s.push(v);
            }
            if let Some(v) = extract_u32(val) {
                out.u32s.push(v);
            }
            for (_, v) in map.iter() {
                harvest_numeric_rec(v, out);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                harvest_numeric_rec(v, out);
            }
        }
        _ => {}
    }
}

/// If a `val` is a `{"map": [{"key":..., "val":...}, ...]}` node, return a
/// flat list of `(symbol_name, val_node)` pairs for keys that are symbols.
fn soroban_map_entries(val: &Value) -> Vec<(String, &Value)> {
    let mut out = Vec::new();
    let Some(map_arr) = val.get("map").and_then(|v| v.as_array()) else {
        return out;
    };
    for kv in map_arr {
        let k = kv.get("key");
        let v = kv.get("val");
        if let (Some(k), Some(v)) = (k, v) {
            if let Some(sym) = k.get("symbol").and_then(|s| s.as_str()) {
                out.push((sym.to_string(), v));
            }
        }
    }
    out
}

fn is_symbol_key(key_val: &Value, symbol: &str) -> bool {
    if let Some(vec) = key_val.get("vec").and_then(|v| v.as_array()) {
        if let Some(first) = vec.first() {
            if let Some(s) = first.get("symbol").and_then(|s| s.as_str()) {
                return s == symbol;
            }
        }
    }
    false
}

/// Walk contract_instance storage looking for Params / State maps, and any
/// position-like maps (amount/scaled_amount/debt fields).
fn harvest_structured(val: &Value, out: &mut ExtractedFields) {
    match val {
        Value::Object(map) => {
            // Contract instance storage?
            if let Some(ci) = map.get("contract_instance") {
                if let Some(storage) = ci.get("storage").and_then(|v| v.as_array()) {
                    for kv in storage {
                        let key = kv.get("key");
                        let inner_val = kv.get("val");
                        if let (Some(k), Some(v)) = (key, inner_val) {
                            if is_symbol_key(k, "Params") {
                                out.market_params.push(extract_market_params(v));
                            } else if is_symbol_key(k, "State") {
                                out.market_states.push(extract_market_state(v));
                            }
                        }
                    }
                }
            }

            // Any map with `amount` or `scaled_amount` or `borrowed`/`supplied` -- treat as position.
            let entries = soroban_map_entries(val);
            for (sym, v) in &entries {
                if matches!(
                    sym.as_str(),
                    "amount" | "scaled_amount" | "supplied" | "borrowed" | "debt" | "collateral"
                ) {
                    if let Some(x) = extract_i128(v) {
                        if x > 0 {
                            out.position_amounts.push(x);
                        }
                    }
                }
            }

            // Recurse everywhere.
            for (_, v) in map.iter() {
                harvest_structured(v, out);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                harvest_structured(v, out);
            }
        }
        _ => {}
    }
}

fn extract_market_params(val: &Value) -> MarketParamsFields {
    let mut p = MarketParamsFields::default();
    for (sym, v) in soroban_map_entries(val) {
        match sym.as_str() {
            "base_borrow_rate_ray" => p.base_borrow_rate_ray = extract_i128(v),
            "slope1_ray" => p.slope1_ray = extract_i128(v),
            "slope2_ray" => p.slope2_ray = extract_i128(v),
            "slope3_ray" => p.slope3_ray = extract_i128(v),
            "mid_utilization_ray" => p.mid_utilization_ray = extract_i128(v),
            "optimal_utilization_ray" => p.optimal_utilization_ray = extract_i128(v),
            "max_borrow_rate_ray" => p.max_borrow_rate_ray = extract_i128(v),
            "reserve_factor_bps" => p.reserve_factor_bps = extract_i128(v),
            "asset_decimals" => p.asset_decimals = extract_u32(v),
            _ => {}
        }
    }
    p
}

fn extract_market_state(val: &Value) -> MarketStateFields {
    let mut s = MarketStateFields::default();
    for (sym, v) in soroban_map_entries(val) {
        match sym.as_str() {
            "supply_index_ray" => s.supply_index_ray = extract_i128(v),
            "borrow_index_ray" => s.borrow_index_ray = extract_i128(v),
            "supplied_ray" => s.supplied_ray = extract_i128(v),
            "borrowed_ray" => s.borrowed_ray = extract_i128(v),
            "revenue_ray" => s.revenue_ray = extract_i128(v),
            "last_timestamp" => s.last_timestamp = extract_u64(v),
            _ => {}
        }
    }
    s
}

fn extract_snapshot(path: &Path) -> Option<ExtractedFields> {
    let data = fs::read_to_string(path).ok()?;
    let json: Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("warn: skipping {} ({})", path.display(), e);
            return None;
        }
    };

    let ledger_entries = json.get("ledger").and_then(|l| l.get("ledger_entries"))?;

    let mut fields = ExtractedFields::default();
    harvest_numeric_rec(ledger_entries, &mut fields);
    harvest_structured(ledger_entries, &mut fields);
    Some(fields)
}
// Packing (per target)

fn push_i128_le(buf: &mut Vec<u8>, v: i128) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_u64_le(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_u16_le(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_u32_le(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_i64_le(buf: &mut Vec<u8>, v: i64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// Pack `fp_math` seeds for the MulDiv, DivByInt, and Rescale arms.
/// Layout: kind, a, b, choice, extra in little-endian order.
fn pack_fp_math(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();

    let push = |out: &mut Vec<Vec<u8>>, kind: u8, a: i128, b: i128, choice: u8, extra: u8| {
        let mut buf = Vec::with_capacity(35);
        buf.push(kind);
        push_i128_le(&mut buf, a);
        push_i128_le(&mut buf, b);
        buf.push(choice);
        buf.push(extra);
        out.push(buf);
    };

    // --- MulDiv arm (kind = 0): pair i128s as (a, b); try all 3 divisors.
    // Index pairs (supply_idx, borrow_idx) exercise the realistic RAY*RAY/RAY
    // path the protocol actually hits during compounding.
    let mut pairs: Vec<(i128, i128)> = Vec::new();
    for s in &f.market_states {
        if let (Some(a), Some(b)) = (s.supply_index_ray, s.borrow_index_ray) {
            pairs.push((a, b));
        }
    }
    for chunk in f.i128s.chunks(2) {
        if chunk.len() == 2 {
            pairs.push((chunk[0], chunk[1]));
        }
    }
    for (a, b) in &pairs {
        for d in 0u8..3 {
            push(&mut out, 0, *a, *b, d, 0);
        }
    }

    // --- DivByInt arm (kind = 1): i128 pairs with b > 0.
    for chunk in f.i128s.chunks(2) {
        if chunk.len() < 2 {
            continue;
        }
        let (a, b) = (chunk[0], chunk[1]);
        if b <= 0 {
            continue;
        }
        push(&mut out, 1, a, b, 0, 0);
    }

    // --- Rescale arm (kind = 2): single i128 × common precision transitions.
    // 27->18 (RAY->WAD), 18->7 (WAD->asset_decimals=7), 18->6 (USDC), etc.
    let transitions: [(u8, u8); 8] = [
        (27, 18),
        (18, 27),
        (18, 7),
        (7, 18),
        (18, 6),
        (6, 18),
        (27, 4),
        (4, 27),
    ];
    for &a in f.i128s.iter().take(200) {
        for &(from, to) in &transitions {
            push(&mut out, 2, a, 0, from, to);
        }
    }

    out
}

/// `fp_ops`: 29 bytes matching `fuzz_targets/fp_ops.rs::In`.
fn pack_fp_ops(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut mags: BTreeSet<u64> = BTreeSet::new();
    for &v in f.i128s.iter().chain(f.position_amounts.iter()) {
        mags.insert(v.unsigned_abs().min(u64::MAX as u128) as u64);
    }
    for v in [0, 1, 10_000_000, 1_000_000_000_000_000_000, u64::MAX / 2] {
        mags.insert(v);
    }

    let mags: Vec<u64> = mags.into_iter().take(8).collect();
    for (idx, &a) in mags.iter().enumerate() {
        for &b in mags.iter().skip(idx).take(3) {
            for bps in [0u16, 1, 5_000, 10_000, 15_000] {
                for decimals in [0u8, 6, 7, 18, 27] {
                    let mut buf = Vec::with_capacity(29);
                    push_u64_le(&mut buf, a);
                    buf.push((idx & 1) as u8);
                    push_u64_le(&mut buf, b);
                    buf.push(((idx + 1) & 1) as u8);
                    push_u16_le(&mut buf, bps);
                    buf.push(decimals);
                    push_i64_le(&mut buf, (a.min(i64::MAX as u64) as i64) / 10);
                    out.push(buf);
                }
            }
        }
    }
    out
}

/// `rates_and_index`: 61 bytes matching the `In` struct in
/// `fuzz_targets/rates_and_index.rs`. The seed layout combines rate-model
/// geometry with accrual, borrow, and starting-index fields so mutations can
/// cross related inputs in one target.
fn pack_rates_and_index(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    const MS_PER_YEAR: i128 = MILLISECONDS_PER_YEAR as i128;

    // Extract realistic time deltas from market states + fixed ladder.
    let mut deltas: BTreeSet<u64> = BTreeSet::new();
    for s in &f.market_states {
        if let Some(t) = s.last_timestamp {
            deltas.insert(t);
        }
    }
    for &t in &f.u64s {
        if t > 0 && t < 1_000 * MS_PER_YEAR as u64 {
            deltas.insert(t);
        }
    }
    for d in [
        0u64,
        1_000,
        60_000,
        3_600_000,
        86_400_000,
        MS_PER_YEAR as u64,
        10 * MS_PER_YEAR as u64,
    ] {
        deltas.insert(d);
    }
    let delta_samples: Vec<u64> = deltas.iter().copied().take(4).collect();

    // Extract realistic borrowed amounts from position_amounts.
    let mut borroweds: BTreeSet<u64> = BTreeSet::new();
    for &a in &f.position_amounts {
        let v = a.clamp(0, u64::MAX as i128) as u64;
        borroweds.insert(v);
    }
    for b in [1_000u64, 1_000_000, 100_000_000_000, 1_000_000_000_000_000] {
        borroweds.insert(b);
    }
    let borrowed_samples: Vec<u64> = borroweds.iter().copied().take(4).collect();

    // Cap matches make_params (the decoder): the rate model is verified ≤ 2·RAY.
    const CAP: i128 = MAX_BORROW_RATE_RAY;
    for p in &f.market_params {
        // Inverse of make_params' decode: pick the byte each field will decode
        // back to (≈) its snapshot value, so seeds replay real high-rate curves
        // instead of collapsing to the low end. Cumulative slopes mirror the
        // decoder step by step using the decoded (not raw) running value.
        let pick = |v: Option<i128>| v.unwrap_or(0).max(0);

        let base_r = pick(p.base_borrow_rate_ray).min(CAP);
        let base_pct = (base_r * 1_024 / CAP).clamp(0, 255) as u8;
        let dbase = CAP * base_pct as i128 / 1_024;

        let s1_r = pick(p.slope1_ray).clamp(dbase, CAP);
        let s1_pct = ((s1_r - dbase) * 256 / (CAP - dbase).max(1)).clamp(0, 255) as u8;
        let ds1 = dbase + (CAP - dbase) * s1_pct as i128 / 256;

        let s2_r = pick(p.slope2_ray).clamp(ds1, CAP);
        let s2_pct = ((s2_r - ds1) * 256 / (CAP - ds1).max(1)).clamp(0, 255) as u8;
        let ds2 = ds1 + (CAP - ds1) * s2_pct as i128 / 256;

        let s3_r = pick(p.slope3_ray).clamp(ds2, CAP);
        let s3_pct = ((s3_r - ds2) * 65_536 / (CAP - ds2).max(1)).clamp(0, 65_535) as u16;
        let ds3 = ds2 + (CAP - ds2) * s3_pct as i128 / 65_536;

        let max_r = pick(p.max_borrow_rate_ray).clamp(ds3, CAP);
        let max_pct = ((max_r - ds3) * 65_536 / (CAP - ds3).max(1)).clamp(0, 65_535) as u16;

        // Breakpoints: the decoder recovers the percentage via `% N + 1`, so
        // write (percentage − 1).
        let mid_p = (pick(p.mid_utilization_ray) * 100 / RAY).clamp(1, 98);
        let mid_pct = (mid_p - 1) as u8;
        let dmid = RAY * mid_p / 100;
        let opt_frac = (101 * (pick(p.optimal_utilization_ray) - dmid).max(0)
            / (RAY - dmid).max(1))
        .clamp(1, 99);
        let opt_pct = (opt_frac - 1) as u8;

        let reserve_pct =
            (pick(p.reserve_factor_bps).clamp(0, BPS - 1) * 255 / (BPS - 1)).clamp(0, 255) as u8;

        // Keep seed count per market param bounded: 4 utils × 2 max-utils × 4
        // deltas × 4 borroweds = 128. Times ~1.4k snapshots → ~180k total seeds.
        // max_util has no snapshot source; seed two high-utilization variants.
        for util in [0u16, 5000, 9500, 10000] {
            for max_util_pct in [128u8, 230u8] {
                for &delta_ms in &delta_samples {
                    for &borrowed_units in &borrowed_samples {
                        let mut buf = Vec::with_capacity(61);
                        push_u16_le(&mut buf, util);
                        buf.push(base_pct);
                        buf.push(s1_pct);
                        buf.push(s2_pct);
                        push_u16_le(&mut buf, s3_pct);
                        buf.push(mid_pct);
                        buf.push(opt_pct);
                        push_u16_le(&mut buf, max_pct);
                        buf.push(max_util_pct);
                        buf.push(reserve_pct);
                        push_u64_le(&mut buf, delta_ms);
                        push_u64_le(&mut buf, borrowed_units);
                        // Appended In fields. Seeds use representative values;
                        // the fuzzer mutates and decorrelates from here.
                        push_u64_le(&mut buf, borrowed_units); // supplied_units → supplied ≈ 2× borrowed
                        push_u64_le(&mut buf, delta_ms); // chunk_units (decorrelated by mutation)
                        push_u64_le(&mut buf, 0); // borrow_index_units → start at RAY
                        push_u64_le(&mut buf, u64::MAX); // supply_index_units → supply ≈ borrow
                        out.push(buf);
                    }
                }
            }
        }
    }
    out
}

/// `pool_native`: 82 bytes matching `fuzz_targets/pool_native.rs::In`.
fn pack_pool_native(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut params = Vec::new();

    let to_pct = |v: Option<i128>, default: i128| -> u8 {
        v.map(|r| r * 100 / RAY)
            .unwrap_or(default)
            .clamp(0, u8::MAX as i128) as u8
    };

    for p in f.market_params.iter().take(12) {
        params.push((
            to_pct(p.base_borrow_rate_ray, 1),
            to_pct(p.slope1_ray, 4),
            to_pct(p.slope2_ray, 10),
            to_pct(p.slope3_ray, 150) as u16,
            to_pct(p.mid_utilization_ray, 50),
            to_pct(p.optimal_utilization_ray, 80),
            to_pct(p.max_borrow_rate_ray, 200) as u16,
            p.reserve_factor_bps
                .map(|r| (r / 100).clamp(0, 50) as u8)
                .unwrap_or(10),
        ));
    }

    if params.is_empty() {
        params.push((1, 4, 10, 150, 50, 80, 200, 10));
        params.push((0, 20, 75, 200, 30, 70, 200, 0));
    }

    let op_sets: [[(u32, u32, u8); 8]; 4] = [
        [
            (2_000, 0, 0),      // supply
            (3_000, 1, 1),      // borrow
            (4_000, 60, 2),     // withdraw
            (5_000, 3_600, 3),  // repay
            (6_000, 3_600, 4),  // update indexes
            (7_000, 86_400, 5), // add rewards
            (8_000, 86_400, 6), // update params
            (9_000, 0, 7),      // claim revenue
        ],
        [
            (1_000, 0, 8),        // seize borrow position
            (1_000, 3_600, 8),    // seize deposit position
            (1_000, 7_200, 9),    // create strategy
            (1_000, 604_800, 10), // views
            (1_000, 1, 0),
            (1_000, 1, 1),
            (1_000, 1, 2),
            (1_000, 1, 3),
        ],
        [
            (10, 8_640_000, 0),
            (100, 8_640_000, 1),
            (1_000, 8_640_000, 2),
            (10_000, 8_640_000, 3),
            (100_000, 8_640_000, 4),
            (1_000_000, 8_640_000, 5),
            (1_000_000, 8_640_000, 6),
            (1_000_000, 0, 7),
        ],
        [
            (500, 0, 10),
            (750, 120, 0),
            (1_250, 240, 1),
            (1_500, 360, 2),
            (1_750, 480, 3),
            (2_000, 600, 4),
            (2_250, 720, 5),
            (2_500, 840, 6),
        ],
    ];

    for (base, s1, s2, s3, mid, opt, max, reserve) in params {
        for ops in op_sets {
            let mut buf = Vec::with_capacity(82);
            buf.push(base);
            buf.push(s1);
            buf.push(s2);
            push_u16_le(&mut buf, s3);
            buf.push(mid);
            buf.push(opt);
            push_u16_le(&mut buf, max);
            buf.push(reserve);
            for (price, dt, kind) in ops {
                push_u32_le(&mut buf, price);
                push_u32_le(&mut buf, dt);
                buf.push(kind);
            }
            out.push(buf);
        }
    }
    out
}

fn flow_op(op: u8, a: u8, b: u8, c: u8, d: u8) -> [u8; 5] {
    [op, a, b, c, d]
}

fn flow_seq(ops: &[[u8; 5]]) -> Vec<u8> {
    ops.iter().flatten().copied().collect()
}

/// `flow_e2e`: 5-byte ops: `[op, user/debtor, asset, size/frac/hours, mode]`.
/// The target derives amounts from live positions, so these seeds are mostly
/// path selectors.
fn pack_flow_e2e(_f: &ExtractedFields) -> Vec<Vec<u8>> {
    vec![
        flow_seq(&[flow_op(0, 0, 0, 128, 0)]), // supply
        flow_seq(&[flow_op(1, 0, 2, 96, 0)]),  // borrow
        flow_seq(&[flow_op(3, 0, 2, 128, 0)]), // repay existing debt
        flow_seq(&[flow_op(2, 0, 0, 96, 0)]),  // partial withdraw
        flow_seq(&[flow_op(2, 1, 1, 0, 1)]),   // withdraw-all sentinel
        flow_seq(&[
            flow_op(1, 0, 2, 220, 0), // borrow near boundary
            flow_op(4, 0, 2, 192, 1), // stress prices, liquidate
        ]),
        flow_seq(&[flow_op(5, 0, 0, 96, 0)]), // good flash loan
        flow_seq(&[flow_op(5, 0, 0, 96, 1)]), // bad flash loan
        flow_seq(&[
            flow_op(8, 0, 0, 192, 0), // add rewards
            flow_op(7, 0, 0, 12, 0),  // accrue
            flow_op(9, 0, 0, 0, 0),   // claim revenue
        ]),
        flow_seq(&[
            flow_op(6, 0, 0, 200, 0), // oracle deviation
            flow_op(1, 0, 2, 64, 0),  // borrow under shifted oracle
        ]),
        flow_seq(&[flow_op(10, 1, 0, 0, 0)]), // clean bad debt attempt
    ]
}

/// `flow_strategy`: 5-byte ops: `[op, asset_a, asset_b, size/hours, mode]`.
fn pack_flow_strategy(_f: &ExtractedFields) -> Vec<Vec<u8>> {
    vec![
        flow_seq(&[flow_op(0, 0, 1, 96, 0)]),  // multiply
        flow_seq(&[flow_op(1, 2, 0, 96, 0)]),  // swap debt
        flow_seq(&[flow_op(2, 0, 1, 96, 0)]),  // swap collateral
        flow_seq(&[flow_op(3, 0, 2, 96, 0)]),  // repay with collateral
        flow_seq(&[flow_op(3, 0, 2, 255, 1)]), // close through collateral
        flow_seq(&[flow_op(4, 0, 0, 8, 0)]),   // accrue
        flow_seq(&[
            flow_op(0, 0, 1, 64, 1),
            flow_op(1, 2, 0, 128, 0),
            flow_op(2, 0, 1, 128, 0),
            flow_op(3, 1, 0, 192, 0),
        ]),
    ]
}
// Output

fn write_corpus(target: &str, inputs: Vec<Vec<u8>>, out_dir: &Path) -> std::io::Result<usize> {
    let target_dir = out_dir.join(target);
    fs::create_dir_all(&target_dir)?;
    let mut written = 0;
    let mut seen: BTreeSet<[u8; 8]> = BTreeSet::new();
    for bytes in inputs {
        if bytes.is_empty() {
            continue;
        }
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let full = hasher.finalize();
        let mut short = [0u8; 8];
        short.copy_from_slice(&full[..8]);
        if !seen.insert(short) {
            continue;
        }
        let name: String = short.iter().map(|b| format!("{:02x}", b)).collect();
        let path = target_dir.join(&name);
        if !path.exists() {
            fs::write(&path, &bytes)?;
        }
        written += 1;
    }
    Ok(written)
}
// Main

fn parse_args() -> PathBuf {
    let mut args = std::env::args().skip(1);
    let mut out = PathBuf::from("corpus");
    while let Some(a) = args.next() {
        match a.as_str() {
            "--output" | "-o" => {
                if let Some(v) = args.next() {
                    out = PathBuf::from(v);
                }
            }
            _ => {}
        }
    }
    out
}

fn main() -> std::io::Result<()> {
    let out_dir = parse_args();
    fs::create_dir_all(&out_dir)?;

    // Find the repo root from tests/fuzz/ (where cargo runs), unless
    // the binary is launched manually from repo root.
    let cwd = std::env::current_dir()?;
    let search_root = if cwd.ends_with("fuzz") {
        cwd.parent()
            .and_then(|p| p.parent())
            .unwrap_or(&cwd)
            .to_path_buf()
    } else {
        cwd.clone()
    };

    eprintln!("seed_corpus: scanning {}", search_root.display());
    let snapshots = walk_snapshots(&search_root);
    eprintln!("seed_corpus: found {} snapshot files", snapshots.len());

    let mut merged = ExtractedFields::default();
    let mut parsed = 0usize;
    let mut skipped = 0usize;

    for path in &snapshots {
        match extract_snapshot(path) {
            Some(f) => {
                parsed += 1;
                merged.i128s.extend(f.i128s);
                merged.u64s.extend(f.u64s);
                merged.u32s.extend(f.u32s);
                merged.market_params.extend(f.market_params);
                merged.market_states.extend(f.market_states);
                merged.position_amounts.extend(f.position_amounts);
            }
            None => skipped += 1,
        }
    }

    // De-duplicate numeric vectors to keep pack sizes sane.
    merged.i128s.sort_unstable();
    merged.i128s.dedup();
    merged.u64s.sort_unstable();
    merged.u64s.dedup();
    merged.u32s.sort_unstable();
    merged.u32s.dedup();
    merged.position_amounts.sort_unstable();
    merged.position_amounts.dedup();

    eprintln!(
        "seed_corpus: parsed={} skipped={} | i128s={} u64s={} market_params={} market_states={} positions={}",
        parsed,
        skipped,
        merged.i128s.len(),
        merged.u64s.len(),
        merged.market_params.len(),
        merged.market_states.len(),
        merged.position_amounts.len(),
    );

    // Per-target packing.
    let targets: Vec<(&str, Vec<Vec<u8>>)> = vec![
        ("fp_math", pack_fp_math(&merged)),
        ("fp_ops", pack_fp_ops(&merged)),
        ("rates_and_index", pack_rates_and_index(&merged)),
        ("flow_e2e", pack_flow_e2e(&merged)),
        ("flow_strategy", pack_flow_strategy(&merged)),
        ("pool_native", pack_pool_native(&merged)),
    ];

    for (target, inputs) in targets {
        if inputs.is_empty() {
            eprintln!("seed_corpus: {}: no inputs packed -- skipping", target);
            continue;
        }
        let n = write_corpus(target, inputs, &out_dir)?;
        eprintln!("seed_corpus: {}: wrote {} files", target, n);
    }

    Ok(())
}
