# Post-Remediation Findings (adversarial loop series)

Baseline: HEAD `96a93c4`, post-remediation commit `d59afe1`. Differentiated against MVX sibling `/Users/mihaieremia/GitHub/rs-lending` HEAD `426639b`.

Canonical invariant catalog lives in [`architecture/INVARIANTS.md`](../architecture/INVARIANTS.md). The subset below annotates WHICH invariants the adversarial-loop findings broke, plus new invariants (I15+) that the loops introduced.

---

## Invariants Referenced by N-Findings

### Core invariants

- **I1** — `supply()` can only maintain or increase a position's HF. **[BROKEN Stellar, see Finding N-01]**
- **I2** — `revenue_scaled ≤ supplied_scaled` (pool accounting). Enforced via M-04 `actual_burn = min(scaled_to_burn, revenue, supplied)`.
- **I3** — `supply_index_ray ≥ SUPPLY_INDEX_FLOOR_RAW = 10^18`. `add_protocol_revenue*` skip accrual below floor (H-03 fix applied to both variants).
- **I4** — Per asset: `liquidation_threshold_bps > loan_to_value_bps > 0`, `liquidation_threshold_bps ≤ BPS`, `liquidation_bonus_bps ≤ MAX_LIQUIDATION_BONUS`, `flashloan_fee_bps ∈ [0, MAX_FLASHLOAN_FEE_BPS]`.
- **I5** — Liquidator bonus is bounded: cannot seize more than `debt_repaid × (1 + bonus_bps/BPS)`, and capped at total collateral.
- **I6** — Total value conserved up to explicit reserves, fees, spread, bad debt: `Δ(supplied_scaled × supply_index) = Δ(transfers_in) - Δ(transfers_out) - Δ(bad_debt_socialized)`.
- **I7** — Scaled ↔ nominal conversion after accrual is consistent: `nominal = scaled_ray × index_ray / RAY` (Ray math, half-up).
- **I8** — Over-repay refund exact: `process_excess_payment` refunds with `new_usd = new_amount × price` (M-10 fix).
- **I9** — Keeper's `update_account_threshold` refuses LT drop if resulting `HF < 1.05e18` (THRESHOLD_UPDATE_MIN_HF). **[Bypassed by supply-topup path — Finding N-01.]**
- **I10** — `claim_revenue` transfers to pool's immutable accumulator address (L-05 fix). `caller` arg is discarded for destination.
- **I11** — Over-liquidate refund exact; no double-spend.
- **I12** — Bulk ≡ sequential: `borrow_batch([A, B])` final state equals `borrow(A); borrow(B)` final state (except for HF check batching).
- **I13** — Paused markets reject all mutating endpoints except liquidations with `allow_disabled_market_price`.
- **I14** — `revenue_scaled + user_scaled_supplied = supplied` (pool total).

### Loop-series additions

- **I15** (CRITICAL, **broken by N-02**) — pool math must never accept negative `amount` on user-facing entrypoints. Every user-callable amount parameter on controller and pool must be validated as `> 0` before entering scaled/ray math; otherwise `saturating_sub_ray` and sign-naïve additions mint phantom state. MVX enforces this via `ManagedDecimal` (unsigned BigUint) at the type system; Stellar must enforce it manually.
- **I16** (Medium, **broken by N-05**) — isolated-debt counter increment and decrement must use oracle prices from the same tolerance regime. Borrow-side `handle_isolated_debt` uses `allow_unsafe_price=false` (strict); repay-side `adjust_isolated_debt_usd` uses `allow_unsafe_price=true` (lax/TWAP). The asymmetry lets the counter drift below real USD debt during oracle divergence, leaking the governance-set ceiling.
- **I17** (Low, **broken by N-06**) — pool accumulator must be rotatable by the controller without redeploying the pool. L-05 fix baked the accumulator into pool instance storage with no setter beyond `__constructor`; `upgrade` does not re-run construction. Revenue-key rotation requires a full pool redeploy + user-state migration.
- **I18** (Medium, **broken by N-07**) — production markets must have cross-validation between at least two price samples. `ExchangeSource::SpotOnly` is a dev-only pricing mode; `configure_market_oracle` must reject it outside a test feature flag (or at least outside `MarketStatus::PendingOracle`). SpotOnly produces single-aggregator pricing with NO tolerance / divergence / TWAP check and permits one-call weakening of oracle integrity by any ORACLE-role key.
- **I19** (Medium, **broken by N-08**) — worst-case bulk endpoints must fit within one Soroban transaction's resource envelope (events, write entries, instructions, footprint). Static analysis puts the 32+32 liquidation path at ~34 KB of event data against a 16 KB tx budget. Empirical benchmarking is the outstanding checklist item (`audit/AUDIT_CHECKLIST.md:110`); this finding makes the budget breach concrete.
- **I20** (Medium, **broken by N-09**) — deprecated e-mode category must either (a) block all risk-increasing operations (borrow, withdraw) for affected users, or (b) force subsequent operations to recompute HF/LTV against live base asset_config. Currently `process_borrow` and `process_withdraw` use stored-position boosted LTV/LT without checking category deprecation. Strategy path (`handle_create_borrow_strategy`) correctly calls `ensure_e_mode_not_deprecated`; regular-user borrow path does not — asymmetric enforcement.
- ~~I21~~ (**refuted loop 23**) — oracle ingress must validate `pd.price > 0` before HF/LTV/debt math. Already enforced at composition level via `oracle/mod.rs:41-44` `if price <= 0 { panic }`. N-11 was a false positive.
- **I22** (Low/Hardening, **broken by N-12**) — all reachable `(rate, delta_ms)` inputs to `compound_interest` must produce bounded-error output AND must not overflow i128. The 8-term Taylor is accurate only for `x ≤ 2` (annual rate × years ≤ 2); accuracy degrades to 6.8% error at x=5 and >30% at x=10; x ≥ ~25 overflows `x_pow8` and panics, permanently bricking the pool via repeated `global_sync` failure.
- **I23** (Informational, **broken by N-13**) — `calculate_health_factor` must produce a defined result (i128::MAX for infinitesimal debt) across all asset-decimal configurations. Current code divides without overflow clamping; for 18-decimal borrow tokens with $170+ collateral and 1-wei debt, overflow panic locks the account until the dust debt is repaid.

### Key formulas

- `HF = Σ(collateral_i.amount × price_i × LT_i) / Σ(debt_j.amount × price_j)` (all in WAD; LT in bps/BPS).
- `utilization = borrowed_nominal / (supplied_nominal - revenue_nominal + reserves)`.
- `borrow_index(t) = borrow_index(t-1) × (1 + borrow_rate × Δt/SECONDS_PER_YEAR)` via `compound_interest` (i256 internally).
- `supply_rate = borrow_rate × utilization × (1 - reserve_factor_bps/BPS)`.
- Liquidation: `capped = min(repay_usd_x_bonus, debt_usd, collateral_usd)`; `base_amount = capped / (1 + bonus_bps/BPS)` rounded DOWN (M-05 fix). `bonus_portion = capped - base_amount`. `protocol_fee = bonus_portion × fees_bps / BPS`.

### Unresolved proof obligations

- **PO-1** — `claim_revenue` partial path: verify that `ratio = amount/treasury` (Ray::from_raw on asset-decimal i128s) remains dimensionally safe at min/max edges. (M-04 touched this; need property test.)
- **PO-2** — `seize_position` Deposit branch: revenue+supplied accounting matches spec (L-12 flagged as doc-only; re-verify under bad-debt cascades).
- **PO-3** — TWAP `min(timestamps)` staleness path (M-07 fix): verify no off-by-one where a stale sample exactly at the `max_price_stale_seconds` boundary is accepted.
- **PO-4** — Flash-loan `flash_loan_end` delta check under rebase tokens (H-07). Operator-policy-only; no test harness for rebase yet.
- **PO-5** — Liquidation seize rounding: `base_amount` floored, `bonus_portion = capped - base_amount` → verify `bonus_portion` cannot become negative under extreme close-factor edges.

---

## N-01 — [Medium] Supply top-up refreshes `liquidation_threshold_bps` with no HF guard; bypasses keeper's 1.05 buffer

**Repo / files**:
- `controller/src/positions/supply.rs:157-158` (`update_deposit_position`) — unconditional LT refresh, no HF check after refresh.
- `controller/src/positions/supply.rs:10, 296-328` (`update_asset_params_for_account`) — keeper-invoked sibling that DOES enforce `HF ≥ THRESHOLD_UPDATE_MIN_HF = 1.05e18` when refreshing LT (the `has_risks` branch).
- `controller/src/positions/supply.rs:42` — comment "Supply is risk-decreasing." (now false).
- MVX comparator `rs-lending/controller/src/positions/supply.rs:178-189` (`update_deposit_position`) — refreshes only LTV/bonus/fees, never LT, preserving "supply risk-decreasing" invariant.
- MVX keeper `rs-lending/controller/src/positions/supply.rs:504-565` (`update_position_threshold`) — same 1.05 HF safety guard as Stellar keeper path.

**Preconditions**:
1. Owner/admin calls `edit_asset_config(asset_X, cfg_new)` where `cfg_new.liquidation_threshold_bps < cached position LT` (legitimate de-risk action; e.g. 8500 → 7000 bps).
2. User has existing supply position on `asset_X` with stored LT = 8500 and outstanding debt such that current HF at the stored LT is `[1.00, ~1.21)` (below the 1.05 safety margin that the keeper path would enforce post-refresh).
3. User calls `process_supply([(asset_X, dust_amount)])`.

**Exploit / failure path**:
- `get_or_create_deposit_position` returns the existing position with stored `LT=8500`.
- Lines 157-158 overwrite `position.liquidation_threshold_bps ← cfg_new.liquidation_threshold_bps = 7000`.
- `update_market_position` transfers dust to the pool and adds it to `scaled_amount_ray`.
- `process_supply` writes the mutated account to storage and returns — **no HF check fires**.
- On the next block, any liquidator can call `process_liquidation` on the user; the recomputed HF at `LT=7000` is now below 1, triggering liquidation, bonus seize, and protocol fee — exactly the outcome the keeper path was engineered to prevent via its 1.05 buffer.
- The user had no warning: the action they thought was "adding collateral to improve HF" instead flipped them into insta-liquidation.

