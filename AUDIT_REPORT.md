# XOXNO Lending Protocol — Security Audit Report

**Scope:** `contracts/price-aggregator`, `contracts/pool`, `contracts/controller`, `contracts/governance`, `contracts/xoxno-oracle` (+ shared `common/`), Soroban / Rust.
**Method:** deep manual review of the core (fixed-point math, rate/index engine, pool accounting, controller risk + liquidation) → 5 parallel context-mapping agents → a find→adversarial-verify→severity-council→patch multi-agent workflow (89 agents; Opus finders; dual-lens refutation; 3-judge council) → a focused non-standard-token trust-boundary workflow (12 agents). Inline comments were treated as **untrusted** and verified against code.

> **Findings are proposed remediations only — no protocol logic was changed.** The one applied edit is a corrected docstring (I1). This report is kept local (not committed).

---

## 1. Overall posture

The protocol is **strongly engineered and heavily hardened**. Systematic strengths confirmed in review:

- **Protocol-favorable rounding everywhere** — supply-mint floors / supply-burn ceils / borrow ceils / repay floors; collateral valued floor, debt valued ceil; `HF = floor(weighted_collateral_floored / total_debt_ceiled)`, saturating. Rounding can never favor a user against the protocol.
- **Pool fully owner-gated** (owner = controller), with an internally-tracked `cash` counter that direct token donations cannot inflate (donation-attack resistant); every cash debit is preceded by `require_reserves`.
- **Oracle hard read path is genuinely fail-closed** — missing/pending config, non-positive price, out-of-sanity-band, staleness, primary/anchor deviation each revert; money paths use this hard path, never the soft `price_status` diagnostic.
- **`simulate_update_indexes` provably mirrors the mutating `global_sync`** — controller HF/pricing reads agree with pool state.
- **Extensive assurance:** ~822 integration tests, ~910 inline, ~355 common, 8 proptests, 6 fuzz targets, **221 Certora rules**, a differential BigRational liquidation reference.

**No direct-theft (steal-funds) vector survived verification.** The one **High** finding is a *protocol design gap* — the absence of any way to socialize bad debt when a position's collateral cannot be seized (frozen/clawed) and is worth more than the $5 dust threshold. It is triggered by a realistic compliance event on a freeze-capable listed asset (USDC-on-Stellar is clawback-enabled), not a maliciously-crafted token. The remaining findings are permissionless-keeper param propagation, liquidation-liveness edges, and governance/oracle operational hardening.

---

## 2. Findings summary

| # | Severity | Title | Location |
|---|----------|-------|----------|
| **H1** | **High** | Bad debt is permanently unsocializable when collateral cannot be seized (frozen/clawed) and is valued above the $5 dust threshold — no governance force-socialize path | `controller/.../liquidation/{apply.rs:67-94, math.rs:89-91, bad_debt.rs:15-55}`; `constants.rs:6` |
| **M1** | **Medium** | Permissionless `update_account_threshold(has_risks=false)` restamps a victim's `liquidation_bonus`/LTV/fees with no HF gate → worsened-terms liquidation front-run | `controller/src/pool_ops/mod.rs:396-402`; sink `positions/liquidation/math.rs` |
| L1 | Low | Liquidation seizure still cash-gated (`require_reserves`); a cash-drained collateral market blocks liquidation until re-supply | `pool/src/lib.rs:212-216`; `.../liquidation/apply.rs` |
| L2 | Low | Clawback of pool tokens overstates tracked `cash` (never reconciled to live balance) → last withdrawers trap; no reserve-reconciliation path | `pool/src/cache.rs:37-46,113-141`; `lib.rs:977-980` |
| L3 | Low | Issuer freeze/deauthorize of the pool trustline = market-wide DoS (supply/withdraw/borrow/repay/liquidation) | `pool/src/cache.rs:135-141`; every pool token movement |
| L4 | Low | Fee-on-transfer / negative-rebase asset overstates `cash` — supply/repay credit nominal, not received | `controller/src/payments/mod.rs:28-39`; `pool/src/lib.rs:136,256` |
| L5 | Low | Governance role grant/revoke resolve at Standard delay tier, escalating faster than the 7-day Sensitive floor | `governance/src/op.rs:101-126`; `timelock.rs:39-46` |
| L6 | Low | Timelock constructor enforces only a nonzero delay, not the documented 48h floor | `governance/src/access.rs:205-215`; `timelock.rs:48-50` |
| L7 | Low | `set_sanity_band` has no **minimum** band width → a compromised ORACLE key can pinch a band and DoS all risk reads | `price-aggregator/src/config.rs:102-123`; `common/src/validation.rs:139-145` |
| L8 | Low | Oracle-config live-price containment probed only at propose-time → timelock-window drift can store an out-of-band config and brick a market | `governance/src/op.rs:296-314`; `price-aggregator/src/config.rs:22-26` |
| I1 | Info | Wrong docstring: `update_account_threshold` HF gate blocks threshold **lowerings**, not raises **(fixed in place)** | `controller/src/pool_ops/mod.rs:241-247` |
| I2 | Info | `min_borrow_collateral` floor blocks *all* partial withdrawals for a below-floor debt-bearing account (capital lockup) | `controller/src/risk/validation.rs:38-72` |

