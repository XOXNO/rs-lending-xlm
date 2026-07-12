# Certora Spec Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the formal-verification gaps identified in the Aave V4 Certora report comparison (`architecture/research/aave-v4-certora-fv-comparison.md`), in the priority order: pool fully verified → Health-Factor gate airtight → borrow capacity / parameter safety — using the local prover for fast iteration and the LIA-first encoding policy proven on 2026-07-09.

**Architecture:** Layered, isolated rules (Aave-V4-style modularity): every new rule seeds bounded symbolic state directly and calls one entry point — no deep cross-contract traces. Pool conservation is proven as **per-operation Δ-rules** (state delta == position delta), which give the Σ-invariant by induction without ghost sums (Sunbeam has no CVL ghosts). HF safety is proven as pure-function lemmas + per-entry-point gate rules.

**Tech Stack:** Certora Sunbeam (Rust `#[rule]` specs via `cvlr`, compiled to WASM), locally built CertoraProver (`certoraSorobanLocal` wrapper), soroban-sdk, `make certora-wasm`.

---

## Context primer (read first, zero-context engineers)

- Specs are Rust files under `verification/certora/{common,pool,controller}/spec/*_rules.rs`, compiled **into the contract WASM** behind the `certora` cargo feature. A rule is a `#[rule]` fn using `cvlr_assume!` / `cvlr_assert!` / `cvlr_satisfy!`.
- Confs under `verification/certora/*/confs/*.conf` list rule names + the prebuilt WASM. `check_orphans.py` fails if a `#[rule]` fn is in no conf or a conf names a missing rule — **every new rule needs a conf entry.**
- Build/prove loop (all local, no cloud):

```bash
./verification/certora/compile_all.sh            # cargo check all certora paths + orphan/coverage checks
make certora-wasm                                # rebuild artifacts/wasm/certora/*.wasm  (REQUIRED after any rule/contract edit)
cd verification/certora/<layer>/confs && certoraSorobanLocal <conf> [--rule <name>]
```

- `certoraSorobanLocal` (in `~/.local/bin`) runs the locally built prover; expect `Verified: <rule>` lines. A rule that traps/panics on every path passes a `cvlr_satisfy!(false)` revert-rule; `rule_sanity: basic` flags vacuous asserts.
- Encoding policy: **never** add `"precise_bitwise_ops": true` to a new conf. It bit-blasts 128-bit math and causes timeouts (proven: common/math went 4/8-timeouts → 8/8 Verified in 6 min without it). Escalate a single rule only if its counterexample is bitwise-spurious.
- Prover run artifacts (`.certora_internal/`, `emv-*/`) are junk — never commit them. Check `.gitignore` covers them before your first commit; if not, add them in Task 1.

## File structure

| File | Status | Responsibility |
|---|---|---|
| `verification/certora/controller/confs/{math,tolerance-math,boundary-*}.conf` | modify | drop bit-blasting where LIA-sound (Phase A) |
| `verification/certora/pool/spec/setup.rs` | create | shared seed/wrapper helpers for pool rules |
| `verification/certora/pool/spec/conservation_rules.rs` | create | Δ-conservation + cash-integrity rules (Phase B) |
| `verification/certora/pool/confs/conservation.conf` | create | conf for the above |
| `verification/certora/pool/spec/integrity_rules.rs` | modify | add `create_market_rejects_existing` |
| `verification/certora/pool/spec/mod.rs` | modify | register new modules |
| `verification/certora/controller/spec/hf_lemma_rules.rs` | create | pure-function HF lemmas (Phase C) |
| `verification/certora/controller/confs/hf-lemmas.conf` | create | conf for the above |
| `verification/certora/controller/spec/health_rules.rs` | modify | strategy gate rules + unhealthy-only-improves |
| `verification/certora/controller/confs/health.conf` | modify | register new health rules |
| `verification/certora/controller/spec/account_isolation_rules.rs` | modify | liquidation frame rule (Phase D) |
| `verification/certora/controller/spec/mod.rs` | modify | register `hf_lemma_rules` |
| `verification/certora/run_profile.py` | modify | `--local` flag (Phase E) |
| `verification/certora/profiles.json` | modify | new confs into `fast`/`sanity` profiles |
| `architecture/INVARIANTS.md` | modify | document Δ-conservation ⇒ Σ-invariant induction |

---

## Phase A — LIA-first encoding sweep

### Task 1: De-bit-blast controller math.conf

**Files:**
- Modify: `verification/certora/controller/confs/math.conf:4`

- [ ] **Step 1: Remove the flag**

In `verification/certora/controller/confs/math.conf` delete the line:

```json
    "precise_bitwise_ops": true,
```

- [ ] **Step 2: Rebuild WASM (only if contracts changed since last build; skip if `artifacts/wasm/certora/controller.wasm` is current)**

Run: `make certora-wasm`
Expected: exits 0, prints the three wasm paths.

- [ ] **Step 3: Prove the whole conf locally**

```bash
cd verification/certora/controller/confs && certoraSorobanLocal math.conf
```

Expected: `Verified:` for all 12 rules (`mul_half_up_commutative`, `mul_half_up_zero`, `mul_half_up_identity`, `div_half_up_inverse`, `div_half_up_zero_numerator`, `mul_half_up_rounding_direction`, `div_half_up_rounding_direction`, `rescale_upscale_lossless`, `rescale_roundtrip`, `signed_mul_away_from_zero`, `i256_no_overflow`, `div_by_zero_sanity`). If any rule reports **Violated**, inspect the counterexample in the printed `emv-*/Reports` dir: if the CEX assigns values only reachable through overapproximated bitwise ops (e.g. impossible carries), restore the flag **for that conf only** and record the rule name in the conf's `"msg"`; a genuine violation is a real bug — stop and report it.

- [ ] **Step 4: Commit**

```bash
git add verification/certora/controller/confs/math.conf
git commit -m "perf(certora): drop bit-blasting from controller math conf"
```