**Broken invariants**:
- **I1** — "supply() cannot decrease HF." Stellar now violates this; MVX still honors it.
- **I9** — "keeper-side LT refresh enforces HF ≥ 1.05." The user-side bypass voids the defense for any user willing to make a dust top-up.

**Severity rationale**: **Medium**. No direct protocol drain, but:
1. Breaks a load-bearing UX/safety invariant (the codebase's own comment asserts it).
2. Bypasses an explicit protocol defense (`THRESHOLD_UPDATE_MIN_HF` buffer) that exists specifically to prevent the exact outcome this path produces.
3. Race-able by a griefer who watches `edit_asset_config` events and front-runs a victim's routine supply top-up (e.g., an integrator's automatic rebalancer) to force liquidation.
4. Cross-chain divergence: MVX does not have this bug; the fix in M-06 over-reached relative to the sibling implementation.

**Fix directions** (pick one):
- **A (surgical)**: After lines 154-165, if `position.liquidation_threshold_bps` was just lowered (`new_lt < old_lt`), recompute HF at end of `process_supply` and panic with `HealthFactorTooLow` when `hf < THRESHOLD_UPDATE_MIN_HF`. Mirror the keeper-path guard.
- **B (align with MVX)**: Remove the LT refresh at lines 157-158 entirely. Leave LT propagation exclusively to the keeper path (which has the guard). Accept "stale LT until next keeper propagation" as the designed behavior — same choice MVX made.
- **C**: Refuse the refresh when `new_lt < old_lt` (only allow relaxations on supply-topup). Narrow and preserves "supply can only help" invariant.

**Recommended**: **B** — matches MVX, avoids subtle HF-bypass, simplest code delta.

**Repro sketch** (Stellar native test style):
```rust
// Setup: user has position with LT=8500 and HF=1.02 (above liquidation, below 1.05).
// Admin calls edit_asset_config lowering LT to 7000.
// Compute: new HF at LT=7000 = 1.02 * (70/85) ≈ 0.84 → liquidatable.
// User calls process_supply([(asset, 1)]).  // 1 unit dust
// Expected: panic HealthFactorTooLow (if B) OR no-op on LT (if C) OR panic (if A).
// Actual: returns Ok; liquidator can immediately liquidate on next tx.
```

**Status**: ✅ verified via code + cross-chain differential. PoC test not yet written; deferred to next loop.

---

## N-02 — [**CRITICAL**] `withdraw(amount < 0)` mints phantom collateral and drains the pool

**Repo / files**:
- `controller/src/positions/withdraw.rs:85` — `let withdraw_amount = if amount == 0 { i128::MAX } else { amount };` — zero becomes the full-withdraw sentinel, but **negative values pass through unchecked**. No `require_amount_positive` anywhere in `process_withdraw` / `process_single_withdrawal`.
- `controller/src/positions/supply.rs:75`, `borrow.rs:388`, `repay.rs:50`, `liquidation.rs:39` — every other user entrypoint calls `validation::require_amount_positive(env, amount)` which panics on `amount <= 0`. Withdraw is the sole exception.
- `pool/src/lib.rs:171-227` — `pool::withdraw` performs the full math on signed i128 without checking sign.
- `pool/src/lib.rs:44-50` — `saturating_sub_ray(a, b)`: `if b.raw() >= a.raw() { ZERO } else { a - b }`. Given positive `a` and negative `b`, the guard is false, so `a - b = a + |b|` — the subtraction silently **inflates** rather than saturates.
- `pool/src/cache.rs:83-86` — `has_reserves(amount) = reserves >= amount`. With `amount < 0`, always true (reserves ≥ 0). Check is useless for negative amounts.
- `common/src/fp.rs:68-74` — `Ray::from_asset` propagates the sign through `rescale_half_up`; the core primitive explicitly handles negatives (fp_core.rs:68-74). No panic.

**MVX immunity**: `rs-lending/controller/src/positions/withdraw.rs:60-71` takes `amount: ManagedDecimal<Self::Api, NumDecimals>` — MVX's `ManagedDecimal` is backed by `BigUint`, **unsigned at the type level**. A negative amount cannot be expressed. Stellar's use of plain `i128` re-introduces a signedness hazard MVX doesn't have.

**Preconditions**:
1. Attacker controls **any account** (`require_auth`) with an existing supply position on any supported asset. No admin, no special role, no oracle manipulation.
2. Pool is live and not paused. No flash-loan active.

**Exploit path** (for `withdraw([(asset_X, amount = -A)])`, `A > 0` in asset decimals, concrete numbers: asset_decimals = 7, supply_index = RAY, pre-position `scaled_amount_ray = 100 * RAY`, pre-supplied = 1000 * RAY, A = 10⁷ = 1 token):

1. `process_withdraw`: `require_auth` passes; `require_not_paused`; `require_not_flash_loaning`; owner check passes.
2. `process_single_withdrawal` line 85: `amount = -10⁷ ≠ 0`, so `withdraw_amount = -10⁷` flows through.
3. `pool.withdraw(controller, -10⁷, position, false, 0, price)`:
   - line 184-186: `pos_scaled = 100 * RAY`, `current_supply_actual = 100 * 10⁷ = 10⁹`.
   - line 187: `if amount >= current_supply_actual` → `-10⁷ >= 10⁹` → **false**. Takes partial branch.
   - line 192: `scaled = calculate_scaled_supply(-10⁷) = Ray::from_asset(-10⁷, 7).div(supply_index) = Ray(-10²⁷ * RAY / RAY) = Ray(-RAY)`. **Negative Ray.**
   - line 196: `remaining_scaled = saturating_sub_ray(Ray(100*RAY), Ray(-RAY))` → guard `-RAY >= 100*RAY` is false → `a - b = 100*RAY - (-RAY) = 101*RAY`. **Remaining grew beyond original.**
   - line 197-202: `remaining_actual = 101 * 10⁷` (not zero), returns `(scaled = Ray(-RAY), amount = -10⁷)` as partial.
   - line 206-213: `net_transfer = -10⁷`, not liquidation, skip fee branch.
   - line 216: `has_reserves(-10⁷) = reserves >= -10⁷` → **true** (reserves always ≥ 0).
   - line 220: `cache.supplied = saturating_sub_ray(cache.supplied, Ray(-RAY)) = 1000*RAY + RAY = 1001*RAY`. **Pool-wide supplied inflated by RAY out of thin air.**
   - line 221: `position.scaled_amount_ray -= (-RAY) = 100*RAY + RAY = 101*RAY`. **User's scaled collateral inflated.**
   - line 224: `if net_transfer > 0` → `-10⁷ > 0` → **false**. Transfer skipped. **No tokens leave the pool, yet accounting credits the user.**
   - Returns `PoolPositionMutation { position: scaled=101*RAY, actual_amount: -10⁷, .. }`.
4. Controller `update::update_or_remove_position(account, &result.position)`: `scaled_amount_ray = 101*RAY ≠ 0`, so `map.set(asset, position)` — the inflated position is stored.
5. HF check at `withdraw.rs:43-52`: more collateral means higher HF → passes trivially (or skipped entirely if no debt exists).
6. `storage::set_account(env, account_id, &account)` persists the inflated state.

**Monetizing the phantom collateral** (two independent paths):
- **Direct drain**: user immediately calls `withdraw([(asset_X, +10⁷)])` with the original amount, positive this time. Pool sends 1 real token the user never deposited. Repeat the two-step `(negative, positive)` pair in a loop until the pool is drained or until `cache.supplied` overflows i128.
- **Leverage phantom as collateral for real borrows**: the inflated `scaled_amount_ray` is counted by `helpers::calculate_health_factor` and by the borrow cap logic. User calls `borrow_batch([(asset_Y, amount)])` on a DIFFERENT asset pool; the controller's HF check uses the inflated collateral; borrow succeeds; user receives real `asset_Y` tokens they never collateralized. Default on the borrow → bad-debt socialized to honest LPs of `asset_Y`'s pool.

**Broken invariants**:
- **I6** — "total value conserved up to explicit reserves/fees/spread/bad-debt" — phantom collateral enters accounting without a corresponding transfer_in.
- **I7** — "scaled ↔ nominal conversion consistent" — user's position now redeems for more than any real deposit ever funded.
- **I14** — "`revenue_scaled + user_scaled_supplied = supplied_scaled`" — still holds by construction (both sides bumped by `|scaled|`), which is why none of the post-check assertions catch the bug; but the combined `supplied` is no longer backed by reserves.

**Severity rationale**: **Critical**.
- Direct, permissionless drain of any pool the attacker has a supply position in.
- No precondition beyond a supply position (creatable via 1 unit `process_supply`).
- Exploitable in a single transaction, no front-running, no oracle manipulation, no multi-step timing.
- Cross-chain: MVX is immune by type-system construction; Stellar's loss of that safety crossed over as a silent regression.
- Detection is easy post-fact (supplied grows without transfer-in), but the attacker completes the drain in the same tx they perform it.

**Fix directions**:
- **A (minimal, required)**: add `validation::require_amount_positive(env, amount)` at the top of `process_single_withdrawal` in `controller/src/positions/withdraw.rs` (before line 84), panicking when `amount < 0`. Preserve the `amount == 0 → full-withdraw` sentinel if that behavior is intentional (also see L-03).
- **B (defense-in-depth at the pool)**: reject negative `amount` at the top of `pool::withdraw` (pool/src/lib.rs:180) with `FlashLoanError::NegativeFlashLoanFee`-style guard, so even a buggy controller cannot drive this path.
- **C (replace the sentinel)**: expose a separate `withdraw_all` endpoint and make `process_single_withdrawal` enforce `amount > 0` strictly. This also closes L-03.

