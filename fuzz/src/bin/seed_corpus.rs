//! Seed libFuzzer corpora from existing Soroban test snapshots.
//!
//! Walks `*/test_snapshots/**/*.json`, extracts numeric fields (i128, u32, u64)
//! from `ledger_entries.contract_data` maps (MarketParams, MarketState,
//! positions, etc.), then packs them into per-target byte layouts matching
//! each fuzz target's `Arbitrary`-derived input struct.
//!
//! The goal is NOT byte-perfect matching of the `Arbitrary` decoder — libFuzzer
//! tolerates short/long/imperfect seeds as long as they decode to a valid input.
//! The real payoff is populating the input space with realistic numeric
//! magnitudes (RAY indexes, bps rates, timestamps, position amounts) so the
//! mutation engine has something non-trivial to bit-flip from iteration 0.
//!
//! Usage:
//!   cargo run --release --bin seed_corpus -- --output corpus
//!
//! Snapshots that fail to parse are logged and skipped; never abort the run.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

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

    // Structured per-market fields — when we can identify them by symbol key.
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

// ---------------------------------------------------------------------------
// Walking + parsing
// ---------------------------------------------------------------------------

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
            // Recurse into any dir; we rely on the file-name filter below.
            walk_dir(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "json")
            .unwrap_or(false)
        {
            // Only pick up files under a `test_snapshots` ancestor.
            if path
                .components()
                .any(|c| c.as_os_str() == "test_snapshots")
            {
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

/// Walk the entire JSON tree, harvesting every i128/u32/u64 we see.
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

            // Any map with `amount` or `scaled_amount` or `borrowed`/`supplied` — treat as position.
            let entries = soroban_map_entries(val);
            for (sym, v) in &entries {
                if matches!(
                    sym.as_str(),
                    "amount"
                        | "scaled_amount"
                        | "supplied"
                        | "borrowed"
                        | "debt"
                        | "collateral"
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

// ---------------------------------------------------------------------------
// Packing (per target)
// ---------------------------------------------------------------------------

fn push_i128_le(buf: &mut Vec<u8>, v: i128) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_u64_le(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_u32_le(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_u16_le(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// `fp_mul_div`: In { a: i128, b: i128, d_choice: u8 } — 33 bytes LE.
fn pack_fp_mul_div(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    // Pair consecutive i128s as (a, b). Also include index-pair seeds from
    // market states, which exercise the realistic RAY * RAY / RAY path.
    let mut pairs: Vec<(i128, i128)> = Vec::new();
    for s in &f.market_states {
        if let (Some(a), Some(b)) = (s.supply_index_ray, s.borrow_index_ray) {
            pairs.push((a, b));
        }
    }
    // Also any amount * index pairs we can find.
    for chunk in f.i128s.chunks(2) {
        if chunk.len() == 2 {
            pairs.push((chunk[0], chunk[1]));
        }
    }
    for (i, (a, b)) in pairs.into_iter().enumerate() {
        for d in 0u8..3 {
            let mut buf = Vec::with_capacity(33);
            push_i128_le(&mut buf, a);
            push_i128_le(&mut buf, b);
            buf.push(d);
            out.push(buf);
            // Cap at ~3 divisor variants per pair to avoid blowup.
            let _ = i;
        }
    }
    out
}

/// `fp_rescale`: In { a: i128, from: u8, to: u8 } — 18 bytes.
fn pack_fp_rescale(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    // Common precision transitions in the protocol: 27→18 (RAY→WAD), 18→7
    // (WAD→asset_decimals=7), 18→6 (USDC), 4→18, 27→4, etc.
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
            let mut buf = Vec::with_capacity(18);
            push_i128_le(&mut buf, a);
            buf.push(from);
            buf.push(to);
            out.push(buf);
        }
    }
    out
}

/// `fp_div_by_int`: In { a: i128, b: i128 } — 32 bytes.
fn pack_fp_div_by_int(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    // Pair i128s; skip when b <= 0 since the target short-circuits on that.
    for chunk in f.i128s.chunks(2) {
        if chunk.len() < 2 {
            continue;
        }
        let (a, b) = (chunk[0], chunk[1]);
        if b <= 0 {
            continue;
        }
        let mut buf = Vec::with_capacity(32);
        push_i128_le(&mut buf, a);
        push_i128_le(&mut buf, b);
        out.push(buf);
    }
    out
}

/// `rates_borrow`: In { util_bps: u16, base_pct: u8, s1_pct: u8, s2_pct: u8,
///                      s3_pct: u16, mid_pct: u8, opt_pct: u8, max_pct: u16,
///                      flip: u8 }
///
/// That's 2+1+1+1+2+1+1+2+1 = 12 bytes matching Arbitrary's derive-default LE
/// decoding (integers are consumed LE, bytes in field order).
fn pack_rates_borrow(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;
    for p in &f.market_params {
        // Convert RAY-denominated params to the %-scale the target uses.
        // RAY * pct / 100 == raw  =>  pct = raw * 100 / RAY.
        let to_pct = |v: Option<i128>| -> i128 { v.map(|r| r * 100 / RAY).unwrap_or(0) };
        let base_pct = to_pct(p.base_borrow_rate_ray).clamp(0, 50) as u8;
        let s1_pct = to_pct(p.slope1_ray).clamp(0, 50) as u8;
        let s2_pct = to_pct(p.slope2_ray).clamp(0, 100) as u8;
        let s3_pct = to_pct(p.slope3_ray).clamp(0, 500) as u16;
        let mid_pct = to_pct(p.mid_utilization_ray).clamp(1, 98) as u8;
        let opt_pct = to_pct(p.optimal_utilization_ray).clamp(mid_pct as i128 + 1, 99) as u8;
        let max_pct = to_pct(p.max_borrow_rate_ray).clamp(1, 1000) as u16;

        for util in [0u16, 2500, 5000, 7500, 9500, 9999, 10000] {
            for flip in [0u8, 1, 7, 8] {
                let mut buf = Vec::with_capacity(12);
                push_u16_le(&mut buf, util);
                buf.push(base_pct);
                buf.push(s1_pct);
                buf.push(s2_pct);
                push_u16_le(&mut buf, s3_pct);
                buf.push(mid_pct);
                buf.push(opt_pct);
                push_u16_le(&mut buf, max_pct);
                buf.push(flip);
                out.push(buf);
            }
        }
    }
    out
}

/// `compound_monotonic`: In { rate_apr_bps: u16, delta_ms: u64 } — 10 bytes.
fn pack_compound_monotonic(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    const RAY: i128 = 1_000_000_000_000_000_000_000_000_000;
    const MS_PER_YEAR: i128 = 31_556_926_000;

    // Convert borrow rates (per-ms, RAY) into APR bps. rate_per_ms * MS_YR / RAY * 10_000.
    let mut rates_bps: BTreeSet<u16> = BTreeSet::new();
    for p in &f.market_params {
        // max_borrow_rate_ray is *annual* (RAY-scaled), so apr_bps = v * 10_000 / RAY.
        if let Some(v) = p.max_borrow_rate_ray {
            let bps = v.saturating_mul(10_000) / RAY;
            rates_bps.insert(bps.clamp(0, 50_000) as u16);
        }
        if let Some(v) = p.base_borrow_rate_ray {
            rates_bps.insert(((v.saturating_mul(10_000) / RAY).clamp(0, 50_000)) as u16);
        }
    }
    // Seed a handful of representative APRs even if no params found.
    for apr in [0u16, 100, 500, 1000, 5000, 10_000, 25_000, 50_000] {
        rates_bps.insert(apr);
    }

    // Time deltas: use last_timestamp values + fixed ladder.
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

    for apr in rates_bps.iter().take(32) {
        for dt in deltas.iter().take(32) {
            let mut buf = Vec::with_capacity(10);
            push_u16_le(&mut buf, *apr);
            push_u64_le(&mut buf, *dt);
            out.push(buf);
        }
    }
    out
}

/// `flow_supply_borrow_liquidate`: reads bytes[0..=3] as u8s controlling
/// supply/borrow_frac/time_jump/liq_frac. 4 bytes are plenty.
fn pack_flow_supply_borrow_liquidate(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    // Derive supply size buckets from position amounts (map to u8 via log).
    let mut supply_bytes: BTreeSet<u8> = BTreeSet::new();
    for &amt in &f.position_amounts {
        let scaled = (amt / 1_000_000).clamp(0, 255) as u8;
        supply_bytes.insert(scaled);
    }
    for b in [0u8, 1, 8, 32, 64, 128, 200, 255] {
        supply_bytes.insert(b);
    }
    for s in supply_bytes.iter().take(16) {
        for bf in [0u8, 64, 128, 192, 230, 255] {
            for jump in [0u8, 1, 4, 16, 64, 200] {
                for lf in [0u8, 64, 128, 200, 255] {
                    out.push(vec![*s, bf, jump, lf]);
                }
            }
        }
    }
    out
}

/// `flow_flash_loan`: Arbitrary over { seed_usdc: u32, loan_usdc: u32, use_bad: bool }
/// — 9 bytes LE + 1 byte bool.
fn pack_flow_flash_loan(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    // Derive seed/loan from i128 magnitudes / 10^18 (they're WAD-scaled).
    let mut seeds: BTreeSet<u32> = BTreeSet::new();
    for &x in &f.i128s {
        if x > 0 {
            // Take a few magnitudes.
            let v = (x / 1_000_000_000_000_000_000).clamp(0, u32::MAX as i128) as u32;
            seeds.insert(v);
        }
    }
    for s in [1_000u32, 10_000, 50_000, 100_000, 500_000, 1_000_000] {
        seeds.insert(s);
    }
    for seed in seeds.iter().take(16) {
        for loan in [1u32, 100, 1_000, 10_000, 50_000] {
            for bad in [0u8, 1] {
                let mut buf = Vec::with_capacity(9);
                push_u32_le(&mut buf, *seed);
                push_u32_le(&mut buf, loan);
                buf.push(bad);
                out.push(buf);
            }
        }
    }
    out
}

/// `flow_oracle_tolerance`: Arbitrary over
/// { supply_amt: u32, deviation_bps: u16, direction_up: bool, zero_price: bool }
/// — 4 + 2 + 1 + 1 = 8 bytes.
fn pack_flow_oracle_tolerance(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut amts: BTreeSet<u32> = BTreeSet::new();
    for &x in &f.i128s {
        if x > 0 {
            let v = (x / 1_000_000_000_000_000_000).clamp(0, u32::MAX as i128) as u32;
            amts.insert(v);
        }
    }
    for a in [100u32, 1_000, 10_000, 100_000] {
        amts.insert(a);
    }
    for amt in amts.iter().take(16) {
        for dev in [0u16, 199, 200, 499, 500, 1_000, 5_000] {
            for dir in [0u8, 1] {
                for zero in [0u8, 1] {
                    let mut buf = Vec::with_capacity(8);
                    push_u32_le(&mut buf, *amt);
                    push_u16_le(&mut buf, dev);
                    buf.push(dir);
                    buf.push(zero);
                    out.push(buf);
                }
            }
        }
    }
    out
}

/// `flow_isolation_emode_xor`: { choose_emode: bool, supply_amt: u32 }
fn pack_flow_isolation_emode_xor(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut amts: BTreeSet<u32> = BTreeSet::new();
    for &x in &f.position_amounts {
        let v = (x / 1_000_000_000_000_000_000).clamp(0, u32::MAX as i128) as u32;
        amts.insert(v);
    }
    for a in [100u32, 1_000, 10_000, 50_000] {
        amts.insert(a);
    }
    for amt in amts.iter().take(16) {
        for choose in [0u8, 1] {
            let mut buf = Vec::with_capacity(5);
            buf.push(choose);
            push_u32_le(&mut buf, *amt);
            out.push(buf);
        }
    }
    out
}

/// `flow_cache_atomicity`: { supply_usdc: u32, borrow_eth: u32 }
fn pack_flow_cache_atomicity(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut amts: BTreeSet<u32> = BTreeSet::new();
    for &x in &f.position_amounts {
        let v = (x / 1_000_000_000_000_000_000).clamp(0, u32::MAX as i128) as u32;
        amts.insert(v);
    }
    for a in [1_000u32, 5_000, 10_000, 50_000] {
        amts.insert(a);
    }
    for s in amts.iter().take(16) {
        for b in [1u32, 100, 500, 1_000, 5_000, 10_000] {
            let mut buf = Vec::with_capacity(8);
            push_u32_le(&mut buf, *s);
            push_u32_le(&mut buf, b);
            out.push(buf);
        }
    }
    out
}

/// `flow_supply_borrow_tsan_smoke`: reads a single byte.
fn pack_flow_supply_borrow_tsan_smoke(f: &ExtractedFields) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut bytes: BTreeSet<u8> = BTreeSet::new();
    for &x in &f.position_amounts {
        bytes.insert((x & 0xFF) as u8);
    }
    for b in 0u8..=255 {
        if b % 7 == 0 {
            bytes.insert(b);
        }
    }
    for b in bytes {
        out.push(vec![b]);
    }
    out
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

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

    // Find the stellar/ repo root: one level up from fuzz/ (where cargo runs),
    // OR the current working directory if already at repo root.
    // cargo run sets CWD to `stellar/fuzz`; snapshots live at `stellar/*/test_snapshots/`.
    let cwd = std::env::current_dir()?;
    let search_root = if cwd.ends_with("fuzz") {
        cwd.parent().unwrap().to_path_buf()
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
        ("fp_mul_div", pack_fp_mul_div(&merged)),
        ("fp_rescale", pack_fp_rescale(&merged)),
        ("fp_div_by_int", pack_fp_div_by_int(&merged)),
        ("rates_borrow", pack_rates_borrow(&merged)),
        ("compound_monotonic", pack_compound_monotonic(&merged)),
        (
            "flow_supply_borrow_liquidate",
            pack_flow_supply_borrow_liquidate(&merged),
        ),
        ("flow_flash_loan", pack_flow_flash_loan(&merged)),
        (
            "flow_oracle_tolerance",
            pack_flow_oracle_tolerance(&merged),
        ),
        (
            "flow_isolation_emode_xor",
            pack_flow_isolation_emode_xor(&merged),
        ),
        ("flow_cache_atomicity", pack_flow_cache_atomicity(&merged)),
        (
            "flow_supply_borrow_tsan_smoke",
            pack_flow_supply_borrow_tsan_smoke(&merged),
        ),
    ];

    for (target, inputs) in targets {
        if inputs.is_empty() {
            eprintln!("seed_corpus: {}: no inputs packed — skipping", target);
            continue;
        }
        let n = write_corpus(target, inputs, &out_dir)?;
        eprintln!("seed_corpus: {}: wrote {} files", target, n);
    }

    Ok(())
}
