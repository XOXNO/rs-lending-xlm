# Audit Findings — Pre-Audit Hunt

**Frozen commit**: `5ee115c` · **Date**: 2026-04-16 · **Method**: 8 parallel hunt agents (sharp-edges, insecure-defaults, variant-analysis, token-integration, spec-compliance, supply-chain, code-maturity, function-deep-dive).

Severity legend: **C**ritical / **H**igh / **M**edium / **L**ow / **I**nformational.

Status legend: ✅ verified · ⚠️ verified, partial mitigation · ❌ refuted on verification · 📝 documentation/process · 🔧 fix candidate

---

## Critical Findings

_None._

---

## High Findings

### H-01 — Pool flash-loan endpoints accept `asset: Address` unrelated to the pool's own asset
**Files**: `pool/src/lib.rs:317-376` (`flash_loan_begin`/`flash_loan_end`); `pool-interface/src/lib.rs:42-43`
**Status**: ✅ verified
**Description**: `flash_loan_begin(env, asset, amount, receiver)` and `flash_loan_end(env, asset, amount, fee, receiver)` accept an `asset` argument and build `token::Client::new(env, &asset)` against it (lines 329, 351). The reserve check at line 323, the `add_protocol_revenue` at line 371, and `cache.params.asset_id` at line 373 all use the pool's *home* asset. A buggy or upgraded controller passing the wrong asset would (a) reserve-check asset A, (b) transfer asset B, (c) book fee revenue against asset A. Pool must ignore caller-supplied `asset` and use `cache.params.asset_id`.
**Repro**:
```rust
// Pool's home asset is XLM. Controller (buggy) passes USDC:
pool.flash_loan_begin(USDC_addr, 1000, receiver);
// has_reserves(1000) on XLM passes; transfer pulls USDC from pool;
// books "phantom" XLM accounting.
```
**Fix**: drop the `asset` parameter from the pool ABI, OR `assert_eq!(asset, cache.params.asset_id)` at entry.
🔧

### H-02 — Pool ABI parameter ordering is inconsistent between `borrow` and `repay`
**Files**: `pool-interface/src/lib.rs:17-23, 33-39`; call sites `controller/src/positions/borrow.rs:197`, `controller/src/positions/repay.rs:117`
**Status**: ✅ verified
**Description**: `borrow(caller, amount, position, price_wad)` orders i128 args as `amount, …, price_wad`. `repay(caller, position, price_wad, amount)` orders i128 args as `price_wad, …, amount`. Both endpoints take two i128s at swappable positions; a controller-side typo silently transposes them. The compiler accepts the swap; magnitude-overlapping test cases miss it.
**Repro**:
```rust
// safe
pool_client.repay(caller, position, &price_wad, &amount);
// transposed — same types, compiles, wrong economics
pool_client.repay(caller, position, &amount, &price_wad);
```
**Fix**: align the parameter order across `borrow`/`repay`/`withdraw`/`supply`. Better: use a typed wrapper struct with named fields.
🔧

### H-03 — `add_protocol_revenue` (asset-decimal variant) lacks `SUPPLY_INDEX_FLOOR_RAW` guard
**Files**: `pool/src/interest.rs:53-61`; callers `pool/src/lib.rs:206` (liquidation withdraw fee), `pool/src/lib.rs:371` (flash-loan fee)
**Status**: ✅ verified — already documented in `architecture/MATH_REVIEW.md §5.5`, never fixed
**Description**: `add_protocol_revenue_ray` skips fee accrual when `supply_index < SUPPLY_INDEX_FLOOR_RAW` (line 71). The asset-decimal sibling does NOT mirror this guard. Both call sites (liquidation withdraw fee, flash-loan fee) run AFTER `interest::global_sync`, which may have just clamped the index to the floor `10^18`. The next call to `add_protocol_revenue` divides `Ray::from_asset(fee, dec)` by the near-zero index, producing astronomical scaled amounts that overflow downstream additions to `supplied`/`revenue`.
**Fix**: mirror `add_protocol_revenue_ray`'s floor check into `add_protocol_revenue`.
🔧 (1-line fix)