**Recommended**: **A + B** together (defense-in-depth). The pool is library-like; both controller upgrades and external controller abuse are in scope.

**Repro sketch** (Stellar native test harness):
```rust
// 1. Admin setup: create pool for asset X, mint 1000 tokens to Alice.
// 2. Alice supplies 100 X; position.scaled_amount_ray = 100 * RAY.
// 3. pool.supplied = 100 * RAY, pool.reserves = 100 X.
// 4. Alice calls controller.withdraw(alice_account, vec![(X, -10_000_000)])  // -1 X
// 5. Post-state assertions:
//    assert_eq!(alice.supply_positions[X].scaled_amount_ray, 101 * RAY);  // phantom +1
//    assert_eq!(pool.supplied, 101 * RAY);                                 // phantom +1
//    assert_eq!(token.balance(alice), 900 * 10^7);                         // unchanged
//    assert_eq!(token.balance(pool), 100 * 10^7);                          // unchanged
// 6. Alice calls controller.withdraw(alice_account, vec![(X, 101 * 10^7)]) // +101 X
//    → transfers 101 X to Alice; Alice's net: started with 1000 X, ended with 1001 X.
```

**Status**: ✅ verified by static trace + cross-chain differential. Runtime PoC queued for next loop.

**Loop 13 fuzz-gap confirmation**: `fuzz/fuzz_targets/flow_multi_op.rs` uses `arb_amount(amount, 1.0, 50_000.0)` (positive-bounded float range) for every Op including `Withdraw`. Negative values provably cannot reach `try_withdraw`. This is the proximate cause of why N-02 was never caught by the existing fuzz harness. A one-line generator change (`amount ∈ {-100.0, -1.0, 0.0, ...}`) would surface the bug immediately.

---

## N-03 — [Low / Hardening] Pool-level endpoints lack defense-in-depth sign guards; rely entirely on controller validation

**Repo / files** (every admin-only pool endpoint that accepts an `amount: i128`):
- `pool/src/lib.rs:106-132` `supply(position, price_wad, amount)` — no sign check; feeds `calculate_scaled_supply(amount)` → `cache.supplied + scaled`.
- `pool/src/lib.rs:134-169` `borrow(caller, amount, position, price_wad)` — no sign check; saved only by `tok.transfer(contract, caller, amount)` (line 155), which panics on negative at the SEP-41/SAC token level.
- `pool/src/lib.rs:171-241` `withdraw(...)` — **N-02 root**; no sign check.
- `pool/src/lib.rs:243-285` `repay(caller, amount, position, price_wad)` — no sign check; negative would `saturating_sub_ray(borrowed, negative_scaled)` → inflate debt AND `position.scaled_amount_ray -= negative` → inflate user's debt; `actual_applied = amount.min(current_debt)` returns negative.
- `pool/src/lib.rs:307-321` `add_rewards(price_wad, amount)` — no sign check; feeds `Ray::from_asset(amount, dec)` → `update_supply_index(supplied, supply_index, amount_ray)` which at `rates.rs:111-120` computes `factor = Ray::ONE + rewards_ratio` → a negative `amount_ray` drives `factor < 1` and the new supply_index shrinks (or goes negative at extremes), silently deflating all supplier balances.
- `pool/src/lib.rs:348-385` `flash_loan_end(amount, fee, receiver)` — `fee < 0` explicitly rejected (line 354); `amount` has no sign check.
- `pool/src/lib.rs:387-432` `create_strategy(caller, position, amount, fee, price_wad)` — only a `fee > amount` check; neither `fee` nor `amount` sign-checked.

**Why every case today is unreachable (but brittle)**:
- `supply` receives `actual_received = balance_after − balance_before` with a `<= 0` panic in `controller/src/positions/supply.rs:213-219` (balance-delta is always positive or panics).
- `borrow` receives `amount` only after `require_amount_positive` at `borrow.rs:388`; token-transfer negative-amount panic is a secondary net.
- `withdraw` **fails** — the controller-side guard was never written; this is the N-02 bug.
- `repay` receives `actual_received` via the same balance-delta pattern at `repay.rs:63-71`.
- `add_rewards` is REVENUE-only at `lib.rs:646-650` and routed through `router::add_reward` with `require_amount_positive` at `router.rs:242` plus token-transfer panic.
- `flash_loan_begin/end` are `verify_admin`-gated and orchestrated by `controller/src/flash_loan.rs` with `require_amount_positive` at line 25.
- `create_strategy` amount / fee are `require_amount_positive`-checked at `strategy.rs:68, 73, 79, 244, 246, 372, 374, 541`.

**Broken invariant / risk**: every pool endpoint assumes "the controller has already validated sign." N-02 proved that assumption is fallible. Any future controller upgrade that omits a single `require_amount_positive` in any new or modified path reintroduces the class bug. The fix for N-02 (add the guard in `process_single_withdrawal`) does not close the class — it only closes the one known instance.

**Severity rationale**: **Low** (hardening recommendation). No currently-reachable exploit beyond N-02. The N-02 incident is itself strong evidence that the controller-side monopoly on sign validation is brittle.

**Fix direction** (hardening):
- Add `if amount < 0 { panic_with_error!(&env, GenericError::AmountMustBePositive); }` (or use an existing `common::errors` variant) at the top of each pool mutating endpoint listed above, immediately after `verify_admin(&env)`. MVX uses `ManagedDecimal<BigUint>` for the same parameters, making the equivalent pool methods impossible to invoke with a negative value at the type level; Stellar's `i128` choice requires manual enforcement in every layer.
- Optional: introduce a `PositiveI128` newtype in `common::fp` that wraps `i128` with a constructor that panics on `<= 0`. Adopt on all cross-contract ABIs carrying user amounts. Makes the invariant syntactically enforceable.

**Status**: ✅ verified by signedness survey of every `amount: i128` signature in `controller/` and `pool/`. Not currently exploitable beyond N-02; recorded as defense-in-depth + methodology hardening note.

---

## N-04 — [Informational] `add_rewards` to a zero-supply pool strands tokens as phantom reserves

**Repo / files**:
- `common/src/rates.rs:111-114` `update_supply_index`: `if supplied == Ray::ZERO || rewards_increase == Ray::ZERO { return old_index; }` — early return without booking anything.
- `controller/src/router.rs:240-253` `add_reward`: transfers `amount` tokens from caller to pool (line 252), then invokes `pool.add_rewards(price, amount)` (line 253). The transfer lands unconditionally; the pool's `supply_index` update is skipped when `supplied == Ray::ZERO`.
- `pool/src/lib.rs:307-321` `add_rewards`: does not touch `cache.supplied`, `cache.revenue`, or `cache.borrowed` — only mutates `supply_index` via `update_supply_index`. When that function short-circuits, the transferred tokens are not reflected anywhere in pool accounting.

**Failure path**:
1. Operator deploys a new market; before any supplier deposits, the REVENUE role calls `add_rewards([(asset, amount)])`.
2. `tok.transfer(caller, pool, amount)` succeeds — tokens land in the pool's balance.
3. `pool.add_rewards` calls `update_supply_index(supplied=0, old_index, ray_amount)`; early return leaves supply_index unchanged.
4. Subsequent suppliers deposit at `supply_index = RAY` (1.0). Their `scaled_amount_ray = deposit_amount`. Their redeemable balance is `scaled * supply_index / RAY = deposit_amount`.
5. The seeded reward tokens remain in `tok.balance(pool)` but are not claimable by suppliers or via `claim_revenue` (`treasury_actual = revenue_scaled * supply_index / RAY = 0`; the `min(reserves, treasury)` transfer sends 0).
6. The tokens are effectively stranded for the lifetime of the pool (or until a new bad-debt or utilization path happens to consume them).

**Severity rationale**: **Informational**. REVENUE-role operator footgun; no user loss; no protocol drain. Does indicate a missing validation — either panic with a descriptive error when `supplied == 0`, or defer the transfer and refund the caller.

**Fix direction**:
- Add an explicit guard at `pool::add_rewards` (`pool/src/lib.rs:307`): `if cache.supplied == Ray::ZERO { panic_with_error!(&env, GenericError::NoSuppliersToReward); }`. (Introduce the error variant if needed.)
- OR: defer the `tok.transfer` until AFTER `update_supply_index` confirms the reward was applied, and return a boolean from the pool to let the controller short-circuit.

**Status**: ✅ verified from `update_supply_index` behavior + `claim_revenue` arithmetic. Operator-policy-only concern; documented for DEPLOYMENT runbook.

---

## N-05 — [Medium] Isolated-debt ceiling bypass via `allow_unsafe_price=true` on user repay path

**Repo / files**:
- `controller/src/positions/repay.rs:23` — `process_repay` constructs `ControllerCache::new_with_disabled_market_price(env, /* allow_unsafe_price = */ true)`. The lax flag means oracle tolerance violations return `safe_price` (TWAP) silently instead of panicking.
- `controller/src/positions/repay.rs:131-142` — after `pool.repay`, `execute_repayment` calls `utils::adjust_isolated_debt_usd(env, account, result.actual_amount, &price_wad, feed.asset_decimals, cache)` with `price_wad = feed.price_wad` — the same lax-oracle price.
- `controller/src/utils.rs:61-92` `adjust_isolated_debt_usd`: `usd_wad = token_amount_wad × price_wad`; `new_debt = current − usd_wad` (floored at 0, with sub-$1 dust erasure at line 86-88). Decrements the isolated-debt counter by a USD figure derived from the LAX price.
- `controller/src/positions/borrow.rs:204-242` `handle_isolated_debt`: mirror operation on the borrow side, but the cache for borrow was built with `allow_unsafe_price=false` (`borrow.rs:111`). Strict tolerance; oracle PANICS on divergence beyond LAST tolerance. So the increment always uses a tolerance-checked price (aggregator, average, or block).
- `controller/src/oracle/mod.rs:108-148` `calculate_final_price`: when `agg`/`safe` prices diverge beyond LAST tolerance, returns `safe_price` if `allow_unsafe_price=true`, panics otherwise (`OracleError::UnsafePriceNotAllowed` at line 137).