### Task 2: De-bit-blast tolerance-math.conf

**Files:**
- Modify: `verification/certora/controller/confs/tolerance-math.conf:4`

- [ ] **Step 1: Remove `"precise_bitwise_ops": true,` from the conf** (same one-line deletion as Task 1)

- [ ] **Step 2: Prove locally**

```bash
cd verification/certora/controller/confs && certoraSorobanLocal tolerance-math.conf
```

Expected: `Verified:` for all 7 rules (`zero_anchor_returns_false`, `equal_prices_within_symmetric_first_band`, `par_ratio_is_bps`, `divergent_prices_outside_tight_first_band`, `beyond_tolerance_permissive_returns_primary`, `liquidation_rejects_unsafe_dual_source_prices`, `tolerance_math_reachability`). Same triage rule as Task 1 Step 3 on any Violated.

- [ ] **Step 3: Commit**

```bash
git add verification/certora/controller/confs/tolerance-math.conf
git commit -m "perf(certora): drop bit-blasting from tolerance-math conf"
```

### Task 3: Triage boundary confs (rule-by-rule)

Boundary rules assert exact overflow/limit semantics (`mul_at_max_i128`, `compound_taylor_accuracy`, …) and are the one family that may genuinely need bit-precision. Do NOT bulk-remove; split.

**Files:**
- Modify: `verification/certora/controller/confs/boundary-math.conf`
- Modify: `verification/certora/controller/confs/boundary-rates.conf`
- Modify: `verification/certora/controller/confs/boundary-oracle.conf`

- [ ] **Step 1: For each of the three confs, make a temp copy without the flag and run it**

```bash
cd verification/certora/controller/confs
for c in boundary-math boundary-rates boundary-oracle; do
  python3 - "$c" <<'EOF'
import json, sys
c = json.load(open(f"{sys.argv[1]}.conf")); c.pop("precise_bitwise_ops", None)
json.dump(c, open(f"/tmp/{sys.argv[1]}-lia.conf", "w"), indent=4)
EOF
  cp "/tmp/$c-lia.conf" "$c-lia-test.conf"
  certoraSorobanLocal "$c-lia-test.conf" 2>&1 | grep -E 'Verified|Violated|Timeout' | tee "/tmp/$c-lia-results.txt"
done
```

- [ ] **Step 2: Apply the decision rule per rule**

- Every rule `Verified` in the LIA run → stays; if ALL rules of a conf verify, delete `"precise_bitwise_ops": true,` from the real conf.
- Any rule `Violated` under LIA but previously passing → bitwise-spurious CEX; keep that rule under a bit-precise conf: move it into a new `boundary-<x>-bv.conf` (copy of the original conf keeping the flag, `rule` list = only the escalated rules) and remove it from the LIA conf's `rule` list.

- [ ] **Step 3: Delete the temp confs, run the alignment checks**

```bash
rm verification/certora/controller/confs/*-lia-test.conf
python3 verification/certora/check_orphans.py
```

Expected: exit 0 (every rule still owned by exactly the intended confs).

- [ ] **Step 4: Commit**

```bash
git add verification/certora/controller/confs/
git commit -m "perf(certora): LIA-first boundary confs, bit-precise escalation split"
```

---

## Phase B — Pool fully verified (priority 1)

### Task 4: Shared pool spec helpers

The seed/wrapper helpers currently live privately in `integrity_rules.rs`. The new conservation rules need them; extract once instead of duplicating.

**Files:**
- Create: `verification/certora/pool/spec/setup.rs`
- Modify: `verification/certora/pool/spec/integrity_rules.rs` (delete its private copies, import from `setup`)
- Modify: `verification/certora/pool/spec/mod.rs`

- [ ] **Step 1: Create `verification/certora/pool/spec/setup.rs`**

Move (verbatim, adding `pub(crate)`) these items from `integrity_rules.rs`: `valid_params`, `valid_state`, `seed_pool`, `read_state`, `position`, `action`, `supply_first`, `borrow_first`, `withdraw_first`, `repay_first`. The file starts:

```rust
//! Shared state-seeding and bulk-of-one wrappers for pool-layer rules.
use soroban_sdk::{Address, Env};

use common::constants::RAY;
use common::types::{
    MarketParamsRaw, PoolAction, PoolKey, PoolStateRaw, ScaledPositionRaw,
};
use pool_interface::LiquidityPoolInterface;

pub(crate) fn valid_params(asset: Address) -> MarketParamsRaw {
    // ... body exactly as in integrity_rules.rs today ...
}
```

(Repeat for each function; bodies are copy-paste from `integrity_rules.rs` — no behavior changes.)

- [ ] **Step 2: Register the module and update imports**

In `verification/certora/pool/spec/mod.rs` add:

```rust
pub(crate) mod setup;
```

In `integrity_rules.rs` delete the moved functions and add at the top:

```rust
use super::setup::{
    action, borrow_first, position, read_state, repay_first, seed_pool, supply_first, valid_params,
    valid_state, withdraw_first,
};
```

- [ ] **Step 3: Compile**

Run: `./verification/certora/compile_all.sh`
Expected: exit 0 (pure refactor; orphan check unchanged).

- [ ] **Step 4: Commit**

```bash
git add verification/certora/pool/spec/
git commit -m "refactor(certora): extract shared pool spec setup helpers"
```

### Task 5: Pool Δ-conservation + cash-integrity rules

The key insight: Σ(account scaled positions) == market totals needs no ghost sums if every operation preserves it as a **delta equality** — `state.supplied_ray` moves by exactly the same amount as the returned position (induction step). Controller-side persistence of the returned position is already proven (`controller_supply_persists_pool_returned_position`).

**Files:**
- Create: `verification/certora/pool/spec/conservation_rules.rs`
- Create: `verification/certora/pool/confs/conservation.conf`
- Modify: `verification/certora/pool/spec/mod.rs`

- [ ] **Step 1: Write the rules file**