### H-04 — `flashloan_fee_bps` lower-bound NOT validated on `create_liquidity_pool`
**Files**: `controller/src/router.rs:12-29` (`validate_market_creation`) → `controller/src/validation.rs:114-151` (`validate_asset_config`)
**Status**: ✅ verified
**Description**: Audit-prep added a `flashloan_fee_bps >= 0` check inside `config::edit_asset_config:69-74`, but `validate_asset_config` (used by `create_liquidity_pool`) does not check it. An admin who creates a pool with `AssetConfig { flashloan_fee_bps: -1, .. }` succeeds. `pool/src/lib.rs:346-348` rejects negative fee at `flash_loan_end` runtime (defense-in-depth holds), but a fee like `5_001` (50.01%) above `MAX_FLASHLOAN_FEE_BPS` is also accepted at create-time and silently applies until `edit_asset_config` runs.
**Fix**: move both `flashloan_fee_bps` bounds into `validate_asset_config` (or call the extra checks from `validate_market_creation`).
🔧

### H-05 — `seize_position` borrow branch uses non-saturating subtraction on `cache.borrowed`
**File**: `pool/src/lib.rs:439`
**Status**: ✅ verified
**Description**: Every other accumulator path in `pool/src/lib.rs` uses `saturating_sub_ray` (defined at lines 44-50). The `Borrow` branch of `seize_position` does `cache.borrowed = cache.borrowed - Ray::from_raw(position.scaled_amount_ray)`. After multiple bad-debt cleanups where prior caps prevented full debt removal, the position's scaled amount can exceed tracked `cache.borrowed` due to interest accrual, causing a panic that blocks future cleanups for the same account.
**Fix**: replace with `cache.borrowed = saturating_sub_ray(cache.borrowed, Ray::from_raw(position.scaled_amount_ray))`.
🔧 (1-line fix)

### H-06 — Fee-on-transfer tokens break borrow / withdraw / liquidation seizure / add_rewards
**Files**: `pool/src/lib.rs:149` (borrow), `pool/src/lib.rs:218-220` (withdraw), `controller/src/positions/liquidation.rs:97` (seizure via withdraw), `controller/src/router.rs:251` (add_rewards)
**Status**: ⚠️ verified, depends on operator allowlisting a FoT token
**Description**: Supply (controller/src/positions/supply.rs:205-213) and repay (repay.rs:63-71) use balance-delta accounting and are FoT-safe. The pool→user flows (borrow, withdraw, liquidation seizure) and controller→pool (`add_rewards`) do not. With a 1% FoT token allowlisted: borrowers receive `amount × 0.99` while debt is booked at `amount`; liquidators get `bonus × 0.99` while the math books `bonus`; `add_rewards` credits `amount` to supply index while pool only received `amount × 0.99`. Liquidation math collapses → bad debt accumulates.
**Mitigation paths**: (a) operator-policy doc explicitly bans FoT tokens from `approve_token_wasm` (no on-chain enforcement); (b) add balance-delta on egress in pool transfers.
📝 / 🔧

### H-07 — Rebasing tokens break reserve↔scaled-supply accounting
**Files**: `pool/src/cache.rs:88-91` (`get_reserves_for`); `pool/src/lib.rs:467-468` (`claim_revenue`); `pool/src/lib.rs:358-368` (`flash_loan_end` delta check)
**Status**: ⚠️ verified, depends on operator allowlisting a rebase token
**Description**: `get_reserves_for(asset)` reads `tok.balance(pool)` live. With a positive-rebase token (aXLM-style, cbETH-style), reserves grow over time without `cache.supplied` changing. Effects: (1) `has_reserves(amount)` becomes more lenient; (2) `claim_revenue` transfers `min(reserves, treasury_actual)` — extra rebase delta may flow into the accumulator beyond protocol revenue intent; (3) `flash_loan_end` delta check `balance_after >= pre_balance + fee` becomes too lenient if a rebase happens mid-loan, allowing a receiver to repay less than `amount + fee`. Negative rebases starve withdrawals while debt accounting unchanged.
**Mitigation**: operator-policy doc bans rebase tokens; add property test of pool with mock rebase token.
📝