**Preconditions**:
1. An asset is configured as an isolated collateral with `asset_config.isolation_debt_ceiling_usd_wad > 0`.
2. Market conditions produce a price divergence between aggregator (spot) and safe (TWAP) beyond the configured LAST tolerance band. This is natural during sharp crashes / rallies: TWAP smooths, spot moves; the two diverge by more than the tolerance, then eventually reconverge as TWAP catches up.
3. Attacker has an isolated borrow position at (or near) the debt ceiling.

**Exploit path** (numerical example: ceiling $10,000; attacker at $10,000 debt; TWAP tolerance LAST = ±10%):
1. At t₀, spot = TWAP = $100. Attacker's debt = $10,000 (ceiling).
2. At t₁, spot drops to $50; TWAP lags at $80 (spot/TWAP ratio 0.625 — beyond LAST tolerance). `calculate_final_price` takes the line 134-139 branch.
3. Attacker calls `process_repay([(debt_token, 1 unit)])`:
   - `cache.cached_price` returns `safe_price = $80` (line 139).
   - `pool.repay` settles the real debt in scaled terms (index-driven; oracle irrelevant to pool).
   - `adjust_isolated_debt_usd` decrements `isolated_debt` by `1 × $80 = $80`. `new_isolated_debt = $9,920`.
4. Real USD repaid (at real spot $50) = $50. Real outstanding debt = $9,950.
5. `isolated_debt` counter now underestimates real debt by $30.
6. Attacker waits for spot and TWAP to reconverge (both at $50, or within FIRST tolerance).
7. Attacker calls `borrow_batch([(debt_token, 0.6 unit)])`:
   - Borrow cache is strict (`allow_unsafe_price=false`); oracle returns $50 cleanly.
   - `handle_isolated_debt` increments: `new_debt = $9,920 + 0.6 × $50 = $9,950`. ≤ ceiling $10,000 — PASSES.
8. Attacker's real USD debt: $9,950 (real) + $30 (step 7 excess) = $9,980. Counter: $9,950.
9. Repeat across price events: each divergence window lets the attacker decrement the counter more than real-USD repaid, creating persistent drift between counter and reality.
10. Over many cycles, the attacker accumulates real debt that exceeds the governance-set `isolation_debt_ceiling_usd_wad` by an arbitrary margin.

**Broken invariants**:
- **I14-iso (new)**: "`isolated_debt_wad` must equal the sum of USD-valued real debt outstanding against the isolated collateral, priced at the same tolerance level used for borrow-side admission." Current code increments at strict price, decrements at lax price — dimensionally asymmetric.
- The debt ceiling governance lever — intended to cap aggregate exposure on new/volatile isolated assets — becomes a soft cap that leaks during crashes.

**Severity rationale**: **Medium**.
- Not a direct drain: the excess debt is still subject to HF checks (strict oracle) at the moment of borrow. The attacker cannot mint phantom collateral or escape liquidation.
- Weakens a deliberate governance safety mechanism designed for the exact class of assets (newly listed, illiquid, correlated-risk) where the ceiling matters most.
- Exploitable by any isolated-position holder during normal oracle behavior (no oracle manipulation required; divergence naturally occurs during crashes).
- Cumulative: each price event can widen the counter-vs-real gap; ceilings degrade over time.
- Cross-chain: worth confirming MVX's equivalent path for the same asymmetry — left as next-loop work.