`verification/certora/pool/spec/conservation_rules.rs`:

```rust
//! Δ-conservation rules: every pool operation moves the market aggregate by
//! exactly the delta of the returned position, and moves `cash` by exactly
//! the token amount. Together with the controller-side persistence proofs
//! (consistency_rules) these give Σ(account scaled) == market total by
//! induction, without ghost sums.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env};

use common::constants::RAY;

use super::setup::{
    action, borrow_first, position, read_state, repay_first, seed_pool, supply_first,
    valid_state, withdraw_first,
};

const MAX_AMOUNT: i128 = 1_000_000_000_000i128;

/// supply: supplied_ray delta == returned position delta; cash delta == amount.
#[rule]
fn supply_delta_conserves_totals(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    scaled_before: i128,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_AMOUNT);
    cvlr_assume!((0..=100 * RAY).contains(&scaled_before));
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let pre = read_state(&e, &asset);
    let before = position(scaled_before);
    let result = supply_first(&e, action(before.clone(), amount, asset.clone()), i128::MAX);
    let post = read_state(&e, &asset);

    cvlr_assert!(
        post.supplied_ray - pre.supplied_ray
            == result.position.scaled_amount_ray - before.scaled_amount_ray
    );
    cvlr_assert!(post.borrowed_ray == pre.borrowed_ray);
    cvlr_assert!(post.cash - pre.cash == result.actual_amount);
}

/// withdraw: supplied_ray delta == position delta; cash decreases by the
/// net transfer (actual_amount minus retained protocol fee is <= actual).
#[rule]
fn withdraw_delta_conserves_totals(
    e: Env,
    admin: Address,
    asset: Address,
    amount: i128,
    scaled_before: i128,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_AMOUNT);
    cvlr_assume!((1..=100 * RAY).contains(&scaled_before));
    seed_pool(
        &e,
        admin.clone(),
        asset.clone(),
        valid_state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let pre = read_state(&e, &asset);
    let before = position(scaled_before);
    let result = withdraw_first(&e, admin, action(before.clone(), amount, asset.clone()), false, 0);
    let post = read_state(&e, &asset);

    cvlr_assert!(
        pre.supplied_ray - post.supplied_ray
            == before.scaled_amount_ray - result.position.scaled_amount_ray
    );
    cvlr_assert!(post.borrowed_ray == pre.borrowed_ray);
    // protocol_fee = 0 in this rule, so the whole actual amount leaves as cash.
    cvlr_assert!(pre.cash - post.cash == result.actual_amount);
}

/// borrow: borrowed_ray delta == position delta; cash decreases by amount.
#[rule]
fn borrow_delta_conserves_totals(
    e: Env,
    admin: Address,
    asset: Address,
    receiver: Address,
    amount: i128,
    scaled_before: i128,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_AMOUNT);
    cvlr_assume!((0..=50 * RAY).contains(&scaled_before));
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, scaled_before, 0, e.ledger().timestamp()),
    );

    let pre = read_state(&e, &asset);
    let before = position(scaled_before);
    let result = borrow_first(&e, receiver, action(before.clone(), amount, asset.clone()), i128::MAX);
    let post = read_state(&e, &asset);

    cvlr_assert!(
        post.borrowed_ray - pre.borrowed_ray
            == result.position.scaled_amount_ray - before.scaled_amount_ray
    );
    cvlr_assert!(post.supplied_ray == pre.supplied_ray);
    cvlr_assert!(pre.cash - post.cash == result.actual_amount);
}

/// repay: borrowed_ray delta == position delta; cash grows by the amount
/// actually applied (over-payment is refunded, not retained).
#[rule]
fn repay_delta_conserves_totals(
    e: Env,
    admin: Address,
    asset: Address,
    payer: Address,
    amount: i128,
    scaled_before: i128,
) {
    cvlr_assume!(amount > 0 && amount <= MAX_AMOUNT);
    cvlr_assume!((1..=100 * RAY).contains(&scaled_before));
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, scaled_before, 0, e.ledger().timestamp()),
    );

    let pre = read_state(&e, &asset);
    let before = position(scaled_before);
    let result = repay_first(&e, payer, action(before.clone(), amount, asset.clone()));
    let post = read_state(&e, &asset);

    cvlr_assert!(
        pre.borrowed_ray - post.borrowed_ray
            == before.scaled_amount_ray - result.position.scaled_amount_ray
    );
    cvlr_assert!(post.supplied_ray == pre.supplied_ray);
    cvlr_assert!(post.cash - pre.cash == result.actual_amount);
}

/// Bulk supply with two entries on the same asset conserves the aggregate:
/// the market total moves by the sum of both position deltas (covers the
/// bulk-loop accumulation, not just the bulk-of-one body).
#[rule]
fn supply_bulk_two_entries_conserves_totals(
    e: Env,
    admin: Address,
    asset: Address,
    amount1: i128,
    amount2: i128,
) {
    cvlr_assume!(amount1 > 0 && amount1 <= MAX_AMOUNT);
    cvlr_assume!(amount2 > 0 && amount2 <= MAX_AMOUNT);
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );

    let pre = read_state(&e, &asset);
    let mut entries: soroban_sdk::Vec<common::types::PoolSupplyEntry> = soroban_sdk::Vec::new(&e);
    entries.push_back(common::types::PoolSupplyEntry {
        action: action(position(0), amount1, asset.clone()),
        supply_cap: i128::MAX,
    });
    entries.push_back(common::types::PoolSupplyEntry {
        action: action(position(0), amount2, asset.clone()),
        supply_cap: i128::MAX,
    });
    let results = crate::LiquidityPool::supply(e.clone(), entries);
    let post = read_state(&e, &asset);

    let delta = results.get_unchecked(0).position.scaled_amount_ray
        + results.get_unchecked(1).position.scaled_amount_ray;
    cvlr_assert!(post.supplied_ray - pre.supplied_ray == delta);
}

#[rule]
fn pool_conservation_reachability(e: Env, admin: Address, asset: Address, amount: i128) {
    cvlr_assume!(amount > 0 && amount <= MAX_AMOUNT);
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, 0, 0, e.ledger().timestamp()),
    );
    let result = supply_first(&e, action(position(0), amount, asset.clone()), i128::MAX);
    cvlr_satisfy!(result.position.scaled_amount_ray > 0);
}
```