### H-08 — SAC issuer upgrades silently invalidate cached `asset_decimals` / `cex_decimals`
**Files**: `controller/src/router.rs:44-47`, `controller/src/config.rs:347, 353`; `architecture/STELLAR_NOTES.md` Q9
**Status**: ⚠️ verified — design tradeoff (Soroban exposes no `code_hash(addr)`)
**Description**: `cex_decimals`/`asset_decimals` read once at `create_liquidity_pool` and `configure_market_oracle`, then cached in `MarketConfig`/`MarketParams` and never re-read. Stellar issuers can upgrade their issued-asset SAC. If the new version changes `decimals()`, all `from_token`/`to_token` math drifts silently. `update_params` only updates the rate model — no endpoint refreshes `asset_decimals`.
**Fix**: add `refresh_market_decimals(asset)` admin endpoint that re-reads `token.decimals()` and refuses (with operator review) if it changed. Emit alert event for monitoring.
🔧

---

## Medium Findings

### M-01 — `disable_token_oracle` is a single-call kill-switch with no two-step / pause
**File**: `controller/src/config.rs:449-453`
**Status**: ✅ verified
**Description**: ORACLE role can call this and immediately set `MarketStatus::Disabled`, blocking all pricing for that market. Withdrawals stall (no `cached_price`); only liquidations can proceed via `allow_disabled_market_price`. No cool-down, no two-step, no pause requirement.
**Fix**: mirror `transfer_ownership` pattern — require `live_until_ledger`, emit a delayed event monitoring can react to.

### M-02 — `__constructor` fuses Owner with KEEPER + REVENUE + ORACLE roles
**File**: `controller/src/lib.rs:105-122`
**Status**: ✅ verified — defeats the role separation the codebase advertises
**Description**: Constructor grants all three operational roles to the deployer admin via `default_operational_roles`. Compromising the Owner key before the operator manually separates roles gives an attacker all three privilege tiers.
**Fix**: grant only `KEEPER` (the minimal bootstrap need); require explicit `grant_role` for `REVENUE`/`ORACLE`. OR start paused.

### M-03 — Construct does not pause-on-deploy
**File**: `controller/src/lib.rs:105-122`
**Status**: ✅ verified
**Description**: Constructor does not call `pausable::pause`. Post-construct, supply/borrow/withdraw/repay/flash_loan reach end-state subject only to `require_asset_supported` (which fails until `create_liquidity_pool` runs). An operator skipping the post-deploy hardening checklist (set aggregator/accumulator/template, approve tokens, configure oracle) can leave admin defaults exploitable.
**Fix**: call `pause(&env)` at end of `__constructor`. Owner must `unpause` after wiring.

### M-04 — `claim_revenue` partial-claim branch may break `revenue ≤ supplied` invariant
**File**: `pool/src/lib.rs:478-501`
**Status**: ⚠️ ambiguous — needs property-test verification
**Description**: When `amount_to_transfer < treasury_actual`, the code computes `ratio = amount/treasury` (using `Ray::from_raw` on asset-decimal i128s — dimensionally fragile but works because units cancel), then `scaled_to_burn = revenue_scaled * ratio`. `actual_revenue_burn` and `actual_supplied_burn` are clamped independently against different accumulators. When they diverge (small-revenue/large-supplied edge), post-state may not preserve `revenue ≤ supplied`.
**Fix**: compute one `min` against both accumulators before subtracting, OR add a property test under low-reserve conditions.

