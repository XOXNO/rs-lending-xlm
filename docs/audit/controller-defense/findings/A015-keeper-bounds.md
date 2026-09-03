# A015 — Keeper bounds: `update_indexes`, `claim_revenue`, `update_account_threshold`

- Agent: A015
- Theme: T1 (entry surface & auth), with T3/T4 touchpoints on revenue routing and input bounds
- Severity: low (input-length hygiene); core money/risk defenses are sound
- Status: defended (recipient + health-floor); partial (Vec length / empty-input bounds)
- Paths:
  - `contracts/controller/src/keepers.rs:17–237`
  - `contracts/controller/src/lib.rs:374–396` (`#[when_not_paused]` wrappers)
  - `contracts/controller/src/risk/params.rs:10–120` (`RiskRefreshScope`, gated liq tuple, `clears_min_hf`)
  - `contracts/controller/src/constants.rs:7` (`THRESHOLD_UPDATE_MIN_HF_RAW = 1.05 WAD`)
  - `contracts/controller/src/storage/protocol.rs:75–84` (accumulator get/set)
  - `contracts/controller/src/config/registry.rs:32–36` + `lib.rs:577–582` (`set_accumulator` owner-only)
  - `contracts/controller/src/events/revenue.rs:16–24` (`ClaimRevenueEvent`)
  - `contracts/pool/src/ops/revenue.rs:19–55` (pool pays Ownable owner = controller)
  - `scripts/permissionless_entrypoints.txt:68–70`
- Defense: flash guard + pause + caller auth; revenue recipient fixed to governance accumulator; measured forward (F-8); dual HF floor on `has_risks`
- Gap: no `MAX_VIEW_INPUTS`-style length cap or non-empty check on keeper Vec inputs (views bound at 256)
- Impact: cannot redirect revenue or push an account into liquidation via threshold refresh; residual is resource/UX only
- Evidence: INV-AUTH-03, INV-ACCT-03, INV-ACCT-06, INV-IDX-04, INV-RISK-01 (analogous floor); STRIDE I10; harness `tests/controller/keeper.rs`, `outbound_transfer_measurement.rs`, `pool/revenue.rs`
- Opinion: The three keeper verbs are correctly permissionless for *timing* and correctly gated for *effect*. Revenue always lands at the configured accumulator. `has_risks` cannot restamp liquidation params into the liquidatable region. The only clear defensive gap is missing Vec bounds on a surface that otherwise carefully fail-closes.

---

## 1. Scope and attack surface

Permissionless maintenance entrypoints (anyone who signs as `caller` / pays fees):

| Entrypoint | Body | Pause | Flash guard | Privileged role |
|---|---|---|---|---|
| `update_indexes(caller, assets)` | `keepers::update_indexes` | `#[when_not_paused]` | `require_not_flash_loaning` | none — `caller.require_auth()` only |
| `claim_revenue(caller, assets) -> Vec<i128>` | `keepers::claim_revenue` | same | same | none |
| `update_account_threshold(caller, has_risks, account_ids)` | `keepers::update_account_threshold` | same | same | none |

Declared in `scripts/permissionless_entrypoints.txt` under INV-AUTH-03 with the claim that these paths “cannot increase another account's risk.” STRIDE I10 matches: auth is caller signature only; monetary/risk side effects must be destination- or floor-bound.

Out of scope for this agent but adjacent: `recapitalize` (A016), `renew_account` (A017).

---

## 2. Shared defenses (all three)

### 2.1 Auth and reentrancy

```17:24:contracts/controller/src/keepers.rs
pub(crate) fn update_indexes(env: &Env, caller: Address, assets: Vec<HubAssetKey>) {
    caller.require_auth();
    validation::require_not_flash_loaning(env);

    let mut cache = Cache::new(env);
    let pool_addr = cache.cached_pool_address();
    pool_update_indexes_call(env, &pool_addr, &assets);
}
```