- [ ] **Step 2: Register the module**

In `verification/certora/pool/spec/mod.rs` add:

```rust
pub mod conservation_rules;
```

- [ ] **Step 3: Compile**

Run: `./verification/certora/compile_all.sh`
Expected: cargo check passes; `check_orphans.py` FAILS with the six new rules unowned — that is the expected "failing test" state before the conf exists.

- [ ] **Step 4: Write the conf**

`verification/certora/pool/confs/conservation.conf`:

```json
{
    "msg": "Pool delta-conservation and cash integrity",
    "optimistic_loop": true,
    "loop_iter": "2",
    "rule_sanity": "basic",
    "rule": [
        "supply_delta_conserves_totals",
        "withdraw_delta_conserves_totals",
        "borrow_delta_conserves_totals",
        "repay_delta_conserves_totals",
        "supply_bulk_two_entries_conserves_totals",
        "pool_conservation_reachability"
    ],
    "server": "prover",
    "cargo_features": [
        "certora"
    ],
    "prover_args": [
        "-maxBlockCount 300000"
    ],
    "independent_satisfy": true,
    "smt_timeout": "900",
    "global_timeout": "3600",
    "files": [
        "../../../../artifacts/wasm/certora/pool.wasm"
    ]
}
```

- [ ] **Step 5: Verify alignment now passes**

Run: `./verification/certora/compile_all.sh`
Expected: exit 0.

- [ ] **Step 6: Rebuild WASM and prove**

```bash
make certora-wasm
cd verification/certora/pool/confs && certoraSorobanLocal conservation.conf
```

Expected: `Verified:` ×6. If a Δ-equality is Violated, read the CEX — a genuine off-by-rounding between the position path and the aggregate path is exactly the class of bug this rule exists to catch (Aave V4 M-01/M-02 were this shape); report it rather than weakening the rule.

- [ ] **Step 7: Add to profiles**

In `verification/certora/profiles.json`, append to the `fast` profile list:

```json
{
    "conf": "verification/certora/pool/confs/conservation.conf",
    "args": []
}
```

and to the `sanity` profile:

```json
{
    "conf": "verification/certora/pool/confs/conservation.conf",
    "args": ["--rule", "pool_conservation_reachability"]
}
```

- [ ] **Step 8: Commit**

```bash
git add verification/certora/pool/ verification/certora/profiles.json
git commit -m "feat(certora): pool delta-conservation and cash integrity rules"
```

### Task 6: create_market re-registration rule

Production already guards (`GenericError::AssetAlreadySupported`, `contracts/pool/src/lib.rs:198-204`); this rule pins it forever (Aave V4 M-03 was exactly this hole).

**Files:**
- Modify: `verification/certora/pool/spec/integrity_rules.rs`
- Modify: `verification/certora/pool/confs/integrity.conf`

- [ ] **Step 1: Add the rule** (append to `integrity_rules.rs`)

```rust
/// Re-registering an existing market must revert (would otherwise zero the
/// live aggregates — the Aave V4 M-03 bug shape).
#[rule]
fn create_market_rejects_existing(e: Env, admin: Address, asset: Address) {
    seed_pool(
        &e,
        admin,
        asset.clone(),
        valid_state(100 * RAY, 25 * RAY, RAY, e.ledger().timestamp()),
    );

    crate::LiquidityPool::create_market(e.clone(), valid_params(asset.clone()));

    // Reaching this line means a second registration succeeded.
    cvlr_satisfy!(false);
}
```

- [ ] **Step 2: Register in the conf** — add `"create_market_rejects_existing"` to the `rule` list of `verification/certora/pool/confs/integrity.conf`.

- [ ] **Step 3: Compile, rebuild, prove**

```bash
./verification/certora/compile_all.sh && make certora-wasm
cd verification/certora/pool/confs && certoraSorobanLocal integrity.conf --rule create_market_rejects_existing
```

Expected: `Verified: create_market_rejects_existing` (the satisfy(false) is unreachable because the guard traps).

- [ ] **Step 4: Commit**

```bash
git add verification/certora/pool/
git commit -m "feat(certora): prove create_market rejects re-registration"
```

---

## Phase C — Health-Factor layer (priority 2)

### Task 7: HF pure-function lemma module

Lemmas on the value helpers and `calculate_health_factor` itself — zero entry-point tracing, so each rule is seconds of solver time.

**Files:**
- Create: `verification/certora/controller/spec/hf_lemma_rules.rs`
- Create: `verification/certora/controller/confs/hf-lemmas.conf`
- Modify: `verification/certora/controller/spec/mod.rs`

- [ ] **Step 1: Verify the helper signatures you are about to target**

```bash
grep -n 'pub fn position_value\|pub fn weighted_collateral' -A6 contracts/controller/src/helpers/*.rs
```

Expected shapes (from existing call sites in `verification/certora/controller/spec/health_rules.rs:24-72`): `position_value(env, scaled: Ray, index: Ray, price)` and `weighted_collateral(env, value: Wad, threshold: Bps)`. If the `price` parameter type is not `Wad`, adapt the two `Wad::from(price_raw)` lines below to the actual type — everything else is type-independent.

- [ ] **Step 2: Write the lemma file**

`verification/certora/controller/spec/hf_lemma_rules.rs`:

```rust
//! Pure-function lemmas on the Health-Factor computation layer.
//!
//! No entry points are traced: each rule feeds bounded symbolic values
//! straight into the helpers that `calculate_health_factor` is built from,
//! plus the aggregate function itself on empty/one-sided position maps.
//! These are the L2 lemmas that justify treating the HF gate rules (L3,
//! health_rules.rs) as the protocol's extraction barrier.
use cvlr::macros::rule;
use cvlr::{cvlr_assert, cvlr_assume, cvlr_satisfy};
use soroban_sdk::{Address, Env, Map};

use crate::constants::WAD;
use common::constants::{BPS, RAY};
use common::math::fp::{Bps, Ray, Wad};
use common::types::{AccountPositionRaw, DebtPositionRaw};

/// Threshold-weighted collateral never exceeds the raw collateral value
/// (thresholds are <= 100%): the HF numerator cannot inflate value.
#[rule]
fn weighted_collateral_le_value(e: Env, value: i128, threshold: i128) {
    cvlr_assume!((0..=1_000_000 * WAD).contains(&value));
    cvlr_assume!((0..=BPS).contains(&threshold));

    let weighted = crate::helpers::weighted_collateral(&e, Wad::from(value), Bps::from(threshold));
    cvlr_assert!(weighted.raw() <= value);
    cvlr_assert!(weighted.raw() >= 0);
}

/// Weighted collateral is monotone in the collateral value: more collateral
/// can never lower the HF numerator.
#[rule]
fn weighted_collateral_monotone_in_value(e: Env, v1: i128, v2: i128, threshold: i128) {
    cvlr_assume!((0..=1_000_000 * WAD).contains(&v1));
    cvlr_assume!((v1..=1_000_000 * WAD).contains(&v2));
    cvlr_assume!((0..=BPS).contains(&threshold));

    let w1 = crate::helpers::weighted_collateral(&e, Wad::from(v1), Bps::from(threshold));
    let w2 = crate::helpers::weighted_collateral(&e, Wad::from(v2), Bps::from(threshold));
    cvlr_assert!(w2.raw() >= w1.raw());
}

/// position_value is monotone in the scaled amount at a fixed index/price:
/// more debt shares can never shrink the HF denominator.
#[rule]
fn position_value_monotone_in_scaled(e: Env, s1: i128, s2: i128, index: i128, price: i128) {
    cvlr_assume!((0..=100 * RAY).contains(&s1));
    cvlr_assume!((s1..=100 * RAY).contains(&s2));
    cvlr_assume!((RAY..=10 * RAY).contains(&index));
    cvlr_assume!((1..=1_000_000 * WAD).contains(&price));

    let v1 = crate::helpers::position_value(&e, Ray::from(s1), Ray::from(index), Wad::from(price));
    let v2 = crate::helpers::position_value(&e, Ray::from(s2), Ray::from(index), Wad::from(price));
    cvlr_assert!(v2.raw() >= v1.raw());
}

/// HF division rounds down (div_floor): the reported health factor never
/// overstates safety relative to half-up rounding.
#[rule]
fn hf_division_rounds_against_borrower(e: Env, weighted: i128, debt: i128) {
    cvlr_assume!((0..=1_000_000 * WAD).contains(&weighted));
    cvlr_assume!((1..=1_000_000 * WAD).contains(&debt));

    let floor = Wad::from(weighted).div_floor(&e, Wad::from(debt));
    let half_up = Wad::from(weighted).div(&e, Wad::from(debt));
    cvlr_assert!(floor.raw() <= half_up.raw());
}

/// No borrow positions => HF is the infinite sentinel; the gate can never
/// spuriously block a debt-free account.
#[rule]
fn hf_no_debt_is_infinite(e: Env) {
    let supply: Map<Address, AccountPositionRaw> = Map::new(&e);
    let borrows: Map<Address, DebtPositionRaw> = Map::new(&e);
    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);

    let hf = crate::helpers::calculate_health_factor(&e, &mut cache, &supply, &borrows);
    cvlr_assert!(hf.raw() == i128::MAX);
}

#[rule]
fn hf_lemmas_reachability(e: Env, value: i128) {
    cvlr_assume!(value > 0 && value <= WAD);
    let w = crate::helpers::weighted_collateral(&e, Wad::from(value), Bps::from(BPS));
    cvlr_satisfy!(w.raw() > 0);
}
```

Note: if `weighted_collateral` / `position_value` / `calculate_health_factor` are not `pub` at those paths, widen visibility with `pub(crate)` — controller spec modules are compiled inside the controller crate (see `use crate::helpers::...` in `health_rules.rs`), so `pub(crate)` suffices; do not make them `pub`.

- [ ] **Step 3: Register module** — in `verification/certora/controller/spec/mod.rs` add `pub mod hf_lemma_rules;`

- [ ] **Step 4: Write the conf**

`verification/certora/controller/confs/hf-lemmas.conf`:

```json
{
    "msg": "Health-factor pure-function lemmas",
    "optimistic_loop": true,
    "loop_iter": "1",
    "rule_sanity": "basic",
    "rule": [
        "weighted_collateral_le_value",
        "weighted_collateral_monotone_in_value",
        "position_value_monotone_in_scaled",
        "hf_division_rounds_against_borrower",
        "hf_no_debt_is_infinite",
        "hf_lemmas_reachability"
    ],
    "server": "prover",
    "cargo_features": [
        "certora"
    ],
    "prover_args": [
        "-maxBlockCount 100000"
    ],
    "independent_satisfy": true,
    "smt_timeout": "600",
    "global_timeout": "1800",
    "files": [
        "../../../../artifacts/wasm/certora/controller.wasm"
    ]
}
```

- [ ] **Step 5: Compile, rebuild, prove**

```bash
./verification/certora/compile_all.sh && make certora-wasm
cd verification/certora/controller/confs && certoraSorobanLocal hf-lemmas.conf
```

Expected: `Verified:` ×6.

- [ ] **Step 6: Add to profiles** — append the conf to `fast` (no args) and to `sanity` with `["--rule", "hf_lemmas_reachability"]` in `verification/certora/profiles.json`.

- [ ] **Step 7: Commit**