**Verified & refuted** (§4): swap slippage/sandwich value-loss, delegate value extraction, cross-function reentrancy via the flash-loan receiver, the reward-shortfall accrual trap, the `revenue ⊆ supplied` claim-revenue brick, the `bonus_factor` over-max bonus, `multiply` initial-payment ordering.

---

## 3. Detailed findings & proposed patches

### H1 — High — Permanent unsocializable bad debt for un-seizable collateral above the dust threshold
**Where:** `controller/.../liquidation/apply.rs:67-94` (`apply_liquidation_seizures` → `pool.withdraw(is_liquidation)` → `cache.transfer_out(liquidator)`, `pool/src/cache.rs:135-141`); socialization gate `is_socializable_bad_debt` (`.../liquidation/math.rs:89-91`) → `BAD_DEBT_USD_THRESHOLD = 5·WAD` (`constants.rs:6`); transfer-free writedown `bad_debt.rs:15-55` reachable **only** through that gate (`.../liquidation/mod.rs:169-171`).

**Mechanism (confirmed):** liquidation seizure requires a pool→liquidator SAC transfer, which **traps** when the pool's trustline for the collateral asset is deauthorized (frozen) or the pool balance was clawed below the seize amount. The seizure is pro-rata across *all* of the account's collaterals in one bulk `pool.withdraw` batch, so a single un-seizable leg reverts the **entire** liquidation — including the repay-in leg — even against the account's other healthy collateral. The only transfer-free way to retire the debt is `execute_bad_debt_cleanup` (supply-index write-down, no transfer), which is gated by `is_socializable_bad_debt` requiring `total_collateral ≤ $5`. **There is no owner/governance branch that bypasses this gate.**

**Impact:** an underwater borrower whose collateral is worth more than $5 and cannot be seized has bad debt that is **neither liquidatable nor socializable**. The loss never reaches the supply index; the borrowed-asset market is progressively under-backed until its last suppliers cannot withdraw. No protocol recovery path — only issuer un-freeze/re-authorization cures it. The trigger (USDC-on-Stellar is `AUTH_REVOCABLE` + clawback-enabled) is a realistic compliance/sanctions event, not a crafted token.

**Severity note (transparent):** the two independent verifiers and my own cross-check rated this **Medium** under a strict "trusted-listing absorbs all freeze-capable tokens" reading; the final adjudicator rated it **High** because the missing writedown path is a genuine, fixable protocol design gap *independent of the token*. Rated **High** here; effective severity depends on whether freeze/clawback-capable assets are ever accepted as collateral. Either way the remediation is the same and cheap.

**Proposed patch:** add a **governance-only, transfer-free force-socialize entrypoint** that writes the residual debt down via the supply-index path (exactly as `execute_bad_debt_cleanup` does) for positions whose collateral seizure transfer fails — **decoupled from the $5 threshold**. Guard it behind the timelock and require evidence the seize transfer traps (or the market is frozen). This closes the permanent-bad-debt hole without weakening the dust threshold for normal cleanup. Add a test using the built-but-unused `with_freezable_market` harness primitive.

---

### M1 — Medium — Permissionless liquidation-param restamp on a passive victim
**Where:** `controller/src/pool_ops/mod.rs` — `update_account_threshold` (`:252`) → `sync_account_thresholds` `!has_risks` branch (`:399-401`, persisted `:418`). Sink: stamped `liquidation_bonus` read by the liquidation planner (`.../liquidation/math.rs`, `get_account_bonus_params`).