### M-05 — Liquidation seizure split rounding direction favors liquidator over protocol
**File**: `controller/src/positions/liquidation.rs:362-364`
**Status**: ✅ verified — Low-Medium impact
**Description**: `base_amount = capped / one_plus_bonus` rounds half-up, so `base_amount` can be slightly larger than the mathematical base. Consequently `bonus_portion = capped - base_amount` shrinks and `protocol_fee = bonus * fees_bps` shrinks. Net: protocol receives marginally less fee than the spec intends, by ≤1 ULP per asset per liquidation.
**Fix**: round `base_amount` DOWN (truncating) so `bonus_portion ≥` true bonus. Conservative direction-of-rounding rule.

### M-06 — `liquidation_threshold_bps` cached in supply position is never refreshed on `edit_asset_config`
**Files**: `controller/src/positions/supply.rs:151-159`, `controller/src/helpers/mod.rs:95, 161`
**Status**: ✅ verified — known KEEPER-mitigated drift
**Description**: When an existing supply position is topped up, the cached `loan_to_value_bps` / `liquidation_bonus_bps` / `liquidation_fees_bps` refresh from `AssetConfig`, but `liquidation_threshold_bps` does NOT. After `edit_asset_config` raises an asset's LT, existing supply positions retain the old LT until KEEPER's `update_account_threshold` propagates. Liquidators querying `view::health_factor` see stale economics relative to live AssetConfig.
**Fix**: refresh all four fields together, OR document that `update_account_threshold` MUST run after every `edit_asset_config` and add an alert.

### M-07 — TWAP staleness check uses `max(slot_timestamps)`, not `min`
**Files**: `controller/src/oracle/mod.rs:235-241` (`cex_spot_and_twap_price`), L284-289 (`cex_twap_price`)
**Status**: ✅ verified
**Description**: `latest_ts = max(record.timestamp)` then `check_staleness(latest_ts)`. If 4 of 5 TWAP slots are stale and only 1 is fresh, the staleness check passes while the average is mostly stale data. The TWAP becomes a slow-moving anchor that lags real markets during oracle outages.
**Fix**: check staleness against `min` (oldest fresh sample) OR reject if any sample is older than `max_price_stale_seconds × 2`.

### M-08 — `validate_bulk_position_limits` silently no-ops on unknown `position_type`
**File**: `controller/src/validation.rs:54-60`
**Status**: ✅ verified
**Description**: Branches on DEPOSIT (1) and BORROW (2); any other u32 value (0, 3+) skips the limit check via `return;`. A future enum variant or upstream typo bypasses the limit guard without error.
**Fix**: panic with `GenericError::InvalidPositionType` in the else arm.

### M-09 — DEX oracle path reuses `cex_symbol` because no `dex_symbol` field exists
**Files**: `controller/src/oracle/mod.rs:321`; `common/src/types.rs:422-437` (`MarketConfig`)
**Status**: ✅ verified
**Description**: `dex_spot_price` calls `to_reflector_asset(asset, &market.dex_asset_kind, &market.cex_symbol)` — the CEX symbol is forwarded to the DEX feed. With `dex_asset_kind = ReflectorAssetKind::Other`, the DEX is queried with the wrong symbol. Additionally `resolve_oracle_decimals` (config.rs:347-356) probes only the CEX `lastprice` at config time — DEX feed integrity is never tested.
**Fix**: add `dex_symbol` field to `MarketConfig`; probe `dex_client.lastprice(...)` at config time when `dex_oracle.is_some()`.

### M-10 — `process_excess_payment` produces (amount, usd_wad) records with internal drift
**File**: `controller/src/positions/liquidation.rs:373-407`
**Status**: ✅ verified — observability concern, not fund risk
**Description**: Partial-refund branch computes `new_amount = amount * ratio` (precise) and `new_usd = usd - remaining_excess` (independent precision path). The `(amount, usd_wad)` pair in the resulting `repaid_tokens` record can be internally inconsistent for the partially-refunded last token. Downstream consumers reading both fields see drift.
**Fix**: recompute `new_usd = new_amount * price` after the refund.