```bash
git add verification/certora/controller/ verification/certora/profiles.json
git commit -m "feat(certora): health-factor pure-function lemma layer"
```

### Task 8: HF gate rules for strategy entry points

`multiply` opens leverage; `swap_debt`/`swap_collateral` restructure positions — all three are risk-increasing and must land inside the safety inequality, exactly like `hf_safe_after_borrow`. New rules live in `health_rules.rs` to reuse its private `inline_weighted_collateral_wad` / `inline_total_borrow_wad` helpers.

**Files:**
- Modify: `verification/certora/controller/spec/health_rules.rs`
- Modify: `verification/certora/controller/confs/health.conf`

- [ ] **Step 1: Append the three gate rules + sanity to `health_rules.rs`**

```rust
// Rule 5: Health-factor safety after multiply (math-anchored)

/// A freshly opened leverage position must satisfy the safety inequality.
/// Uses the minimal-mode shim (new account, no initial payment, no convert
/// steps) — the account id it returns is the one we audit.
#[rule]
fn hf_safe_after_multiply(
    e: Env,
    caller: Address,
    collateral_token: Address,
    debt_token: Address,
    flash_amount: i128,
    steps: common::types::StrategySwap,
) {
    cvlr_assume!(flash_amount > 0 && flash_amount <= WAD * 1000);
    cvlr_assume!(collateral_token != debt_token);

    let account_id = crate::spec::compat::multiply_minimal(
        e.clone(),
        caller,
        0, // no e-mode category
        collateral_token,
        flash_amount,
        debt_token,
        1, // PositionMode::Multiply
        steps,
    );

    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assert!(weighted.raw() >= total_debt.raw());
}
// Rule 6: Health-factor safety after swap_debt (math-anchored)

#[rule]
fn hf_safe_after_swap_debt(
    e: Env,
    caller: Address,
    existing_debt_token: Address,
    new_debt_amount: i128,
    new_debt_token: Address,
    steps: common::types::StrategySwap,
) {
    let account_id: u64 = 1;
    cvlr_assume!(new_debt_amount > 0 && new_debt_amount <= WAD * 1000);
    cvlr_assume!(existing_debt_token != new_debt_token);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    crate::Controller::swap_debt(
        e.clone(),
        caller,
        account_id,
        existing_debt_token,
        new_debt_amount,
        new_debt_token,
        steps,
    );

    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assert!(weighted.raw() >= total_debt.raw());
}
// Rule 7: Health-factor safety after swap_collateral (math-anchored)

#[rule]
fn hf_safe_after_swap_collateral(
    e: Env,
    caller: Address,
    current_collateral: Address,
    amount: i128,
    new_collateral: Address,
    swap: soroban_sdk::Bytes,
) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);
    cvlr_assume!(current_collateral != new_collateral);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    crate::Controller::swap_collateral(
        e.clone(),
        caller,
        account_id,
        current_collateral,
        amount,
        new_collateral,
        swap,
    );

    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assert!(weighted.raw() >= total_debt.raw());
}

#[rule]
fn hf_multiply_sanity(
    e: Env,
    caller: Address,
    collateral_token: Address,
    debt_token: Address,
    flash_amount: i128,
    steps: common::types::StrategySwap,
) {
    cvlr_assume!(flash_amount > 0);
    cvlr_assume!(collateral_token != debt_token);
    crate::spec::compat::multiply_minimal(
        e, caller, 0, collateral_token, flash_amount, debt_token, 1, steps,
    );
    cvlr_satisfy!(true);
}
```

(If `StrategySwap` lives elsewhere than `common::types`, match the import already used at the top of `strategy_rules.rs`.)

- [ ] **Step 2: Register the four new rules in `health.conf`'s `rule` list**

```json
        "hf_safe_after_multiply",
        "hf_safe_after_swap_debt",
        "hf_safe_after_swap_collateral",
        "hf_multiply_sanity"
```

- [ ] **Step 3: Compile, rebuild, prove**

```bash
./verification/certora/compile_all.sh && make certora-wasm
cd verification/certora/controller/confs
certoraSorobanLocal health.conf --rule hf_multiply_sanity
certoraSorobanLocal health.conf --rule hf_safe_after_multiply
certoraSorobanLocal health.conf --rule hf_safe_after_swap_debt
certoraSorobanLocal health.conf --rule hf_safe_after_swap_collateral
```

Expected: sanity first (proves the paths are reachable, i.e. the gate rules are not vacuous), then `Verified:` for each gate rule. Strategy paths are heavier than borrow/withdraw — if a rule times out at the conf's 900s, re-run that single rule with `--smt_timeout 1800` before considering restructuring.

- [ ] **Step 4: Commit**

```bash
git add verification/certora/controller/
git commit -m "feat(certora): HF gate rules for multiply and swap strategies"
```

### Task 9: Unhealthy-accounts-can-only-improve rules

The Aave V4 Spoke P-04 property our suite lacks. Division-free formulation: on an account already below the safety line, permitted operations must not grow debt and must not shrink weighted collateral.

**Files:**
- Modify: `verification/certora/controller/spec/health_rules.rs`
- Modify: `verification/certora/controller/confs/health.conf`

- [ ] **Step 1: Append the rules**