**Mechanism (verified firsthand + triple council-confirmed):** `update_account_threshold` is permissionless — `caller.require_auth()` authorizes only the caller; `account_ids` is attacker-chosen. The `has_risks=true` branch restamps `liquidation_threshold` under an HF gate (`hf ≥ 1.05`, `:420-434`). The `has_risks=false` branch restamps `loan_to_value`/`liquidation_bonus`/`liquidation_fees` with **no HF or ownership gate** (and loads no debt). The planner reads the stamped `liquidation_bonus` straight from storage and never refreshes from live config.

**Impact:** after governance raises a spoke's `liquidation_bonus`, an attacker calls `update_account_threshold(has_risks=false, [victim])` to ratchet a dormant/underwater victim's stamped bonus up to the new value, then liquidates for the extra bonus — taken from the borrower's residual collateral (solvent-toxic) or socialized onto suppliers (insolvent). LTV restamp also cuts a victim's borrow capacity. Bounded by governance config (`validate_risk_bounds`); the harm is the ability to *force and time* the worst-for-victim config onto a non-transacting account. Permissionless, untested.

**Proposed patch (recommended — surgical):** clamp bonus propagation downward: `updated_pos.liquidation_bonus = updated_pos.liquidation_bonus.min(cfg_bonus)` (raw u32 bps) instead of `= cfg_bonus` — mirrors the LT-lowering grandfathering in `risk/params.rs::apply_liquidation_threshold`; a third party can never ratchet bonus **up**, favorable propagation still works. (An HF gate is the wrong tool for bonus — bonus is not an HF input.) *Broader option:* make the HF gate unconditional across both branches to also cover LTV/fees, at the cost of an oracle-priced risk walk per keeper call. Add a restamp-then-liquidate front-run test.

---

### L1 — Low — Liquidation blocked by a cash-drained collateral market
**Where:** `pool/src/lib.rs:212-216` (`withdraw_accounting`), via liquidation seize (`.../liquidation/apply.rs`).
**Mechanism:** for `is_liquidation=true` only the *utilization cap* is skipped (`:214`); `require_reserves(net_transfer)` (`:212`) still reverts `InsufficientLiquidity` when pool `cash < net_transfer`. A cash-drained collateral market blocks liquidation of accounts holding it until re-supply.
**Impact:** temporary, self-healing liquidation delay — no theft. Inherent to share-based pools (you cannot transfer out underlying that was borrowed away); `clean_bad_debt` (no transfer) is unaffected.
**Proposed patch:** none strictly required; document, rely on `clean_bad_debt` for the socializable tail, and add keeper monitoring for cash-starved-but-liquidatable markets.