### M-11 — `set_position_limits` silently overwrites with no two-step
**Files**: `controller/src/config.rs:95-108`, lib.rs:471-474
**Status**: ✅ verified
**Description**: Owner can raise to 32/32 instantly. Combined with M-02 (Owner-fused-with-keeper-roles), an Owner-key compromise can immediately raise position limits and execute the bulk-liquidation gas-griefing scenarios from `THREAT_MODEL.md §3.3`.
**Fix**: gate permissiveness-increasing config changes behind a two-step `live_until_ledger` rail.

### M-12 — Allowlist keyed by Address, not WASM hash; not re-checked at runtime
**Files**: `controller/src/lib.rs:525-533`, `controller/src/storage/mod.rs:33-50`, `controller/src/config.rs:61`
**Status**: ⚠️ design tradeoff (Soroban exposes no `code_hash(addr)`)
**Description**: `is_token_approved` is checked only at `create_liquidity_pool` — never at supply/borrow/repay/withdraw. If an issuer upgrades their SAC to malicious WASM, `revoke_token_wasm` does NOT stop existing pools. The `keccak256(address)` "wasm_hash" emitted in the event is schema padding, not a real hash check.
**Fix**: document explicitly as a creation-time gate. Operator must `pause()` and migrate if a token's WASM goes hostile.

---

## Low Findings

### L-01 — `OracleProviderConfig::default_for` builds a permissive placeholder
**File**: `common/src/types.rs:236-252`; used at `controller/src/router.rs:90`
**Status**: ✅ accidentally fail-safe today; future-fragile
**Description**: Default values (`max_price_stale_seconds: 0`, all-zero tolerances) are protected only by `MarketStatus::PendingOracle` and the `oracle_type: None` panic at oracle/mod.rs:37. If a future maintainer removes the status guard, fail-safe direction holds (zero staleness causes panic, not acceptance), but the design is implicit. Add a `// SAFETY:` comment.

### L-02 — `add_e_mode_category` defaults `is_deprecated: false` with no opt-in step
**File**: `controller/src/config.rs:114-135`
**Status**: design intent — no grace-period before usable
**Fix**: optional `activate_e_mode_category` step.

### L-03 — `i128::MAX` magic sentinel for "withdraw all" silently triggers full withdrawal on `amount == 0`
**File**: `controller/src/positions/withdraw.rs:84-85`
**Status**: ✅ verified — documented in `audit/AUDIT_PREP.md` as known doc drift, sentinel preserved
**Description**: `amount == 0` semantic is opposite to every other position endpoint. Repro: `withdraw(caller, account_id, vec![(USDC, 0)])` drains the full position when the caller may have intended a no-op.
**Fix**: introduce `withdraw_all` endpoint OR change `0` semantic to "reject as positive-amount-required" matching other endpoints.

### L-04 — `borrow_cap = 0` and `supply_cap = 0` mean "unlimited" (magic-value reuse)
**Files**: `controller/src/positions/borrow.rs:281-282`, supply cap site
**Status**: ✅ verified — common operator-error class
**Fix**: `Option<NonZeroI128>` or `(unlimited: bool, cap: i128)`.

### L-05 — `pool::claim_revenue(caller, ...)` transfers revenue to caller-passed address
**File**: `pool/src/lib.rs:454-477`
**Status**: ⚠️ trusted controller, but pool API is permissive
**Description**: Pool transfers to whatever `caller` the controller passes. A controller bug or upgrade mis-wiring `caller` would route revenue out-of-band. The controller's REVENUE-gated wrapper currently always passes through to the accumulator.
**Fix**: pool stores an immutable accumulator address at construction; ignore `caller` for transfer destination.