**Fix directions**:
- **A (recommended, oracle-independent)**: make `adjust_isolated_debt_usd` proportional rather than USD-denominated. Compute `repaid_fraction = actual_amount / outstanding_debt_before_repay` in token units (oracle-free; derived from pool's `calculate_original_borrow`). Decrement `isolated_debt` by `current_isolated_debt × repaid_fraction`. Symmetric with the increment path because borrow's `handle_isolated_debt` uses the SAME strict price at add-time; full repay via this route zeroes the counter (matching the sub-$1 dust erasure at line 86-88).
- **B (surgical)**: force `adjust_isolated_debt_usd` to always use a strict-oracle price, independent of cache flag. If strict oracle would panic, the repay path can still proceed for user convenience, but the isolated-debt counter is simply NOT decremented on this call (the next strict-oracle repay gets it). Accept the UX degradation for correctness.
- **C (least-invasive)**: at `adjust_isolated_debt_usd`, if `cache.allow_unsafe_price == true` AND the aggregator price is available and within FIRST tolerance of safe, use the aggregator. Otherwise, skip the decrement. Preserves UX for normal conditions; blocks the bypass during divergence.

**Repro sketch** (native test):
```rust
// 1. Deploy market with isolation_debt_ceiling_usd_wad = $10,000.
// 2. Alice borrows 100 tokens @ $100/token => isolated_debt = $10,000.
// 3. Simulate oracle divergence: spot = $50 (via aggregator mock), TWAP = $80.
// 4. Alice calls process_repay(1 token). Assert:
//    a. pool.repay booked 1 token worth of debt reduction (real).
//    b. adjust_isolated_debt_usd decremented by $80 (safe_price), not $50.
//    c. cache.get_isolated_debt = $9,920 (vs. real remaining $9,950).
// 5. Oracle reconverges; Alice borrows 0.6 token @ $50.
//    Assert handle_isolated_debt increment passes at counter $9,920 + $30 = $9,950 ≤ ceiling.
// 6. Real debt = $9,980 > ceiling $10,000 − $20 = $9,980. Ceiling is breached by $−20 (not caught).
```

**Status**: ✅ verified by static trace across `oracle::calculate_final_price`, `utils::adjust_isolated_debt_usd`, `borrow::handle_isolated_debt`. Reproducer queued.

**Cross-chain parity (loop 5)**: MVX `rs-lending/controller/src/positions/repay.rs:51-77` (`update_isolated_debt_after_repayment`) has the same SHAPE — uses `feed.price_wad` on the repay side.

**Loop 7 deep-dive**: MVX `rs-lending/controller/src/oracle/mod.rs:60-68` documents an explicit "Operation Safety Matrix":
| Operation | First Tolerance | Second Tolerance | High Deviation |
|-----------|----------------|------------------|----------------|
| Supply / Repay | safe | average | **average** |
| Borrow / Withdraw / Liquidate | safe | average | **BLOCKED** |

MVX uses `(aggregator + safe) / 2` for supply/repay at ANY deviation, while Stellar's lax path (`calculate_final_price` branch 3 at `controller/src/oracle/mod.rs:134-139`) uses `safe_price` (TWAP) alone when deviation exceeds LAST tolerance. This makes MVX's N-05 exposure **strictly smaller**: the decrement-inflation magnitude is capped at the average, not the TWAP-only extreme. MVX has the same bug class with half the lever. Stellar's TWAP-only fallback at high deviation is the amplifier that makes this Stellar-side a distinct Medium.

---

## N-06 — [Low / Hardening] Pool accumulator address is construct-only; no rotation path

**Repo / files**:
- `pool/src/lib.rs:77-100` `__constructor(env, admin, params, accumulator)` writes `PoolKey::Accumulator` exactly once at pool deployment.
- `pool/src/lib.rs:485-504` `claim_revenue` reads `PoolKey::Accumulator` and transfers revenue unconditionally to that address.
- `pool/src/lib.rs:540-618` `update_params` is the only pool mutator that changes stored config — it updates `MarketParams` fields (rate model + reserve factor) but never touches the accumulator slot.
- `pool/src/lib.rs:620-623` `upgrade` only swaps WASM bytecode; Soroban's `upgrade` does not re-run `__constructor` and therefore cannot rewrite the `Accumulator` instance-storage entry.
- Controller has `controller::config::set_accumulator` at `controller/src/config.rs:44-47` + `controller/src/lib.rs:465-467`, but these only update the controller's own `ControllerKey::Accumulator`; they do not propagate to any deployed pool.

**Design context**: L-05 fix baked the accumulator into the pool at construction specifically to ignore caller-supplied destinations. The trade-off — deliberate or accidental — is that rotation becomes impossible without a new deploy.

**Failure path** (operational, not an exploit):
1. Revenue-key is compromised (private-key leak, insider, custody provider breach). The accumulator at `0xOLD` is no longer trusted.
2. Owner calls `controller.set_accumulator(0xNEW)` — controller's own stored accumulator updates, but this address is not consulted by any pool.
3. A subsequent `controller.claim_revenue([A, B, C, …])` call routes through `router::claim_revenue_for_asset` → `pool.claim_revenue` → each pool transfers to its stored `0xOLD`.
4. Net effect: until every pool is redeployed (or pool WASM is upgraded to expose a rotation endpoint), accrued + future revenue continues flowing to the compromised address.

**Redeploy path**: `create_liquidity_pool` (config.rs / router.rs) deploys a new pool contract with a new accumulator, but migrating live state (supply_index, borrow_index, supplied/borrowed/revenue accumulators, user scaled balances) from old to new pool has no in-protocol path. The pool is the storage owner for market accounting; a fresh deployment means users' scaled balances are stranded unless a migration script is written per market.

**Broken invariant / risk**: operator-rotation assumption from L-05's design does not hold in the "revenue-key-compromised" scenario. If the L-05 threat model included key rotation as a recovery path, the current architecture forecloses it.

**Severity rationale**: **Low** (hardening).
- Not an exploit; requires the upstream compromise event (revenue-key loss).
- During a compromise window, ongoing revenue is mis-routed; user funds and pool accounting are unaffected.
- No pause or kill-switch on `claim_revenue` alone; operator can pause the whole controller (`pausable::pause`) to stop new claims but revenue continues accruing on pool-internal counters.
- Reachable only through governance / ops failures, not user action.

**Fix directions**:
- **A (recommended)**: read the accumulator from the CONTROLLER at `claim_revenue` time instead of from pool storage. Pool calls back into the controller for the destination address. Controller's `ControllerKey::Accumulator` is the single source of truth, and `set_accumulator` (owner-only) rotates across all pools instantly. Preserves the L-05 fix (pool still ignores caller-supplied destination) while regaining rotation.
- **B (alternative)**: add a pool-level `set_accumulator(env, addr)` endpoint `verify_admin`-gated, callable only by the controller. Add a controller-level `rotate_pool_accumulator(asset, addr)` owner-gated wrapper. Explicit rotation ceremony.
- **C (document as-is)**: if the design intentionally forecloses rotation (e.g., to prevent a compromised owner from redirecting revenue), make this explicit in `architecture/ACTORS.md` + `architecture/DEPLOYMENT.md` as a known limitation with a runbook for the "deploy new pool + user migration" path.

**Status**: ✅ verified. No rotation path exists in pool; design docs do not discuss the rotation scenario.

---

## N-07 — [Medium] `ExchangeSource::SpotOnly` is accepted in production `configure_market_oracle` with no fencing

**Repo / files**:
- `common/src/types.rs:43-44` — `ExchangeSource::SpotOnly = 0` is the default enum variant.
- `common/src/types.rs:240-242` — `OracleProviderConfig::default_for` returns `exchange_source: ExchangeSource::SpotOnly`.
- `controller/src/oracle/mod.rs:82-86` — SpotOnly branch calls `cex_spot_price`, returning aggregator spot directly. **No TWAP, no tolerance check, no divergence fallback.** Only `check_staleness` (max_price_stale_seconds) applies.
- `controller/src/config.rs:363-426` `configure_market_oracle` — accepts `config.exchange_source` unchecked. No guard rejects `SpotOnly` in production. The three-branch validation of `MarketStatus::(PendingOracle|Active|Disabled)` at lines 369-374 permits mutation from `Active` state.
- `controller/src/lib.rs:565-572` — public endpoint `configure_market_oracle` is `#[only_role(caller, "ORACLE")]` gated.
- `architecture/CONFIG_INVARIANTS.md:50`, `architecture/INVARIANTS.md:557-559` — comments label `SpotOnly` as "DEV" but no code enforces this.

**Preconditions**:
1. Adversary possesses (or has compromised) the ORACLE role key.
2. Any allowlisted market is Active or Disabled (i.e., past `PendingOracle`).

**Exploit path**:
1. Adversary calls `configure_market_oracle(asset_X, { exchange_source: SpotOnly, max_price_stale_seconds: 86_400, cex_oracle: ..., cex_symbol: ..., tolerance: any, twap_records: 0, ... })`.
2. Lines 393-404 overwrite `market.oracle_config.exchange_source = SpotOnly`. Tolerance bands are stored but SpotOnly path never consults them.
3. Market status persists at `Active`.
4. Any subsequent price read for `asset_X` goes through `cex_spot_price`: a single `lastprice(reflector_asset)` call, subject only to the 86_400-second staleness check.
5. Adversary (or any party with aggregator-manipulation capability) can then:
   - Flash-loan-based CEX/DEX aggregator manipulation on a Reflector source feeding this market → oracle price briefly reflects an extreme value → liquidate healthy positions at the manipulated price, seizing bonus-priced collateral.
   - Alternatively: set TWAP-depending functions to evaluate against a fresh-but-manipulated spot that would otherwise have been smoothed.
6. After the attack, the ORACLE role can call `configure_market_oracle` again to restore a safe mode, obscuring the attack in the event stream.

**Compared to alternative modes**:
- `SpotVsTwap` (production default): `calculate_final_price` enforces first/last tolerance between spot and TWAP; if spot diverges beyond LAST tolerance under strict paths (borrow, withdraw, liquidation), the call panics. Manipulation-resistant by construction.
- `DualOracle`: CEX TWAP vs DEX spot cross-validation. Requires `dex_oracle.is_some()` (config-validated).
- `SpotOnly`: ZERO cross-validation. ZERO divergence check. Only staleness.

**Broken invariant**: new I18 — "production markets must have cross-validation between at least two price samples (spot vs TWAP, or CEX vs DEX); `ExchangeSource::SpotOnly` is a dev-only pricing mode and must be rejected at config time on live networks."

**Severity rationale**: **Medium**.
- Gated by ORACLE role, so not permissionless.
- ORACLE role is separately compromisable (M-02 mitigation leaves it explicit-grant rather than auto-granted at construction; but once granted, one call is enough to remove all oracle defenses).
- Effect: strictly weakens oracle integrity on the target market, enabling a downstream-permissionless exploit (manipulation + liquidation).
- Single-call weaponization; no time delay, no two-step, no emergency-pause prerequisite. Contrast with M-01 (`disable_token_oracle`) which DOES stop trading; this bypasses tolerance while keeping the market OPEN.
- ORACLE role in the "compromised or malicious" threat model already covers many attacks, but the SpotOnly switch is a particularly potent one because it degrades silently — all endpoints keep working, just without protection.

**Fix directions**:
- **A (recommended)**: reject `exchange_source == ExchangeSource::SpotOnly` in `configure_market_oracle` unless the build is in a test/dev feature flag. Explicit:
  ```rust
  #[cfg(not(feature = "testing"))]
  if matches!(config.exchange_source, ExchangeSource::SpotOnly) {
      panic_with_error!(env, OracleError::SpotOnlyNotProductionSafe);
  }
  ```
- **B (alternative)**: keep SpotOnly accepted but require `market.status = PendingOracle` (i.e., only at initial setup, never to an Active/Disabled market). This makes downgrading a live market impossible.
- **C (defense-in-depth, regardless)**: in the oracle module's SpotOnly branch, reuse the three-branch `calculate_final_price` with `aggregator=spot, safe=None` so `allow_unsafe_price=false` callers still benefit from at least a staleness-anchored sanity check.

**Status**: ✅ verified. Test coverage confirms that `configure_market_oracle` permits SpotOnly (no rejection in `test_configure_market_oracle_*` suite at `lib.rs:1332-1400`). Operator-policy documentation (`CONFIG_INVARIANTS.md`) labels SpotOnly as "DEV" but no code enforces this.

---

## N-08 — [Medium] Worst-case liquidation exceeds Soroban `tx_max_contract_events_size_bytes` budget, bricking bad-debt cleanup

**Repo / files**:
- Soroban baseline budget (`architecture/SOROBAN_LIMITS.json`): `tx_max_contract_events_size_bytes = 16384` (16 KB).
- `controller/src/positions/liquidation.rs:56-91` — per-debt-asset `emit_update_position(action="liq_repay", ...)` emits one `UpdatePositionEvent` per repaid asset (up to 32, per `PositionLimits.max_borrow_positions`).
- `controller/src/positions/liquidation.rs:94-126` — per-collateral-asset `emit_update_position(action="liq_seize", ...)` emits one `UpdatePositionEvent` per seized asset (up to 32, per `PositionLimits.max_supply_positions`).
- `pool/src/lib.rs:300-304, 298-305` (supply/borrow/repay/withdraw/seize_position/claim_revenue) — each mutation emits `emit_market_update` → `UpdateMarketState` event. Liquidation triggers 64 pool calls (32 `pool.repay` + 32 `pool.withdraw` with `is_liquidation=true`), so 64 additional market-update events.
- `common/src/events.rs:249-269` `UpdatePositionEvent` payload: `Symbol action` + `i128 index` + `i128 amount` + `EventAccountPosition { position_type, asset_id: Address, scaled_amount_ray: i128, account_nonce: u64, 4 × i128 risk params }` + `Option<i128>` + `Option<Address>` + `Option<EventAccountAttributes>`. Rough byte cost after XDR encoding: ~350-400 bytes per event, including topics.
- `common/src/events.rs:236-248` `UpdateMarketState`: 6 × i128 + Address + u64 → ~140-180 bytes per event.

**Budget arithmetic** (worst-case single-tx 32+32 liquidation):
- 32 `UpdatePositionEvent(liq_repay)` × ~380 bytes = ~12,160 bytes.
- 32 `UpdatePositionEvent(liq_seize)` × ~380 bytes = ~12,160 bytes.
- 64 `UpdateMarketState` (one per pool call) × ~160 bytes = ~10,240 bytes.
- **Total ≈ 34.5 KB — 2.1× the 16 KB tx event budget.**

**Failure path**:
1. Attacker opens an account with 32 supply positions + 32 borrow positions (legitimate usage under current `PositionLimits` defaults).
2. Market moves; account becomes liquidatable.
3. Liquidator submits `process_liquidation(account, debt_payments_covering_all_32_debts)`.
4. Execution reaches the seize loop at `liquidation.rs:94`, but **before completing all 32 iterations, the transaction aborts with `ExceededLimit` on event-size budget.**
5. The entire liquidation reverts — NO debt reduced, NO collateral seized, NO partial progress. Attacker's position stays underwater; LPs of the debt assets wear the bad debt.
6. No liquidator can ever atomically resolve this account within current budgets; recovery requires either a governance intervention (lowering `max_supply_positions`/`max_borrow_positions` after the fact, which does not shrink existing positions) or off-chain coordination.

**`clean_bad_debt_standalone` (liquidation.rs:447-476)** has the same pattern — iterates all supply + borrow positions, each triggering `pool.seize_position` which emits a market-update event. Below the seize threshold ($5 collateral), the account has likely been liquidated down, so positions are fewer. Still a latent risk.

**Broken invariant**: new I19 — "worst-case bulk endpoints must fit within a single Soroban transaction's resource envelope (event size, write entries, instruction count, footprint)." The empirical benchmark for the 32+32 liquidation path is listed as outstanding at `audit/AUDIT_CHECKLIST.md:110` — this finding makes the risk concrete.

**Severity rationale**: **Medium** (griefing + liveness + bad-debt accumulation).
- No direct token drain; attacker cannot steal. BUT:
- Any account with 32+32 positions is un-liquidatable atomically → LPs absorb its bad debt during adverse market moves.
- A griefer with modest capital can intentionally create such accounts on low-liquidity markets to maximize protocol damage during volatility.
- Severity capped by the fact that `PositionLimits` defaults are operator-tunable (M-11 documents immediate-effect changes) — but the limits CANNOT be reduced on existing accounts. Existing worst-case positions remain un-liquidatable even after a limit change.

**Fix directions**:
- **A (immediate, measurable)**: empirical benchmark — write an integration test in `test-harness/` that builds a 32+32 position account, triggers liquidation, and confirms actual event-size cost. If within budget, retain; if not, proceed to (B).
- **B (operator-side)**: reduce `PositionLimits` to counts that benchmark within budget (e.g., 16+16 or 8+8). `set_position_limits` is owner-only immediate-effect — but existing accounts keep their positions.
- **C (structural)**: emit a SINGLE aggregate event per liquidation (summarizing repaid and seized totals) rather than per-asset events. Decomposed per-asset data can be reconstructed off-chain from pool-level `UpdateMarketState` events already emitted. Roughly: 1 `LiquidationSummary` event (< 1 KB) + 64 pool market-updates (still within budget if position count ≤ 16 per side, or reduce market-update emission frequency).
- **D (partial-liquidation path)**: accept that worst-case atomic liquidation isn't feasible; design a staged liquidation where liquidator submits `process_liquidation_staged(account, debt_asset_subset, seize_asset_subset)` and iterates across multiple transactions. Current design's seize loop iterates ALL supply positions regardless of debt payment subset, which prevents partial progress.

**Recommended path**: **A** first (confirm the problem is real), then **C** or **D** depending on benchmark magnitude.

**Status**: ⚠️ verified by static analysis + Soroban budget numbers. Exact event-size cost needs empirical measurement (calls out to the same outstanding benchmark the audit checklist already flagged: `audit/AUDIT_CHECKLIST.md:110`).

**Loop 7 amplification — split-storage architecture**: the account state is persisted as one `AccountMeta(u64)` entry + one `SupplyPosition(u64, Address)` per held asset + one `BorrowPosition(u64, Address)` per held asset (`controller/src/storage/mod.rs:256-281`). Loading a 32+32 account costs 65 distinct `disk_read_entries` BEFORE any market/pool reads. Worst-case budget accounting for a 32+32 liquidation:
- Read entries: 65 (user state) + 32 (market configs) + 64 (pool Params+State across 32 pools) ≈ 161 distinct. Budget: 200 (`tx_max_disk_read_entries`). ~80% consumed.
- Write entries: up to 65 (user state if every position changes) + 32 (pool state writes) + 4 (isolated-debt flushes) ≈ 101. Budget: 200 (`tx_max_write_ledger_entries`). ~50% consumed.
- Footprint entries: ~170. Budget: 400 (`tx_max_footprint_entries`). Comfortable.
- Events: ~34 KB (original finding). Budget: 16 KB. **2.1× over — HARD breach.**
The event-size limit is the binding constraint; reads/writes are tight but probably fit. Empirical measurement should focus on events first, then confirm read/write headroom at live market-config sizes.

---

## N-09 — [Medium] Deprecated e-mode category leaves cached-inflated LTV / LT on user positions; borrow and withdraw bypass the deprecation wind-down

**Repo / files**:
- `controller/src/positions/emode.rs:11-27` `apply_e_mode_to_asset_config`: when `cat.is_deprecated`, returns early WITHOUT applying e-mode overrides. `asset_config` stays at base values.
- `controller/src/positions/emode.rs:63-69` `ensure_e_mode_not_deprecated`: panics when category is deprecated. Called in a **subset** of paths, not all.
- `controller/src/positions/supply.rs:67` — supply path DOES call `ensure_e_mode_not_deprecated` (blocks supply into deprecated).
- `controller/src/positions/account.rs:20` — account creation DOES call it (blocks new accounts).
- `controller/src/positions/borrow.rs:32` — `handle_create_borrow_strategy` (strategy path) DOES call it.
- **`controller/src/positions/borrow.rs:376-440` `process_borrow` (the regular-user borrow path) DOES NOT** call `ensure_e_mode_not_deprecated`. It calls `apply_e_mode_to_asset_config` (line 398), which silently falls back to base config for the asset_config check, BUT the post-batch HF-equivalent validation (`validate_ltv_collateral`) uses `ltv_base_amount_wad` computed from `helpers::calculate_ltv_collateral_wad`.
- `controller/src/helpers/mod.rs:40-58` `calculate_ltv_collateral_wad`: iterates `supply_positions` and uses `position.loan_to_value_bps` (the STORED, possibly-boosted value) — NOT a live asset_config value. `ltv = Σ position.amount × position.LTV × price`.
- `controller/src/positions/withdraw.rs:43-52` — withdraw HF check uses `helpers::calculate_health_factor`, which uses `position.liquidation_threshold_bps` (the STORED, possibly-boosted LT). No `ensure_e_mode_not_deprecated` gate.
- `controller/src/positions/supply.rs:315-328` — keeper's `update_asset_params_for_account` path refreshes LT but enforces `HF ≥ THRESHOLD_UPDATE_MIN_HF = 1.05e18`. If the refresh would drop HF below 1.05, it **panics** — the keeper cannot force-refresh an already-overextended position back to base values.

**Preconditions**:
1. An e-mode category is configured with LTV/LT strictly greater than the base asset config (the whole point of e-mode — see `add_e_mode_category` at `controller/src/config.rs:102-123`).
2. A user has a position in that category with cached LTV/LT matching the e-mode boost.
3. The Owner/ORACLE admin calls `edit_e_mode_category` (or some deprecation path) to set `is_deprecated = true`.
4. User's position is outside the 1.05 HF buffer the keeper would enforce on refresh (i.e., user's HF at base LT would be < 1.05).