### L2 — Low — Clawback overstates tracked `cash` (no reserve reconciliation)
**Where:** `pool/src/cache.rs:37-46,113-141`; `lib.rs:977-980`.
**Mechanism:** `cash` is a pure internal counter, never reconciled to live SAC balance (a deliberate donation-attack defense). A clawback-enabled issuer can burn pool tokens out-of-band, so `cash > real balance`; `require_reserves` passes on the overstated counter and `transfer_out` then traps for late withdrawers.
**Impact:** the loss (the clawback itself) is inherent to holding a clawback token and outside protocol control; the protocol-attributable effect is loss *distribution* — first-come withdrawers exit whole, the last ones trap, instead of a socialized write-down. Bounded value-leak/fairness, not silent insolvency (you can never extract more than the real remaining balance).
**Proposed patch:** do **not** add naive live-balance reconciliation (breaks donation-attack safety). Instead add a governance-only reserve-reconciliation entrypoint that writes down `cash` and the supply index together when an out-of-band balance loss is detected, socializing the loss rather than trapping late withdrawers. (Composes with H1's force-socialize path.)

### L3 — Low — Issuer freeze of the pool trustline = market-wide DoS
**Where:** `pool/src/cache.rs:135-141` + every pool token movement.
**Mechanism:** all value movement for an asset routes through the pool address. Freezing the pool's trustline reverts supply/withdraw/borrow/repay/liquidation for that market. (Freezing a *user* does not brick the protocol — the pool holds the tokens, and repay is permissionless.)
**Impact:** issuer-reversible liveness DoS; absorbed by listing policy but the protocol ships no migration/off-ramp for stuck positions.
**Proposed patch:** document the freeze-capable-asset liveness assumption; optionally add a governance pause+`net_settle`-based debt off-ramp so positions in a frozen market can be unwound without pool→user transfers.

### L4 — Low — Fee-on-transfer / rebase overstates `cash`
**Where:** `controller/src/payments/mod.rs:28-39` (`transfer_amount` returns nominal); `pool/src/lib.rs:136` (supply `credit_cash(amount)`), `:256` (repay `credit_cash(net_repay)`).
**Mechanism:** supply/repay credit the *nominal* transferred amount, never the measured received delta; a fee-on-transfer/negative-rebase token leaves `cash` overstated (same failure surface as L2). `flash_loan` is unaffected — its pre/post balance bracketing fails closed.
**Impact:** low — Stellar SACs are not fee-on-transfer, so this does not apply to the realistic USDC-style listing; requires an exotic custom token. Real inconsistency (the balance-delta pattern already exists in flash-loan/swap paths).
**Proposed patch:** credit the measured `post_balance − pre_balance` delta on supply and repay, mirroring the flash-loan/swap measurement (`lib.rs:334-357`). Cheap; removes an entire token-class of cash overstatement.

### L5 — Low — Governance role grants escalate faster than the Sensitive floor
**Where:** `governance/src/op.rs:101-126` (`GrantGovRole`/`RevokeGovRole` → `DelayTier::Standard`); `timelock.rs:39-46`.
**Mechanism:** Standard uses the unfloored `min_delay`; Sensitive applies the ~7-day floor. `execute_self(executor=None)` is permissionless once ready. A captured PROPOSER can self-grant `GUARDIAN`/`ORACLE` (immediate `pause`/`set_spoke_asset_flags`/`set_sanity_band`) maturing at `min_delay`, faster than upgrades.
**Impact:** availability/oracle DoS reachable faster than intended; precondition is a compromised trusted role, CANCELLER can veto — defense-in-depth.
**Proposed patch:** reclassify `GrantGovRole`/`RevokeGovRole` to `DelayTier::Sensitive` (reuses the existing tier). No path granting immediate-power roles should mature faster than an upgrade.

### L6 — Low — Timelock constructor missing the 48h floor
**Where:** `governance/src/access.rs:205-215` → `timelock.rs:48-50` (`require_nonzero_delay` only). `TIMELOCK_MIN_DELAY_LEDGERS` (48h) is referenced by no enforcement code.
**Mechanism/impact:** a misconfigured deploy can set a sub-floor `min_delay`, collapsing the Standard-tier veto window (compounds L5). Not independently exploitable (deployer becomes owner/admin), but a latent config footgun.
**Proposed patch:** add `require_min_delay` (assert `min_delay ≥ TIMELOCK_MIN_DELAY_LEDGERS`, reuse `GenericError::InvalidTimelockDelay`) in `__constructor`. Non-decreasing updates keep it ≥ floor thereafter. Update the stale "shorter delays on non-mainnet" comment.

### L7 — Low — `set_sanity_band` has no minimum width (single-key market DoS)
**Where:** `price-aggregator/src/config.rs:102-123`; shared `common/src/validation.rs:139-145`.
**Mechanism:** setter guards enforce `min<max`, overlap, single-source *max* width, live-price containment — but no **minimum** width; anchored markets have no width cap. A `[p, p+1]` band around the current price passes containment; the next real print reverts `SanityBoundViolated` on every hard read (immediate `ORACLE_ROLE`, no timelock), bricking borrow/withdraw/liquidation until another ORACLE action.
**Impact:** single-key, no-timelock, recoverable market DoS (liquidation DoS can convert to bad debt — composes with H1).
**Proposed patch:** add `MIN_SANITY_BAND_BPS` (e.g. 50 bps half-width) to `validate_sanity_bounds` — `mul_div_floor(max-min, BPS, max+min) ≥ MIN`, reuse `OracleError::InvalidSanityBounds`. Choke point covers both `set_oracle_config` and `set_sanity_band`; legit ~500 bps bands unaffected.

### L8 — Low — Oracle-config containment checked only at propose-time (timelock TOCTOU)
**Where:** propose-time `governance/src/validate/oracle_probe.rs:63-73` vs execute-time `price-aggregator/src/config.rs:22-26` (`set_oracle_config` does shape validation only, no live-price probe — unlike `set_sanity_band`).
**Mechanism/impact:** the band is frozen at propose time; a normal price move outside `[min,max]` during the delay stores an out-of-band config, and the fail-closed hard read then reverts for that asset — a dead-on-arrival market. Not attacker-driven, governance-gated, recoverable via `set_sanity_band`.
**Proposed patch:** re-probe the live price at execute-time in `set_oracle_config`, mirroring `set_sanity_band`'s containment check (`resolve_with_config` under the new band before storing).

### I1 — Info — Wrong docstring on `update_account_threshold` (FIXED)
`weighted_collateral` rises monotonically with `liquidation_threshold`, so the `hf ≥ 1.05` gate can only fail when the restamp **lowers** the threshold (a raise only increases HF). The prior comment claimed it blocks threshold *raises* — inverted. **Corrected in place** at `pool_ops/mod.rs:241-247`.

### I2 — Info — `min_borrow_collateral` floor locks partial withdrawals
**Where:** `controller/src/risk/validation.rs:38-72`.
**Mechanism:** the floor branch reverts `MinBorrowCollateralNotMet` on post-op state whenever debt remains and `ltv_collateral < floor`; `repay` does not route through the gate, so a below-floor debt account can only exit by fully repaying. Default floor `5·WAD`.
**Impact:** temporary capital lockup / poor UX, no fund loss.
**Proposed patch:** scope the floor to debt-increasing paths — add `enforce_min_collateral_floor: bool` to `require_post_pool_risk_gates`, `true` from borrow/strategy-finalize, `false` from withdraw. HF (`≥1`) and LTV (`ltv_collateral ≥ total_debt`) legs still bind on withdraw, so solvency is preserved.

---

## 4. Investigated and refuted (transparency)

Cleared by dual-lens adversarial verification and independent cross-check:

- **Swap slippage / sandwich value loss (no controller min-out)** — controller enforces only `received > 0`; slippage is delegated to the trusted governance-set aggregator + caller-encoded route. Not a controller-boundary bug.
- **Delegate value extraction** (`close_position`/refunds → `caller`) — a delegate is a governance-approved *active* position-manager the owner explicitly added; `withdraw`/`borrow` already accept a `to` param, so a delegate is full-custody **by design**; the close/refund destinations add no capability.
- **Cross-function reentrancy via the arbitrary flash-loan receiver** — CEI is technically violated (transfer before the post-pool gate) and `require_not_flash_loaning` doesn't wrap plain borrow/withdraw, but a real Stellar SAC has no transfer hook and cannot re-enter; owner-auth + atomic rollback prevent any bypass. Defense-in-depth only (optional: wrap borrow/withdraw in the guard).
- **Supply-index reward-shortfall accrual trap** — the `+RAY` virtual offset is calibrated so `distributed ≤ rewards` for all validated inputs (verified numerically); safe-by-design.
- **`revenue ⊆ supplied` (I5) claim-revenue brick** — `add_protocol_revenue` mints into both counters; no reachable path drives `revenue > supplied` under correct bookkeeping. Defense-in-depth assert reasonable, not a live bug.
- **Liquidation bonus exceeding the HF-safe ceiling via `bonus_factor`** — `validate_liquidation_curve` caps `bonus_factor_bps ≤ BPS` (1.0×), so bonus ≤ ceiling.
- **`multiply` initial-payment pulled before payment-asset listing check** — pulls the caller's own funds under their auth; no victim.

---

## 5. Documentation / comment issues

- **I1 corrected in place.**
- Candidate imprecise comments to review (not auto-corrected — verify intent first): `common/errors.rs` `AnchorConfigMismatch` (error 227) defined but never raised; the price-aggregator "one hop, no quote chains" comment is not enforced for a quote's *anchor* leg (bounded by the cycle guard, so not a security bug, but the comment overstates the guarantee).

---

## 6. Test-coverage recommendations

Add adversarial tests for the confirmed edges (all currently untested): **H1** frozen-collateral-above-threshold unliquidatable+unsocializable (use the built-but-unused `with_freezable_market` primitive); **M1** restamp-then-liquidate front-run; **L1** cash-starved-market liquidation revert; **L4** fee-on-transfer cash overstatement; **L7** pinched-sanity-band DoS; **L8** timelock-window price-drift dead market; **I2** partial-withdraw-while-below-floor.

---

## 7. Methodology & effort

Phase 0–2 (understand / invariants / tests): manual review of the math/pool/liquidation/risk core + 5 mapping agents. Phase 3–6 (threat model / adversarial generation / council / patches): an 89-agent workflow — 7 Opus domain finders (high/xhigh effort) → dual-lens refutation → 3-judge severity council (conservative / attacker-economist / formal-methods) → minimal-patch research; plus a 12-agent focused non-standard-token workflow. Totals: 40 candidate findings → 16 survived adversarial verification → **12 confirmed** (1 high, 1 medium, 8 low, 2 info) after council dedup/ruling.