```rust
// Rule 8: Unhealthy accounts can only improve (repay leg)

/// On an account whose weighted collateral no longer covers debt, repay must
/// strictly shrink debt and leave collateral untouched — division-free form
/// of "HF below 1 can only increase".
#[rule]
fn unhealthy_repay_only_improves(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let pre_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let pre_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assume!(pre_weighted.raw() < pre_debt.raw()); // account is unhealthy

    crate::spec::compat::repay_single(e.clone(), caller, account_id, asset, amount);

    let mut cache2 =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let post_weighted = inline_weighted_collateral_wad(&e, &mut cache2, account_id);
    let post_debt = inline_total_borrow_wad(&e, &mut cache2, account_id);

    cvlr_assert!(post_debt.raw() <= pre_debt.raw());
    cvlr_assert!(post_weighted.raw() >= pre_weighted.raw());
}
// Rule 9: Unhealthy accounts can only improve (supply leg)

#[rule]
fn unhealthy_supply_only_improves(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() <= 1);

    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let pre_weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let pre_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assume!(pre_weighted.raw() < pre_debt.raw());

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset, amount);

    let mut cache2 =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let post_weighted = inline_weighted_collateral_wad(&e, &mut cache2, account_id);
    let post_debt = inline_total_borrow_wad(&e, &mut cache2, account_id);

    cvlr_assert!(post_debt.raw() <= pre_debt.raw());
    cvlr_assert!(post_weighted.raw() >= pre_weighted.raw());
}
```

- [ ] **Step 2: Register both rule names in `health.conf`'s `rule` list.**

- [ ] **Step 3: Compile, rebuild, prove**

```bash
./verification/certora/compile_all.sh && make certora-wasm
cd verification/certora/controller/confs
certoraSorobanLocal health.conf --rule unhealthy_repay_only_improves
certoraSorobanLocal health.conf --rule unhealthy_supply_only_improves
```

Expected: `Verified:` ×2. A violation on the supply leg would indicate a threshold-tightening path reachable on unhealthy accounts — that is the historical `supply threshold-tightening bypass` bug class; report immediately, do not weaken.

- [ ] **Step 4: Commit**

```bash
git add verification/certora/controller/
git commit -m "feat(certora): unhealthy accounts can only improve (Aave V4 Spoke P-04 analog)"
```

---

## Phase D — Parameter safety + liquidation frame (priority 3)

### Task 10: Threshold-downgrade gate rule

Production gates threshold downgrades on indebted accounts through a strict HF read (`contracts/controller/src/helpers/risk_params.rs:102`, `health_factor_for_threshold_downgrade`). Formalize: the strict-policy HF used for a downgrade decision equals the risk-increasing-policy HF (the gate cannot be computed under a laxer oracle policy).

**Files:**
- Modify: `verification/certora/controller/spec/health_rules.rs`
- Modify: `verification/certora/controller/confs/health.conf`

- [ ] **Step 1: Read the gate call sites**

```bash
grep -rn 'health_factor_for_threshold_downgrade' contracts/controller/src/ | head
```

Note the public entry that reaches it (the supply path with a tightened dynamic config) and the constant it compares against.

- [ ] **Step 2: Append the rule** — supply on an indebted account with a *lower* threshold in the incoming position must leave the safety inequality intact:

```rust
// Rule 10: Threshold downgrade cannot bypass the safety gate

/// Supplying with a tightened liquidation threshold on an indebted account is
/// a risk-increasing reconfiguration; the post-state must still satisfy the
/// safety inequality (formalizes the fix for the historical supply
/// threshold-tightening bypass).
#[rule]
fn threshold_downgrade_keeps_account_safe(e: Env, caller: Address, asset: Address, amount: i128) {
    let account_id: u64 = 1;
    cvlr_assume!(amount > 0 && amount <= WAD * 1000);

    let pre_account = crate::storage::get_account(&e, account_id);
    cvlr_assume!(pre_account.supply_positions.len() <= 1);
    cvlr_assume!(pre_account.borrow_positions.len() == 1); // indebted

    crate::spec::compat::supply_single(e.clone(), caller, account_id, asset.clone(), amount);

    let mut cache =
        crate::cache::Cache::new(&e, crate::oracle::policy::OraclePolicy::RiskIncreasing);
    let weighted = inline_weighted_collateral_wad(&e, &mut cache, account_id);
    let total_debt = inline_total_borrow_wad(&e, &mut cache, account_id);
    cvlr_assert!(weighted.raw() >= total_debt.raw());
}
```

The rule differs from `supply_cannot_decrease_hf` by NOT assuming pre-state safety and by pinning `borrow_positions.len() == 1`: any reachable post-state (including ones where the stored threshold changed) must land safe. If the supply path cannot change thresholds without a separate entry point, Step 1 will show it — then retarget the rule at that entry point with the same post-state assertion and rename accordingly.

- [ ] **Step 3: Register in `health.conf`, compile, rebuild, prove**

```bash
./verification/certora/compile_all.sh && make certora-wasm
cd verification/certora/controller/confs && certoraSorobanLocal health.conf --rule threshold_downgrade_keeps_account_safe
```

Expected: `Verified`.

- [ ] **Step 4: Commit**

```bash
git add verification/certora/controller/
git commit -m "feat(certora): formalize threshold-downgrade safety gate"
```

### Task 11: Liquidation frame rule (third accounts untouched)