**Exploit path** (numerical: base LTV = 7500, base LT = 8000; e-mode LTV = 8800, LT = 9000; collateral = $1000, debt = $850):
1. Pre-deprecation state:
   - `position.loan_to_value_bps = 8800`, `position.liquidation_threshold_bps = 9000`.
   - Real LTV cap at e-mode = $1000 × 0.88 = $880. Debt $850 ≤ $880. OK.
   - Real HF at e-mode = (1000 × 0.9) / 850 = 1.058. Healthy.
2. Admin calls `edit_e_mode_category(X, ..., is_deprecated = true)`.
3. Keeper tries `update_account_threshold(alice_account, asset)` to refresh LT to base 8000:
   - Computed HF post-refresh = (1000 × 0.8) / 850 = 0.941 < 1.05 → `HealthFactorTooLow` panic.
   - **Refresh blocked. Position stays at stored LT = 9000.**
4. Alice calls `borrow_batch([(USDT, $25)])`:
   - `calculate_ltv_collateral_wad` uses `position.loan_to_value_bps = 8800` → ltv = $880.
   - `validate_ltv_collateral`: total_debt + new_borrow = 850 + 25 = 875 ≤ 880. **Passes under cached boost.**
   - Borrow succeeds. Alice gains $25 real debt.
   - Post-borrow, using stored LT = 9000: HF = 900/875 = 1.029. Still looks healthy.
   - Real HF at base LT = 8000: HF = 800/875 = 0.914. Underwater.
5. Alice calls `process_withdraw([(USDC, small_amount)])`:
   - Withdraw HF check uses `position.liquidation_threshold_bps = 9000`.
   - Withdraw up to ~$26 keeps HF at stored LT ≥ 1 (999×0.9 = $899.1 ≥ 875 debt). Passes.
   - Real collateral remaining: $974 at base LT = $779. Debt $875. Gap = $96 bad debt.
6. Alice repeats step 5 across multiple transactions until `position.scaled_amount` × cached-LT = debt + epsilon.
7. Liquidator attempts `process_liquidation`:
   - HF using stored LT = 1.0 (just barely). `validate_liquidation_health_factor` rejects (requires HF < 1).
   - **Position is un-liquidatable.** No one can recover the bad debt.
8. Alice holds $25 new borrow + $~26 withdrawn collateral ≈ $51 of value extracted. Protocol bears ~$100 uncollateralized debt. Protocol suppliers absorb via bad-debt socialization (when eventually triggered via `clean_bad_debt_standalone` once collateral_usd ≤ $5).

**Amplification**: admin-triggered scenario. A single e-mode deprecation that touches a large category (many users) creates many simultaneously zombie positions. Each user can unilaterally drain their own boost slack. Aggregate protocol loss = `Σ over users (e_mode_ltv - base_ltv) × collateral`.

**Broken invariants**:
- The I1 class: "risk-decreasing endpoints don't break HF." Borrow uses stored-position LTV that diverges from current asset_config under deprecation. Similar shape to N-01 but on the LTV axis (borrow cap) rather than LT (liquidation threshold).
- **Missing I20**: "deprecated e-mode category must either block all risk-increasing operations for affected users OR force an HF recomputation against base config." Current code neither blocks borrow nor recomputes against live base config.

**Severity rationale**: **Medium**.
- Admin-triggered (Owner or ORACLE role — e-mode deprecation path TBD on exact role, but admin-gated). Not permissionless.
- Once triggered, every user in the category becomes unilaterally drainable up to the boost slack. No further admin action needed.
- Keeper mitigation fails by design (HF buffer blocks refresh on overextended positions).
- No bypass of per-position debt ceiling, but leaks asset-level LTV cap.
- Recovery requires governance to revert deprecation, wait for users to unwind, OR accept bad-debt socialization.