Same pattern on `claim_revenue` and `update_account_threshold`. Flash-callback reentry into keepers is refused (`FlashLoanOngoing` #400). Covered by `tests/test-harness/tests/meta/reentrancy_matrix.rs`.

### 2.2 Pause

All three wrappers carry `#[when_not_paused]` in `lib.rs`. Global pause stops index accrual sweeps, revenue claims, and threshold restamps. (`recapitalize` deliberately does not — not this scope.)

### 2.3 Instance TTL

`Cache::new` renews instance TTL on every successful path. Empty Vec still constructs a cache (cheap instance renew). Not a fund-risk issue.

---

## 3. `update_indexes` — bounds and effect

### 3.1 Behavior

Controller does not mutate position storage. It only FFI-calls the pool owner-gated `update_indexes`, which accrues each listed hub market to the current ledger (`contracts/pool/src/ops/market.rs:68–83`). Zero elapsed time is a no-op write with a snapshot event.

### 3.2 Bounds checklist

| Input concern | Enforced? | Notes |
|---|---|---|
| Caller auth | yes | `require_auth` |
| Empty `assets` | no | Loop over empty Vec → no-op |
| Max length | **no** | Unlike views (`MAX_VIEW_INPUTS = 256` in `views.rs`) |
| Duplicate hub keys | no explicit reject | Second pass sees no elapsed time → snapshot only |
| Unknown / unlistable hub | pool-side load fails | Call reverts; no controller corruption |
| Negative / overflow amounts | n/a | No amount parameter |
| Can lower balances | no | INV-IDX-04: index accrual is monotone; finer cadence can only raise realized rate within the rate cap |

### 3.3 Foreign-risk judgment

A keeper choosing *when* to accrue cannot create a foreign position, redirect funds, or reduce another user's share claim. Worst case is MEV/timing around liquidations (accepted public-state risk in the threat model) or paying fees to spam large asset lists (self-funded resource use).

**Status: defended** for protocol invariants; **partial** for Vec length hygiene.

---

## 4. `claim_revenue` — who receives revenue

### 4.1 Recipient is not the caller

Flow:

1. Read accumulator from controller instance storage; if unset → `OracleError::NoAccumulator` (#211) **before** the pool call (`keepers.rs:102–103`).
2. Snapshot controller token balance.
3. `pool_claim_revenue_call` — pool burns claimable revenue shares, enforces utilization + solvency (INV-ACCT-06), transfers cash to the pool's Ownable **owner** (the controller).
4. `balance_delta_since` measures what actually arrived (INV-ACCT-03 / F-8).
5. If `received > 0`, `transfer_amount_measured` forwards **from controller to accumulator** (never to `caller`).
6. `ClaimRevenueEvent` records `caller`, `accumulator`, and the **measured** `amount` (indexers must not trust the pool's reported figure).

```102:140:contracts/controller/src/keepers.rs
    let accumulator = storage::try_get_accumulator(env)
        .unwrap_or_else(|| panic_with_error!(env, OracleError::NoAccumulator));
    // ...
    let received = balance_delta_since(env, asset, &controller, before);

    if received > 0 {
        payments::transfer_amount_measured(
            env,
            asset,
            &controller,
            &accumulator,
            received,
            GenericError::AmountMustBePositive,
        );
        events::ClaimRevenueEvent { /* caller, accumulator, amount: received */ }
            .publish(env);
    }
    received
```

`set_accumulator` is `#[only_owner]` (`lib.rs:577–582`). The keeper picks markets and timing only — matching the permissionless_entrypoints.txt line and threat-model row “Revenue goes only to the configured accumulator.”

### 4.2 Bounds checklist

| Input concern | Enforced? | Notes |
|---|---|---|
| Accumulator configured | yes | Fail closed #211 |
| Caller as recipient | impossible | No recipient argument; hard-coded storage address |
| Empty `assets` | no | Returns empty `Vec<i128>` |
| Max length | **no** | Same gap as indexes |
| Duplicates | allowed | First claim drains; later entries return 0 |
| Zero revenue | yes | `received == 0` → no transfer, no event (avoids indexer spam) |
| Fee-on-transfer / under-delivery | yes | Forward measured receipt only; pre-existing controller dust untouched (`outbound_transfer_measurement.rs`) |
| Pool insolvency / util ceiling | yes | Pool guards before paying (INV-ACCT-06) |
| Negative `received` | not asserted | Only `> 0` forwards; a pathological mid-call drain would return a non-positive figure without paying the caller. No redirect path. |

### 4.3 Tests locking the recipient

- `tests/test-harness/tests/pool/revenue.rs::test_claim_revenue_routes_through_controller_to_accumulator` — controller balance unchanged; accumulator delta equals claimed; pool released equals claimed.
- `claim_revenue_forwards_the_measured_amount_and_leaves_controller_dust_intact` — F-8: cannot raid stranded controller balance to top up an under-delivering token.
- `claim_revenue_without_accumulator_panics` — unit test expects #211.
- Permissionless call-by-third-party succeeds for timing (`test_permissionless_revenue_endpoints`).

**Status: defended** for “who receives revenue.”

---

## 5. `update_account_threshold` — bounds and health-floor when `has_risks`

### 5.1 Scope selection

```79:91:contracts/controller/src/keepers.rs
    let scope = if has_risks {
        risk::RiskRefreshScope::FullTuple
    } else {
        risk::RiskRefreshScope::LtvOnly
    };
```

| `has_risks` | Scope | Writes | Debt loaded | Post HF assert |
|---|---|---|---|---|
| `false` | `LtvOnly` | `loan_to_value` only | no (empty map) | **no** |
| `true` | `FullTuple` | LTV + gated `(LT, bonus, fees)` | yes | **yes** — `hf >= 1.05 WAD` |

Liquidation / health factor arithmetic uses **liquidation threshold**, not LTV (`risk/totals.rs:206–220`). LTV-only restamps therefore cannot move HF or create liquidatable state. Borrow capacity uses `min(LTV, LT)`, so raising LTV above a frozen LT cannot inflate origination power past the stamped threshold.

### 5.2 Per-position gate (FullTuple)

`refresh_supply_risk_params` always stamps LTV from the current spoke listing. Liquidation params go through `apply_gated_liquidation_params`:

- If the listing change **favors the liquidator** (lower LT, higher bonus, or lower fees) **and** the account has debt **and** a hypothetical LT swap would leave HF `< 1.05 WAD`, the entire liquidation tuple for that asset is **left at its vintage**.
- Otherwise the full listed `(LT, bonus, fees)` is applied.
- Debt-free accounts take the full tuple without the HF gate (no liquidation surface).
- Delisted / missing spoke assets are skipped (`cached_spoke_asset` → `None`); deprecated spokes keep reading their own config (no silent spoke-0 fallback) — see keeper harness tests.

`clears_min_hf` hypothesizes **only** the new liquidation threshold (bonus/fees do not enter HF). That is correct for the HF predicate; bonus/fee adversity is still blocked whenever current HF is already below the floor, because the same `clears_min_hf` check fails and the whole adverse tuple is skipped.

### 5.3 Portfolio post-condition (FullTuple)

After optional storage write:

```221:234:contracts/controller/src/keepers.rs
    if full_tuple {
        let hf = risk::calculate_account_risk_totals(
            env,
            cache,
            &account.supply_positions,
            &account.borrow_positions,
        )
        .health_factor;
        assert_with_error!(
            env,
            hf >= Wad::from(THRESHOLD_UPDATE_MIN_HF_RAW),
            CollateralError::HealthFactorTooLow
        );
    }
```

Floor constant: `THRESHOLD_UPDATE_MIN_HF_RAW = 1_050_000_000_000_000_000` (1.05 WAD). Liquidation eligibility remains `hf < 1 WAD` (`liquidation/plan.rs`). So the update floor sits **strictly above** the liquidation line: a keeper cannot restamp thresholds in a way that leaves the account liquidatable, and cannot complete a FullTuple refresh while the account sits in the `[1.0, 1.05)` band.

Transaction atomicity: storage is written before the assert, but a failing assert aborts the Soroban transaction and rolls back. Harness `test_update_account_threshold_rejects_bonus_raise_below_min_hf` asserts stamps are unchanged after `HealthFactorTooLow`.

The post-assert runs even when `any_changed == false`. Consequence: `has_risks=true` on an already-tight account (`1.0 ≤ HF < 1.05`) always reverts, including pure no-ops and borrower-favorable listing moves that still leave HF below 1.05. That is an **operational lockout**, not a foreign-risk hole — LTV can still be refreshed with `has_risks=false`.

Multi-asset sequencing: each asset's gate sees prior in-memory updates; the final portfolio assert is the cumulative backstop if independent per-asset clears would otherwise compose poorly.

### 5.4 Skip / fail-closed account filters

`sync_account_thresholds` returns early (no panic) when:

- no account metadata,
- empty supply map,
- NFT owner unresolvable (`try_account_owner`).

Unknown ids in a batch are skipped; a later FullTuple HF failure still reverts the whole batch (including prior accounts' writes in that tx).

Side effect: successful sync renews the account TTL (`renew_user_account`) even if stamps are unchanged. That is broader than owner-only `renew_account` (STRIDE I12) but does not alter risk params; flag for A017 if TTL policy is inventoried separately.

### 5.5 Bounds checklist

| Input concern | Enforced? | Notes |
|---|---|---|
| `has_risks` bool | yes | Selects scope; no other values |
| Empty `account_ids` | no | No-op loop |
| Max length | **no** | Contrast `MAX_VIEW_INPUTS` on views |
| Duplicates | allowed | Redundant work; second pass usually no-op |
| Invalid id | skip | Fail closed, not ownerless |
| Force-delisted asset | skip stamp | Position keeps vintage params |
| Oracle fail on FullTuple | revert | Fail closed; adverse update does not commit |
| Push into liquidation | prevented | Dual gate + 1.05 floor vs 1.0 liquidate line |

### 5.6 Tests locking the health floor

From `tests/test-harness/tests/controller/keeper.rs`:

| Test | Claim |
|---|---|
| `test_update_account_threshold_safe` | `has_risks=false` moves LTV only; bonus/fees/LT unchanged |
| `test_update_account_threshold_rejects_low_hf` | FullTuple reverts `HealthFactorTooLow` when HF depressed |
| `regression_third_party_keeper_cannot_force_adverse_tuple_below_min_hf` | Adverse listing restamp blocked; account stays non-liquidatable; LTV may still move |
| `test_update_account_threshold_rejects_bonus_raise_below_min_hf` | Rejected batch leaves stamps untouched |
| `test_update_account_threshold_propagates_adverse_tuple_to_healthy_account` | Healthy (≥ floor) accounts accept the full adverse vintage together |
| `test_update_account_threshold_mixed_spokes_batch` | Per-spoke isolation on ungated path |
| Unit `threshold_update_min_hf_is_one_point_zero_five_wad` | Constant lock |

**Status: defended** for “health-floor when `has_risks`.”

---

## 6. Input-bound gap (shared)

Views enforce `values.len() <= MAX_VIEW_INPUTS` (256). The three keeper mutators accept unbounded `Vec`s with no `require_non_empty_*` analogue.

Practical mitigators on Soroban: transaction resource limits; the caller pays fees; unknown keys tend to revert early on pool load. This is therefore **not** a fund-theft or foreign-liquidation vector, but it is inconsistent hygiene versus the view surface and vs payment aggregators that reject empty inputs.

Recommended remediation (non-blocking): reuse `MAX_VIEW_INPUTS` (or a dedicated `MAX_KEEPER_INPUTS`) on `assets` / `account_ids` before work; optionally reject empty Vecs if indexers/keepers should fail loud.

Cross-link: A061/A062 (validation wave) if they inventory Vec bounds globally.

---

## 7. Invariant / threat-model mapping

| Claim | Mapping | Live verdict |
|---|---|---|
| Permissionless keepers do not create foreign risk | INV-AUTH-03, threat-model “Permissionless maintenance” | Holds: no recipient control; threshold path cannot liquidate |
| Revenue claims stay solvent | INV-ACCT-06 | Pool guards; controller only forwards |
| Credit equals measured receipt | INV-ACCT-03 / F-8 | Measured forward; dust not raided |
| Accrual time-consistent | INV-IDX-04 | Pool simulate/accrue path; controller is a thin relay |
| Risk-increasing actions re-prove solvency | INV-RISK-01 (analog) | FullTuple uses 1.05 WAD floor (stricter than post-pool 1.0) |
| STRIDE I10 | Keeper ↔ Controller ↔ Pool | Auth + pause + flash + destination/floor controls |

---

## 8. Residual risks (accepted or low)

1. **Governance-set accumulator is trusted.** A compromised owner can point revenue at an arbitrary address via `set_accumulator`. Not a keeper bug.
2. **`[1.0, 1.05)` FullTuple lockout.** Prevents even beneficial LT refreshes until HF recovers or LTV-only is used. Safety-favoring operational friction.
3. **Public timing / MEV** on index accrual vs liquidations — accepted in threat model.
4. **Permissionless TTL renew** as a side effect of threshold sync — maintenance helpful; document vs owner-only `renew_account`.
5. **Unbounded Vec** — fee-funded resource use / inconsistency with views.

---

## 9. Verdict

| Question | Answer |
|---|---|
| Bounds on keeper inputs? | Auth/pause/flash yes; amount n/a; **Vec length/emptiness: missing** |
| Health-floor when `has_risks`? | **Defended** — per-asset liquidator-favor gate + portfolio assert at 1.05 WAD; cannot push into HF &lt; 1 liquidation |
| Who receives revenue? | **Configured accumulator only**; caller is event metadata, never the payout address; unset accumulator fail-closes before claim |

Overall: **defended** on the money and risk questions this agent was scoped to answer; record a **low/partial** gap on Vec input bounds for the validation backlog.