Extends `account_isolation_rules.rs` to the liquidation path (Aave V4's `noChangeToOtherAccounts_liquidationCall`).

**Files:**
- Modify: `verification/certora/controller/spec/account_isolation_rules.rs`
- Modify: `verification/certora/controller/confs/account-isolation.conf`

- [ ] **Step 1: Append the rule** (mirrors `supply_does_not_change_other_account_positions`, target = liquidation via the existing compat wrapper `crate::spec::compat::liquidate(env, liquidator, account_id, debt_payments)`):

```rust
#[rule]
fn liquidation_does_not_change_other_account_positions(
    e: Env,
    liquidator: Address,
    debt_asset: Address,
    debt_amount: i128,
) {
    let target_account: u64 = 1;
    let other_account: u64 = 2;
    cvlr_assume!(debt_amount > 0 && debt_amount <= WAD * 1000);

    let other_supply_before = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Deposit,
        &asset_for_frame(&e, &debt_asset),
    );
    let other_borrow_before = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Borrow,
        &asset_for_frame(&e, &debt_asset),
    );

    let mut payments: soroban_sdk::Vec<common::types::Payment> = soroban_sdk::Vec::new(&e);
    payments.push_back(common::types::Payment {
        asset: debt_asset.clone(),
        amount: debt_amount,
    });
    crate::spec::compat::liquidate(e.clone(), liquidator, target_account, payments);

    let other_supply_after = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Deposit,
        &asset_for_frame(&e, &debt_asset),
    );
    let other_borrow_after = crate::storage::positions::get_scaled_amount(
        &e,
        other_account,
        AccountPositionType::Borrow,
        &asset_for_frame(&e, &debt_asset),
    );

    cvlr_assert!(other_supply_after == other_supply_before);
    cvlr_assert!(other_borrow_after == other_borrow_before);
}

/// Frame asset: audit the same asset the liquidation repays, which is the
/// asset most likely to be touched by a buggy cross-account write.
fn asset_for_frame(_e: &Env, debt_asset: &Address) -> Address {
    debt_asset.clone()
}
```

Adapt the `Payment` literal to the exact `Vec<Payment>` element type the compat `liquidate` wrapper takes (`verification/certora/controller/spec/compat.rs:337`) — if it takes `(Address, i128)` tuples as `process_liquidation` does in `health_rules.rs:150-152`, use that form and drop the struct literal.

- [ ] **Step 2: Register `liquidation_does_not_change_other_account_positions` in `account-isolation.conf`'s `rule` list.**

- [ ] **Step 3: Compile, rebuild, prove**

```bash
./verification/certora/compile_all.sh && make certora-wasm
cd verification/certora/controller/confs && certoraSorobanLocal account-isolation.conf --rule liquidation_does_not_change_other_account_positions
```

Expected: `Verified`.

- [ ] **Step 4: Commit**

```bash
git add verification/certora/controller/
git commit -m "feat(certora): liquidation frame rule for third accounts"
```

---

## Phase E — Tooling + docs

### Task 12: `--local` flag for run_profile.py

**Files:**
- Modify: `verification/certora/run_profile.py`

- [ ] **Step 1: Add the flag**

In `run_profile.py`, `command_line()` currently returns (`run_profile.py:47`):

```python
    return conf_path.parent, ["certoraSorobanProver", conf_path.name, *args, *extra_args]
```

Thread an `args.local` boolean through to it and change to:

```python
    prover = "certoraSorobanLocal" if local else "certoraSorobanProver"
    return conf_path.parent, [prover, conf_path.name, *args, *extra_args]
```

Add to the argparse block:

```python
    parser.add_argument(
        "--local",
        action="store_true",
        help="run with the locally built prover (certoraSorobanLocal) instead of the cloud",
    )
```

and update the preflight check at `run_profile.py:85` to check for the selected binary:

```python
    binary = "certoraSorobanLocal" if args.local else "certoraSorobanProver"
    if not args.dry_run and shutil.which(binary) is None:
        raise SystemExit(f"error: {binary} is not installed or not on PATH")
```

- [ ] **Step 2: Smoke it**

```bash
./verification/certora/run_profile.py sanity --local --dry-run
```

Expected: printed command lines start with `certoraSorobanLocal`.

- [ ] **Step 3: Run the sanity profile fully local**

```bash
./verification/certora/run_profile.py sanity --local
```

Expected: all 16 reachability rules Verified (this is also the end-to-end regression for Phases A–D).

- [ ] **Step 4: Commit**

```bash
git add verification/certora/run_profile.py
git commit -m "feat(certora): --local flag runs profiles on the local prover"
```

### Task 13: Document the new invariants

**Files:**
- Modify: `architecture/INVARIANTS.md`
- Modify: `verification/certora/pool/spec/README.txt`
- Modify: `verification/certora/controller/spec/README.txt`

- [ ] **Step 1: INVARIANTS.md** — add to the pool accounting section:

```markdown
### Scaled-Total Conservation

Every pool operation moves the market aggregate (`supplied_ray` /
`borrowed_ray`) by exactly the delta of the position it returns, and moves
`cash` by exactly the token amount transferred. Because the controller
persists returned positions verbatim (consistency rules), these per-operation
delta equalities give Σ(account scaled positions) == market total by
induction over the operation history — no ghost sums required.

| Runtime | Verification |
|---|---|
| `contracts/pool/src/lib.rs` (supply/withdraw/borrow/repay) | `conservation_rules`, `consistency_rules` |
```

and to the health-factor section:

```markdown
### Unhealthy Accounts Only Improve

On an account whose threshold-weighted collateral no longer covers debt, the
permitted operations (supply, repay) must not grow debt and must not shrink
weighted collateral; risk-increasing operations revert on the HF gate. The
strategy entry points (multiply, swap_debt, swap_collateral) satisfy the same
post-state safety inequality as borrow/withdraw.

| Runtime | Verification |
|---|---|
| `contracts/controller/src/positions/*`, `strategies/*` | `health_rules` (gate + only-improves), `hf_lemma_rules` |
```

- [ ] **Step 2: Update the two spec README.txt conf→spec maps** — pool README gains `conservation.conf — spec/conservation_rules.rs`; controller README gains `hf-lemmas.conf — spec/hf_lemma_rules.rs` under the Health theme.

- [ ] **Step 3: Run the coverage check**

```bash
python3 verification/certora/check_invariant_coverage.py
```

Expected: exit 0 (new INVARIANTS.md sections reference existing spec modules).

- [ ] **Step 4: Commit**

```bash
git add architecture/INVARIANTS.md verification/certora/
git commit -m "docs(certora): document conservation and HF-gate invariant coverage"
```

---

## Deferred (explicitly out of scope for this plan)

- **View isomorphism rules** (`bulk_get_sync_data` == post-settle state): wait for the risk-premium settle work to land — the view surface is about to change.
- **Risk-premium rules** (settle idempotency, drawn==0 ⇒ premium==0, accrual-not-skipped-when-only-premium): belong to the risk-premium branch, not main.
- **External SAC balance == cash equivalence**: `cash` is internally tracked; tying it to the real SAC balance needs a token-summary harness extension — revisit after Phase B lands and only if the SAC summary already models balances.
