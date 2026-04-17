//! Seed libFuzzer corpora from existing Soroban test snapshots.
//!
//! Walks `*/test_snapshots/**/*.json`, extracts numeric fields (i128, u32, u64)
//! from `ledger_entries.contract_data` maps (MarketParams, MarketState,
//! positions, etc.), then packs them into per-target byte layouts matching
//! each fuzz target's `Arbitrary`-derived input struct.
//!
//! The goal is NOT byte-perfect matching of the `Arbitrary` decoder -- libFuzzer
//! tolerates short/long/imperfect seeds as long as they decode to a valid input.
//! The real payoff is populating the input space with realistic numeric
//! magnitudes (RAY indexes, bps rates, timestamps, position amounts) so the
//! mutation engine has something non-trivial to bit-flip from iteration 0.
//!
//! Usage:
//!   cargo run --release --features seed-corpus --bin seed_corpus -- --output corpus
//!
//! Snapshots that fail to parse are logged and skipped; never abort the run.

use common::constants::{BPS, MILLISECONDS_PER_YEAR, RAY};
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

    // Structured per-market fields -- when we can identify them by symbol key.
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

// ---------------------------------------------------------------------------
// Packing (per target)
// ---------------------------------------------------------------------------

fn push_i128_le(buf: &mut Vec<u8>, v: i128) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_u64_le(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_u16_le(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// `fp_math`: In { kind: u8, a: i128, b: i128, choice: u8, extra: u8 } -- 35
/// bytes LE. `kind % 3` dispatches to the MulDiv / DivByInt / Rescale arm;
/// each arm interprets the shared fields as needed. See
/// `fuzz_targets/fp_math.rs` for the layout contract.
///
/// This packer emits seeds for all three arms from the same extracted numeric
/// pool so libFuzzer can cross-pollinate bytes between arms during mutation.
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

/// `rates_borrow`: In { util_bps: u16, base_pct: u8, s1_pct: u8, s2_pct: u8,
///                      s3_pct: u16, mid_pct: u8, opt_pct: u8, max_pct: u16,
///                      flip: u8 }
///
/// That's 2+1+1+1+2+1+1+2+1 = 12 bytes matching Arbitrary's derive-default LE
/// decoding (integers are consumed LE, bytes in field order).
/// `rates_and_index`: 29 bytes matching the `In` struct in
/// `fuzz_targets/rates_and_index.rs`. Merges the retired `rates_borrow` and
/// `compound_monotonic` packers so libFuzzer can cross-pollinate rate/params
/// bytes with accrual/borrow bytes inside a single target.
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

    for p in &f.market_params {
        // Convert RAY-denominated params to the %-scale the target uses.
        let to_pct = |v: Option<i128>| -> i128 { v.map(|r| r * 100 / RAY).unwrap_or(0) };
        let base_pct = to_pct(p.base_borrow_rate_ray).clamp(0, 50) as u8;
        let s1_pct = to_pct(p.slope1_ray).clamp(0, 50) as u8;
        let s2_pct = to_pct(p.slope2_ray).clamp(0, 100) as u8;
        let s3_pct = to_pct(p.slope3_ray).clamp(0, 500) as u16;
        let mid_pct = to_pct(p.mid_utilization_ray).clamp(1, 98) as u8;
        let opt_pct = to_pct(p.optimal_utilization_ray).clamp(mid_pct as i128 + 1, 99) as u8;
        let max_pct = to_pct(p.max_borrow_rate_ray).clamp(1, 1000) as u16;
        let reserve_pct = p
            .reserve_factor_bps
            .map(|r| r.clamp(0, BPS - 1) / 100)
            .unwrap_or(10)
            .clamp(0, 50) as u8;

        // Keep seed count per market param bounded: 4 utils × 2 flips × 4 deltas
        // × 4 borroweds = 128. Times ~1.4k snapshots → ~180k total seeds.
        for util in [0u16, 5000, 9500, 10000] {
            for flip in [0u8, 1] {
                for &delta_ms in &delta_samples {
                    for &borrowed_units in &borrowed_samples {
                        let mut buf = Vec::with_capacity(29);
                        push_u16_le(&mut buf, util);
                        buf.push(base_pct);
                        buf.push(s1_pct);
                        buf.push(s2_pct);
                        push_u16_le(&mut buf, s3_pct);
                        buf.push(mid_pct);
                        buf.push(opt_pct);
                        push_u16_le(&mut buf, max_pct);
                        buf.push(flip);
                        buf.push(reserve_pct);
                        push_u64_le(&mut buf, delta_ms);
                        push_u64_le(&mut buf, borrowed_units);
                        out.push(buf);
                    }
                }
            }
        }
    }
    out
}

/// `flow_supply_borrow_liquidate`: Arbitrary layout is 8 bytes LE:
/// `{ supply_raw: u32, borrow_frac_raw: u8, jump_hours: u16, liq_frac_raw: u8 }`.
/// `flow_e2e`: Arbitrary over `{ ops: Vec<Op> }`. libFuzzer's vec-length
/// prefix means we don't need to emit precise byte layouts — a handful of
/// short seeds kickstart mutation, and the coverage-guided engine builds out
/// interesting op-sequence prefixes on its own.
///
/// The seeds below cover the common bootstraps the retired flow targets used
/// to seed explicitly: a supply, a supply+borrow, a supply+borrow+liquidate
/// sequence, a flash-loan op, and an empty vec.
fn pack_flow_e2e(_f: &ExtractedFields) -> Vec<Vec<u8>> {
    vec![
        // Empty op sequence.
        vec![],
        // Single Supply (Op discriminant 0, user=0, asset=0, amount=small).
        vec![0, 0, 0, 0, 0x00, 0x10, 0x00, 0x00],
        // Supply → Borrow pair.
        vec![
            0, 0, 0, 0, 0x00, 0x10, 0x00, 0x00, // Supply ALICE USDC
            1, 0, 1, 0, 0x00, 0x10, 0x00, 0x00, // Borrow ALICE ETH
        ],
        // Supply → Borrow → AdvanceAndSync → Liquidate.
        vec![
            0, 0, 0, 0, 0x00, 0x40, 0x00, 0x00, // Supply ALICE USDC large
            1, 0, 1, 0, 0x00, 0x08, 0x00, 0x00, // Borrow ALICE ETH
            7, 0x20, 0x00, // AdvanceAndSync 32h
            4, 0, 1, 0xC0, // Liquidate ALICE ETH 75%
        ],
        // FlashLoan good + bad.
        vec![5, 0, 0, 0, 0x00, 0x04, 0x00, 0x00, 0], // good
        vec![5, 0, 0, 0, 0x00, 0x04, 0x00, 0x00, 1], // bad
    ]
}

/// `flow_strategy`: short Vec<Op> seeds covering each strategy variant plus
/// an AdvanceAndSync. The bootstrap does the heavy setup, so seeds stay tiny.
fn pack_flow_strategy(_f: &ExtractedFields) -> Vec<Vec<u8>> {
    vec![
        // Empty — forces bootstrap-only path.
        vec![],
        // Multiply USDC-collateral / ETH-debt.
        vec![0, 0, 1, 0x00, 0x10, 0x00, 0x00, 0],
        // SwapDebt XLM → USDC.
        vec![1, 2, 0, 0x00, 0x10, 0x00, 0x00],
        // SwapCollateral USDC → ETH.
        vec![2, 0, 1, 0x00, 0x10, 0x00, 0x00],
        // RepayWithCollateral USDC→XLM, close_position=false.
        vec![3, 0, 2, 0x00, 0x10, 0x00, 0x00, 0],
        // AdvanceAndSync 8h.
        vec![4, 0x08, 0x00],
    ]
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
        ("fp_math", pack_fp_math(&merged)),
        ("rates_and_index", pack_rates_and_index(&merged)),
        ("flow_e2e", pack_flow_e2e(&merged)),
        ("flow_strategy", pack_flow_strategy(&merged)),
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