### L-06 — KEEPER `keepalive_*` endpoints take a `caller: Address` parameter that's discarded
**File**: `controller/src/lib.rs:325-340, 563, 575, 581, 633`
**Status**: ✅ verified — observability concern
**Description**: External SDK consumers may assume `caller` becomes audit-trail principal in events; the macro consumes it and `let _ = caller;` discards. Either delete the param or emit it.

### L-07 — `expect("swap output went down")` uses Rust panic instead of structured error
**Files**: `controller/src/strategy.rs:510, 412, 602`
**Status**: ✅ verified
**Fix**: use `panic_with_error!(env, GenericError::InternalError)` consistently.

### L-08 — Auto-sync index update emits market events with `price_wad = 0`
**File**: `controller/src/oracle/mod.rs:474`; `pool/src/lib.rs:285-299`
**Status**: ✅ verified — observability
**Description**: `update_asset_index` auto-sync calls `pool.update_indexes(&0)`. Off-chain consumers see zero-price snapshots interleaved with real-price ones.
**Fix**: pass cached price OR introduce a separate `update_indexes_no_event` variant.

### L-09 — Magic literal `5 * WAD` for bad-debt threshold appears twice without shared constant
**Files**: `controller/src/positions/liquidation.rs:429, 458`
**Fix**: `pub const BAD_DEBT_USD_THRESHOLD: i128 = 5 * WAD;` in `common/src/constants.rs`.

### L-10 — `seize_position` lacks `else` arm for unknown `position_type`
**File**: `pool/src/lib.rs:425-446`
**Status**: ✅ verified — defensive gap
**Fix**: `else { panic_with_error!(env, GenericError::InvalidPositionType); }`.

### L-11 — `cache.supplied.mul(cache.supply_index)` may approach i128 limits at extreme TVL
**File**: `pool/src/interest.rs:91`
**Status**: ⚠️ theoretical at >10^11 RAY supplied; verify `mul_div_half_up` widens to i256
**Note**: `common/src/fp_core.rs` already uses I256 for `compound_interest`; verify the same for the bad-debt accumulator path.

### L-12 — architecture/INVARIANTS.md §4 under-documents the `seize_position` Deposit revenue path
**File**: `architecture/INVARIANTS.md §4`
**Status**: 📝 doc-only; code is correct
**Description**: §4 documents only `add_protocol_revenue` as the "increment both revenue and supplied" path. `pool/src/lib.rs:441-446` (`seize_position` Deposit branch) is a third path that increments revenue without re-incrementing supplied; the invariant `revenue ≤ supplied` still holds because the seized position was already counted in supplied. Add a sentence to §4.

### L-13 — Liquidation event `account_attrs` snapshot is pre-execution
**File**: `controller/src/positions/liquidation.rs:46`
**Status**: ✅ verified — observability
**Fix**: re-derive attrs from post-state when emitting `liq_repay`/`liq_seize` events.

---

## Informational

### I-01 — CVLR git deps not pinned by `rev` in workspace Cargo.toml
**Status**: ⚠️ lockfile freezes today; `cargo update` could float
**Fix**: pin `rev = "..."` for every cvlr-* git source.

### I-02 — Tighten caret pins on on-chain deps for the audit window
**Status**: 📝
**Suggestion**: change `soroban-sdk = "25.3.1"` and `stellar-* = "0.7.0"` to `=25.3.1` / `=0.7.0` until post-audit.

### I-03 — OpenZeppelin Stellar contracts (~79★, project started Dec 2024) are youngest deployed dep
**Status**: 📝
**Suggestion**: targeted manual review of `stellar-access`/`stellar-macros`/`stellar-contract-utils` paths exercised by controller (`only_owner`, `only_role`, ownable, pausable, upgradeable).

---

## Refuted (claimed by hunt agents but not reproducible on verification)