**Fix directions**:
- **A (align with strategy path)**: add `emode::ensure_e_mode_not_deprecated(env, &e_mode)` at the top of `process_borrow` (borrow.rs:376-395), matching the guard already in `handle_create_borrow_strategy` (borrow.rs:32). Deprecated users can only repay and withdraw (against stale LT, which remains safe because withdraw-all at stored LT cannot exceed the collateral value itself).
- **B (stricter)**: refuse WITHDRAW as well for deprecated e-mode users until keeper refresh has run (essentially freeze the position at current state). This prevents the withdraw leg of the exploit but creates a UX lock until the keeper can safely refresh.
- **C (recompute on use)**: in `calculate_ltv_collateral_wad` and `calculate_health_factor`, check each position's e-mode category; if deprecated, fall back to the live asset_config base values for that position (ignore the stored-position boosted values). Keeps operations open but snaps to safe math.
- **D (HF buffer relaxation)**: reduce `THRESHOLD_UPDATE_MIN_HF` from 1.05 to 1.0 on deprecation-driven refreshes so keeper can force users back to base values whenever real HF ≥ 1. Trades UX (users liquidatable immediately after refresh) for safety (no zombie positions).

**Recommended**: **A + C** together (A closes borrow leak, C closes withdraw leak with no keeper dependency).

**Status**: ✅ verified by static trace of `emode.rs`, `borrow.rs`, `helpers/mod.rs`, `withdraw.rs`, `supply.rs` deprecation handling. Runtime PoC queued.

**Loop 9 reaffirmation**: `validate_ltv_collateral` (borrow.rs:311-338) uses `ltv_base_amount_wad` (the pre-computed cap from `calculate_ltv_collateral_wad`). No second live-asset-config recomputation. `total_borrowed_wad` is built from live borrow positions × live prices, but the CAP side uses cached position LTV. Asymmetry confirmed; N-09 stands exactly as written.

**Loop 18 cross-chain parity (critical update)**:
- **MVX `borrow` endpoint** (`rs-lending/controller/src/lib.rs:252`) calls `self.ensure_e_mode_not_deprecated(&e_mode)` BEFORE the per-token `process_borrow` loop (line 264).
- **Stellar `borrow_batch`** (`controller/src/positions/borrow.rs:97-134`) has NO such check.
- **Therefore**: the borrow leg of N-09 is Stellar-specific. MVX blocks deprecated-e-mode borrows at endpoint entry.
- **MVX withdraw** (`rs-lending/controller/src/lib.rs:172-188`) also has NO deprecation check — same as Stellar. The withdraw leg of N-09 is SHARED across both chains.
- **Net MVX exposure**: withdraw-only leg remains exploitable (user can extract collateral up to cached-boosted LT HF limit), but cannot compound the exploit by borrowing additional debt. Magnitude is roughly half of Stellar's full exploit.
- **Fix for Stellar = align with MVX**: add `emode::ensure_e_mode_not_deprecated(env, &e_mode)` to `borrow_batch` entry (mirror pattern at MVX lib.rs:252). This closes the most dangerous leg and matches cross-chain behavior.

---

## N-10 — [Informational] Pause blocks `clean_bad_debt`; interest accrues during pause and releases in a single jump

**Repo / files**:
- `controller/src/lib.rs:355-359` `clean_bad_debt(caller, account_id)`: calls `validation::require_not_paused(&env)` before delegating to `liquidation::clean_bad_debt_standalone`. Pause blocks bad-debt recovery.
- `controller/src/positions/liquidation.rs:29` — same for `process_liquidation`.
- `pool/src/interest.rs:17-48` `global_sync` computes `delta_ms = current_timestamp - cache.last_timestamp` and applies compound interest. No mutations occur during pause (every mutator gates on `require_not_paused`), so `last_timestamp` doesn't advance until pause is released. At release, the first pool call sees `delta_ms = full_pause_duration` and applies the full compound on that single mutation.

**Failure path** (operational, not exploit):
1. Incident triggers `pausable::pause`. All mutators halt, including liquidation and bad-debt cleanup.
2. Market conditions degrade during pause — ordinarily healthy accounts slide underwater. Cannot be liquidated during pause.
3. Per the outstanding issue on M-01: `disable_token_oracle` can stop specific markets, but `pause` blocks the recovery path too.
4. At unpause: `global_sync` on the first pool mutation sees a very large `delta_ms`. Compound interest jumps. Borrowers' debts balloon. Suppliers' share-price jumps too, but borrowers may see their HF drop (interest accrues faster than collateral appreciates unless tokens have native yield).
5. Cascading liquidations trigger on release. Bad debt that accumulated during pause is socialized in one shot via the supply-index drop.

**Observations**:
- `pause` is an emergency brake, not a fine-grained tool. Aave and Compound have similar semantics; standard.
- For the protocol's use case, this is usually acceptable — operators pause in true emergencies and unpause as soon as conditions permit.
- Worth documenting in DEPLOYMENT runbook so operators understand: "pausing during a market stress event DOES trap debt accumulation; unpause triggers a pulse of liquidations."

**Severity rationale**: **Informational**. Design choice with standard semantics. No exploitation path. The `compound_interest` step at pool/src/interest.rs has a time-cap (see `i128::MAX` guards in `rates.rs`), so a multi-year pause won't overflow arithmetic — but the user-perceived debt jump is real.

**Fix directions (optional)**:
- **A (documentation)**: note the pause-blocks-recovery behavior in `architecture/DEPLOYMENT.md` runbook.
- **B (scope narrowing)**: consider whether `clean_bad_debt` should bypass pause — bad-debt socialization is a risk-reducing cleanup action, arguably orthogonal to the reasons for pausing. Current design couples them; decoupling would require auditor sign-off.
- **C (interest cap)**: during `global_sync`, cap `delta_ms` at some maximum (e.g., 7 days) to prevent unboundedly-large interest jumps on unpause. Trades accuracy of interest accrual for bounded post-pause surprise.

**Status**: ✅ verified by call-site audit of every `require_not_paused` call and review of `global_sync` timestamping.

---

## N-11 — [Low / Hardening] Oracle pipeline accepts negative `pd.price` from Reflector without sign validation

**Repo / files**:
- `controller/src/oracle/reflector.rs:22-25` — `ReflectorPriceData { price: i128, timestamp: u64 }`. Price is signed.
- `controller/src/oracle/mod.rs:202` `cex_spot_price`: `Wad::from_token(pd.price, market.cex_decimals).raw()` — no sign check on `pd.price`.
- `controller/src/oracle/mod.rs:224, 236, 289` — `cex_spot_and_twap_price` and `cex_twap_price` sum prices into an i128 accumulator without checking per-sample sign: `sum += pd.price`.
- `controller/src/oracle/mod.rs:108-148` `calculate_final_price`: aggregator/safe args are raw i128; tolerance check (`is_within_anchor`) divides and compares without requiring positive inputs.
- `controller/src/helpers/mod.rs:54,86,106,155,176,368` — HF / LTV / debt calculations use `Wad::from_raw(feed.price_wad)` and multiply by position amounts; negative `price_wad` flows through.

**Failure paths (all theoretical under SEP-40 normal behavior, but defense-in-depth gaps)**:
1. **Negative-price leak**: a compromised or misbehaving Reflector returns `pd.price < 0`. The price enters the pipeline unchecked. HF numerator turns negative for that asset's contribution. Depending on signs across multiple collaterals/debts, HF can become `> 1` for an insolvent account or `< 1` for a healthy one. Neither direction is acceptable.
2. **TWAP sum overflow**: `sum += pd.price` with 60 samples at i128::MAX / 60 ≈ 2.8 × 10^36 per sample would overflow. Realistic Reflector prices are far smaller (~10^18 WAD); no realistic exploit, but an unchecked `+=` on user-reachable data is a code-review flag.
3. **Zero-price silent continuation**: `pd.price = 0` produces `spot_wad = 0`. `calculate_final_price` doesn't reject zero prices — they'd flow through tolerance comparisons (division by zero in `is_within_anchor`?) and produce nonsensical `price_wad` values downstream. Some HF paths have explicit `price_wad == 0` skip logic (e.g., `liquidation.rs:329`), but not all.

**Broken invariants** (defense-in-depth):
- "Reflector oracle data must be sign-validated at ingress; the price must be strictly positive."
- "TWAP accumulator must be `checked_add` or justified unbounded."

**Severity rationale**: **Low / Hardening**.
- Requires Reflector (an external trusted oracle) to be compromised or misbehaving. If Reflector is honest, these defenses are never needed.
- BUT: SEP-40 spec allows signed i128 for `price`; nothing in the interface guarantees positivity. The Reflector team could add a derived feed tomorrow whose pricing math produces negatives (unlikely but possible).
- Protocol's existing defense-in-depth (e.g., H-03 supply_index floor guard, M-04 triple-min clamp) suggests the team values redundancy. Sign check is a natural peer.
- No exploit path under current Reflector behavior; entirely a regression-resistance concern.

**Fix directions**:
- **A (surgical)**: in `cex_spot_price`, `cex_spot_and_twap_price`, and `cex_twap_price`, reject `pd.price <= 0` at ingress with `OracleError::InvalidPrice`. Two lines per callsite. Covers negative AND zero.
- **B (type-level)**: introduce a `PositivePrice` newtype in `common::fp` that wraps `i128` with a constructor panic on `<= 0`. Adopt at the `ReflectorPriceData → Wad` boundary.
- **C (additional)**: replace `sum += pd.price` with `sum = sum.checked_add(pd.price).unwrap_or_else(|| panic!(...))` to document the invariant.

**Status**: ❌ **REFUTED in loop 23.** The check EXISTS at `controller/src/oracle/mod.rs:41-44`: after `find_price_feed` computes the final price, `if price <= 0 { panic_with_error!(OracleError::InvalidPrice) }` fires BEFORE the price leaves `token_price` and enters any HF/LTV math. My original review of the individual helpers (cex_spot_price etc.) missed this composition-level guard. `OracleError::InvalidPrice = 217` (`common/src/errors.rs:103`) is the exact error I proposed adding. It's already there.

