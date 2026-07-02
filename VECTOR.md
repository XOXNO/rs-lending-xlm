# Attack Vector & Edge-Case Log

Adversarial review of `contracts/pool/`, `contracts/controller/`, `contracts/governance/`,
and `contracts/defindex-strategy/`. Each entry: assumption → analysis → verdict → defense.
Valid findings are backed by a POC unit test before the answer is written.

Legend: **VALID** (real, reproducible) · **NOT VALID** (defended/by-design) · severity where applicable.

---

## Executive summary (63 iterations · 2026-06-22)

**Bottom line: no fund-theft path found.** The value-moving core is conservatively rounded
(floor-collateral / ceil-debt), fail-closed (oracle), and comprehensively auth-gated. All real
findings are LOW griefing/asymmetry. One potential-HIGH integration question was raised and then
**resolved benign** against the authoritative DeFindex spec.

**4 VALID findings — all LOW, POC-backed, green in full suites + clippy-clean. 1.2 is now FIXED:**

| # | Finding | Verdict | POC / regression |
|---|---------|---------|-----|
| 1.2 | `harvest` unauth → spoofable `from` in event | **FIXED** (was VALID — LOW) | `harvest` now calls `from.require_auth()` (lib.rs:213); regression `defindex-strategy/tests/strategy.rs::harvest_requires_from_auth` |
| 5.1 | Permissionless dust account-creation spam (state-bloat / keeper-scan DoS) | **VALID — LOW** (deep pass) | `controller/supply.rs::poc_single_actor_spams_unbounded_dust_accounts` |
| 12.1 | `supply` lacks owner-match → non-owner deposit / slot griefing | **VALID — LOW** | `controller/supply.rs::poc_non_owner_can_supply_into_victims_account` |
| 14.1 | DeFindex strategy NAV inflation via direct `supply` into its controller account | **VALID — LOW** (cross-contract caveat, deep pass) | `defindex-strategy/tests/strategy.rs::poc_third_party_inflates_strategy_balance_via_controller_supply` |