### R-01 — "Function-level deep dive" claimed `clean_bad_debt_standalone` is permissionless
**File**: `controller/src/positions/liquidation.rs:442-471`
**Status**: ❌ refuted
**Description**: Function is `pub fn` at module level, NOT `#[contractimpl]`. Only callable from `lib.rs:351` (which IS `#[only_role(caller, "KEEPER")]` gated) and `liquidation.rs:127` (inside `process_liquidation` which has `liquidator.require_auth()`). Not exposed to external callers. ENTRYPOINT_AUTH_MATRIX entry stands.

---

## Variant Analysis Summary (no findings; protocol absent of these classic variants)

| Variant | Verdict |
|---|---|
| Aave/Compound rounding asymmetry | **ABSENT** — uniform half-up everywhere |
| Cream/Iron Bank donation inflation | **ABSENT** — index-driven, not balance-driven |
| Euler donation/reserves desync | **PARTIAL** — donation is one-way gift; not exploitable |
| bZx oracle manipulation in same tx | **ABSENT** — cache memoization + flash-loan guard |
| Compound v2 round-down-borrow / round-up-supply | **ABSENT** — symmetric, protocol-favoring on `.min()` |
| MakerDAO surplus auction griefing | **ABSENT** — capped + `min`'d |
| Aave isolated-mode bypass | **ABSENT** — gated at supply AND borrow |
| 4626 share-token 1-wei attack | **ABSENT** — no asset/share division surface |
| Rebasing tokens | **PARTIAL** → see H-07 |
| ERC-777 reentry hooks | **ABSENT for SAC**; allowlist trust for custom (M-12) |

---

## Maturity Assessment Summary

| Category | Rating |
|---|---|
| Documentation | **Strong** |
| Testing & Verification | **Strong** |
| Access Controls | Satisfactory |
| Code Stability | Satisfactory |
| Monitoring | Satisfactory |
| Arithmetic | Moderate |
| Front-running / Oracle | Moderate |
| Token Integration | Moderate |
| Centralization | **Weak** (no contract-level timelock or multisig) |

**Overall**: Satisfactory (2.78/4).

---

## Spec-to-Code Compliance Summary

16 of 18 architecture/INVARIANTS.md sections enforced cleanly with citable code. 1 partial (§4 spec under-documents the `seize_position` revenue path — see L-12). 1 not testable from a static pass (§18 process checklist). No exploitable drift beyond the two known items already in `architecture/MATH_REVIEW.md`.

---

## Recommended Fix Order (pre-audit hand-off)

1. **H-03** (`add_protocol_revenue` floor guard) — 1-line fix; closes a math overflow path.
2. **H-05** (`saturating_sub_ray` in `seize_position`) — 1-line fix; unblocks repeated bad-debt cleanup.
3. **H-04** (`flashloan_fee_bps` bounds in `validate_asset_config`) — moves existing checks into the create-pool path.
4. **H-01** (drop `asset` arg from pool flash-loan ABI, OR assert it matches) — small ABI change; eliminates a controller-bug class.
5. **H-02** (align `borrow`/`repay` parameter order, OR introduce typed wrappers) — ABI change; eliminates a typo class.
6. **M-08** (panic in `validate_bulk_position_limits` else arm) — defensive.
7. **M-09** (add `dex_symbol` field, probe DEX `lastprice` at config) — closes a misconfig path.
8. **M-04** (revisit `claim_revenue` partial-claim invariant) — needs property test.
9. **L-09 / L-10** (extract bad-debt constant; defensive `else` arm) — code hygiene.

H-06, H-07, H-08, M-12 are operator-policy items (document in operator runbook + SCOPE.md): **DO NOT allowlist FoT or rebasing tokens; SAC issuer upgrade requires manual `refresh_market_decimals` migration**.

M-01, M-02, M-03, M-11 cluster around centralization/admin-key risk — best addressed by an off-chain operator multisig + timelock policy plus pause-on-deploy.