Remaining hardening opportunities (not vulnerabilities):
- TWAP sum accumulator uses `sum += pd.price` without `checked_add`. At realistic price magnitudes (10^18 WAD scale) × 60 samples max, sum < 10^20, well below i128 max 10^38. Not exploitable.
- Per-sample sign check would short-circuit BEFORE TWAP aggregation, avoiding the case where multiple negative samples aggregate into a positive-looking average that passes the final composer check. Still defense-in-depth; the final `price <= 0` check catches the composed result either way.

**Reclassification**: N-11 downgraded from Low to **refuted**. No finding.

---

## N-12 — [Low / Hardening] `compound_interest` Taylor approximation degrades at high `x` and can overflow i128, permanently bricking the pool

**Repo / files**:
- `common/src/rates.rs:67-105` `compound_interest(env, rate, delta_ms) -> Ray`.
- Formula: `x = rate.raw × delta_ms` (Ray-scale), then `e^x ≈ 1 + x + x²/2 + x³/6 + x⁴/24 + x⁵/120 + x⁶/720 + x⁷/5040 + x⁸/40320`.
- `rate` is per-millisecond; `calculate_borrow_rate` at `rates.rs:41` divides annual rate by `MILLISECONDS_PER_YEAR = 31_556_926_000` before returning.
- `x = (annual_rate / MS_PER_YEAR) × delta_ms`; for a market idle T years at APR R, `x ≈ T × R`.
- `compound_interest` is called from `pool/src/interest.rs:26` (`global_sync`), which runs on every pool mutation.

**Taylor series degradation**:
- Code comment (`rates.rs:83-87`): "error bound drops from ~1.66% at x=2 (5 terms) to < 0.01% at x=2 (8 terms)."
- At `x = 5` (1 year × 500% APR OR 5 years × 100% APR): 8-term Taylor = 138.31, actual `e^5 = 148.41`. **Relative error: 6.8% under-estimate.**
- At `x = 10`: Taylor = ~14,932; actual = 22,026. **Relative error: 32%.**
- Direction of error: underestimates true exponential. Borrowers under-pay interest; suppliers under-earn. NOT a drain, but accounting fidelity degrades during long-idle windows.

**i128 overflow bricking**:
- `x_pow8 = x.mul(env, x)^8` where `Ray::mul` keeps results in Ray-scale (×/÷ by RAY each step). `x^8.raw = (x.raw)^8 / RAY^7`.
- For `x = 25` (Ray), `x.raw = 25 × 10^27`. `x^8.raw = 25^8 × 10^(8×27) / 10^(7×27) = 25^8 × 10^27 = 1.5 × 10^38`. **Within i128 max (~1.7 × 10^38) but close.**
- For `x = 30`: `x^8.raw = 30^8 × 10^27 = 6.56 × 10^38`. **Exceeds i128 max → `mul_div_half_up` → `to_i128()` panics with `MathOverflow`.**
- Reachable conditions:
  - 6+ years idle at 500% APR (extreme but not impossible in a paused market).
  - Governance misconfig: `max_borrow_rate = 100 * RAY` (10,000% APR) × 1 year → x = 100 → overflow instantly on first mutation.

**Failure path**:
1. Market goes idle (no mutations) for a long period (years).
2. `delta_ms` grows unboundedly.
3. First post-idle mutation calls `global_sync` → `calculate_borrow_rate` + `compound_interest(rate, delta_ms)`.
4. `compound_interest` evaluates `x_pow8` via Ray::mul chain; overflow triggers `MathOverflow` panic.
5. Transaction reverts.
6. Every subsequent mutation ALSO panics at the same step — market is permanently bricked.
7. No on-chain recovery path: the panic occurs BEFORE any state update, so the market is in a state that cannot accept any call.

**Pre-panic degradation path (complementary concern)**:
- For `x ∈ [5, 25]` (high but not overflow-triggering): Taylor error climbs from 7% to >90%. Borrow index under-grows; accrued supplier rewards are understated; protocol under-collects reserve-factor revenue; user balances drift from economic reality.
- Direction of error favors borrowers over suppliers. Could be monetized by borrowing into a market known to be long-idle, waiting for the under-accrual, then repaying the artificially low debt — though in practice this requires multi-year idleness and coordination.

**Broken invariants**:
- "All reachable `(rate, delta_ms)` inputs produce correct compound-interest output within a bounded error." — currently, the error bound holds only for `x ≤ 2`.
- "No valid protocol state can render a pool permanently unusable." — overflow panic bricks the pool.

**Severity rationale**: **Low / Hardening**.
- Requires either (a) multi-year market idleness (operator neglect) or (b) governance misconfig of `max_borrow_rate`. Neither is an attack surface in the normal threat model.
- Bricking is recoverable only via `upgrade` (replace WASM with a version that handles long deltas).
- Accounting fidelity degradation is economic but not theft.
- Analogous concerns exist in Aave (which uses compact exponential approximations) and Compound (which uses linear interest). Both have learned to bound their compound-over-delta.

**Fix directions**:
- **A (recommended)**: cap `delta_ms` in `global_sync` (`pool/src/interest.rs:18`) at a maximum tractable value (e.g., 365 days × MS_PER_DAY). If `delta_ms` exceeds the cap, apply compound interest over the capped interval, advance `last_timestamp` by that cap, and loop until `current_timestamp` is reached. This bounds `x` per call, keeps Taylor accurate, and prevents overflow.
- **B (simpler)**: cap `delta_ms` inline in `compound_interest` and require callers to drive sync in multiple ticks. Shifts the loop up one level.
- **C (conservative)**: replace 8-term Taylor with a rational approximation (Padé, or direct binary exponentiation via `e^x = e^(x/2)^2`). Extends accurate range; same overflow concern requires the cap too.

**Status**: ✅ verified by:
1. Algebra on Ray-scale multiplication (`Ray::mul` keeps RAY scale → `x^8.raw = x.raw^8 / RAY^7`).
2. Numerical check: Taylor 8-term underestimates `e^5` by 6.8%; `e^10` by 32%; `e^25` is the overflow threshold.
3. Reviewed `mul_div_half_up` at `fp_core.rs:13-20` — uses I256 internally but `to_i128()` panics on overflow.

---

## N-13 — [Informational] `calculate_health_factor` can overflow i128 for high-decimal borrow tokens at extreme collateral/debt ratios

**Repo / files**:
- `controller/src/helpers/mod.rs:66-117` `calculate_health_factor`.
- Formula at line 116: `weighted_collateral_total.div(env, total_borrow).raw()` — uses `Wad::div` which calls `mul_div_half_up(w, WAD, tb)`. `mul_div_half_up` → `to_i128()` panics on overflow (`fp_core.rs:13-20, 91-93`).
- Used by `process_withdraw` at `controller/src/positions/withdraw.rs:43-52` (post-batch HF check) and `process_liquidation` at `liquidation.rs:156` (validate liquidation HF).

**Overflow condition**:
- `HF.raw = weighted_collateral.raw × WAD / total_borrow.raw`.
- Fits i128 (max ~1.7 × 10^38) only when `weighted_collateral × WAD / total_borrow ≤ 1.7 × 10^38`.
- For `total_borrow.raw = 1` (minimum non-zero WAD raw = $10⁻¹⁸), overflow at `weighted_collateral > ~1.7 × 10^20 WAD = ~$170 USD`.

**Asset-decimal translation**:
- 7-decimal token (Stellar SACs, XLM): 1 asset-decimal unit × $1 = $10⁻⁷ = 10¹¹ WAD raw. Well above overflow threshold. **Not reachable.**
- 18-decimal token (wrapped ETH, wBTC-wrapped): 1 asset-decimal unit × $1 = $10⁻¹⁸ = 1 WAD raw. **Overflow threshold reached at $170 USD collateral.**
- 24-decimal token (some wrapped derivatives): even lower dust threshold.

**Failure path** (requires 18-decimal or higher token listed as borrow asset):
1. Operator lists a high-decimal wrapped asset.
2. User supplies $1000 collateral (weighted ~$700-900 at typical LT).
3. User borrows 1 wei of the high-decimal asset (smallest borrow amount). Debt in WAD: ~1 raw.
4. User calls `process_withdraw` for any amount. Post-withdraw HF check runs `calculate_health_factor`.
5. Division overflows → `MathOverflow` panic in `to_i128()` at fp_core.rs:93.
6. Withdraw reverts. User's funds are LOCKED because HF computation can never succeed while the 1-wei debt position exists.
7. Repay path is unblocked (no HF check on repay). User can repay the 1 wei; once debt is zero, `borrow_positions.is_empty()` short-circuits HF to `i128::MAX` (line 72-74) and user can withdraw.

**Severity rationale**: **Informational**.
- Current production configs (`configs/testnet_markets.json`) use 7-decimal tokens only — not reachable today.
- Requires operator to list an 18+ decimal asset AND user to hit the dust-debt edge.
- Recovery path exists (repay the dust, then withdraw).
- No drain path; pure fund-locking footgun.

**Fix directions**:
- **A (most restrictive)**: reject `asset_decimals > 18` in `validate_market_creation` / `configure_market_oracle`. Stellar's SAC is 7-decimal; non-SAC SEP-41 tokens are effectively operator-managed. Explicit cap avoids the overflow class.
- **B (defensive)**: before division at `helpers/mod.rs:116`, check whether the result would overflow. If so, return `i128::MAX` (treat as "infinitely healthy" — dust debt with large collateral IS effectively infinite HF). Matches the semantic of the `borrow_positions.is_empty()` short-circuit.
- **C (doc-only)**: document the high-decimal constraint in `DEPLOYMENT.md` / `CONFIG_INVARIANTS.md` runbook — never allowlist assets with decimals > 7 (or some bound).

**Recommended**: **B** — localized fix with the right semantic (infinite HF for infinitesimal-debt, large-collateral positions). Codifies the "HF = infinity when debt → 0" invariant that section 9 of INVARIANTS.md already claims.

**Status**: ✅ verified by division-overflow arithmetic. Not reachable in current operator configs; filed as informational latent risk for future asset onboarding.

---