**Status:** 1.2 **FIXED**; **3 VALID remaining** (5.1, 12.1, 14.1), all LOW. 12.1+14.1 share one root
fix (owner-match on `supply`, action item #1).

**1 resolved:** 16.1 (strategy returns cumulative balance) — **NOT VALID**, the DeFindex `StrategyTrait`
spec *requires* the post-op balance (iter 18).

**~55 NOT-VALID / by-design / governance-trust** across every surface (iteration refs):
- **Auth/gateway:** controller 100% authenticated (51), pool 100% owner-gated mutators (38), strategy no-admin (33), deploy one-time (21).
- **Governance/timelock:** bypass (4), cancel=CANCELLER (29), role-grant PROPOSER-gated+timelocked (45), delay one-way ratchet (46), pause & market-deactivation freeze entry not exits (31/32).
- **Cross-entrypoint invariants (direct == leverage):** spoke spoke caps (20.1), hub cap (35), min-borrow floor incl. withdraw (47), position limits (48), capability flags (53), market-active (54).
- **Oracle:** manipulation/divergence fail-closed (8), invalid/zero price (36), cache-policy poisoning (44), read-reentrancy [DiD note] (59), RedStone pull-sourcing (60), Reflector pull + thin-TWAP guard (63).
- **Value math:** accrual overflow (9), valuation overflow I256 (49), directional rounding (10/13), index inflation (2), spoke per-asset params (34).
- **Flash/strategy/migration:** reentrancy (5.2), fee derivation (24), repayment verification (37), fee accounting (57), callback-target griefing (62), aggregator extraction + no-lingering-allowance (6.1/52), Blend migration (7), strategy NAV/return-semantics (14/16), same-asset swap (55).
- **Liquidation:** threshold-grief (3), same-asset netting (23), repeated-call over-seize + self-liq block (30), collateral cherry-pick (40).
- **Funds/accounting:** flash-fee in (57), claim_revenue out (58), accumulator transfer-only (61).
- **Misc:** caps exhaustion (25), liquidity squeeze (41), dust-debt floor (42), bulk dedup (15), token-trust (11), bad-debt socialization (17), storage key-collision (50).

**Action items for the team:**
1. **Highest-leverage code fix** — add `require_account_owner_match` to controller `supply` for an
   existing `account_id`: closes **both 12.1 and 14.1**.
2. **Defense-in-depth (LOW, 59.1)** — wrap oracle reads in the same `FlashLoanOngoing` reentrancy
   guard as the aggregator (so a compromised governance-set oracle can't reenter), or document the
   no-reentrant-oracle trust assumption.
3. **Documented governance-trust boundaries (no code bug)** — list only SAC / no-fee-on-transfer /
   no-clawback-freeze assets (11.1); oracle divergence intentionally pauses liquidations (8.1);
   spoke category deprecation abruptly reprices positions to base and can trigger liquidations
   (56.1 — announce + wind down ahead of the flip).
4. **Test hygiene (LOW) — FIXED.** The stale `test_permissionless_revenue_endpoints` (iter 27;
   pre-existing failure, not a vuln) now seeds a supplier and funds the caller before asserting
   `add_rewards` succeeds — reflecting the correct `NoSuppliersToReward` + caller-funds-the-reward
   behavior. Pool harness suite is now 69/69.

**Verification base:** controller 329, pool 113 lib + 69 harness (incl. the now-fixed
`test_permissionless_revenue_endpoints`), governance 69 lib + 57 harness, defindex-strategy 15 — all
green; `clippy` clean on the four contracts (iter 39). All 4 VALID POCs re-verified green
(iter 17/22 and again post-fix). Substantive coverage reached saturation ~iter 18; iters 20–63
systematically verified each subsystem (caps, governance, oracle, fund-flow, external-call targets,
numerical, storage) and surfaced the 59.1 defense-in-depth item.

---

## Iteration 1 — 2026-06-22

### 1.1 — Stale vault→account_id reuse confusion in defindex-strategy — **NOT VALID**

**Assumption.** `defindex-strategy` stores `VaultAccount(vault) = account_id` and, on every
read, trusts that id if `controller.account_exists(&stored)` returns `true`
(`reconcile_vault_account`, lib.rs:332). If the controller **recycled** a closed account id,
a stale mapping would silently point a vault at a *different* user's live account, leaking or
misattributing collateral across vaults.

**Analysis.** Account ids are allocated only by `storage::increment_account_nonce`
(controller `helpers/account.rs:33` → `storage/instance.rs:177`). It reads the current
`AccountNonce`, adds 1, persists it, and returns the new value; it panics on `u64` overflow and
**never reuses** a freed id. `remove_account` deletes the entry but does not decrement the nonce.
So a closed id is permanently retired — `account_exists(stored)` can never resurrect as a
*different* account.

**Verdict / defense.** Defended by the strictly-monotonic `AccountNonce` counter; ids are never
recycled, so `account_exists`-gated reconciliation cannot cross-link vaults. Covered indirectly by
`test_supply_clears_stale_vault_mapping_after_full_withdraw` and
`test_two_vaults_have_isolated_lending_accounts` (defindex `tests/strategy.rs`). No POC (invalid).

### 1.2 — `harvest` is unauthenticated; `from` attribution is spoofable — **VALID (LOW)**

**Assumption.** `Strategy::harvest(env, from, data)` (defindex lib.rs:212) emits a
`strategy/harvest` event carrying `from`, but does **not** call `from.require_auth()`. Any
external actor can therefore emit a harvest event attributed to an arbitrary vault address they do
not control.

**Analysis.** Confirmed: `harvest` has no `require_auth` on `from` (contrast `deposit` lib.rs:190
and `withdraw` lib.rs:236, which both gate `from`). `harvest` is otherwise read-only — `amount` is
hardcoded `0` and `price_per_share` is the *global* market supply index (independent of the
vault). So there is **no fund or on-chain-state impact**. The exposure is off-chain: an integrator
/ indexer that consumes `strategy/harvest` and trusts `from` as "which vault harvested" can be
poisoned with spurious entries, and the permissionless entrypoint allows cheap event-log spam.

**POC.** `contracts/defindex-strategy/tests/strategy.rs::poc_harvest_is_unauthenticated_and_from_is_spoofable`.
After seeding a real position, the test calls `env.set_auths(&[])` (enforce real auth, no mocks),
then shows `harvest(attacker_chosen_from)` **succeeds** and emits a genuine pps, while
`deposit(attacker_chosen_from)` is **rejected** under the same empty-auth context — isolating the
missing auth check as the cause. Passing (`cargo test -p defindex-strategy --test strategy`).

**Verdict / defense.** Permissionless harvest is itself conventional (keepers trigger it), but the
*spoofable `from` field* is undefended. Severity LOW (informational/integrator-trust + event-spam
DDoS; no value at risk). Suggested hardening: `from.require_auth()` in `harvest`, or document
`from` as untrusted and have indexers ignore it. No existing guard prevents the spoof.

> **RESOLVED (fixed).** `harvest` now calls `from.require_auth()` (`defindex-strategy/src/lib.rs:213`),
> matching `deposit` (`:190`) and `withdraw` (`:237`) — a caller can no longer emit a `strategy/harvest`
> event attributed to a `from` they don't control, and the entrypoint is no longer permissionlessly
> spammable with arbitrary `from`. The POC was converted to a regression test
> (`harvest_requires_from_auth`) asserting the unauthenticated harvest is now **rejected** under
> enforced auth. Strategy suite 15/15 green, clippy clean. (The keeper still triggers harvest by
> signing as the vault `from`, the conventional pattern.)

---

## Iteration 2 — 2026-06-22

### 2.1 — First-depositor / donation (ERC4626) inflation attack via `add_rewards` — **NOT VALID**

**Assumption.** `add_rewards(caller, rewards)` is **permissionless** (`router.rs:108`, only
`caller.require_auth()` + `require_not_flash_loaning`; the caller funds the reward). It bumps the
market's `supply_index`. Classic ERC4626 play: attacker becomes the sole supplier with a tiny
balance, donates a large reward to inflate the per-share index, then a victim's later deposit
rounds down to **0 scaled shares** — the victim's tokens are captured by the attacker's share.

**Analysis.** The attack does not translate from balance-based vaults to this protocol's
RAY-scaled **multiplicative index** accounting:

- `update_supply_index` (`common/rates.rs:108`) computes
  `new_index = old_index · (1 + rewards/total_supplied_value)`, where
  `total_supplied_value = supplied · old_index`. Inflating the index by a factor `F` therefore
  costs the attacker `(F−1) · total_pool_value` in donated rewards — and those rewards flow back to
  the attacker pro-rata (they are the sole supplier), so the donation is recovered, not a lever.
- To round a realistic victim deposit to 0 scaled, the index must exceed the deposit's RAY
  magnitude. `calculate_scaled_supply` (`pool/cache.rs:140`) is `amount_ray.div(supply_index)` =
  **half-up** `mul_div_half_up(amount_ray, RAY, supply_index)`. For a 1-USDC deposit
  (`amount_ray ≈ 1e27`), zeroing needs `supply_index > ~2e54`.
- `old_index.mul(factor)` is checked half-up (`Ray::mul` → `mul_div_half_up`, `fp.rs:44`), which
  raises `GenericError::MathOverflow` once the i128 result is exceeded (max ≈ 1.7e38). The index
  **cannot reach ~1e54** — any attempt to inflate that far reverts long before. Even dust victim
  deposits (`amount_ray ≈ 1e20`) need `index > ~1e47`, still unreachable.
- Half-up (not floor) rounding additionally favors the depositor at the boundary.

**Verdict / defense.** Defended by RAY-scaled multiplicative index accounting bounded by
checked/`MathOverflow`-guarded `Ray::mul`/`div` (1e27 precision) — the index is physically
un-inflatable to the ~1e27× required to zero out a deposit, and donated rewards are recovered
pro-rata so there is no profit lever. The balance/share inflation attack is structurally absent.
No POC (invalid).

---

## Iteration 3 — 2026-06-22

### 3.1 — Permissionless `update_account_threshold` to grief / force-liquidate other users — **NOT VALID**

**Assumption.** `update_account_threshold(caller, has_risks, account_ids)` (`router.rs:116`) is
permissionless — `caller.require_auth()` authenticates only the *tx signer*, while `account_ids`
is an arbitrary list of **other users' accounts**, and `has_risks` is attacker-chosen. The stored
per-position risk snapshot is what liquidation math actually reads (e.g. `liquidation_math.rs:544`
reads `position.liquidation_bonus`). So the idea: an attacker re-syncs a victim's stored
`liquidation_threshold`/`ltv`/`bonus` to push the victim below HF=1 (enabling liquidation) or to
inflate the bonus they will seize, without owning the account.

**Analysis.** The handler (`sync_account_thresholds`, `router.rs:324`) is a bounded keeper utility:

- It can only write the **current governance config** values (`asset_config.liquidation_threshold`
  / `loan_to_value` / `liquidation_bonus`), never attacker-supplied numbers — it merely propagates
  what governance already set, which the victim is already subject to.
- The only HF-lowering field is `liquidation_threshold`, and it is updated **only** in the
  `has_risks == true` branch, which is followed by a hard gate:
  `assert hf >= THRESHOLD_UPDATE_MIN_HF_RAW` = **1.05 WAD** (`risk_params.rs:17`). If applying the
  new threshold would drop the account below 1.05, the whole call reverts. An attacker therefore
  cannot use it to push a victim toward (let alone below) the HF=1 liquidation line — there is a
  5% buffer.
- The ungated `has_risks == false` branch touches only `loan_to_value_bps` (gates *new borrows*,
  not HF) and `liquidation_bonus_bps`. Raising the stored bonus only matters if the victim is
  *already* liquidatable (this branch cannot make them so), and seizure is still bounded by
  `max_bonus_for_threshold` (`liquidation_math.rs:503`) so total seizure ≤ collateral.

Residual (informational, by-design): the false branch lets a prospective liquidator opportunistically
sync a victim's stored bonus up to the *current* governance value before liquidating — but that is
the legitimate current param, capped, and applies no HF change. Not an exploit.

**Verdict / defense.** Defended by (a) the HF≥1.05 floor gating the only HF-lowering (threshold)
field, (b) the risk-decreasing branch touching only non-HF fields (LTV/bonus, the latter capped),
and (c) the handler writing only current governance config, never attacker-chosen values.
Related: liquidation over-seizure is independently bounded by HF-targeted partial close
(`estimate_liquidation_amount`), `max_bonus_for_threshold`, and per-asset `min(actual_ray)`
(`liquidation_math.rs:274`). No POC (invalid).

---

## Iteration 4 — 2026-06-22

### 4.1 — Timelock / governance bypass by an external actor — **NOT VALID**

**Assumption.** Governance gates privileged controller changes behind a 48h timelock. An external
actor might (a) call a controller admin setter directly, skipping the timelock; (b) `execute` a
forged operation with args differing from what was scheduled; (c) `execute` before the delay
elapses; (d) schedule an operation without the PROPOSER role; (e) `pause` the protocol to DoS it;
or (f) seize ownership.

**Analysis.** Every control path is gated:

- **Direct setter bypass — blocked.** All controller admin setters are `#[only_owner]`
  (`governance/config.rs`, `governance/access.rs`; owner = the governance contract). The only way
  to satisfy `owner.require_auth()` is for the governance contract itself to be the invoker, which
  happens only inside `execute_operation` (post-timelock) or a governance-self inline op. A direct
  call by any other address fails the owner gate.
- **Arg/hash forgery — blocked.** `execute` (`timelock.rs:90`) reconstructs the `Operation` from
  the passed `(target, function, args, predecessor, salt)` and hands it to OZ `execute_operation`,
  which looks the op up by `hash_operation`. Args differing from the scheduled set produce a
  different hash → `Unset`/not-ready → revert.
- **Early execution — blocked.** OZ enforces `Ready` (delay elapsed); `execute_before_delay_reverts`
  (#4002) and the grace-period expiry (`require_operation_not_expired`, #40) bracket the window.
- **Unauthorized propose — blocked.** Typed `propose_*` (forward.rs) require the PROPOSER role;
  the generic OZ `schedule` is not exposed. `non_proposer_cannot_propose` covers this.
- **Pause DoS — blocked.** `pause`/`unpause` are `#[only_owner]` (`access.rs:121/127`); no
  permissionless guardian. `accept_ownership` is open but OZ requires the *pending* owner (named by
  the current owner via `transfer_ownership`) to authorize, so ownership cannot be stolen.
- **Open execution is by-design, not a hole.** `execute(executor=None, …)` lets anyone trigger an
  already-scheduled, already-waited operation — the EXECUTOR role is optional. This is safe: the op
  was validated at propose time and survived the full delay; the caller cannot change what runs.
  `execute` also rejects `target == self` (`timelock.rs:102`) to prevent self-reentry.

**Verdict / defense.** Defended in depth: `#[only_owner]`(=governance) setters, PROPOSER-gated
proposers, OZ hash-matched + Ready-gated + grace-bounded execution, and only_owner pause/upgrade.
External actors cannot bypass the timelock, forge operations, halt the protocol, or seize
ownership. (Strategy/flash entrypoints are independently `caller.require_auth()` +
`when_not_paused` + `require_wasm_receiver` / `is_blend_pool_approved` allowlist.) Documented
residual from the prior split audit: an oracle-config TOCTOU re-check at execute time (LOW, already
tracked). No POC (invalid).

---

## Iteration 5 — 2026-06-22

### 5.1 — Permissionless dust account-creation spam → state-bloat / keeper-scan DoS — **VALID (LOW–MED)**

**Assumption.** `supply(caller, account_id, spoke_id, assets)` creates a brand-new controller account
whenever `account_id == 0` (`supply.rs:90` → `create_account` → monotonic `AccountNonce`). If there
is no minimum-deposit floor, one external address can call `supply(0, …, [(asset, 1)])` in a loop to
mint unbounded accounts with 1-raw-unit deposits, each writing `AccountMeta` + `SupplyPositions`
persistent entries.

**Analysis.** Confirmed there is **no dust/min-value floor** on supply: `validate_deposit`
(`supply.rs:123`) enforces only `require_positive_amount` (> 0), `can_supply`, market-active,
spoke, and per-account position limits. Position limits cap positions *within* an account, not the
*number of accounts*. So a single actor can create accounts without bound, each persisting in
controller storage. Impact:

- **State bloat** of the controller's persistent storage (2 entries/account).
- **Keeper-scan DoS**: the keeper scans a bounded set of accounts (config `max_accounts_scan`,
  ~50k, "loud warn on overflow"). Spamming past that window pushes *legitimate* accounts out of
  scan coverage, so their TTLs are not bumped → eventual archival. Once archived, a keeper
  `ExtendFootprintTtl` cannot revive them (restore required) — a real availability hit for
  unattended users.
- **Indexer load** from position events on fake accounts.

**POC.** `tests/test-harness/tests/controller/supply.rs::poc_single_actor_spams_unbounded_dust_accounts`.
One attacker address calls `supply(&attacker, 0, 0, [(USDC, 1)])` 64 times; every call returns a
**fresh, strictly-increasing** account id, each `account_exists`, with a 1-raw-unit deposit accepted
(no dust-floor revert). Passing (`cargo test -p test-harness --test controller`).

**Verdict / defense.** Real but **bounded** griefing, not theft. Severity LOW–MED. Mitigating
factors (not eliminating it): Soroban charges the attacker per-entry storage rent (cost scales with
the spam); users can self-rescue live accounts via permissionless `renew_account`; the keeper scan
cap localizes (but does not prevent) the impact. **No on-chain dust floor currently prevents this.**
Suggested hardening: a minimum first-deposit USD value (mirroring `min_borrow_collateral`) or a
per-caller open-account cap, so account creation costs more than dust.

### 5.2 — Flash-loan callback reentrancy into position paths — **NOT VALID**

**Assumption.** `flash_loan` invokes the receiver's `execute_flash_loan` mid-call; a malicious
receiver could re-enter `supply`/`borrow`/`withdraw`/`repay`/`liquidate`/strategy to manipulate
state while the loan is outstanding.

**Analysis.** The controller sets a global `FlashLoanOngoing` flag around the pool call
(`flash_loan.rs:59/63`), and **every** user-callable mutating entrypoint asserts
`require_not_flash_loaning` first: `supply`, `borrow`, `withdraw`, `repay`, `liquidate`,
`clean_bad_debt`, `flash_loan` (self), `multiply`, `swap_debt`, `swap_collateral`,
`repay_debt_with_collateral`, `migrate_from_blend`, `add_rewards`, `update_account_threshold`,
`claim_revenue`, `update_indexes` (verified call sites). Admin forwarders are `#[only_owner]`
(unreachable by the receiver). Re-entry during the callback reverts with `FlashLoanOngoing`. The
pool independently commits state before the invoke (CEI) and brackets repayment with balance checks.

**Verdict / defense.** Defended by comprehensive `require_not_flash_loaning` coverage across all
mutating entrypoints + pool-side CEI and balance-bracket repayment. No POC (invalid).

---

## Iteration 6 — 2026-06-22

### 6.1 — Aggregator-swap value extraction in `multiply`/`swap_*` (opaque route bytes) — **NOT VALID**

**Assumption.** Strategy flows pass attacker-supplied **opaque `Bytes`** to a swap aggregator and
then credit "received collateral." If the controller trusted the router's *reported* output, or
mis-measured it, an attacker could credit collateral they never received (mint collateral from air)
or under-spend the borrowed debt and pocket it while keeping a solvent-looking position.

**Analysis.** `swap.rs` explicitly does not trust router reports:

- The router is a **single governance-set address** (`storage::get_aggregator`), not attacker-chosen.
  The attacker controls only the route bytes; the trusted aggregator executes them.
- Output is measured by **balance delta** (`verify_router_output`, snapshot before / balance after),
  with `received > 0` required; the router's return value is discarded.
- Input spend is bounded: `verify_router_input_spend` asserts `actual_in_spent <= amount_in`, and
  `refund_router_underspend` returns leftover to the caller (no trapped funds; under-spend just
  reduces leverage).
- A `FlashLoanOngoing` guard wraps the router call, so the route cannot re-enter controller
  position paths.
- Bad routes / sandwiching only reduce output → `strategy_finalize`'s post-pool HF gate
  (`require_post_pool_risk_gates`, HF ≥ 1) reverts, so the *caller* bears slippage; the protocol
  never books unbacked collateral.

**Verdict / defense.** Defended by balance-delta accounting + trusted (governance-set) aggregator +
input-spend cap/refund + reentrancy guard + post-pool HF gate. The opaque bytes only choose a
route; they cannot fabricate collateral. No POC (invalid).

### 6.2 — Spoke spoke-cap counter drift (DoS via over-count / bypass via under-count) — **NOT VALID**

**Assumption.** Spoke spoke caps are enforced against a per-(category, asset) scaled usage counter
(`helpers/spoke_caps.rs`) incremented on supply/borrow and decremented on withdraw/repay. If any
value-moving path updated the counter asymmetrically, it would drift: over-count → premature
`SpokeSupplyCapReached`/`SpokeBorrowCapReached` (DoS for honest users), or under-count → cap bypass.

**Analysis.** Increment/decrement are symmetric across every path:

- Supply (`supply.rs:192`), borrow (`borrow.rs:144`), withdraw (`withdraw.rs:211`), repay
  (`repay.rs:134`), and **both** liquidation legs — normal seize/repay via the shared
  `settle_withdraw_entries`/`settle_repay_actions`, and bad-debt cleanup
  (`liquidation.rs:320-325`) — all route through `apply_*_after_pool`, with
  `cache.persist_spoke_usage()` committing.
- The increment delta (`new_scaled − old_scaled`, settle_deposit) and the decrement delta
  (`old_scaled − new_scaled`, `finish_withdrawal:212`) are derived from the *same* position-scaled
  transition + the pool's returned mutation, so a supply-then-withdraw cycle nets to zero (no
  rounding drift).
- The account's `spoke_id` is **immutable**: `resolve_supply_account` (`supply.rs`)
  panics `SpokeMismatch` if a non-zero category differs from the account's, so usage is always
  updated under one category — no cross-category increment/decrement mismatch.
- `set_usage` removes the entry only when *both* supplied and borrowed scaled reach 0, so a live
  position always keeps its entry for the matching decrement; `checked_sub` would panic (revert) on
  any underflow rather than silently wrapping.

**Verdict / defense.** Defended by symmetric `apply_*_after_pool` accounting on all paths (including
both liquidation legs), matched supply/withdraw scaled deltas, an immutable per-account category,
and underflow-checked decrements. No drift channel for an external actor. No POC (invalid).

---

## Iteration 7 — 2026-06-22

### 7.1 — `migrate_from_blend`: arbitrary-pool call, victim-position theft, or zero-fee borrow extraction — **NOT VALID**

**Assumption.** The Blend one-click migration (`strategies/migrate_blend.rs`) does a **zero-fee**
`create_strategy` borrow, calls an external Blend pool's `submit`, and re-supplies the proceeds. An
external actor might: (a) pass a contract they control as `blend_pool` to get an arbitrary external
call / fee-free flash loan; (b) migrate a *victim's* Blend position; (c) over-borrow at zero fee via
an inflated `debt_caps` `max` and pocket the excess; or (d) inflate credited collateral by donating
tokens to the controller mid-migration.

**Analysis.** Every leg is constrained:

- **Pool allowlist (closes a prior flagged gap).** `process_migrate_blend` asserts
  `is_blend_pool_approved(blend_pool)` (`migrate_blend.rs:117`); `approve_blend_pool`/`revoke` are
  `#[only_owner]` (`governance/config.rs:165-173`), i.e. governance + 48h timelock. An attacker
  cannot substitute a contract they control → no arbitrary external call / fee-free flash loan.
- **Caller-only.** `caller.require_auth()` and Blend `submit(from = caller)` (`guarded_submit`):
  Blend itself calls `from.require_auth()`, so only the caller's *own* Blend position is swept. A
  victim's position cannot be migrated (the victim never signs the nested submit).
- **No zero-fee extraction.** Each debt asset borrows `max`, Blend pulls `max` (authorized for
  exactly `max` via `authorize_repay_pulls`, emitted immediately before the submit — no intervening
  cross-call, mirroring the swap pre-auth pattern), repays the real debt, and refunds the excess.
  `reconcile_debt_refunds` measures the refund by **balance delta** and repays it against the new
  debt, so final controller debt = exactly what cleared Blend. The over-borrow is conserved, not
  pocketed.
- **Donation-proof crediting.** `deposit_withdrawn` credits each asset by `balance_delta` against a
  snapshot taken *after* phase 1 and *before* the withdraw submit, so any pre-existing/donated
  balance is excluded. Looped (same-asset collateral+debt) positions use two phase-scoped submits
  with separate snapshots so repay-refund and collateral-withdraw deltas never alias.
- **Guards.** Reentrancy guard around each submit; final `strategy_finalize` HF≥1 gate.

**Verdict / defense.** Defended by the governance-gated Blend-pool allowlist, Blend's own
`from`-auth (caller-only), balance-delta refund reconciliation + collateral crediting, two-phase
same-asset isolation, reentrancy guard, and the end-state HF gate. The previously-flagged
allowlist/fee-bypass concern is **closed** by the `is_blend_pool_approved` assert. Live-validated on
testnet (conservation verified). No POC (invalid).

---

## Iteration 8 — 2026-06-22

### 8.1 — Oracle price manipulation / divergence DoS by an external actor — **NOT VALID**

**Assumption.** An attacker manipulates an oracle source to (a) inflate their collateral price and
over-borrow, (b) deflate a victim's collateral to force liquidation, (c) read a manipulable spot
price for a value-extracting flow, or (d) push primary vs anchor apart to **block liquidations** of
their own underwater position (fail-closed → bad-debt DoS).

**Analysis.** The oracle is fail-closed and band-bounded across three layers:

- **Policy matrix** (`oracle/policy.rs`): `RiskIncreasing` and `Liquidation` reject *every*
  loosening — `stale_source`, `unsafe_deviation`, `degraded_dual_source`, `sanity_violation` all
  `false`. Only risk-*reducing* flows (`RiskDecreasing`/`Repay`/`View`) are permissive, and per the
  policy's own invariant a manipulated price there can only *reduce* account risk (no extraction).
- **Tolerance blending** (`tolerance.rs:calculate_final_price`): for risk-increasing/liquidation
  (`requires_blended_first_band`), the final price is the **midpoint** of primary and anchor within
  both the first and last bands — so a single manipulated source has only *half* influence, capped
  at the governance-set band width (e.g. ≤5%). Beyond the last band, `allows_unsafe_deviation` is
  false → `panic UnsafePriceNotAllowed`. A lone source cannot bias the price beyond half the band.
- **TWAP, not spot** (`reflector/twap.rs`): the Reflector anchor reads a TWAP (manipulation-resistant).
  `twap_fallback_or_panic` degrades to spot **only** when `allows_degraded_dual_source` (risk-reducing
  policies); for `RiskIncreasing`/`Liquidation` it `panic`s (twap.rs:142). So a manipulable spot price
  is never read by a value-extracting flow. RedStone (primary) is signed off-chain → not on-chain
  manipulable.

For (d): forcing primary↔anchor divergence beyond the last band requires moving a *real* oracle
source past the band — RedStone is off-chain-signed (infeasible), and the Reflector TWAP needs
sustained capital to shift past ~5%. That same capability would let the attacker move the price
directly; it is not a cheap DoS. The resulting liquidation pause during genuine divergence is a
**deliberate fail-closed tradeoff** (owner directive: trap must revert, never single-source a
risk-increasing read), accepting transient bad-debt risk over seizing at an uncertain price.

**Verdict / defense.** Defended by the fail-closed policy matrix (risk-increasing/liquidation reject
all loosenings), midpoint blending bounded by the governance tolerance band, and TWAP-with-no-spot-
fallback for value-extracting flows. External price manipulation is bounded to ≤½ the band and
requires moving a real (off-chain-signed or TWAP) source. Documented residual: intentional
liquidation pause under true oracle divergence (bad-debt tradeoff, not cheaply externally
triggerable). No POC (invalid).

---

## Iteration 9 — 2026-06-22

### 9.1 — Interest-accrual overflow / gas-DoS to brick a market — **NOT VALID**

**Assumption.** Every pool operation calls `global_sync` (`pool/interest.rs:12`) to accrue interest
since the last touch. An external actor might brick a market by (a) forcing the borrow index to
overflow i128 (so the checked-mul panics and every future op reverts), (b) driving utilization into
a rate-curve branch that overflows/divides-by-zero, or (c) leaving a market dormant so the next
op pays an unbounded chunk loop (gas-DoS).

**Analysis.** Accrual is bounded on every axis:

- **Rate cap + bounded chunk.** `calculate_borrow_rate` (`common/rates.rs:13`) caps the annual rate
  at `params.max_borrow_rate` before compounding, and `global_sync` chunks the elapsed time at
  `MAX_COMPOUND_DELTA_MS` (1 year). So the Taylor input `x = rate_per_ms · delta_ms ≤ max_rate`
  (≈2·RAY); `x^2…x^8` stay ≪ i128 max. `compound_interest` additionally promotes `rate · delta_ms`
  to **I256** before narrowing, with a checked `to_i128` (`rates.rs:70-76`).
- **No div-by-zero / curve overflow.** The slope-3 branch divides by `range = ONE − optimal_util`,
  and governance config validation enforces `mid < optimal < max ≤ RAY`, so `range > 0`. Excess is
  bounded because borrows are gated by `require_utilization_below_max` (< max < RAY) and
  `require_reserves`; utilization cannot be pushed to the ~3e11 needed to overflow `excess·slope3`,
  and the supply-index floor (`SUPPLY_INDEX_FLOOR_RAW`) keeps `supplied_original` from collapsing to
  near-zero. Any residual overflow is a **checked panic (revert), not corruption**.
- **No gas-DoS.** Ledger time is not attacker-amplifiable; the chunk loop runs `elapsed / 1yr`
  iterations, i.e. a handful even after months of dormancy. An attacker cannot fast-forward time to
  inflate the loop.
- **Index-overflow ceiling is non-practical.** Reaching `borrow_index ≈ i128::MAX` from RAY needs
  ~13 years of *continuous* max-rate (200%) accrual; real rates track utilization and fluctuate, so
  this is centuries away and not externally accelerable.

**Verdict / defense.** Defended by the capped rate, 1-year compound chunking, I256-promoted
products, governance-validated curve kinks (`range > 0`), utilization/reserve gates bounding excess,
the supply-index floor, and checked math that reverts rather than corrupts. No externally reachable
accrual panic or gas-DoS. No POC (invalid).

---

## Iteration 10 — 2026-06-22

### 10.1 — Rounding-direction value extraction via supply/withdraw/repay round-trips — **NOT VALID**

**Assumption.** Scaled-share conversions round at each step. An attacker who repeatedly
supplies-then-withdraws (or over-repays) a dust amount might accumulate sub-unit rounding in their
favor, draining the pool one ulp per cycle.

**Analysis.** Every conversion rounds **in the protocol's favor**, and the in/out directions oppose:

- **Supply in / withdraw out.** `calculate_scaled_supply` credits shares **half-up**
  (`cache.rs:140`), but `resolve_withdrawal` pays the asset-out via **floor**
  (`unscale_supply_floor`, `cache.rs:158/186`). A supply→full-withdraw round-trip therefore nets
  ≤ 0 for the user: the ≤0.5-ulp scaled gain from half-up is reclaimed by the floor on exit. Worked
  through at index 1.5: supply 1 raw → scaled ≈ 6.667e19; withdraw floor(scaled·index)→asset = 1.
  The sub-unit share surplus (≤0.5 scaled-ray-ulp ≈ <1 ray after ×index) floors to **0 asset units**.
- **Repay.** Full-close uses **ceil** (`unscale_borrow_ceil`, `cache.rs:224`) so a borrower cannot
  underpay and leave indexed dust; the overpayment refund is the exact ceil remainder. Borrow scales
  debt half-up (rounds the borrower's *owed* shares up). Both favor the protocol.
- **Revenue claim.** `burn_claimable_revenue` (`cache.rs:202`) transfers only to the **owner**
  (governance), capped by `min(reserves, treasury_actual)`, burning scaled revenue from both
  `revenue` and `supplied`. Permissionless `claim_revenue` just moves the protocol's own cut to
  governance — no attacker benefit, no supplier-fund impact.

Memory's "full-close half-up quantization" note concerns the *controller* matching the *pool's*
full-close rounding so the internal dust gate doesn't spuriously revert (consistency), not an
extraction channel.

**Verdict / defense.** Defended by directional rounding discipline (half-up shares in, floor asset
out, ceil on debt full-close) that strands sub-unit dust in the pool, not with the user. Round-trips
net ≤ 0 for the caller. No POC (invalid).

### 10.2 — Permissionless `update_indexes` / `claim_revenue` event-spam amplification — **NOT VALID (same class as 5.1)**

**Assumption.** `update_indexes(caller, assets)` and `claim_revenue(caller, assets)` are
permissionless and loop over a caller-supplied asset list with no dedup, emitting a market-state
event per entry — so `[USDC; 1000]` in one tx emits 1000 events (indexer spam amplification).

**Analysis.** Real but bounded and self-funded: the assets must be supported (a small
governance-listed set), the operations are idempotent within a ledger (re-sync to `now` is a no-op
on state; `claim_revenue` to the owner nets nothing extra), and the attacker pays gas for every
entry. No protocol-state or fund impact; the only effect is off-chain indexer load — the same
event-spam class already captured by 5.1, with no on-chain consequence.

**Verdict / defense.** Bounded by per-entry gas cost borne by the attacker, a small supported-asset
set, and ledger-idempotent accrual; no state/fund impact. Hardening (if desired): dedup the asset
list. No POC (subsumed by 5.1).

---

## Iteration 11 — 2026-06-22

### 11.1 — Non-standard token (fee-on-transfer / clawback / freeze) breaks pool accounting — **NOT VALID (external) / governance-trust boundary**

**Assumption.** The pool credits the *requested* transfer amount, not the amount actually received:
`transfer_amount` (`helpers/utils.rs:26`) calls `sac_transfer_call(...)` and simply **returns
`amount`** (no balance-delta measurement), and `supply_one` then credits that amount as scaled
shares + `credit_cash(amount)`. A **fee-on-transfer** token would deliver less than `amount` to the
pool while suppliers are credited the full `amount` → cumulative over-credit → reserve shortfall, so
the last withdrawers cannot be paid (insolvency). Separately, a classic-asset SAC issuer with
`AUTH_CLAWBACK_ENABLED`/`AUTH_REVOCABLE` could **claw back or freeze** the pool's holdings of that
asset — draining reserves or bricking the market.

**Analysis.** The transfer-measurement gap is real and confirmed (matches the known
`transfer_and_measure_received`-name-lies / SAC-only note), but the **entry point is fully
governance-gated**:

- A token can only become a market after `approve_token` (`governance/config.rs:150`, `#[only_owner]`)
  and market creation (`create_liquidity_pool` asserts `is_token_approved`). Both are governance +
  48h timelock. A permissionless external actor **cannot list** a fee-on-transfer / clawback /
  freeze-capable asset.
- For an *already-listed* classic-asset SAC, the **issuer is an external party**: if governance
  lists an asset whose issuer retains clawback/freeze, that issuer can attack the pool's holdings.
  This is inherent issuer-trust, not a contract bug — and it is bounded to that one market.

**Verdict / defense.** Not exploitable by a permissionless external actor: asset onboarding is
`#[only_owner]`+timelock, and the protocol's stated assumption is **SAC-only, no fee-on-transfer**.
This is therefore a **governance asset-selection requirement**, not an open attack surface — but it
is a *latent external-actor risk for non-native assets* (a malicious/upgradeable issuer with
clawback/freeze over a listed asset). Recommended: (a) document the no-fee-on-transfer / no-clawback
/ no-freeze listing criteria explicitly; (b) optionally add a received-amount balance-delta check in
`transfer_amount` (defense-in-depth, mirroring the swap/migration paths which already measure
deltas) so a mis-listed fee token fails safe instead of silently over-crediting. No POC (entry point
is governance-gated; not permissionlessly reachable).

---

## Iteration 12 — 2026-06-22

### 12.1 — `supply` lacks `require_account_owner_match`: non-owner deposits into a victim's account — **VALID (LOW)**

**Assumption.** Every position flow should verify the caller owns the target account. If `supply`
omits that check, a stranger can deposit collateral into a *victim's* existing account, consuming
the victim's bounded supply-position slots.

**Analysis.** Confirmed asymmetry. `require_account_owner_match(account, caller)` is enforced on
**withdraw** (`withdraw.rs:66`), **borrow** (`borrow.rs:41`), **multiply** (`multiply.rs:231`),
**migrate_blend** (`migrate_blend.rs:212`), **repay_debt_with_collateral** (`:77`), **swap_debt**
(`:74`), and **swap_collateral** (`:76`) — i.e. every value-extracting / collateral-moving flow, so
**funds cannot be stolen** from another account. But `process_supply` (`supply.rs:45`) calls only
`caller.require_auth()` + `resolve_supply_account` (which checks spoke match, **not** ownership),
so any caller may `supply(victim_account_id, …)`. Consequences:

- **Gifting (benign-to-victim):** the supplied tokens come from the caller and land on the victim's
  account as collateral the victim (sole owner) can later withdraw — the stranger cannot get them
  back. Net: the attacker *loses* funds; the victim gains. No theft.
- **Slot-exhaustion griefing (the real, narrow surface):** supply positions are capped per account
  (`PositionLimits.max_supply_positions`, default 10). A stranger can gift dust in distinct assets
  to fill a victim's remaining slots, so the victim cannot add a *new* collateral asset until they
  withdraw the gifted dust. Bounded: top-ups of an *already-held* asset need no new slot, the victim
  recovers the dust on withdraw (profit), and the attacker pays for every gift.

**POC.** `tests/test-harness/tests/controller/supply.rs::poc_non_owner_can_supply_into_victims_account`.
ALICE opens an account; BOB (a stranger) calls `supply(&bob, alice_id, …, ETH)` — it **succeeds**
with no owner-match revert, the ETH position lands on ALICE's account, and `get_account_owner` is
still ALICE. Passing (`cargo test -p test-harness --test controller`).

**Verdict / defense.** VALID but **LOW** — a behavioral asymmetry, not a theft path (the
owner-match guard that protects every withdrawing flow makes the gifted funds unrecoverable by the
attacker). May be intentional Aave-style "supply-on-behalf," but it is the lone position flow
without the owner check and enables narrow slot-exhaustion griefing. Suggested hardening: either add
`require_account_owner_match` to `supply` for an existing `account_id`, or explicitly document
third-party supply as intended and make `max_supply_positions` resilient to gifted dust (e.g. ignore
zero-value/dust positions when counting slots). No on-chain guard currently restricts third-party
supply.

---

## Iteration 13 — 2026-06-22

### 13.1 — Over-borrow via HF-aggregation rounding (mixed-decimal collateral over-valuation) — **NOT VALID**

**Assumption.** HF/LTV gates aggregate per-asset USD values across 6/7/8-decimal assets. If any
conversion (scaled→ray→wad, ×price, ×LTV/threshold) rounded collateral **up** or debt **down**, an
attacker could repeatedly borrow a rounding margin beyond what collateral supports, or dodge
liquidation — net-draining the pool over many ops.

**Analysis.** `calculate_account_risk_totals_body` (`helpers/math.rs:121`) enforces **directional,
conservative rounding** on the gate inputs:

- **Collateral → floor.** The borrow-capacity / health-factor gates read `gate_value =
  position_value_floor(...)` (`math.rs:150`); LTV capacity uses
  `loan_to_value.apply_to_wad_floor(gate_value)` (`math.rs:158`); the liquidation-threshold
  weighting uses `weighted_collateral(gate_value, …)` off the floored value (`math.rs:159`). Code
  comment: *"the floored chain feeds the borrow-capacity and health-factor gates so no rounding step
  can loosen them."*
- **Debt → ceil.** `total_debt += position_value_ceil(...)` (`math.rs:168`). Comment: *"Ceil the
  whole chain: owed value cannot round downward."*
- **Neutral valuation is gate-isolated.** The half-up `total_collateral` (`position_value`,
  `math.rs:144`) is used only for liquidation seizure *proportions* and bad-debt socialization, not
  for the HF/LTV solvency gates — so its rounding cannot loosen borrow capacity.

Because each asset's conversion applies floor (collateral) or ceil (debt) at its own decimals,
mixed-decimal portfolios (6/7/8) inherit the same conservatism: every rounding step in the solvency
gates rounds **against** the borrower. The worst case is the protocol slightly *under*-crediting a
borrower's capacity (their loss), never over-crediting.

**Verdict / defense.** Defended by floor-collateral / ceil-debt directional rounding throughout the
HF and LTV gates (`position_value_floor` / `apply_to_wad_floor` / `weighted_collateral` vs
`position_value_ceil`), with neutral valuation confined to non-gate proportions. No rounding-margin
over-borrow channel. No POC (invalid).

---

## Iteration 14 — 2026-06-22

### 14.1 — DeFindex strategy NAV inflation: third-party `supply` into the strategy's controller account — **VALID (LOW)**

**Assumption.** `Strategy::balance(vault)` (defindex `lib.rs:223`) reports
`controller.collateral_amount_for_token(account_id, asset)` — the *raw* controller collateral of the
strategy's account for that vault. Because controller `supply` has **no owner check** (VECTOR #12.1),
an external actor can deposit straight into the strategy's controller account (`account_id` is
public via `lending_account_id(vault)`), bypassing `Strategy::deposit` and inflating the vault's
reported balance / NAV without any legitimate vault deposit.

**Analysis.** Confirmed end-to-end. The strategy's account is owned by the strategy *contract*, but
controller `supply(caller, account_id, …)` only checks `caller.require_auth()` (not ownership), so
any funded address can add collateral to it. The strategy then surfaces that injected collateral as
`balance(vault)`. Impact and bounds:

- **NAV griefing of the integrating DeFindex vault.** A DeFindex vault prices shares off
  `strategy.balance()` / `harvest` pps; a third party can push that balance up at will, breaking
  vault-layer balance/NAV assumptions or front-running a victim's deposit/withdraw to perturb share
  pricing.
- **Not profitable (no theft).** The donated collateral lands in the strategy's controller account,
  withdrawable **only** by the vault (`Strategy::withdraw` gates on `from.require_auth()` = the
  vault). The attacker cannot reclaim it — exactly the unrecoverable-donation property of #12.1. So
  the attacker can inflate NAV but cannot extract value; existing vault shareholders are subsidized.
- **Root cause is #12.1** (supply lacks `require_account_owner_match`); the strategy *compounds* it
  by reading raw controller collateral with no isolation between vault-routed deposits and external
  donations.

**POC.** `contracts/defindex-strategy/tests/strategy.rs::poc_third_party_inflates_strategy_balance_via_controller_supply`.
A vault deposits 1,000; a stranger then calls `controller.supply(attacker, strategy_account_id, 0,
[(USDC, 500)])` directly; `strategy.balance(vault)` jumps by ~500 with no vault deposit. Passing
(`cargo test -p defindex-strategy --test strategy`).

**Verdict / defense.** VALID, **LOW** — NAV-inflation griefing of the integrating vault, not theft
(donation unrecoverable by the attacker). No on-chain guard isolates the strategy's controller
account from third-party `supply`. Fixes: (a) add `require_account_owner_match` to controller
`supply` (also closes #12.1) so only the strategy can fund its own account; or (b) have the strategy
track vault-routed principal internally rather than trusting raw controller collateral for
`balance()`; and integrators should use a donation-resistant NAV (virtual shares / internal
accounting) rather than live `balance()`.

---

## Iteration 15 — 2026-06-22

### 15.1 — Bulk-payment aggregation: duplicate-asset double-processing / sentinel confusion — **NOT VALID**

**Assumption.** Bulk endpoints (`supply`/`borrow`/`repay`/`withdraw`/`liquidate`) take a
`Vec<(Address, amount)>`. A crafted list — the same asset repeated, or a positive amount mixed with
the withdraw-all sentinel `0` — might double-process an asset, smuggle a negative, or desync the
per-asset entry the pool settles against.

**Analysis.** `aggregate_payments` (`helpers/utils.rs:39`) normalizes before any settlement:

- **Dedup by asset.** Amounts accumulate into a `Map<Address, i128>` keyed by asset, with
  `order` preserving first-seen sequence; the result has **exactly one entry per asset**, so the
  pool settles each asset once (no double-processing).
- **Checked sums, no negatives/zeros.** `aggregate_payment_amount` panics `AmountMustBePositive` on
  `amount < 0` (always) and on `amount == 0` when `zero_is_withdraw_all == false`
  (supply/borrow/repay), and sums with `checked_add` → `MathOverflow` on overflow.
- **Withdraw-all sentinel is sticky and consistent.** With `zero_is_withdraw_all == true`
  (withdraw), once any entry for an asset is `0` *or* a running total is `0`, the asset's total
  collapses to `0` (full-withdraw) and stays there (`previous == Some(0) → 0`) — so a mixed
  `[(A,100),(A,0)]` deterministically means "withdraw all A," never 100-then-something.
- **Single-payment fast path** (`len == 1`) routes through the same `aggregate_payment_amount`
  gate, so it can't skip the positive/sentinel checks.

A liquidator/borrower listing a non-existent or non-owned debt asset is caught downstream
(`DebtPositionNotFound`); duplicates and sentinels are folded deterministically here.

**Verdict / defense.** Defended by map-based dedup (one entry per asset), checked summation,
negative/zero rejection, and a sticky-consistent withdraw-all sentinel. No double-processing or
accounting desync from a crafted bulk list. No POC (invalid).

---

### Convergence note (after 15 iterations)

Coverage now spans every contract and every standard attack class (auth/theft, oracle
manipulation/divergence, flash reentrancy, aggregator extraction, spoke caps, IRM/accrual overflow,
position-math + solvency-gate rounding, governance/timelock, Blend migration, token-trust, bulk
aggregation, strategy NAV). **No fund-theft path exists**; the value-moving core is conservatively
rounded, fail-closed, and comprehensively auth-gated.

The 4 VALID findings are all LOW griefing/asymmetry (no theft): **1.2** (spoofable harvest `from`),
**5.1** (dust account spam), **12.1** (non-owner `supply`), **14.1** (strategy NAV inflation).
**Highest-leverage fix:** adding `require_account_owner_match` to controller `supply` for an existing
`account_id` closes **both 12.1 and 14.1** at once. Remaining items are deliberate governance-trust
boundaries (11.1 token selection; 8.1 oracle-divergence liquidation pause). Further iterations are
hitting diminishing returns — most fresh probes now re-confirm existing defenses.

---

## Iteration 16 — 2026-06-22

### 16.1 — DeFindex strategy `deposit`/`withdraw` return *cumulative balance*, not the delta — **RESOLVED: NOT VALID / by-design (spec-conformant)** _(see Iteration 18 resolution)_

**Assumption.** A DeFindex vault mints/burns shares using the value its strategy adapter's
`deposit`/`withdraw` returns. If the adapter returns the *cumulative* position balance instead of
the *amount just deposited/withdrawn*, the vault would mint/burn shares against the wrong basis — a
2nd+ depositor credited for the whole position (share over-mint → dilution/theft of earlier
shareholders).

**Analysis.** Confirmed the strategy returns **post-op balances, not deltas**:

- `deposit` (defindex `lib.rs:209`) returns `ctx.collateral(new_or_existing_id)` — the account's
  **total** collateral, not `amount`. Existing test `test_second_deposit_can_be_small_after_account_opened`
  asserts a 1-unit second deposit returns **11 UNIT** (the cumulative total).
- `withdraw` (`lib.rs:259`) returns `ctx.collateral(account_id)` — the **remaining** balance after
  the withdraw, not the amount withdrawn (test names it `remaining`).

This is a deliberate design *within this repo* (asserted by its own tests), with **no doc comment**
on the trait's return contract and **no DeFindex `StrategyTrait` spec/reference present in the repo**
to check against. I therefore cannot verify whether it matches DeFindex's expected semantics:

- **If** the DeFindex vault uses `balance()` for NAV and the *input* `amount` (not the strategy's
  return) for share math — common in vault designs — this is **benign**.
- **If** the vault treats the `deposit` return as "underlying deposited" for share minting, this is a
  **HIGH-severity over-mint**: the 2nd depositor's shares are computed off the whole position,
  diluting/stealing from prior depositors. An attacker would deposit a tiny amount second to capture
  shares for the full balance.

This is *not* a permissionless on-chain attack on our contracts (the controller accounting is
correct regardless); it is an **integration-contract risk** that depends entirely on the external
DeFindex vault's use of the return value.

**Verdict / defense.** Open item, **must be confirmed against the DeFindex `StrategyTrait`
specification before mainnet**. Recommended: (a) verify whether DeFindex's vault consumes the
`deposit`/`withdraw` return as a *delta* or ignores it in favor of `balance()`; (b) if delta is
expected, change the adapter to return the deposited/withdrawn amount (delta), or document the
post-op-balance semantics prominently in the trait and confirm the paired vault matches. No POC
(verdict hinges on the external spec, not reproducible against our contracts alone).

> **RESOLUTION (Iteration 18, against the authoritative DeFindex spec).** The cumulative-balance
> return is **the required DeFindex contract, not a bug** — verdict flips to **NOT VALID / by-design**.
> The `DeFindexStrategyTrait` doc (paltalabs/defindex `apps/contracts/strategies/core/src/lib.rs`)
> states `deposit` returns *"the balance of the `from` address after the deposit"* and stresses *"It
> is very important that the return is the balance of the `from` address … so the vault can keep
> track of the strategy's status"*; `withdraw` likewise returns *"the balance of the `from` address
> after the withdraw"*; `balance` returns *"the underlying asset … not some kind of share or
> derivative."* The reference HODL strategy returns `Ok(read_balance(&e, from))` (cumulative), exactly
> matching our adapter's `ctx.collateral(account_id)`. So returning the post-op balance (not a delta)
> is correct; the DeFindex vault does its own share math and tracks NAV via this balance. No over-mint
> risk. (This also confirms #14.1's premise: DeFindex tracks strategy status via the balance return,
> so a third party inflating that balance perturbs the vault's NAV view — #14.1 stays VALID.)

---

## Iteration 17 — 2026-06-22

### 17.1 — Forced bad-debt socialization / seize-to-revenue griefing — **NOT VALID (not attacker-triggerable)**

**Assumption.** `clean_bad_debt(caller, account_id)` is permissionless and `execute_bad_debt_cleanup`
socializes an account's debt by reducing the market `supply_index`
(`apply_bad_debt_to_supply_index`, harming all suppliers) and moves the account's residual collateral
to protocol revenue (`seize_positions` `Deposit` leg → `revenue += scaled`). An attacker might force or
amplify this to harm suppliers, or target a specific account.

**Analysis.** Not reachable as an attack:

- **Gated by genuine bad debt.** `clean_bad_debt` requires `is_socializable_bad_debt(total_debt,
  total_collateral)` = `debt > collateral AND collateral <= BAD_DEBT_USD_THRESHOLD`
  (`liquidation_math.rs:87`). An attacker cannot manufacture `debt > collateral` — borrow/withdraw
  are gated by the HF≥1 + LTV solvency checks (conservatively rounded, iter 13). Bad debt arises
  only from adverse *oracle* moves, which the attacker cannot induce cheaply (iter 8).
- **No attacker profit.** Cleanup *removes* the bad account and socializes a real, already-incurred
  shortfall; the residual dust collateral routes to protocol revenue (governance-claimable), not to
  the caller. The permissionless caller gains nothing — it is a keeper/janitor action.
- **Socialization is capped.** `apply_bad_debt_to_supply_index` caps the reduction at
  `total_supplied_value` and clamps to `SUPPLY_INDEX_FLOOR_RAW` (iter 9), so the index cannot be
  driven below the floor.

(Protocol-fairness review item — not an external-actor attack: whether the seized residual collateral
should *offset* the socialized loss rather than route to revenue is an accounting-policy question for
the team, with no attacker leverage since the path is bad-debt-gated and profitless.)

**Verdict / defense.** Defended for external-actor scope: bad-debt cleanup is gated on a genuine,
oracle-driven `debt > collateral` state the attacker can't manufacture, yields no caller profit, and
its socialization is floor-capped. No POC (not attacker-triggerable).

### Evidence integrity check (iteration 17)

Re-ran all four VALID-finding POCs to confirm the log's claims hold for review:
- `defindex-strategy --test strategy`: `poc_harvest_is_unauthenticated_and_from_is_spoofable` (1.2),
  `poc_third_party_inflates_strategy_balance_via_controller_supply` (14.1) — **2 passed**.
- `test-harness --test controller`: `poc_non_owner_can_supply_into_victims_account` (12.1),
  `poc_single_actor_spams_unbounded_dust_accounts` (5.1) — **2 passed**.

All POCs green. The 4 VALID findings remain reproducible.

---

## Iteration 20 — 2026-06-22

### 20.1 — Spoke spoke borrow-cap bypass via the leverage/strategy borrow path — **NOT VALID**

**Assumption.** Iteration 6.2 confirmed the spoke spoke caps are enforced + counted on the *normal*
`borrow`/`repay`/`withdraw`/`liquidation` paths. But `multiply`, `swap_debt`, and `migrate_from_blend`
open debt through a *different* primitive — `open_strategy_borrow`/`open_migration_borrow` →
`pool.create_strategy` (the zero/low-fee strategy "flash" borrow). If that leg skipped
`apply_borrow_after_pool`, an attacker could open debt **beyond the spoke spoke borrow cap** via a
leverage entrypoint (cap bypass), and under-count the usage counter (drift).

**Analysis.** Code-traced: the strategy-borrow leg shares the *same* settlement as a normal borrow.
`borrow_strategy_inner` (`borrow.rs:203`) — the body behind both `borrow_for_strategy` (multiply /
swap_debt) and `borrow_for_migration` (migrate) — calls `validate_borrow` and then, after
`pool_create_strategy_call`, routes the result through **`merge_borrow_result`** (`borrow.rs:233`).
`merge_borrow_result` (`borrow.rs:129`) is exactly where `apply_borrow_after_pool` runs
(`borrow.rs:144-146`), and `apply_borrow_after_pool` calls `enforce_spoke_borrow_cap`
(`spoke_caps.rs`, asserts `next_scaled ≤ cap_scaled` → `SpokeBorrowCapReached`) **before**
incrementing the per-(category, asset) usage. So the leverage paths enforce the spoke borrow cap and
increment the counter identically to a plain `borrow`. No bypass; no under-count.

**Verdict / defense.** Defended: every debt-opening path — plain `borrow` *and* the strategy/leverage
legs (`multiply`/`swap_debt`/`migrate_from_blend`) — funnels through the shared `merge_borrow_result`
→ `apply_borrow_after_pool` → `enforce_spoke_borrow_cap`. This closes the residual question left open
in 6.2 (which covered only the non-leverage paths): the spoke spoke borrow cap cannot be evaded via
leverage. No POC (invalid).

---

## Iteration 21 — 2026-06-22

### 21.1 — Permissionless `deploy_pool` / pool re-deploy / ownership-reset — **NOT VALID**

**Assumption.** The controller deploys and owns the single central pool. If `deploy_pool` (or a
pool-setup forwarder) were permissionless or re-callable, an external actor could deploy a rogue
pool, re-deploy to reset pool state, or hijack the pool address the controller routes to — bricking
or draining every market.

**Analysis.** Both axes are gated:

- **Owner-gated.** `deploy_pool` (`router.rs:52-53`), `create_liquidity_pool` (`:73`), and the
  pool-admin forwarders (`update_pool_caps`, `upgrade_liquidity_pool_params`, `upgrade_pool`,
  `:83-94`) are all `#[only_owner]` (owner = the governance contract → 48h timelock). An external
  actor cannot invoke them.
- **One-time / idempotent.** `deploy_pool` asserts `try_get_pool().is_none()` →
  `panic PoolAlreadyDeployed` (`router.rs:57-61`) on any repeat, and deploys with a **fixed salt**
  via `deploy_v2` (deterministic address). So even governance cannot accidentally re-deploy or
  repoint the pool; the address is set once and frozen.
- **Pool side.** The pool's mutating + `upgrade` entrypoints are `#[only_owner]` (owner =
  controller); `accept_ownership`-style claims require a pending owner named by the current owner.
  No external ownership-claim path (consistent with iter 4).

**Verdict / defense.** Defended by `#[only_owner]`(=governance) gating plus the one-time
`PoolAlreadyDeployed` guard and fixed-salt deterministic deploy. No permissionless deploy, re-deploy,
reset, or pool-address hijack. This completes entrypoint-auth coverage across the controller
(position flows, strategies, migration, flash, governance forwarders, deploy/market-setup, admin).
No POC (invalid).

---

### Final coverage note (21 iterations)

Entrypoint-auth coverage is now exhaustive: every controller entrypoint is either (a) owner-gated
(`#[only_owner]` = governance + timelock: deploy/market-setup/caps/upgrade/pause/oracle), (b)
caller-auth + owner-match for account-affecting flows (borrow/withdraw/multiply/swap_*/migrate/
repay-with-collateral), (c) caller-auth permissionless-but-benign (supply [#12.1 asymmetry], repay,
add_rewards, update_indexes, claim_revenue→owner, renew_account [owner-match], update_account_threshold
[bounded]), or (d) pool-internal `#[only_owner]`=controller. The 4 VALID findings (1.2, 5.1, 12.1,
14.1) remain the only real issues, all LOW griefing/asymmetry. No further distinct permissionless
external-actor vectors found; the loop has reached genuine saturation.

---

## Iteration 22 — 2026-06-22 · Full-suite regression integrity

Ran the **complete** test binaries (not just the 4 POCs by name) to confirm the four added POC tests
coexist with every existing test and the log's evidence base is regression-free:

- `cargo test -p defindex-strategy --test strategy` → **15 passed, 0 failed** (13 pre-existing + 1.2
  + 14.1 POCs).
- `cargo test -p test-harness --test controller` → **329 passed, 0 failed** (327 pre-existing + 5.1
  + 12.1 POCs).

No regressions. All four VALID-finding POCs are live in the shared suites and green. The review in
this file is complete, internally consistent, and reproducible for the 24h check.

---

## Iteration 23 — 2026-06-22

### 23.1 — Same-asset liquidation netting (looped spoke collateral == debt) — **NOT VALID**

**Assumption.** In an spoke looped position the collateral and debt can be the *same* asset X.
`process_liquidation` repays X-debt and seizes X-collateral in one call. If the seize and repay legs
aliased the same ledger entry — or the liquidation bonus were sourced from the shared market's cash
rather than the borrower's collateral — a liquidator could net the bonus from the *pool* (other
suppliers), or double-count X's index / spoke usage.

**Analysis.** Supply and borrow are **independent per-(asset, side) index-scaled ledgers**, so
same-asset is structurally identical to cross-asset:

- The two legs touch *different* storage: `apply_liquidation_repayments` → `repay::settle_repay_actions`
  reduces the borrower's `DebtPositions[X]` (borrow_index-scaled), while `apply_liquidation_seizures`
  → `withdraw::settle_withdraw_entries` reduces `SupplyPositions[X]` (supply_index-scaled)
  (`liquidation.rs:190-249`). There is no shared entry to alias even when the asset is the same.
- The bonus is sourced from the **borrower's seized collateral** (`calculate_seized_collateral` off
  the borrower's `total_collateral` proportions, bounded by `max_bonus_for_threshold` and per-asset
  `min(actual_ray)`), not minted from pool cash. The liquidator pays X in (repay) and receives X out
  (seized collateral + bonus); the borrower's supply share is burned by exactly the seized amount, so
  pool solvency is preserved and the bonus differential comes from the borrower, not other suppliers.
- Spoke usage decrements on the **two independent sides** — borrow usage via the repay leg, supply
  usage via the seize leg — through the symmetric `apply_*_after_pool` (iter 6.2). No double-count.
- `global_sync` runs once per market per tx; both legs read the same cached supply/borrow indexes, so
  there is no intra-tx index drift between the legs.

**Verdict / defense.** Defended by the per-(asset, side) independent index-scaled ledger design:
seize touches the supply side, repay touches the borrow side, and the bonus is borrower-sourced and
bounded — same-asset liquidation reconciles exactly like cross-asset, with no pool-drain or
double-count channel. No POC (invalid).

---

## Iteration 24 — 2026-06-22

### 24.1 — Free flash loan via caller-supplied/zeroed fee (protocol-revenue bypass) — **NOT VALID**

**Assumption.** Flash loans charge a fee that accrues to protocol revenue. If the public
`flash_loan` entrypoint let the caller pass (or zero out) the fee — or derived it from caller input —
an external actor could take repeated fee-free flash loans, bypassing protocol revenue, or pass a
negative/oversized fee to corrupt repayment accounting.

**Analysis.** The fee is **not caller-controllable**. The controller's public entrypoint
`flash_loan(caller, asset, amount, receiver, data)` (`strategies/flash_loan.rs:19-28`) has **no fee
parameter**; it computes the fee internally from governance config:
`fee = asset_config.flashloan_fee.flash_loan_fee_on(env, amount)` (`flash_loan.rs:55`), then passes
that derived `fee` to the owner-gated pool call (`pool_flash_loan_call`, `:61`). The caller chooses
only `asset`, `amount`, `receiver`, `data`. Surrounding gates:

- `caller.require_auth()` + `require_not_flash_loaning` (no nested/reentrant flash loan) +
  `require_positive_amount` (`:39-42`).
- `require_market_active` and a per-asset **`is_flashloanable` kill switch** (`:48-52`,
  `FlashloanNotEnabled`) — governance can disable flash loans per asset.
- `require_wasm_receiver` (`:53`) — receiver must be a contract.
- `FlashLoanOngoing` set around the pool call (`:59/63`); the pool verifies repayment of
  principal **+ fee** by balance-bracket after the receiver callback (CEI; iter 5.2). A short
  repayment reverts the whole tx.
- The strategy/migration legs that *do* override the fee (`fee_override`: multiply = configured fee,
  migrate = `Some(0)`) are **internal** functions (`borrow.rs:220`), reached only via the owner-match
  + HF-gated `multiply`/`migrate_from_blend` entrypoints — not the public `flash_loan`, and the
  zero-fee migrate leg opens *permanent user debt*, not a flash loan (iter 7).

So the only externally reachable flash-loan fee is the governance-configured one; it cannot be zeroed
or forged by the caller, and the pool enforces principal+fee repayment.

**Verdict / defense.** Defended: the public `flash_loan` derives the fee from per-asset governance
config (no caller fee input), gated by a per-asset flashloanable switch, reentrancy guard, and
pool-side principal+fee repayment bracket. No free flash loan / revenue bypass. No POC (invalid).

---

## Iteration 25 — 2026-06-22

### 25.1 — Spoke spoke-cap exhaustion griefing (deny honest users entry to an spoke category) — **NOT VALID (by-design capped-market property; LOW soft-DoS residual)**

**Assumption.** The spoke spoke caps are **category-wide**, not per-account: `enforce_spoke_supply_cap`
/`enforce_spoke_borrow_cap` (`spoke_caps.rs:183-219`) check a per-(category, asset) usage counter
shared across *all* accounts in that category against `cfg.supply_cap`/`cfg.borrow_cap`. So an
attacker could supply (or borrow) up to the spoke cap with their own positions, exhausting the
category-wide counter and **denying honest users entry** to that spoke category for that asset — a
griefing/DoS with no theft.

**Analysis.** This is the inherent "fill the cap" property of *any* usage-capped market (Aave supply
caps, Compound borrow caps behave identically); it is not a contract defect, and its cost/scope make
it self-limiting:

- **Capital-bonded, not dust.** The cap is **value-scaled**: `max_scaled_for_cap(env, cap, decimals,
  index)` converts the governance USD/asset cap to scaled units, so filling it requires committing the
  *full cap value* in real tokens (contrast the dust-spam of 5.1). For a sensibly-sized cap that is
  substantial capital the attacker must lock.
- **Scoped to the spoke overlay, not the asset.** Spoke is an *opt-in higher-LTV mode*. Exhausting
  the spoke cap blocks only the spoke path for that asset; honest users can still supply/borrow the
  same asset in **normal mode** under the asset's global caps. The DoS denies the LTV boost, not asset
  access.
- **Borrow side is costly & self-limiting.** Holding the borrow cap full means over-collateralizing
  and paying borrow interest continuously — economically irrational to sustain.
- **Supply side earns yield but is reversible.** A supply-cap squatter earns supply yield (low direct
  cost), but the capital is locked and unproductive elsewhere, and the cap frees the instant they
  withdraw; governance can also raise the cap or disable it (`cap_is_enabled`). The hub cap
  (`enforce_hub_caps`) provides a second governance ceiling.

**Verdict / defense.** Not a bug — category-wide caps are intended ceilings; exhausting one is the
unavoidable flip side of having a cap, and here it is capital-bonded (full cap value, not dust),
scoped to the opt-in spoke overlay (normal-mode access unaffected), costly on the borrow side, and
instantly reversible. Residual: a **LOW** supply-side soft-DoS (a well-capitalized actor can squat an
spoke supply cap while earning yield), mitigated by generous governance cap sizing and the
normal-mode fallback. No POC (by-design ceiling behavior, not a reproducible contract defect).

---

### Saturation note (25 iterations)

Iterations 20–25 each probed a distinct concrete edge beyond the iter-18 baseline (leverage-path
spoke enforcement, pool deploy/ownership, same-asset liquidation netting, flash-fee derivation,
spoke-cap exhaustion) — all confirmed defended or by-design, none surfacing a new VALID issue. The
finding set is stable: **4 VALID (LOW: 1.2, 5.1, 12.1, 14.1), 16.1 resolved benign, all else
NOT-VALID/governance-trust, no fund-theft path.** Full suites green (iter 22). Further iterations will
continue to re-confirm existing defenses; the substantive review is complete for the 24h check.

---

## Iteration 26 — 2026-06-22 · Governance suite regression integrity

Completed regression evidence across **all three in-scope contracts** (iter 22 covered
controller + defindex-strategy; this closes governance):

- `cargo test -p governance --lib` → **69 passed, 0 failed** (timelock lifecycle, proposer auth,
  validation, deploy/ownership).
- `cargo test -p test-harness --test governance` → **57 passed, 0 failed** (gov↔controller
  integration: timelock schedule/execute/cancel, admin validation, owner-ask coverage).

Combined with iter 22 (controller 329, defindex-strategy 15), every in-scope contract's test suite is
green. No regressions anywhere; the log's defense claims rest on a fully-passing test base. The
substantive external-actor review is complete and reproducible for the 24h check.

---

## Iteration 27 — 2026-06-22 · Pool suite regression integrity (+ one pre-existing stale test flagged)

Ran the **pool** suite to complete the four-crate evidence set. Reporting faithfully — one
pre-existing harness test fails (not a vulnerability, not introduced by this review's POCs):

- `cargo test -p pool --lib` → **113 passed, 0 failed** (views, interest, cache, rounding, seize).
- `cargo test -p test-harness --test pool` → **68 passed, 1 FAILED**.

**The failure is a stale test, not a finding.** `revenue::test_permissionless_revenue_endpoints`
(`tests/pool/revenue.rs:284`) asserts an **unfunded** caller (BOB) can `add_rewards(100 USDC)`.
But `add_reward` (`controller/src/router.rs:289-305`) pulls the reward from the caller via
`utils::transfer_amount(env, asset, caller, pool, amount)` **before** bumping the index. The test
does `mock_all_auths()` — which mocks *authorization* but **not token balance** — so BOB (who holds
no USDC) correctly reverts on the transfer. The assertion's premise ("any signed caller may
add_rewards" with no funding) is wrong: a caller must actually *hold and transfer* the reward.

- **Not my change.** `git status` shows only `defindex-strategy/tests/strategy.rs` modified by this
  review; `pool/revenue.rs` and the `add_reward` source are untouched — the failure is pre-existing
  on the tree.
- **It *reinforces* a defense (iter 2.1).** The funding requirement is exactly why the donation /
  index-inflation attack has no free lever: you cannot bump `supply_index` without transferring the
  reward value in. The test is simply asserting the pre-funding-era behavior.
- **Team follow-up (test hygiene, LOW):** fix the test to fund BOB first (mint USDC to BOB, then
  assert success), or split it into "auth is permissionless" (no funds → expect transfer revert) and
  "funded caller succeeds." No contract change needed.

Net: pool contract logic is green (113/113 lib); the lone harness failure is a stale test whose
premise contradicts the (correct, defensive) caller-funds-the-reward behavior.

> **RESOLVED (post-review fix).** `test_permissionless_revenue_endpoints` is now fixed: it seeds a
> supplier (`t.supply(ALICE, "USDC", 10_000.0)` — so `add_rewards` has shares to distribute, else
> `NoSuppliersToReward`, iter 28) and funds the caller (`token_admin.mint(&bob_addr, &100)` — since
> `add_rewards` pulls the reward from BOB) before asserting `add_rewards` succeeds. BOB remains a
> non-admin signed caller, so the test still proves the endpoint is permissionless. No contract
> change. **`cargo test -p test-harness --test pool` → 69 passed, 0 failed.**

---

## Iteration 28 — 2026-06-22

### 28.1 — `add_rewards` on a zero-supplier market: div-by-zero / fund-loss / mis-accounting — **NOT VALID**

**Assumption.** `add_rewards` bumps `supply_index` via
`new_index = old_index·(1 + rewards/total_supplied_value)` where
`total_supplied_value = supplied·supply_index`. On a market with **no suppliers** (`supplied == 0` —
e.g. fully withdrawn, or freshly created), this is `rewards / 0`. An attacker could (a) trigger a
div-by-zero panic to brick the market, (b) lose/strand their transferred reward with no recipient, or
(c) corrupt the index. Recall (iter 27) the controller transfers the reward from the caller to the
pool *before* the pool applies it.

**Analysis.** The pool's `add_rewards` (`pool/src/lib.rs:306`) has an **explicit empty-market guard
before any state change**:

```
assert_with_error!(&env, cache.supplied != Ray::ZERO, GenericError::NoSuppliersToReward);
```

This fires *before* `update_supply_index` and `credit_cash`, so:

- **No div-by-zero.** The guard short-circuits before the `rewards/total_supplied_value` division is
  ever reached.
- **No fund loss.** The controller's caller→pool `transfer_amount` (`router.rs:295`) and this pool
  call are in the **same transaction**; the `NoSuppliersToReward` revert rolls the whole tx back
  (Soroban atomicity), so the reward transfer is undone — the caller keeps their funds.
- **No corruption.** State is only mutated after the guard passes; a reverted call commits nothing.

The complementary reserve-factor path `add_protocol_revenue_ray` (`interest.rs:64`) carries the same
guard (`if cache.supplied == Ray::ZERO { return; }` — "fees on an empty pool are dropped"), and
`update_supply_index` itself early-returns on a zero time delta (`interest.rs:15`). So every
index-bumping path is empty-market-safe. (Together with iter 2.1 [index un-inflatable] and iter 27
[caller must fund the reward], the rewards path is robust on all three axes: inflation, funding, and
empty market.)

**Verdict / defense.** Defended by the explicit `NoSuppliersToReward` pre-state-change assert in
`add_rewards` (mirrored by the `supplied == ZERO` guard in the revenue path), with tx-atomic rollback
of the caller's transfer. No div-by-zero, no stranded funds, no index corruption. No POC (invalid;
clean revert only fails the caller's own tx).

---

## Iteration 29 — 2026-06-22

### 29.1 — Governance-DoS via permissionless timelock `cancel` of scheduled operations — **NOT VALID**

**Assumption.** The timelock lifecycle is propose → wait → execute, with `cancel` to drop a scheduled
operation. Iter 4 verified propose is PROPOSER-gated and execute is open-by-design (safe: the op was
validated + waited). But if **`cancel` were permissionless**, an external actor could cancel every
scheduled governance operation the instant it's proposed — indefinitely blocking *all* governance
actions (param updates, market onboarding, upgrades, pause), a total governance-DoS.

**Analysis.** `cancel` is **role-gated to CANCELLER**, not open. `Governance::cancel`
(`governance/src/timelock.rs`) is:

```
pub fn cancel(env: Env, canceller: Address, operation_id: BytesN<32>) {
    renew_governance_instance(&env);
    canceller.require_auth();
    access_control::ensure_role(&env, &Symbol::new(&env, CANCELLER_ROLE), &canceller);
    cancel_operation(&env, &operation_id);
}
```

- `canceller.require_auth()` — the caller must sign, and
- `access_control::ensure_role(CANCELLER_ROLE, canceller)` — the caller must **hold the CANCELLER
  role** (granted only by governance itself; `access.rs:19`). An external actor without the role
  reverts. So scheduled operations cannot be cancelled by the public.
- Defense-in-depth: `access.rs:109-234` enforces **EXECUTOR/CANCELLER separation** for delegated
  accounts (a single delegate cannot hold both), a separation-of-duties guard against a compromised
  delegate both stalling and force-running ops.

This completes the timelock authorization picture from iter 4: **propose = PROPOSER**, **execute =
open-by-design** (validated + delay-elapsed; caller can't alter what runs), **cancel = CANCELLER**.
All three lifecycle transitions are correctly gated; only the deliberately-open execute is
permissionless, and that is safe.

**Verdict / defense.** Defended by `require_auth` + `ensure_role(CANCELLER_ROLE)` on `cancel`, plus
EXECUTOR/CANCELLER delegate separation. An external actor cannot cancel scheduled operations, so the
timelock cannot be stalled into a governance-DoS. No POC (invalid).

---

## Iteration 30 — 2026-06-22

### 30.1 — Bundled/repeated liquidation in one tx to over-seize beyond the close factor — **NOT VALID**

**Assumption.** A liquidation is a *partial* close that brings HF back toward a target (iter 3). If
the close bound were computed once and reused, a liquidator could bundle N `liquidate` calls against
the same account in a single tx and seize N× the intended close — over-seizing the borrower's
collateral (and N× the bonus) in one block.

**Analysis.** Each `liquidate` is **independently re-gated and re-bounded** on the *current* state, so
bundling self-limits:

- **Fresh HF each call.** `build_liquidation_plan` (`liquidation.rs:136-158`) recomputes
  `calculate_account_risk_totals` from live positions and asserts `totals.health_factor < Wad::ONE`
  (`:154`, else `HealthFactorTooHigh`). After call #1's partial close raises HF, call #2 in the same
  tx re-reads the improved HF; once HF ≥ 1 it **reverts** — no further seizure.
- **Close amount is HF-targeted, not cached.** `calculate_seizure_proportions` /
  `normalize_repayment_plan` size the close to move HF toward the target on *this* call's totals; a
  larger residual position simply allows another bounded partial close, never an unbounded one.
- **Per-call seizure ≤ collateral.** `max_bonus_for_threshold` + per-asset `min(actual_ray)` (iter 3)
  cap each seizure; summed across bundled calls the total still tracks the borrower's genuine
  insolvency (each call only seizes what the live HF<1 warrants).
- **Debt-free / healthy guard.** An account with no debt panics `HealthFactorTooHigh` early
  (`:145-146`); a healthy account fails the `hf < ONE` gate.
- **Self-liquidation blocked.** `validate_liquidation_inputs` asserts `account.owner != liquidator`
  (`AccountNotInMarket`), so an owner cannot self-liquidate to harvest their own bonus.
- Liquidation also asserts `require_not_flash_loaning` at entry (iter 5.2); seized collateral is paid
  out via SAC transfer, which on Soroban invokes **no recipient callback** — a contract liquidator
  cannot reenter on receipt.

**Verdict / defense.** Defended: every `liquidate` recomputes HF and re-gates on `hf < ONE`,
HF-targets a bounded partial close, and caps seizure at collateral — so bundled/repeated calls revert
as soon as HF is restored and can never over-seize. Self-liquidation is blocked outright. No POC
(invalid).

---

## Iteration 31 — 2026-06-22

### 31.1 — Pause-induced fund-lock: can a `pause` trap user/vault exits? — **NOT VALID (safe by design)**

**Assumption.** Governance can `pause` the protocol. If `pause` froze *all* entrypoints — including
the de-risking exits — then a pause (whether a legitimate emergency, an over-broad action, or a
compromised owner key) could **trap user and DeFindex-vault funds**: no withdraw, no repay, and bad
debt could not be cleared. That would make pause itself a fund-lock / liquidation-DoS lever.

**Analysis.** Pause is scoped as a **one-directional risk-freeze**, not a global halt. Verified by
mapping `#[when_not_paused]` across the controller:

- **Gated (frozen on pause) — risk-*increasing* only:** `supply`, `borrow` (`borrow.rs:28`,
  `supply.rs:29`), `multiply`, `swap_debt`, `swap_collateral`, `repay_debt_with_collateral`,
  `migrate_from_blend`, `flash_loan`, plus `add_rewards`/`claim_revenue`/`update_account_threshold`
  (`router.rs`).
- **Un-gated (stay callable while paused) — de-risking exits:** `withdraw`, `repay`, and
  `liquidation` carry **zero** `when_not_paused` (grep-confirmed 0 hits in
  `positions/{withdraw,repay,liquidation}.rs`), as do views and `renew_account` (TTL keep-alive).
  This is documented intent in `governance/access.rs:1-8`: *"Pause is a risk freeze: risk-increasing
  position paths, strategies, and flash loans are gated `when_not_paused`; withdraw, repay,
  liquidation, views, renew_account … remain callable while paused."*

Consequences:

- **Users can always exit.** Withdraw (collateral out) and repay (debt down) work during a pause, so
  no pause can trap a user's funds or force them into avoidable liquidation.
- **Liquidations keep clearing.** Bad debt can still be liquidated while paused, so a pause cannot be
  used to freeze the protocol into insolvency.
- **DeFindex-vault availability.** A lending pause blocks `Strategy::deposit` (→ gated controller
  `supply`) but **not** `Strategy::withdraw` (→ un-gated controller `withdraw`), so a vault can always
  redeem its position even during a lending-protocol pause — answering the iter 14 integration's
  availability question.
- **Blast-radius bound on the pause power itself.** Even a misused or compromised pause is limited to
  freezing *new risk*; it cannot seize, trap, or deny exit — a meaningful defense-in-depth limit on a
  privileged control.

**Verdict / defense.** Not a fund-lock: pause is a risk-freeze gating only risk-increasing flows;
withdraw/repay/liquidation stay open, so neither a legitimate nor an abused pause can trap user/vault
funds or block bad-debt clearing. By-design (documented in `governance/access.rs`), governance-trust
boundary with a deliberately bounded blast radius. No POC (invalid).

---

## Iteration 32 — 2026-06-22

### 32.1 — Per-market deactivation fund-lock: can deactivating a market trap existing positions? — **NOT VALID (safe by design)**

**Assumption.** Governance can deactivate a market (mark it inactive / `can_supply` off). The
risk-increasing flows call `require_market_active`. If that same gate also guarded the exits, a
deactivation (legitimate wind-down, or over-broad/compromised action) would **trap every existing
position in that market** — no withdraw, no repay — and could block liquidation of its bad debt.
This is the per-market analog of the global pause (iter 31).

**Analysis.** The market-active gate is enforced **only on entry/risk-increasing flows**, never on
exits. Verified by mapping `require_market_active` across the controller:

- **Gated:** `borrow` (`borrow.rs:81`), `supply` (`supply.rs:138`), `flash_loan` (`flash_loan.rs:45`).
- **Un-gated (grep-confirmed absent):** `withdraw`, `repay`, `liquidation` carry **no**
  `require_market_active`. So when a market is deactivated:
  - existing suppliers can still **withdraw** their collateral;
  - borrowers can still **repay** to reduce/close debt;
  - **liquidations** still proceed against unhealthy positions in that market;
  - only **new** supply/borrow/flash-loan are blocked.

This mirrors the pause invariant (iter 31): both controls **freeze new risk but never trap exits**.
Market deactivation is therefore a safe wind-down lever — governance can stop new exposure to a
deprecated/risky asset while letting existing positions unwind cleanly, and the asset's bad debt
stays liquidatable. (The DeFindex-vault corollary from iter 31 holds here too: a deactivated market
still permits `Strategy::withdraw` → controller `withdraw`, so vault redemption survives a market
wind-down.)

**Verdict / defense.** Not a fund-lock: `require_market_active` gates only supply/borrow/flash;
withdraw/repay/liquidation remain callable on a deactivated market, so existing positions can always
unwind and stay liquidatable. Same "freeze entry, never trap exits" design as the global pause;
governance-trust boundary with bounded blast radius. No POC (invalid).

---

## Iteration 33 — 2026-06-22

### 33.1 — DeFindex strategy admin/init/controller-hijack surface — **NOT VALID**

**Assumption.** The open-source strategy adapter binds to our controller. If it exposed a settable
controller/pool, a re-callable initializer, or an admin/upgrade entrypoint, an external actor could
**re-point the strategy at a malicious controller** (then drain or mis-report vault funds), re-init it
to hijack config, or upgrade it to backdoored code.

**Analysis.** Enumerated the strategy's *entire* entrypoint surface (`defindex-strategy/src/lib.rs`):

- `__constructor(env, asset, init_args=[controller])` (`:149`) — **deploy-once** (Soroban
  constructors are not re-invocable). It reads the controller from `init_args[0]`, validates the asset
  is a listed market (`controller_client.get_market_config(asset)`), and stores
  `Config { asset, controller, pool }` in **instance storage** (`:159-166`). There is **no setter**
  for `Config` — the (asset, controller, pool) binding is **immutable after deploy**.
- Views: `lending_account_id`, `has_lending_account`, `asset`, `balance` — read-only.
- Trait mutators: `deposit` / `withdraw` both gate `from.require_auth()` (`:190`, `:236`); `harvest`
  is ungated (logged as 1.2 — read-only, no fund/state impact).
- **No `set_controller`, no `admin`, no `upgrade`, no re-callable `initialize`** — grep over the file
  for those returns nothing beyond `__constructor`.

So there is no path for an external actor to mutate the strategy's controller/pool/asset binding,
re-initialize it, or upgrade its code. A third party *can* deploy their **own** strategy pointing at
our controller, but that strategy holds no elevated access — it transacts via the same public,
auth-gated controller entrypoints as any user (covered in iter 1.1/14.1 reasoning), so it is just
another account owner, not a privilege escalation.

Deployment note (not a vuln): the strategy has **no upgrade entrypoint**, so a discovered bug requires
redeploy + vault re-point rather than in-place patching — immutability here doubles as "no admin
backdoor," an acceptable trade for an open adapter.

**Verdict / defense.** Defended: the strategy exposes no admin/init/upgrade/setter surface; its
controller binding is fixed at deploy in a non-re-callable constructor and immutable thereafter, and
deposit/withdraw are `from`-auth-gated. No external config-hijack or re-init path. No POC (invalid).

---

## Iteration 34 — 2026-06-22

### 34.1 — Spoke per-asset param confusion: boosted LTV on a non-qualifying asset (over-borrow) — **NOT VALID**

**Assumption.** Spoke gives correlated assets boosted LTV/threshold. Risk params are now **per-asset**
(`SpokeAssetConfig`, the iter-context refactor). If an spoke account could supply/borrow/swap into an
asset that is *not* configured for its category and still receive the boosted params — or if a missing
per-asset config silently defaulted to a boosted value — an attacker could over-borrow against an
unqualified (uncorrelated, volatile) asset at spoke LTV, then walk away as it depegs.

**Analysis.** Resolution is fail-safe on two independent layers (`controller/src/spoke.rs`):

- **Entry gate rejects non-category assets.** `validate_spoke_lists_asset` (`spoke.rs:74-106`), called on
  `borrow` (`borrow.rs:83`) and `swap_collateral` (`swap_collateral.rs:140`) for any account with
  `spoke_id != 0`, asserts **both**: (a) the asset's market config
  `spokes.contains(category_id)`, and (b) `cached_spoke_asset(category_id, asset).is_some()`
  — else `SpokeNotFound`. So you cannot open spoke-account borrow/swap exposure to an asset
  that the category hasn't registered with its own per-asset config.
- **Param application requires the per-asset config — never defaults to boosted.**
  `apply_spoke_to_asset_config` (`spoke.rs:16-32`) overrides LTV/threshold/bonus **only** when
  `(category, asset_spoke_config)` are *both* `Some`; if the asset has no `SpokeAssetConfig` for the
  category (`None`), the override is skipped and the asset keeps its **base** (non-spoke, more
  conservative) config. A deprecated category (`cat.is_deprecated`) also short-circuits to base
  (and `ensure_spoke_not_deprecated` blocks new exposure under a deprecated category).
- **Mixed positions are valued per-asset.** `effective_asset_config` (`spoke.rs:35-46`) resolves each
  asset independently (base, then boost only if its per-asset config exists), so an spoke account
  holding a mix is valued with boosted params on category assets and base params on any other — never
  an over-credit.

So the only way to get boosted params is for governance to have explicitly registered the asset in the
category with an `SpokeAssetConfig`; absent that, the asset is rejected at entry (borrow/swap) or
valued at conservative base params (valuation) — never wrongly boosted.

**Verdict / defense.** Defended by the dual `validate_spoke_lists_asset` assert (asset must be
category-registered *and* have a per-asset config) at borrow/swap entry, plus
`apply_spoke_to_asset_config` applying the boost only when the per-asset config is present
(else base) and skipping deprecated categories. No boosted-LTV-on-unqualified-asset over-borrow. No
POC (invalid).

---

## Iteration 35 — 2026-06-22

### 35.1 — Hub-cap bypass: exceed the asset-global supply/borrow ceiling (hub-and-spoke) — **NOT VALID**

**Assumption.** Spoke is a hub-and-spoke model: per-category **spoke** caps (verified enforced on
normal + leverage paths, iter 6.2/20.1) sit under an asset-global **hub** cap. If the hub cap were
only a governance-config value with no runtime enforcement — or enforced only on some paths — an
attacker could push an asset's *aggregate* supply/borrow past the protocol-wide ceiling (concentration
risk the cap exists to prevent), or use a path that skips it.

**Analysis.** The hub cap is the **pool's per-asset market `supply_cap`/`borrow_cap`**, enforced by
the **pool on every supply/borrow** — beneath *all* controller flows:

- `enforce_supply_cap` (`pool/utils.rs:48-60`) and `enforce_borrow_cap` (`:64-76`) compare the new
  value-scaled total against the market cap (`Ray::from_asset(cap, decimals)`) and revert
  `SupplyCapReached` / `BorrowCapReached` past it; `cap_is_enabled` lets governance disable (0 =
  unlimited). These run inside the pool's core supply/borrow (`pool/src/lib.rs:116` / `:106`).
- **Universal coverage.** The controller never books supply/borrow except via the pool, so *every*
  flow — plain supply/borrow, spoke, and the leverage/strategy legs (multiply/swap/migrate, which
  go through `create_strategy` → same pool accounting, iter 20.1) — passes through the hub-cap check.
  There is no controller path that mutates supplied/borrowed without the pool enforcing the market
  cap.
- **Two-tier consistency invariant.** `validate_spoke_caps_against_hub` (`spoke_caps.rs:271-289`)
  asserts each `spoke_cap ≤ hub_cap` at governance config time, and
  `validate_hub_caps_against_category_spokes` (`:224`) the converse, so a spoke can never be
  configured to exceed its hub. `apply_hub_caps` (`pool/utils.rs:100`) is set via owner-gated
  `update_caps` (`lib.rs:531` = governance).
- The `limits.rs` views compute `max_supply`/`max_borrow` as `min(hub_headroom, spoke_headroom, …)`,
  so integrators see the binding ceiling.

So aggregate per-asset exposure is bounded by the pool-enforced hub cap on every op, with per-category
spoke sub-caps nested beneath it and a config invariant keeping spoke ≤ hub. No path bypasses the hub
cap; no actor can exceed the asset-global ceiling.

**Verdict / defense.** Defended: the hub (asset-global market) cap is enforced by the pool's
`enforce_supply_cap`/`enforce_borrow_cap` on every supply/borrow — beneath all controller flows
including leverage — with spoke caps nested under it (`spoke ≤ hub` validated at config time). This
completes the cap picture (spoke-on-leverage 20.1, spoke-drift 6.2, spoke-exhaustion 25.1, per-asset
config 34.1, hub enforcement here). No POC (invalid).

---

## Iteration 36 — 2026-06-22

### 36.1 — Malfunctioning feed: zero / negative price injected into valuation — **NOT VALID**

**Assumption.** Iter 8 covered *manipulated but valid* prices. This is the *invalid* case: a feed
(Reflector/RedStone) glitches and returns **0 or negative**. If that reached valuation it could
zero-out collateral (enabling weird liquidation/borrow states), feed a div-by-zero where a price sits
in a denominator, or produce negative USD values that corrupt the HF/LTV math.

**Analysis.** The price resolver rejects non-positive prices **unconditionally, before use**
(`oracle/price.rs`):

- **Universal positive-price assert (all policies):**
  `assert_with_error!(resolved.final_price_wad > 0, OracleError::InvalidPrice)` (`price.rs:41-45`)
  runs for **every** `OraclePolicy` — RiskIncreasing, RiskDecreasing, Repay, View, Liquidation —
  *before* the feed is cached or returned (`:54-61`). A zero or negative *composed* price (after the
  midpoint blend / fallback of iter 8) **always reverts**. So no flow — not even the permissive
  risk-decreasing/view ones — can read a non-positive price. No zero-collateral state, no negative
  valuation, no div-by-zero.
- **Guard is on the final composed price.** `resolved.final_price_wad` is the post-composition value,
  so it catches both a single source reporting ≤0 and any blend that nets to ≤0.
- **Sanity band on top (positive but absurd):** for risk-increasing/liquidation
  (`!allows_sanity_violation`), the price must additionally lie in `[min_sanity, max_sanity]` with
  `max_sanity > 0`, else `SanityBoundViolated` (`price.rs:47-53`). Risk-decreasing/repay/view tolerate
  out-of-band *positive* prices (per the policy invariant they can only reduce risk, iter 8) — but the
  `> 0` floor still binds them.
- **Self-pointer rejected too:** a config whose primary source == the priced asset reverts
  `OracleNotConfigured` (`price.rs:30-39`), closing a degenerate mis-config.

So a glitching feed cannot inject a 0/negative/out-of-band price into any value-extracting flow; the
read reverts (fail-closed) rather than producing a corrupt valuation.

**Verdict / defense.** Defended by the unconditional `final_price_wad > 0` → `InvalidPrice` assert
(all policies, pre-use) plus the policy-gated sanity band for risk-increasing/liquidation. No
zero/negative/absurd price reaches HF/LTV/seizure math. No POC (invalid; a bad feed reverts the read).

---

## Iteration 37 — 2026-06-22

### 37.1 — Flash-loan repayment bypass: receiver keeps the loan / repays nothing — **NOT VALID**

**Assumption.** A flash loan pays out `amount`, calls the receiver, and must be repaid `amount + fee`
in the same tx. Iter 5.2 covered reentrancy and iter 24 the fee derivation; this is the
**repayment-verification mechanism** itself. Since `transfer_amount` doesn't measure received funds
(SAC-only, iter 11), if the pool trusted the receiver's report — or checked its *internal* cash
accounting instead of the real token balance — a malicious receiver could take `amount`, repay
nothing, and walk (free flash loan → pool drain).

**Analysis.** The pool's `flash_loan` (`pool/src/lib.rs:330-399`) verifies against **actual SAC token
balances/allowance**, never a receiver report or internal accounting:

- **Real `token::Client` reads.** It instantiates `tok = token::Client::new(asset_id)` and reads
  `tok.balance(pool)` directly (`:350-351`) — the on-chain SAC balance, not the pool's internal
  `cash`. So the `transfer_amount`-doesn't-measure gap (iter 11) does not apply here.
- **Reserve gate + WASM receiver.** `require_reserves(amount)` (`:344`) and `require_wasm_receiver`
  (`:345`) precede payout.
- **Payout bracketed by an exact-balance assert.** After `tok.transfer(pool→receiver, amount)`
  (`:362`), it asserts `tok.balance(pool) == pre_balance - amount` (`:364-368`, else
  `InvalidFlashloanRepay`).
- **Callback can't move the pool balance.** After invoking `execute_flash_loan` (`:370-382`), it
  re-asserts `tok.balance(pool) == expected_after_payout` (`:385-389`) — the callback "must not retain
  funds or change the pool balance again."
- **Repayment is pulled, not trusted.** It checks `tok.allowance(receiver, pool) >= amount + fee`
  (`:393-397`) then `transfer_from(pool, receiver→pool, amount+fee)` (`:398-399`). The
  `transfer_from` pulls from the *receiver's* real balance — if the receiver lacks funds or approval,
  the SAC call reverts → `InvalidFlashloanRepay` → whole tx rolls back (no payout sticks). The
  balances are per-(token, holder), so repayment with any *other* vault asset is inert (`:347-348`).

So a receiver that fails to repay cannot keep the loan: either the allowance/`transfer_from` reverts,
or the balance asserts fail — both revert the entire transaction atomically, returning the paid-out
`amount`. No trust in receiver-reported repayment; verification is on real token holdings.

**Verdict / defense.** Defended by actual SAC `balance()`/`allowance()` reads (not internal
accounting), exact-balance asserts bracketing the callback, and allowance-gated `transfer_from`
pull of `amount + fee` — a non-repaying receiver reverts the whole tx. Combined with the reentrancy
guard (5.2) and config-derived fee (24), the flash-loan path is fully closed. No POC (invalid).

---

## Iteration 38 — 2026-06-22

### 38.1 — Direct-to-pool bypass: call a pool mutator directly, skipping controller gates — **NOT VALID**

**Assumption.** The security model assumes the controller is the *sole* gateway: it enforces
`require_auth`, owner-match (12.1 aside), HF/LTV solvency, oracle policy, spoke, caps, and the
reentrancy guard, then calls the pool to execute. If **any mutating pool entrypoint were public** (not
owner-gated), an external actor could invoke the pool **directly** — minting/moving supply, borrowing,
seizing, or bumping indexes with *none* of those checks. That would collapse the entire model.

**Analysis.** Enumerated the pool's complete `#[contractimpl]` surface (`pool/src/lib.rs`). **Every
mutating entrypoint is `#[only_owner]`** (owner = the controller, fixed in `__constructor(admin)` at
`:221`):

- `create_market` (`:230`), `supply` (`:265`), `borrow` (`:271`), `withdraw` (`:282`), `repay`
  (`:294`), `update_indexes` (`:299`), `add_rewards` (`:306`), `flash_loan` (`:330`),
  `create_strategy` (`:420`), `seize_positions` (`:466`), `claim_revenue` (`:495`), `update_params`
  (`:519`), `update_caps` (`:531`), `upgrade` (`:540`) — **all carry `#[only_owner]`**.
- The only **un-gated** entrypoints are **read-only views**: `get_utilisation`, `get_reserves`,
  `get_deposit_rate`, `get_borrow_rate`, `get_revenue`, `get_supplied_amount`, `get_borrowed_amount`,
  `get_delta_time`, `get_sync_data`, `get_bulk_indexes` (`:545-581`). These return values; they mutate no
  state. (`get_bulk_indexes` is the simulation the controller reads for index caching — read-only.)

`#[only_owner]` resolves `owner.require_auth()` where owner = the controller contract, satisfiable
only when the controller itself is the invoker. An external address (EOA or contract) calling any
pool mutator fails the owner gate and reverts. So there is **no direct-to-pool write path**: every
state change is funneled through the controller, where the full auth/risk/oracle/cap/reentrancy stack
runs first. This is the foundation under iters 4 (timelock), 21 (deploy), 35 (caps), 37 (flash) —
all rely on "pool mutators are controller-only," now enumerated and confirmed exhaustively.

**Verdict / defense.** Defended: 100% of mutating pool entrypoints are `#[only_owner]` (=controller);
only read-only views are public. No external actor can mutate pool state directly or bypass the
controller's gates — the controller is the verified sole gateway. No POC (invalid).

---

## Iteration 39 — 2026-06-22 · Clippy gate (verification bar completion)

Ran `cargo clippy` on the four in-scope contracts (`controller`, `pool`, `governance`,
`defindex-strategy`; no `--all-features` per the certora-linking constraint) to complete the
verification bar alongside the test suites (iters 22/26/27):

- `cargo clippy -p controller -p pool -p governance -p defindex-strategy` → **clean, 0 warnings /
  0 errors**.

So the in-scope contracts pass lint with no findings, complementing the green test suites. The log's
defense claims now rest on a base that is both **fully tested** (controller 329, pool 113 lib + 69
harness, governance 69 lib + 57 harness, defindex-strategy 15 — the iter-27 stale pool harness test
is now fixed) and **lint-clean**.

---

### Review-completion note (39 iterations)

The external-actor attack-surface review is **complete**. Coverage, by area:

- **Auth / gateway:** every controller entrypoint's tier (4, 12, 21, 38); pool 100% owner-gated on
  all 14 mutators (38); strategy full surface, no admin path (33).
- **Governance / timelock:** propose=PROPOSER, execute=open-by-design, cancel=CANCELLER (4, 29);
  pause & market-deactivation freeze entry but never trap exits (31, 32); deploy one-time + ownership
  (21).
- **Value math:** floor-collateral/ceil-debt solvency rounding (13), round-trip rounding (10),
  accrual overflow/gas (9), index inflation (2).
- **Oracle:** manipulation/divergence fail-closed (8); invalid (zero/negative/out-of-band) prices
  rejected (36).
- **spoke caps:** spoke drift (6.2), leverage-path enforcement (20), exhaustion (25), per-asset
  config (34), hub enforcement + spoke≤hub (35).
- **Flash / strategy / migration:** reentrancy (5.2), fee derivation (24), repayment verification
  (37), aggregator extraction (6.1), Blend migration (7), strategy NAV (14), return semantics (16).
- **Liquidation:** threshold-grief (3), same-asset netting (23), repeated-call over-seize +
  self-liquidation block (30).
- **Misc:** dust account spam (5.1), bulk aggregation (15), token-trust (11), bad-debt socialization
  (17), rewards funding + empty-market (27, 28).

**Outcome:** no fund-theft path. 4 VALID findings, all LOW griefing/asymmetry (1.2, 5.1, 12.1, 14.1),
each POC-backed and green. 16.1 resolved benign against the DeFindex spec. Everything else is a
defended mechanism or a documented governance-trust boundary. Verification base: tests green + clippy
clean. Action items unchanged (top-of-file executive summary) + the iter-27 stale-test hygiene fix.
Subsequent loop iterations re-verify and watch for code changes rather than manufacture new findings.

---

## Iteration 40 — 2026-06-22

### 40.1 — Liquidator collateral cherry-picking (adverse selection → protocol bad-debt) — **NOT VALID**

**Assumption.** When a liquidated account holds *multiple* collateral assets, if the **liquidator
chose which collateral to seize**, they would grab the most liquid/valuable assets and leave the
illiquid/volatile ones behind. Repeated across liquidations this is adverse selection: the protocol's
residual positions concentrate bad collateral, raising systemic bad-debt risk — and the liquidator
front-runs everyone to the good assets.

**Analysis.** Seizure is **protocol-computed and strictly proportional across the whole collateral
basket** — the liquidator has no collateral-selection input. `calculate_seized_collateral`
(`liquidation_math.rs`) iterates **every** supply position and seizes each pro-rata by its USD share:

```
for (asset, position) in iter_typed_positions(&account.supply_positions) {
    let asset_value = actual_amount_wad.mul(env, feed.price);
    let share = asset_value.div(env, total_collateral);          // this asset's fraction
    let seizure_for_asset_usd = total_seizure_usd.mul(env, share); // proportional slice
    ...
    let capped_ray = seizure_ray.min(actual_ray);                 // never exceed the position
}
```

- The `liquidate` entrypoint takes only **debt payments** (which debt to repay); the collateral seized
  is *derived*, not chosen. So a liquidator cannot target a single collateral asset.
- Every collateral asset is reduced by the **same proportion** (`total_seizure_usd / total_collateral`),
  so the borrower's *post*-liquidation collateral mix is unchanged — the liquidator cannot skim the
  liquid assets, and the protocol never accumulates a worse residual basket than it started with.
- Per-asset seizure is capped at the position's actual balance (`seizure_ray.min(actual_ray)`), with
  floor/half-up rounding discipline (base floored, bonus absorbs remainder, protocol fee on bonus,
  full-seizure half-up to match the pool's full-close dust gate) — so no over-seize on any single
  asset either (complements iter 3/30).

So there is no cherry-pick channel: a liquidator takes a proportional slice of *all* collateral or
none. Adverse selection on collateral quality is structurally impossible.

**Verdict / defense.** Defended by proportional, protocol-computed multi-collateral seizure
(`share = asset_value / total_collateral` over all positions, capped at each position), with the
liquidator choosing only the debt to repay — not the collateral. No cherry-picking, no bad-debt
concentration. No POC (invalid).

---

## Iteration 41 — 2026-06-22

### 41.1 — Utilization-lock / liquidity-squeeze: borrow to max-util to strand suppliers — **NOT VALID (by-design lending property; LOW soft-DoS residual)**

**Assumption.** A well-capitalized attacker borrows up to the utilization ceiling so the pool's cash
is lent out, then suppliers wanting to exit find withdrawals revert for lack of liquidity — a soft
bank-run DoS that traps suppliers until the attacker chooses to repay.

**Analysis.** The squeeze is real *as a transient liquidity state* but bounded, capital-bonded, and
self-correcting — the inherent illiquidity property of every utilization-based lending market (Aave,
Compound), not a contract defect:

- **Hard utilization ceiling on borrows.** `require_utilization_below_max` (`pool/utils.rs:118`) gates
  every borrow (`lib.rs:109`) against `max_utilization` (default `RAY*95/100` = 95%), and
  `require_reserves` (`lib.rs:104`) ensures cash for the payout. So borrows can never reach 100%
  utilization — a buffer below the cap is always reserved.
- **Withdrawals are also utilization-gated**, not just reserve-gated: withdraw calls both
  `require_reserves(net_transfer)` (`lib.rs:169`) and `require_utilization_below_max` (`lib.rs:176`).
  This means a *marginal* supplier can be blocked while utilization sits at the cap — that's the
  squeeze — but it is transient, not a seizure of funds.
- **Capital-bonded & expensive.** Sustaining high utilization means the attacker holds a large borrow
  paying the **steep above-optimal rate** (the slope-3 kink, iter 9) continuously; the cost rises the
  longer they hold it.
- **Self-correcting.** That same elevated rate (a) pays suppliers more → attracts fresh supply that
  *lowers* utilization and reopens withdrawals, and (b) pressures the attacker to repay. Any repay or
  new supply immediately frees the marginal withdrawal.
- **Governance-tuned.** `max_utilization` and the rate-curve kinks are governance parameters sized to
  balance capital efficiency against withdrawal availability.

So no funds are lost or seized; at worst a supplier waits out a transient high-utilization window that
is costly for the attacker to maintain and self-resolves as the rate draws in supply. This is the
universal lending-liquidity tradeoff, the same class as the cap-exhaustion residual (25.1).

**Verdict / defense.** Not a bug: borrows are bounded below 100% by `require_utilization_below_max` +
`require_reserves`; the squeeze is a transient, capital-bonded, self-correcting liquidity state
inherent to utilization-based lending, governance-tuned via `max_utilization` and the rate curve.
Residual: a **LOW** transient withdrawal-availability soft-DoS for marginal suppliers at peak
utilization (no fund loss). No POC (by-design liquidity behavior, not a reproducible defect).

---

## Iteration 42 — 2026-06-22

### 42.1 — Dust-debt griefing: open tiny debt positions uneconomical to liquidate → bad-debt accumulation — **NOT VALID**

**Assumption.** Iter 5.1 found supply has no min-deposit floor (dust account spam). If **borrow**
likewise had no minimum, an attacker could open many tiny debt positions whose absolute size is below
what a keeper/liquidator would spend in gas to liquidate. Left unliquidated as they drift underwater,
they'd accumulate as protocol bad debt (socialized to suppliers) — and clutter the liquidation set.

**Analysis.** Borrowing **does** enforce a minimum-collateral floor, unlike supply. The post-borrow
solvency gate (`controller/src/validation.rs:83-92`, run on every risk-increasing flow) asserts both:

```
assert!(totals.health_factor >= Wad::ONE, InsufficientCollateral);
let floor = storage::get_min_borrow_collateral_usd_wad(env);
if floor != 0 && totals.ltv_collateral.raw() < floor {
    panic_with_error!(env, CollateralError::MinBorrowCollateralNotMet);
}
```

- Any account carrying debt must hold **LTV-weighted collateral ≥ `min_borrow_collateral_usd_wad`**
  (a governance USD floor). A dust borrow needs only dust collateral, which fails the floor →
  `MinBorrowCollateralNotMet` → revert. So **sub-threshold debt positions cannot be created**.
- This guarantees every borrowing position is economically meaningful: large enough that a
  proportional liquidation (iter 40) yields a worthwhile seizure, keeping positions liquidatable and
  preventing dust-debt bad-debt build-up.
- **Resolves the iter-5.1 asymmetry as intentional:** dust *supply-only* accounts are permitted (5.1)
  but are **debt-free** → HF saturated → never liquidatable, never bad debt (only the bounded
  state-bloat/keeper-scan griefing of 5.1). Dust *debt* — the genuinely dangerous case — is blocked
  by this floor. So the protocol gates the floor exactly where bad-debt risk lives (borrowing), not
  where it doesn't (supplying).

**Verdict / defense.** Defended by the `min_borrow_collateral_usd_wad` floor in the post-borrow
solvency gate (`validation.rs:89-92`): a borrowing account must hold ≥ the governance USD collateral
minimum, so no dust/uneconomical debt position can be opened and accumulate as bad debt. (Disabled
only if governance sets the floor to 0.) No POC (invalid).

---

## Iteration 43 — 2026-06-22

### 43.1 — Position-mode confusion: mix entrypoints on a leverage account to corrupt accounting — **NOT VALID**

**Assumption.** Accounts carry a `PositionMode` (Normal / Multiply / Long / Short). If an attacker
opened a leverage account in one mode and then operated on it via an entrypoint assuming a different
mode — or mixed plain `borrow`/`withdraw` with a strategy account — mismatched mode handling might
mis-account collateral/debt, grant different risk params, or bypass a solvency gate.

**Analysis.** `mode` is **descriptive metadata, not an accounting/privilege switch**, and the one
cross-mode operation is explicitly gated:

- **Mode-mismatch gate.** A multiply/strategy op on an *existing* account asserts
  `account.mode == mode` → `GenericError::AccountModeMismatch` (`strategies/multiply.rs:232`). So you
  cannot, e.g., graft a Short leg onto a Long account or drive a Multiply account with a mismatched
  mode in one strategy call — the directional intent is pinned to the account.
- **Accounting is mode-independent.** `mode` appears in only three places (grep-confirmed): event
  labeling (`events.rs`, `EventPositionMode`), account creation/storage (`helpers/account.rs:24`),
  and the mismatch assert above. It is **absent from the HF/LTV risk math** (`helpers/math.rs`),
  liquidation, oracle, and cap logic. Collateral, debt, health factor, seizure, and caps are computed
  from the *positions themselves* (supply/debt entries + oracle prices), identically regardless of the
  mode label.
- **So mixing entrypoints is harmless.** Even if a user opens a Multiply account and later calls plain
  `withdraw`/`repay`/`supply` on it, each flow runs the same owner-match, solvency gate
  (`HF ≥ 1` + min-borrow floor), oracle policy, and cap checks. The worst outcome is the mode *label*
  becoming descriptively stale (an indexer/UI cosmetic), never an accounting corruption, mis-valuation,
  or gate bypass. No mode confers boosted params or skips a check.

**Verdict / defense.** Defended: directional strategy ops are pinned by the `AccountModeMismatch`
assert, and `PositionMode` is event/label metadata that never enters risk, liquidation, oracle, or cap
math — so entrypoint mixing cannot corrupt value accounting or bypass solvency. No POC (invalid).

---

## Iteration 44 — 2026-06-22

### 44.1 — Oracle price-cache policy poisoning: lax-validated price reused by a strict solvency gate — **NOT VALID**

**Assumption.** Prices are cached per-tx in `prices_cache` and validated against the *current*
`OraclePolicy` (strict sanity/staleness for risk-increasing/liquidation; lax for risk-decreasing,
iter 8/36). If a strategy flow resolved/cached a price under a **permissive** policy (e.g.
RiskDecreasing for a prefetch) and the final HF gate then **reused that cached price**, the solvency
check would accept a price that never passed the strict risk-increasing sanity gate — a policy bypass
enabling over-leverage on a manipulated/out-of-band price.

**Analysis.** The policy is **fixed for the whole tx per `Cache`**, and the finalize gate reuses the
*same* Cache — so there is no cross-policy reuse, and the lax policy is only ever used where the
solvency gate is vacuous:

- **One policy per Cache, shared with finalize.** Each entrypoint constructs a single
  `Cache::new(env, policy)` and threads it through mutation *and* `strategy_finalize`
  (`require_post_pool_risk_gates`). Strict throughout: `multiply` RiskIncreasing (`multiply.rs:111`
  → finalize `:161`), `migrate_from_blend` RiskIncreasing (`:124`→`:188`), `repay_debt_with_collateral`
  RiskIncreasing (`:79`→`:122`). The HF gate therefore reads prices validated under RiskIncreasing.
- **`swap_collateral`'s conditional policy is debt-gated and sound** (`swap_collateral.rs:78-83`):
  `RiskDecreasing` **only** `if account.borrow_positions.is_empty()`, else `RiskIncreasing`. A
  debt-free account has a saturated/∞ health factor — the finalize HF≥1 gate is *vacuously* satisfied
  regardless of collateral price — so the lax policy there can bypass nothing (there is no solvency to
  violate). The instant the account has debt, the strict policy applies.
- **Lax policy only where HF is moot.** RiskDecreasing is used by `flash_loan` (`:44`, no borrower
  position change to gate) and the debt-free `swap_collateral` branch — never by a flow that must
  prove HF≥1 against existing debt.
- **Cache-write ordering is safe.** Prices are stored only *after* passing the policy's own sanity
  gate (iter 36, `price.rs:60` post-`:47-53`), and the policy is constant within the Cache, so a
  price validated under a lax policy is never consumed by a strict gate within the same tx — they are
  the same policy.

So no value-extracting solvency check ever reads a price that skipped its strict validation; the
permissive policy is confined to contexts where solvency is unaffected.

**Verdict / defense.** Defended: one fixed `OraclePolicy` per `Cache`, shared by the finalize HF gate;
strict (RiskIncreasing) wherever debt/solvency is at stake (including `swap_collateral` once debt
exists), lax (RiskDecreasing) only where the HF gate is vacuous (flash loan, debt-free collateral
swap). No cross-policy cache poisoning. No POC (invalid).

---

## Iteration 45 — 2026-06-22

### 45.1 — Governance takeover via role self-grant (PROPOSER/EXECUTOR/CANCELLER escalation) — **NOT VALID**

**Assumption.** The whole timelock model rests on the PROPOSER/EXECUTOR/CANCELLER roles (iter 4, 29).
If `grant_role`/`revoke_role` were permissionless or weakly gated, an external actor could grant
*itself* PROPOSER (to schedule arbitrary ops) and CANCELLER (to block legit ones) — an instant
governance takeover that sidesteps the 48h timelock entirely. Iter 4 verified proposers are
PROPOSER-gated in general; this targets the **role-management** entrypoints specifically (the
takeover-relevant ones).

**Analysis.** Role grant/revoke are **not** standalone owner setters or public calls — they are
**timelocked self-operations**, and even scheduling one requires the PROPOSER role:

- **Exposed only as timelocked self-ops.** `propose_grant_governance_role` →(delay)→
  `execute_grant_governance_role`, and the revoke pair, generated by the `self_timelock_ops!` table
  (`self_timelock.rs:167-175`) which calls the internal `apply_grant_role`/`apply_revoke_role`
  (`access.rs:129-147`). There is no immediate public `grant_role` entrypoint.
- **Propose is PROPOSER-gated.** Every self-op proposer routes through `begin_proposal`
  (`self_timelock.rs:24-27`): `proposer.require_auth()` + `access_control::ensure_role(PROPOSER_ROLE,
  proposer)`. An address without the PROPOSER role cannot even *schedule* a role grant (test
  `non_proposer_cannot_propose_*`). And PROPOSER is itself only obtainable through this same gated,
  timelocked path — there is no bootstrap an outsider can pull.
- **Execution is delayed.** The grant only applies after the (sensitive-tier) timelock delay via the
  execute half — no instant effect, leaving a 48h window for the CANCELLER to abort a malicious
  schedule.
- **Separation of duties.** `apply_grant_role` enforces `require_executor_canceller_separation`
  (`access.rs:112-127`): no single delegate can hold both EXECUTOR and CANCELLER, so even a granted
  delegate can't both force-run and block ops.
- **`grant_role_no_auth` is internal-only.** Used solely by the constructor (`access.rs:156`),
  ownership-sync recovery (`:76,133`), and post-timelock `apply_*` — never reachable as a public
  unauthenticated entrypoint.

So an external actor cannot self-assign a governance role: they lack PROPOSER to schedule it, the
action is timelocked and cancellable, and role pairs are separated. No instant takeover.

**Verdict / defense.** Defended: role grant/revoke are PROPOSER-gated, timelocked self-ops
(`begin_proposal` require_auth + `ensure_role(PROPOSER)`, then delayed execute), with
EXECUTOR/CANCELLER separation and `grant_role_no_auth` confined to internal/recovery paths. Completes
the governance auth map (propose=PROPOSER, execute=delayed, cancel=CANCELLER, pause=owner,
role-mgmt=PROPOSER-gated timelocked self-op, ownership=two-step). No POC (invalid).

---

## Iteration 46 — 2026-06-22

### 46.1 — Timelock-delay weakening: shorten/zero `min_delay` to neuter the timelock — **NOT VALID**

**Assumption.** The 48h timelock is the core protection on every privileged change. The classic
governance escalation is to **attack the timelock itself**: propose "set `min_delay` → 0" (or a tiny
value), wait out the *current* delay once, execute, and thereafter run arbitrary ops with no delay —
collapsing the timelock. If `update_delay` allowed shortening, a single patient malicious-PROPOSER
action would defeat all the timelock defenses verified in iters 4/45.

**Analysis.** The delay is a **one-way upward ratchet** — it can be maintained or lengthened, never
shortened — and the change is itself fully timelocked:

- **Monotonic non-decreasing guard.** `validate_delay_update` (`timelock.rs:49-57`) asserts
  `new_delay >= current && new_delay <= TIMELOCK_MAX_DELAY_LEDGERS`, plus `require_nonzero_delay`
  (`new_delay >= 1`). So a proposed delay below the current value (including 0) is rejected
  `InvalidTimelockDelay`. The timelock window can only stay the same or grow, never shrink.
- **Enforced at propose *and* execute.** The constraint runs at propose time (the
  `self_timelock_ops!` `validate:` hook, `self_timelock.rs:164`) and again at execute time
  (`apply_update_delay` → `validate_delay_update`, `timelock.rs:59-62`). Even if `current` changed
  between propose and execute, re-validation preserves the no-shortening invariant.
- **The change is a sensitive-tier, PROPOSER-gated, timelocked self-op.** `propose_update_delay`
  carries `delay: sensitive` (`self_timelock.rs:158/162`) and routes through `begin_proposal`
  (PROPOSER role, iter 45), so only a proposer can even schedule it, and it takes the *longer*
  sensitive delay to take effect.
- **Bounded above** by `TIMELOCK_MAX_DELAY_LEDGERS` (≤14 days), so it also can't be set absurdly high
  to brick governance.
- Tests confirm: `propose_update_delay_rejects_shortening`, `_rejects_zero`, `_rejects_above_max_cap`,
  `execute_update_delay_applies_after_delay`.

So no actor — not even a compromised PROPOSER — can shorten or zero the timelock to bypass it; the
protective delay is irreversible downward.

**Verdict / defense.** Defended: `validate_delay_update` enforces `new_delay ≥ current` (no
shortening), `≥ 1`, and `≤ 14d`, at both propose and execute, for a sensitive-tier PROPOSER-gated
timelocked self-op. The timelock window is a one-way upward ratchet and cannot be neutered. No POC
(invalid).

---

## Iteration 47 — 2026-06-22

### 47.1 — Min-borrow-collateral floor bypass via withdraw (re-create dust debt post-borrow) — **NOT VALID**

**Assumption.** Iter 42 showed `borrow` enforces a `min_borrow_collateral_usd_wad` floor, blocking
dust-debt creation. But if `withdraw` only re-checked `HF ≥ 1` (not the floor), a borrower could open
above the floor, then **withdraw collateral down to just above HF=1 but below the floor** — leaving a
sub-floor debt position (uneconomical to liquidate → bad-debt risk), defeating 42 through a different
entrypoint.

**Analysis.** `withdraw` routes through the **same** solvency gate as `borrow`, which re-enforces the
floor — so the floor cannot be circumvented post-borrow:

- `withdraw` calls `validation::require_post_pool_risk_gates` (`withdraw.rs:80`) — the identical gate
  used by the strategy/borrow paths (`validation.rs:58`).
- For any account **with debt**, that gate asserts (`validation.rs:63-92`): `ltv_collateral ≥
  total_debt`, `health_factor ≥ ONE`, **and** the floor — `if floor != 0 && ltv_collateral < floor →
  MinBorrowCollateralNotMet` (`:89-91`). So a withdrawal that would drop a debt-bearing account's
  LTV-weighted collateral below the floor reverts, even if HF would still be ≥ 1. No sub-floor debt
  position can be produced by withdrawing.
- **Debt-conditioned, no false positives.** The gate early-returns when
  `account.borrow_positions.is_empty()` (`:59-61`), so the floor applies *only* to debt-bearing
  accounts — debt-free users can withdraw down to dust freely (harmless; iter 42/5.1). The floor
  guards exactly where bad-debt risk exists (open debt), on both the entry (borrow) and exit
  (withdraw) sides.

So 42's dust-debt prevention is an invariant across entrypoints, not a borrow-time-only check: every
flow that could change a debt-bearing account's collateral/debt re-runs the floor.

**Verdict / defense.** Defended: `withdraw` (and the strategy collateral-moving flows) share
`require_post_pool_risk_gates`, which re-enforces `MinBorrowCollateralNotMet` for any debt-bearing
account — so the min-borrow-collateral floor can't be bypassed by withdrawing collateral after
borrowing. Debt-free accounts are exempt (early return). No POC (invalid).

---

## Iteration 48 — 2026-06-22

### 48.1 — Position-limit bypass via strategy/leverage flows (unbounded positions → HF-loop gas blowup) — **NOT VALID**

**Assumption.** Per-account `max_supply_positions`/`max_borrow_positions` cap how many distinct asset
positions an account holds — bounding the cost of `calculate_account_risk_totals`, which loops over
every position. Direct `supply`/`borrow` enforce the cap (iter 5.1/12.1). But if the **strategy
flows** (`multiply`, `swap_collateral`, `swap_debt`, `migrate_from_blend`) added positions without
the check, an attacker could pack an account past the cap, making its HF computation (run on every
op, including liquidation) so expensive it hits the instruction limit — an unliquidatable/bricked
account (and bad-debt risk).

**Analysis.** Every position-*adding* path funnels through a checkpoint that enforces
`validate_bulk_position_limits` (`validation.rs:95` → `PositionLimitExceeded`):

- **Deposit side — `process_deposit` (`supply.rs:110`)** is the shared deposit routine used by
  *normal* supply **and every strategy deposit leg** (multiply, swap_collateral, migrate's
  `deposit_withdrawn`). It calls `validate_deposit` *and* `validate_bulk_position_limits`
  (`supply.rs:130`) before crediting. So no strategy can add a supply position past the cap.
- **Borrow side — `validate_borrow`** runs `validate_bulk_position_limits` for the Borrow side, and
  the strategy borrow legs route through `borrow_strategy_inner` → `validate_borrow` (iter 20.1). So
  leverage borrows are capped identically to plain borrows.
- **swap_collateral** adds an explicit `validate_bulk_position_limits` preflight (`:149`) for the
  incoming collateral on top of `process_deposit`.
- **Dedup-safe:** the limit check counts deduped assets (iter 15), so a crafted bulk list with
  repeated assets can't inflate or evade the count.

So the position cap is an invariant across *all* entrypoints — direct and strategy — exactly mirroring
how the spoke caps (20.1) and the min-borrow floor (47.1) hold on the leverage paths. An account
cannot be packed beyond the cap, so HF iteration stays bounded and liquidation stays affordable.

**Verdict / defense.** Defended: all position-adding flows (supply, borrow, multiply, swap_*, migrate)
funnel through `process_deposit`/`validate_borrow`, both of which enforce
`validate_bulk_position_limits` (with swap_collateral adding an explicit check). No leverage-path
bypass of `max_supply_positions`/`max_borrow_positions`; HF-loop cost stays bounded. No POC (invalid).

---

## Iteration 49 — 2026-06-22

### 49.1 — Valuation overflow at extreme scale: brick HF math via `amount × price` i128 overflow — **NOT VALID**

**Assumption.** Collateral/debt USD value is `amount_wad × price_wad` (then scaled). For a whale
position in a high-price asset (e.g. a very large BTC holding at ~$100k), this product could exceed
i128 (~1.7e38). If the multiply overflowed and panicked, the account's `calculate_account_risk_totals`
(run on *every* op including liquidation) would always revert — an **un-liquidatable, bricked account**
accruing bad debt; if it silently wrapped, HF would be corrupt (free over-borrow). Iter 9 covered
*accrual* overflow; this is *valuation* overflow.

**Analysis.** Every fixed-point multiply is **I256-intermediate with checked narrowing**, so neither
overflow mode is reachable, and caps bound magnitude well below the limit anyway:

- **I256 intermediates.** `Wad`/`Ray` `mul` route through `mul_div_half_up` (and floor/ceil variants)
  in `common/src/math/fp_core.rs`, which `to_i256_operands` — widens **all three operands to I256**
  (`I256::from_i128`) — computes `(x·y)/d` in I256 (range ~±5.8e76), then narrows. So the
  intermediate `amount_wad · price_wad` (before the `/WAD` scale) **cannot overflow at the multiply**;
  i128 is exceeded only transiently inside I256, which holds it.
- **Checked narrowing, never wraps.** The final `to_i128` is checked — if the *result* exceeds i128 it
  `panic_with_error!(MathOverflow)` (revert), never silently wraps. So a corrupt-HF / free-over-borrow
  outcome is impossible; the worst case is a clean revert.
- **The result ceiling is unreachable.** The valuation result is the USD value in WAD; i128 caps it at
  ~1.7e38 / 1e18 ≈ **1.7e20 USD** — ~170 quintillion dollars, vastly beyond global wealth, let alone a
  single position. No real position approaches it.
- **Caps bound it further.** Hub/spoke supply & borrow caps (iter 35) limit per-asset position size to
  governance-set values orders of magnitude below the overflow threshold, so the revert branch isn't
  even reachable in practice.

So a position cannot be made large enough to overflow its own valuation; if one somehow were, the
computation reverts cleanly (not corrupts), and the caps prevent reaching that size at all.

**Verdict / defense.** Defended by I256-intermediate fixed-point muls (`fp_core.rs` `mul_div_half_up`)
with checked `to_i128` narrowing — `amount × price` never overflows at the multiply, an absurd result
reverts rather than wraps, the ~1.7e20 USD ceiling is unreachable, and hub/spoke caps bound position
size far below it. No bricked/over-borrowing account via valuation overflow. No POC (invalid).

---

## Iteration 50 — 2026-06-22

### 50.1 — Storage key collision / type confusion: craft an input to overwrite unrelated state — **NOT VALID**

**Assumption.** The controller keys many persistent maps by attacker-influenceable values —
`Market(asset)`, `AccountMeta(account_id)`, `SupplyPositions(account_id)`,
`BorrowPositions(account_id)`, `Spoke(id)`, spoke usage `(category, asset)`. If keys were
built by concatenating raw bytes (or shared a flat namespace), a crafted asset Address or account_id
could **collide with a different key space** — e.g. make a `SupplyPositions` write land on another
account's `BorrowPositions`, or an `AccountMeta` overwrite a `Market` config — corrupting unrelated
state or another user's position.

**Analysis.** Keys are **typed, variant-discriminated `#[contracttype]` enums**, not concatenated
bytes, so every key space is namespaced by its enum variant in the canonical XDR encoding:

- `ControllerKey` (`interfaces/controller/src/types/controller.rs`) is an enum:
  `Market(Address)`, `AccountMeta(u64)`, `SupplyPositions(u64)`, `BorrowPositions(u64)`,
  `Spoke(u32)`, plus unit variants (`Pool`, `AccountNonce`, `PositionLimits`, …). Soroban
  serializes each storage key with its **variant tag** plus the typed payload, so the encoding is
  injective per `(variant, value)`. `AccountMeta(5)`, `SupplyPositions(5)`, and `BorrowPositions(5)`
  are three *distinct* keys despite the shared `5`; an `Address`-keyed `Market` can never collide with
  a `u64`-keyed `AccountMeta` (different variant **and** payload type).
- **No attacker-chosen ids.** Account ids come only from the monotonic `AccountNonce` (iter 1.1), so an
  attacker can't pick an id to aim at a specific key; and operating on an *existing* account in any
  value-moving flow is owner-gated (12.1 aside for the benign supply case).
- **Per-account sub-maps.** Supply/borrow positions are `Map<Address, …>` *inside* the per-account
  entry, so asset keys live in a nested map scoped to one account — cross-account/cross-asset
  bleed is structurally impossible.

So no crafted input can make a write land in another key space or another user's slot; the typed-key
encoding plus monotonic ids plus owner-gating fully namespace storage.

**Verdict / defense.** Defended by Soroban typed, variant-discriminated `ControllerKey` enum keys
(injective per variant+value), monotonic non-attacker-chosen account ids, and owner-gated existing-
account access — no storage key collision or type-confusion corruption of unrelated state. No POC
(invalid).

---

## Iteration 51 — 2026-06-22

### 51.1 — Unauthenticated controller mutation (any entrypoint callable without a signature) — **NOT VALID**

**Assumption.** The controller is the sole gateway to the pool (iter 38). If even one *mutating*
controller entrypoint lacked `caller.require_auth()` / `#[only_owner]`, an external actor could move
state (or another user's funds) without authorization — collapsing the auth model regardless of every
downstream gate. This is the controller-side analog of iter 38's pool enumeration: confirm
**completeness**, not just spot checks.

**Analysis.** Enumerated every `pub fn` entrypoint across all controller `#[contractimpl]` modules
(`router.rs`, `positions/*`, `strategies/*`, `governance/*`). Each mutating one authenticates:

- **`caller.require_auth()` — user flows (17):** `supply` (`supply.rs:52`), `withdraw` (`:61`),
  `borrow` (`borrow.rs:37`), `repay` (`repay.rs:47`), `liquidate` (`liquidation.rs:58`),
  `clean_bad_debt` (`:44`), `multiply` (`multiply.rs:67`), `swap_collateral`
  (`swap_collateral.rs:66`), `swap_debt` (`swap_debt.rs:64`), `repay_debt_with_collateral` (`:72`),
  `migrate_from_blend` (`migrate_blend.rs:97`), `flash_loan` (`flash_loan.rs:39`), `add_rewards`
  (`router.rs:109`), `update_indexes` (`:38`), `claim_revenue` (`:102`), `update_account_threshold`
  (`:122`), `renew_account` (`:315`).
- **`#[only_owner]` — admin/governance (=governance contract):** `deploy_pool`, `create_liquidity_pool`,
  `upgrade_liquidity_pool_params`, `update_pool_caps`, `upgrade_pool` (`router.rs:52-94`), `upgrade`,
  `migrate`, `pause`, `unpause`, `transfer_ownership` (`governance/access.rs:88-134`), plus the
  `governance/config.rs` setters (`:36`…).
- **Non-gated are safe:** views (`app_version`, capital/reserve/rate views) mutate nothing;
  `accept_ownership` (`access.rs:148`) is gated by OZ to the *pending* owner named by the current
  owner (iter 4).

So there is no unauthenticated mutation path. The permissionless-but-authenticated flows
(`liquidate`/`repay`/`clean_bad_debt`/`add_rewards`/`update_indexes`/`claim_revenue`/
`update_account_threshold`) still require the caller's signature and are benign/bounded by design
(their respective iterations); `supply` is authenticated but intentionally not owner-matched (the 12.1
LOW finding) — still never *unauthenticated*.

**Verdict / defense.** Defended: 100% of mutating controller entrypoints require authentication —
`caller.require_auth()` (user) or `#[only_owner]` (admin); non-gated entrypoints are views or the
pending-owner-gated `accept_ownership`. Paired with iter 38 (pool mutators are controller-only), the
two-contract auth model is complete: pool writes ⇐ controller-only, controller writes ⇐ authenticated.
No POC (invalid).

---

## Iteration 52 — 2026-06-22

### 52.1 — Lingering aggregator allowance / approval drain — **NOT VALID**

**Assumption.** The swap flows (`multiply`/`swap_*`/`migrate`) hand input tokens to the governance-set
aggregator/router. The classic DeFi-router bug is granting a **persistent SAC `approve` allowance** to
the router and not revoking the unused remainder — leaving a standing approval that the router (or a
later-upgraded/compromised one) can drain from the controller in a *future* transaction. Iter 6.1
covered input-spend capping and output trust; this targets the **allowance lifetime** specifically.

**Analysis.** The controller never sets a persistent allowance. It uses Soroban's **transient
invoker-contract-auth** scoped to a single pinned transfer:

- `pre_authorize_router_pull` (`swap.rs:108-128`) builds an
  `InvokerContractAuthEntry::Contract(SubContractInvocation { context: { contract: token_in, fn_name:
  "transfer", args: (controller, router_addr, amount_in) }, sub_invocations: [] })` and calls
  `env.authorize_as_current_contract([entry])`.
- This authorizes **exactly one** `token_in.transfer(controller → router, amount_in)` sub-call,
  **only within the current invocation frame** — it is not an SAC `approve`, leaves **no standing
  allowance**, and expires when the call returns. There is nothing for the router to pull in a later
  tx.
- **Args are pinned:** recipient (`router_addr`) and amount (`amount_in`) are baked into the
  authorized context, so the router cannot redirect the pull to another address or pull more than
  `amount_in` even within the frame.
- Belt-and-suspenders from 6.1: `verify_router_input_spend` asserts `actual_in_spent <= amount_in`
  (balance-delta) and `refund_router_underspend` returns any unspent input; a reentrancy guard wraps
  the router call; output is balance-delta-measured.
- The same `authorize_as_current_contract` pattern is reused for the Blend migration's repay pulls
  (iter 7), so neither integration leaves a standing approval.

So even a malicious or later-upgraded aggregator has zero residual allowance to exploit, and cannot
over-pull or redirect during the swap itself.

**Verdict / defense.** Defended by per-invocation, args-pinned `authorize_as_current_contract`
authorization (single `transfer(controller→router, amount_in)`), not a persistent SAC allowance —
no lingering approval to drain, no redirect/over-pull, with input-spend capped+refunded and output
balance-delta-measured. No POC (invalid).

---

## Iteration 53 — 2026-06-22

### 53.1 — Per-asset capability-flag bypass via leverage (borrow non-borrowable / supply non-suppliable) — **NOT VALID**

**Assumption.** Each asset carries capability flags — `is_borrowable`, `can_supply`,
`is_collateralizable`. Direct `borrow`/`supply` enforce them, but if the leverage flows
(`multiply`/`swap_debt`/`swap_collateral`/`migrate`) added debt/collateral without re-checking, an
attacker could borrow an asset governance marked non-borrowable, or supply one marked non-suppliable —
evading a risk control (e.g. borrowing a thin/volatile asset governance intended supply-only).

**Analysis.** The capability flags live in the **shared validators** that all paths funnel through
(same invariant pattern as iters 20.1/47/48):

- **Borrow side:** `validate_borrow` (`borrow.rs:66`) asserts `asset_config.is_borrowable` →
  `AssetNotBorrowable` (`:87-88`). Strategy borrows route through `borrow_strategy_inner` →
  `validate_borrow` (iter 20.1), so `multiply`/`swap_debt`/`migrate` cannot open debt in a
  non-borrowable asset.
- **Supply side:** `validate_deposit` (`supply.rs:123`) asserts `asset_config.can_supply()`
  (`:146`). Strategy deposits route through `process_deposit` → `validate_deposit` (iter 48), so the
  leverage deposit legs cannot supply a non-suppliable asset.
- **Collateral counting:** `is_collateralizable` is enforced in the *valuation* layer
  (`effective_asset_config` / `apply_spoke_to_asset_config`, iter 34) — a supplied but
  non-collateralizable asset simply doesn't count toward borrow capacity, on every path. So even if an
  asset is suppliable-but-not-collateral, leverage can't conjure borrowing power from it.

So the capability flags are entrypoint-invariant: enforced identically on direct and leverage paths
via the shared validators and the common valuation layer.

**Verdict / defense.** Defended: `is_borrowable` (`validate_borrow`) and `can_supply`
(`validate_deposit`) gate every borrow/supply including the strategy legs (which route through those
validators), and `is_collateralizable` is enforced in valuation — no leverage-path bypass of per-asset
capability flags. No POC (invalid).

---

## Iteration 54 — 2026-06-22

### 54.1 — Market-active bypass via leverage (enter a deactivated/wind-down market through multiply/swap/migrate) — **NOT VALID**

**Assumption.** Iter 32 showed `require_market_active` gates direct supply/borrow/flash (entry), not
exits — letting governance deactivate a market to wind it down (stop *new* exposure to a deprecated or
risky asset while existing positions unwind). If the leverage flows skipped that gate, an attacker
could open *new* leveraged supply/borrow in a deactivated market via `multiply`/`swap_*`/`migrate`,
leaking the wind-down.

**Analysis.** `require_market_active` lives **inside the shared validators**, so leverage inherits it:

- `validate_deposit` (`supply.rs:123`) calls `require_market_active` at `supply.rs:138`; `validate_borrow`
  (`borrow.rs:66`) calls it at `borrow.rs:81`.
- The strategy deposit legs route through `process_deposit` → `validate_deposit`, and the strategy
  borrow legs through `borrow_strategy_inner` → `validate_borrow` (iter 48/20.1). So `multiply`,
  `swap_collateral`, `swap_debt`, and `migrate_from_blend` all hit `require_market_active` on their
  entry legs — a deactivated market rejects new leveraged supply/borrow exactly as it rejects direct
  ones.
- Consistent with iter 32: exits (withdraw/repay/liquidation) remain ungated so existing positions
  unwind; only *entry* (direct or leveraged) is frozen.

**This completes the cross-entrypoint invariant set.** Every entry-side guard enforced by the shared
validators applies uniformly to direct *and* leverage paths — confirmed individually:
**spoke spoke caps** (20.1), **min-borrow-collateral floor** (47.1), **position limits** (48.1),
**capability flags `is_borrowable`/`can_supply`** (53.1), and **market-active** (here). The design
funnels every position-opening path through `validate_deposit`/`validate_borrow` (+ the post-pool
gate), so no leverage entrypoint can sidestep any entry control.

**Verdict / defense.** Defended: `require_market_active` is enforced inside `validate_deposit`
(`supply.rs:138`) and `validate_borrow` (`borrow.rs:81`), which the strategy legs route through — so
leverage cannot open new positions in a deactivated market. No POC (invalid).

---

## Iteration 55 — 2026-06-22

### 55.1 — Same-asset swap (swap_debt/swap_collateral A→A) no-op/double-count edge — **NOT VALID**

**Assumption.** `swap_collateral` rejects swapping an asset for itself (`AssetsAreTheSame`,
`swap_collateral.rs:69-73`, iter 44). If `swap_debt` *lacked* the symmetric guard, swapping debt
A→A could trigger a degenerate path — borrow A, "swap" A→A (no-op or self-transfer), repay A — that
might double-count the debt position, mis-fire the spoke usage delta, or leave dangling state. The
hypothesis was an **asymmetry** between the two swap entrypoints.

**Analysis.** No asymmetry — `swap_debt` carries the same guard plus the full stack
(`swap_debt.rs:33-47`):

- `assert_with_error!(existing_debt_token != new_debt_token, GenericError::AssetsAreTheSame)`
  (`:36-39`) — a debt-for-itself swap reverts before any borrow/repay, so the degenerate
  same-asset path is unreachable.
- Full guard stack confirmed: `caller.require_auth()` (`:33`), `require_not_flash_loaning` (`:34`),
  `AssetsAreTheSame` (`:36`), `require_account_owner_match` (`:43`), `require_positive_amount`
  (`:47`) — matching `swap_collateral` (iter 44) and the owner-match coverage of iter 12.1.

So both swap entrypoints are consistent: same-asset swaps are rejected, and both run auth +
reentrancy + owner-match + positive-amount before touching positions.

**Verdict / defense.** Defended: `swap_debt` rejects same-asset swaps (`AssetsAreTheSame`,
`swap_debt.rs:36-39`) identically to `swap_collateral` — no no-op/double-count edge, no entrypoint
asymmetry. No POC (invalid).

---

## Iteration 56 — 2026-06-22

### 56.1 — Spoke category deprecation reprices positions to base → forced liquidations — **NOT VALID (external-actor) / governance-operational note**

**Assumption.** An attacker deprecates an spoke category to instantly reprice every position in it
from boosted LTV/threshold to base params, pushing borrowers who relied on the spoke LTV below HF=1
and force-liquidating them.

**Analysis.** The repricing is real but the trigger is governance-only, so it is not an external-actor
attack — though it is a governance-operational consideration worth flagging:

- **Repricing mechanism (confirmed).** `apply_spoke_to_asset_config` early-returns on
  `cat.is_deprecated` (`spoke.rs:23`), so a deprecated category applies *no* override → positions are
  valued at **base** LTV/threshold/bonus. `ensure_spoke_not_deprecated` (`:68-70`) additionally
  blocks *new* exposure under a deprecated category (`SpokeDeprecated`).
- **Trigger is governance-gated, not external.** The `is_deprecated` flag is set only through the
  governance spoke category config op (owner-gated / timelocked). An external actor cannot deprecate
  a category, so cannot use this to force-liquidate anyone — closing the attack framing.
- **Consequence for the team.** Deprecating a category *can* make some of its positions immediately
  liquidatable (they lose the boosted LTV headroom and may fall below HF=1, then liquidate under base
  threshold/bonus). This is the intended wind-down semantics, but it is abrupt: it mirrors the
  documented governance-trust residuals (8.1 oracle-divergence pause, 11.1 asset selection). Practical
  guidance: treat spoke deprecation as a sensitive change — announce it, and prefer winding down
  caps / discouraging new spoke borrows ahead of flipping `is_deprecated`, so borrowers can de-risk
  before the reprice. (The timelock delay already provides a built-in notice window.)

**Verdict / defense.** Not an external-actor attack: spoke deprecation is governance-only
(owner-gated + timelocked), so no attacker can trigger the boosted→base reprice to force liquidations.
Documented as a governance-operational consideration (abrupt repricing on deprecation), alongside the
timelock notice window. No POC (not externally triggerable).

---

## Iteration 57 — 2026-06-22

### 57.1 — Flash-loan fee accounting gap (untracked surplus / accounting drift / double-count) — **NOT VALID**

**Assumption.** A flash loan pays out `amount` and pulls back `amount + fee` (iter 37), so the pool's
actual token balance rises by `fee`. If that `fee` were not credited into the pool's *internal*
accounting (`cash`/revenue), the accounted reserves would drift below the real balance (untracked
surplus that no one can claim, or — worse if mis-signed — an over-credit that lets reserves be
double-counted). An attacker spamming flash loans could amplify any such drift.

**Analysis.** The fee is fully and consistently accounted on both axes (`pool/src/lib.rs`, end of
`flash_loan`):

- **Landing verified first.** After `transfer_from(receiver → pool, amount + fee)` (`:399`), the pool
  asserts `tok.balance(pool) == expected_after_repay` (= `pre_balance + fee`, `:401-405`) — so exactly
  `fee` net was added before any crediting (no over/under).
- **Booked as protocol revenue.** `add_protocol_revenue_ray(&mut cache, fee_ray)` (`:408`) credits the
  fee to the scaled revenue accounting (governance-claimable via `claim_revenue`, and reflected in the
  supply side; zero-supplier-guarded, iter 28).
- **Cash kept consistent.** `cache.credit_cash(fee)` (`:411`) raises accounted `cash` by exactly
  `fee`, matching the real balance delta — the code states the invariant: *"pool sends `amount` and
  receives `amount + fee`, so `cash` increases by `fee`."* So accounted reserves track actual balance
  with no drift.
- **No double-count.** `cash` (liquidity ledger) and `revenue` (protocol's claimable cut) are distinct
  fields; crediting both reflects one real `+fee` correctly (the fee is liquidity that the protocol
  owns), not the same value twice — and `claim_revenue` is bounded by `min(reserves, treasury_actual)`
  (iter 10.1), so revenue can't be over-claimed beyond actual cash.

So every flash loan's fee is captured into accounting exactly once, consistent with the balance; spam
just accrues more (correctly-tracked) revenue. No untracked surplus, no drift, no double-count.

**Verdict / defense.** Defended: the flash fee is balance-verified then booked via
`add_protocol_revenue_ray` (revenue) + `credit_cash` (reserves) for an exact, single, consistent
`+fee` accounting; `claim_revenue` stays bounded by actual reserves. No accounting gap or drift from
flash-loan fees. No POC (invalid).

---

## Iteration 58 — 2026-06-22

### 58.1 — `claim_revenue` accounting drift / over-claim (token-out vs accounted cash) — **NOT VALID**

**Assumption.** `claim_revenue` is permissionless and transfers real tokens out (to governance). The
token-out analog of the iter-57 fee-in question: if it burned scaled revenue but failed to **debit
accounted `cash`** (or weren't capped), repeated/spam claims could drift accounted reserves above the
actual balance (eventual over-report → withdrawal insolvency), or over-claim beyond available reserves.

**Analysis.** The claim is capped and the cash debit mirrors the token-out exactly:

- **Capped at `min(reserves, claimable)`.** `burn_claimable_revenue` (`pool/src/cache.rs`) sets
  `amount = live_reserves().min(unscale_supply(revenue))` — you can claim no more than the pool's
  *actual* available reserves *and* no more than accrued revenue. `amount <= 0` early-returns
  (`amount.max(0)`), so nothing to claim is a clean no-op.
- **Scaled burn from both ledgers.** It burns `scaled_to_burn` from `revenue` *and* `supplied`
  (proportional when partially claimable) — removing the protocol's cut from the supply side too.
- **Cash debited to match the transfer.** The `claim_revenue` entrypoint (`pool/src/lib.rs`):
  `amount = burn_claimable_revenue()` → **`cache.debit_cash(amount)`** → `transfer_out(owner, amount)`.
  Accounted `cash` drops by exactly the tokens sent — the token-out mirror of iter 57's `credit_cash`
  on fee-in. No drift between accounted reserves and real balance.
- **Recipient is governance only** (`ownable::get_owner`), so the permissionless caller gains nothing
  (iter 10.1) — it's a keeper/janitor trigger.
- **Spam-safe.** Once revenue is drained, subsequent claims compute `amount <= 0` and early-return —
  no repeated debit, no drift, no double-spend.

So claim_revenue moves exactly the claimable, reserve-bounded amount to governance while keeping
accounted `cash` consistent with the real balance; spam no-ops.

**Verdict / defense.** Defended: `claim_revenue` caps at `min(live_reserves, claimable)`, debits
accounted `cash` by exactly the transferred amount (mirroring the token-out), burns scaled
revenue+supplied, sends only to the owner, and no-ops once drained. No accounting drift or over-claim.
No POC (invalid).

---

## Iteration 59 — 2026-06-22

### 59.1 — Oracle-read reentrancy (compromised oracle reenters during a price read) — **NOT VALID (external-actor) / governance-trust + defense-in-depth note**

**Assumption.** The controller calls external oracle contracts (RedStone/Reflector) during price
reads — both the pre-mutation prefetch and the **post-pool-mutation `strategy_finalize` HF gate**. A
called contract on Soroban can do anything, including call back. So a malicious/compromised oracle's
"read" method could **reenter the controller** mid-operation. Notably, the aggregator/router call *is*
explicitly reentrancy-guarded (iter 6.1) — is the oracle read?

**Analysis.** The oracle reads are **not** wrapped in the `FlashLoanOngoing` guard, unlike the
aggregator — but the exposure is governance-trust-bounded, not externally reachable:

- **Guard scope.** `set_flash_loan_ongoing(true/false)` wraps only the router call
  (`swap.rs:100-104`); oracle reads (`read_redstone_source`/`read_reflector_source`, via the Cache
  prefetch and the finalize HF gate) run outside it. So an oracle callback during a read would not hit
  `require_not_flash_loaning`.
- **Not attacker-substitutable.** The oracle contracts are set via the governance-only, timelocked
  `set_market_oracle_config` path (owner-gated). An external actor cannot point a market at a contract
  they control, so cannot introduce a reentrant oracle. This requires a *compromised or malicious
  governance-set oracle* — a governance-trust boundary (same class as the aggregator/asset-issuer
  trust of iter 6.1/11.1).
- **CEI dampens impact.** In the normal flows (`borrow`/`supply`/`withdraw`/`repay`), oracle reads
  occur during validation/HF *before* the pool mutation and before `cache.save()`, so a reentrant
  call sees consistent committed state. Any reentrant call is itself fully gated (auth, HF, caps),
  so it cannot extract value from an inconsistent intermediate — at worst it's a normal nested op.
  The post-mutation finalize read (strategy flows) is the sharper case, but still requires the
  malicious-oracle precondition.
- **Trust basis.** RedStone prices are off-chain-signed (the on-chain contract verifies signatures —
  a benign verifier), and Reflector is a vetted public feed; neither is expected to reenter.

So there is **no external-actor reentrancy** here; the residual is a defense-in-depth gap *conditional
on a compromised governance-set oracle*.

**Verdict / defense.** Not externally exploitable: oracle contracts are governance-only/timelocked
config, so an attacker can't introduce a reentrant oracle; normal-flow CEI ordering + fully-gated
reentrant calls bound the impact even then. **Defense-in-depth hardening (LOW):** consider wrapping
oracle reads in the same `FlashLoanOngoing` reentrancy guard as the aggregator (so even a compromised
oracle can't reenter), or explicitly document the no-reentrant-oracle trust assumption. No POC (not
externally triggerable).

---

## Iteration 60 — 2026-06-22

### 60.1 — Caller-supplied RedStone price selection (pick a favorable signed price) — **NOT VALID**

**Assumption.** RedStone prices are off-chain-signed (iter 8). Some RedStone integrations are
**push/caller-supplied**: the user submits a signed price payload per-tx that the contract verifies.
If so, an attacker could, within the set of recent validly-signed prices, **select the most favorable
one** for their operation (highest collateral price when borrowing, lowest when liquidating a victim)
— a manipulation that stays within "valid signatures" and so bypasses naive signature checks.

**Analysis.** This protocol uses a **pull/contract-stored** RedStone model, not caller-push, so price
selection is not in the caller's hands:

- `read_price_data` instantiates `RedStonePriceFeedClient::new(env, contract)` and calls
  `try_read_price_data_for_feed(feed_id)` (`common/src/oracle/providers/redstone.rs:32`) — a **view on
  the governance-set RedStone feed contract's stored latest price**, written by RedStone's
  relayers/keepers. The caller submits no price payload; they get whatever the feed currently holds.
- `contract` and `feed_id` come from the governance-set `MarketOracleConfig` (owner-gated/timelocked),
  not from the caller.
- The stored price is **staleness-checked**: `read_redstone_source` compares
  `package_timestamp`/`write_timestamp` to `env.ledger().timestamp()` against `max_stale`
  (`redstone.rs:41-43`); a stale stored price is rejected for risk-increasing/liquidation flows (the
  fail-closed policy, iter 8).
- And the resolved price is still subject to the full oracle stack — midpoint blend with the Reflector
  TWAP anchor, sanity band, `> 0` guard (iters 8/36) — so even the stored RedStone value can't bias
  the final price beyond ½ the governance band.

So there is no caller-controlled price-selection surface: the price is pulled from a vetted feed
contract, time-bounded, and blended/bounded. The favorable-signed-price attack (a push-model RedStone
hazard) is structurally absent.

**Verdict / defense.** Defended: RedStone is integrated pull-style — prices are read from the
governance-set feed contract's stored value (`read_price_data_for_feed`), not caller-submitted
payloads; staleness-checked and blended/sanity-bounded. The caller cannot select a favorable signed
price. No POC (invalid).

---

## Iteration 61 — 2026-06-22

### 61.1 — Accumulator (protocol-revenue treasury) trust / reentrancy via the claim forward — **NOT VALID**

**Assumption.** The controller's `claim_revenue` claims protocol revenue per market and **forwards it
to the `accumulator`** (a governance-set address). If that forward were a *contract call* into the
accumulator (e.g. `accumulator.deposit(...)`), a malicious/compromised accumulator could reenter the
controller, or the call could be abused; and if the accumulator were attacker-settable, revenue could
be redirected.

**Analysis.** The accumulator is a **passive SAC-transfer recipient**, not a called contract:

- The controller's `claim_revenue_for_asset_with_cache` (`router.rs:253-275`) calls
  `pool_claim_revenue_call` (pool burns reserve-bounded scaled revenue → sends to its owner = the
  controller, iter 58), then forwards the `actual_amount` to the accumulator via **`sac_transfer_call`**
  (`router.rs:265-271`) — a plain SAC `transfer(controller → accumulator, amount)`.
- **No reentrancy:** a SAC `transfer` to a contract address invokes **no recipient code** on Soroban
  (iter 30), so a malicious accumulator cannot reenter the controller on receipt — unlike a
  `deposit()`-style call. (Contrast the aggregator, which *is* a called contract and is
  reentrancy-guarded, iter 6.1.)
- **Governance-set + fail-closed:** the accumulator is set only via `set_accumulator`
  (`config.rs:43`, owner-gated/timelocked); if unset, `claim_revenue` panics `NoAccumulator`
  (`router.rs:256-257`) — no silent misroute. An attacker can't repoint it.
- **No caller benefit / bounded amount:** `claim_revenue` is permissionless-but-authenticated (iter
  51); the funds go to the accumulator (treasury), never the caller, and the amount is the pool's
  reserve-capped claimed revenue (iter 58). Permissionless callers just act as keepers sweeping
  revenue to the treasury.

So the accumulator is the revenue sink, reached by a callback-free token transfer of a reserve-bounded
amount — no reentrancy, no redirection, no caller profit.

**Verdict / defense.** Defended: revenue is forwarded to the governance-set accumulator via a plain
SAC `transfer` (no recipient callback → no reentrancy), fail-closed if unset (`NoAccumulator`), with
the amount reserve-bounded and never payable to the caller. No POC (invalid).

---

## Iteration 62 — 2026-06-22

### 62.1 — Flash-loan receiver griefing (force a callback onto a third party's contract) — **NOT VALID**

**Assumption.** `flash_loan(caller, asset, amount, receiver, data)` invokes a **caller-specified**
`receiver`'s `execute_flash_loan(initiator, asset, amount, fee, pool, data)`. An attacker could pass a
*third party's* contract as `receiver` (with attacker-chosen `data`) to force an unexpected/unwanted
callback on it — griefing or abusing that contract's flash-handling logic.

**Analysis.** The callback can be *invoked* on any WASM contract, but it cannot impose any lasting
effect on an uncooperative target, because the flash is atomic and repayment-gated:

- **`require_wasm_receiver`** (`common/src/validation.rs:29`) restricts the receiver to a contract
  (an EOA can't be targeted), but that alone doesn't pick a *willing* receiver.
- **Repayment-or-revert (iter 37) is the real defense.** After the callback, the pool requires the
  full `amount + fee` to be repaid (allowance + `transfer_from`); if it isn't, the entire transaction
  **reverts atomically**. A third-party receiver that doesn't recognize/authorize this flash will not
  arrange repayment → the whole tx (including any actions its `execute_flash_loan` took) rolls back.
  So no state change can be *forced* onto a victim contract — at worst it momentarily executes and is
  unwound.
- **Receiver self-protection (standard pattern).** The protocol passes the `initiator` to the
  callback; a correct flash receiver validates that it is the controller calling and that the
  `initiator`/`data` correspond to a flash *it* requested (the Soroban analog of EIP-3156's
  `onFlashLoan` initiator check) and rejects otherwise. Protecting against unsolicited callbacks is
  the receiver's responsibility, not a protocol obligation; the protocol's duty (invoke the named
  receiver, enforce repayment) is met.
- The attacker also pays the tx cost and gains nothing (no funds move net; balances are bracketed,
  iter 37).

So forcing a flash callback on an arbitrary contract yields no lasting griefing — it either reverts
(uncooperative/validating receiver) or is a no-op the attacker paid for.

**Verdict / defense.** Defended: `require_wasm_receiver` + atomic repayment-or-revert (iter 37) mean an
unsolicited flash callback on a third-party contract is unwound unless that contract chooses to
participate, and well-built receivers reject unauthorized callbacks by validating the initiator. No
lasting third-party griefing, no protocol-side exposure. No POC (invalid).

---

## Iteration 63 — 2026-06-22

### 63.1 — Reflector price-source manipulation (caller selection / thin-TWAP) — **NOT VALID**

**Assumption.** Completing the per-source read review (RedStone done in iter 60): could the **Reflector**
anchor be biased by the caller — selecting a favorable price — or by a **thin TWAP** computed from
too few observations (so a brief DEX/CEX blip dominates the "average")?

**Analysis.** Reflector is read **pull-style**, identical model to RedStone, plus a TWAP-sufficiency
guard:

- **Contract query, no caller input.** `reflector_lastprice_call` / the TWAP path call
  `ReflectorClient::new(env, oracle).lastprice(asset)` and `.prices(asset, records)`
  (`common/src/oracle/providers/reflector.rs:39-53`). The `oracle` contract, `asset`, and TWAP
  `records` window all come from the **governance-set** `MarketOracleConfig` (owner-gated/timelocked),
  never from the caller. So a caller can't select a favorable price or shrink the window.
- **Thin-TWAP guard.** `min_twap_observations(records) = max(2, ceil(records/2))`
  (`reflector.rs:75`) requires the TWAP to be backed by at least half the configured window's
  observations; an under-populated TWAP (which a flash blip could dominate) is rejected rather than
  used. Combined with the TWAP itself (vs spot) being manipulation-resistant (iter 8), a transient
  DEX/CEX move can't bias the anchor.
- **Bounded downstream.** The Reflector value feeds the same midpoint blend + sanity band + `>0`
  guard + fail-closed policy (iters 8/36/44), so even a shifted anchor moves the final price by at
  most ½ the governance band on risk-increasing/liquidation flows.

This completes the oracle-source-read picture: **both** primary (RedStone, iter 60) and anchor
(Reflector, here) are pull-based contract queries with governance-set parameters and no caller-side
price selection; Reflector additionally enforces TWAP observation-sufficiency.

**Verdict / defense.** Defended: Reflector is queried from the governance-set contract with a
governance-set asset/window (no caller selection), guarded by `min_twap_observations` against
thin-TWAP manipulation, and bounded by the blend/sanity/policy stack. No POC (invalid).

---

## Iteration 64 — 2026-06-22

### 64.1 — Composition of the 4 VALID LOW findings into a higher-severity exploit — **NOT VALID (no escalation)**

**Assumption.** Individually the VALID findings are LOW, but auditors know LOWs can *chain* into a
HIGH. The four: **1.2** (spoofable `harvest.from`), **5.1** (dust account spam), **12.1** (non-owner
`supply`), **14.1** (strategy NAV inflation via 12.1). Could combining them yield theft or a severe
DoS?

**Analysis.** Each pairwise/triple combination was walked through; none escalates beyond LOW:

- **12.1 + 14.1 (already a chain).** 14.1 *is* 12.1 applied to a strategy's account — non-owner supply
  inflates the strategy NAV. But the deposited funds land in the strategy's controller account,
  withdrawable only by the vault (`Strategy::withdraw` gates `from`=vault), so the attacker **funds**
  the inflation and **cannot reclaim** it. Chaining the two yields NAV griefing, not theft — the
  attacker is strictly out-of-pocket. No escalation.
- **5.1 + 12.1 (dust spam + non-owner supply).** Spamming dust into *victims'* accounts is exactly the
  slot-exhaustion sub-case already captured in 12.1 (bounded: top-ups of held assets need no slot,
  victim recovers dust on withdraw, attacker pays per gift). Spamming dust into *new* accounts is 5.1
  (bounded state-bloat). Combining is additive griefing cost to the attacker, still no theft and still
  keeper-localized (5.1 mitigations).
- **1.2 + 14.1 (spoofed harvest + inflated NAV).** `harvest`'s `price_per_share` is the *global* market
  supply index (1.2), independent of the per-vault balance, so a spoofed harvest event can't amplify
  the 14.1 NAV lie — the two mislead off-chain consumers on *different* axes (event attribution vs raw
  balance), neither moving on-chain value. Additive integrator-trust noise, not a compounded on-chain
  exploit.
- **1.2 + 5.1 (event spam).** Both are permissionless event/log-spam (off-chain indexer load); combining
  is more of the same self-funded spam, no on-chain effect (same class).
- **Common ceiling.** Every VALID finding is gated from theft by the *same* core invariants that the
  other 59 iterations verified hold: value-extracting flows are owner-matched (12.1 aside), funds
  donated to others' accounts are unrecoverable by the attacker, and the solvency/oracle/cap stack is
  intact. No combination removes those invariants — they only stack griefing cost (borne by the
  attacker) and off-chain noise.

So the four LOWs are independent griefing/asymmetry issues whose combination remains LOW: no chain
produces fund theft, protocol insolvency, or an unbounded DoS.

**Verdict / defense.** Defended: the VALID LOW findings do not compose into a higher severity — the
strongest chain (12.1→14.1) is already logged and yields unrecoverable-donation NAV griefing, not
theft; the rest are additive, self-funded griefing / off-chain noise bounded by the intact
owner-match + solvency + oracle + cap invariants. No POC (synthesis; the individual POCs stand).

---

## Deep re-review (xhigh pass) — 2026-06-22

A second, deeper adversarial pass over **every** logged verdict, re-deriving the load-bearing claims
against code rather than trusting the first pass. **Net result: no verdict flips, no new fund-theft
path, no NOT-VALID becomes VALID.** Four points were sharpened and three load-bearing mechanisms were
re-verified. Detail:

### Re-verified against code this pass (held)

- **Single-shot `authorize_as_current_contract` ordering (6.1 / 7.1 / 52.1).** This is the exact bug
  class that previously broke the strategy deposit path (the `authorize_as_current_contract` only
  covers sub-invocations of the *next* call; an intervening cross-contract call consumes/voids it).
  Re-checked both consumers: `swap.rs` `pre_authorize_router_pull` (`:46`) is immediately followed by
  the router call (`:48`) with only storage/no-op ops between; `migrate_blend.rs`
  `authorize_repay_pulls` (`:156`) is immediately followed by `guarded_submit` (`:157`). **No
  intervening cross-contract call in either** → the auth is consumed by the intended pull/submit.
  Also note the failure mode if this *were* mis-ordered is a **revert (fail-closed), not extraction**,
  so it could never have been a theft vector regardless — but the current ordering is correct.
  *Standing note (not a vuln):* this pattern is fragile — any future refactor that inserts a
  cross-contract call between the `authorize` and the consuming call would break the feature
  (fail-closed). Worth a code comment at both sites.
- **Index un-inflatability (2.1).** Re-derived the magnitudes: zeroing even a **1-raw-unit** 6-dec
  deposit (`amount_ray = 1e21`) needs `supply_index > 2e48`; the index is an i128 (`max ≈ 1.7e38`)
  and `Ray::mul` reverts (`MathOverflow`) far below that. The ~1e21–1e54 inflation required is
  physically unreachable. Holds with margin.
- **Bad-debt residual routing (17.1).** Confirmed the `seize_positions` `Deposit` leg does
  `cache.revenue.checked_add_assign(scaled)` (pool `lib.rs`) — residual collateral routes to
  **protocol revenue** while the written-off debt is socialized to suppliers via the supply-index
  reduction. Accurate as logged.

### Sharpened points

1. **14.1 severity framing — the cross-contract nuance is the real risk.** The log says "not
   profitable / no theft," which is correct **against our contracts** (the donation lands in the
   strategy's controller account, our accounting stays correct, the attacker can't reclaim it). But
   the donation-inflation *is* the classic ERC-4626 first-depositor/donation attack, and it **is
   realizable as value transfer at a naive integrating DeFindex vault**: an attacker holding
   pre-existing vault shares can front-run a victim's deposit by inflating `strategy.balance()` so the
   victim's deposit mints fewer vault shares, diluting the victim into the inflated NAV — captured
   pro-rata by existing shareholders (incl. the attacker). The profit is realized at the **vault's**
   share-mint rounding, which is outside our contracts. So: **our-scope severity = LOW** (griefing,
   no theft of our funds); **risk to a naive vault could exceed LOW.** The already-logged mitigation
   is the right one and should be emphasized: integrating vaults must use a **donation-resistant NAV**
   (virtual shares / internal principal accounting), not live `balance()`. The cleanest root fix on
   our side remains action item #1 (owner-match on `supply`, isolating the strategy account).

2. **5.1 severity — the "MED" half rests on an unverified keeper assumption.** The state-bloat /
   per-entry-rent griefing is solidly LOW and confirmed. The escalation to MED ("spam pushes
   *legitimate* accounts out of the keeper's bounded `max_accounts_scan` window → missed TTL bumps →
   archival") depends on the keeper's **scan-eviction order**, which lives in a separate workspace and
   was **not verified here**. If the keeper scans ascending by `account_id`, the newest (spam)
   accounts are the ones dropped first — legit accounts stay covered and severity is LOW. Only if it
   evicts by another order could legit accounts be starved. **Recommend:** treat 5.1 as **LOW**
   pending a keeper scan-order check; the dust-floor / per-caller-cap hardening still applies.

3. **17.1 fairness — re-affirmed as a (dust-bounded) team accounting-policy item.** Suppliers absorb
   the socialized loss *and* the residual collateral goes to revenue rather than offsetting that loss.
   Not an attack (not attacker-triggerable, no attacker profit) and bounded to sub-`BAD_DEBT_USD_THRESHOLD`
   dust by the `is_socializable_bad_debt` gate — but the team may want residual collateral to offset
   the supplier loss before routing to revenue. Minor; worth a deliberate decision.

4. **8.1 / 59.1 oracle — characterization holds; emphasis.** The fail-closed divergence pause (8.1)
   and the un-guarded oracle read (59.1 DiD) both remain governance-trust / DiD, not externally
   exploitable. Deeper thought confirms the 59.1 hardening is worth doing precisely because the oracle
   is a *called* contract (unlike the transfer-only accumulator, 61.1): wrapping oracle reads in the
   `FlashLoanOngoing` guard closes the only un-guarded external *call* site in a mutating flow.

### Conclusion of the deep pass

The review stands. The protocol has **no external-actor fund-theft path**; the 4 VALID findings remain
LOW griefing/asymmetry (with 14.1's cross-contract caveat above). The action items are unchanged in
substance, with two emphases added: (#1) the `supply` owner-match fix is also the cleanest mitigation
of the 14.1 vault-donation vector; and a new **standing note** to comment the fragile
`authorize_as_current_contract` ordering at the swap/migrate sites. Severity correction: **5.1 → LOW**
(pending keeper scan-order verification).

---
