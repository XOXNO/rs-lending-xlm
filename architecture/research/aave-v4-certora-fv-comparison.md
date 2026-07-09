# Aave V4 Certora FV Reports vs Our Certora Specs

Comparison of the four Certora formal-verification reports for Aave V4
(Hub, Spoke, Libraries — March 2026; TokenizationSpoke — April 2026) against
`verification/certora/` (230 `#[rule]`s across common/pool/controller as of
this writing). Layer mapping: Aave Hub ≈ our `pool` (+ controller accounting),
Aave Spoke ≈ our `controller`, Aave Libraries ≈ our `common` + controller math,
TokenizationSpoke ≈ no analog (ERC4626 wrapper; relevant only to the V2
pools→vaults direction).

Sources:

- Hub: `Certora-AaveV4-FV-Hub` (P-01..P-10, findings 3M/3L, all fixed)
- Spoke: `Certora-AaveV4-FV-Spoke` (P-01..P-07, findings 1M/1L, all fixed)
- Libraries: `Certora-AaveV4-FV-Libraries` (P-01..P-07, no findings)
- TokenizationSpoke: `Certora-AaveV4TokenizationSpoke-FV` (P-01..P-06, 1 info)

## Property-group parity

| Aave group | Their content | Our coverage | Status |
|---|---|---|---|
| Hub P-01 op integrity | add/remove/draw/restore/deficit/sweep per-op state deltas | `pool/integrity_rules` per-op preservation, `controller/position_rules` | parity |
| Hub P-02 nothing-for-zero | zero in ⇒ revert; non-zero ⇒ real balance movement | `solvency_rules::*_rejects_zero_amount` + `position_rules` effect rules | parity |
| Hub P-03 validity/DoS | only registered spoke acts; no cross-spoke change; op ordering adds no reverts | `account_isolation_rules` (state frame only); auth = Soroban `require_auth` | partial — no revert-non-interference rule |
| Hub P-04 field integrity | index ≥ RAY, fee bounds, offset ≤ shares | `index_rules` (index ≥ RAY, floor), `isolation_rules` param bounds | parity |
| Hub P-05 aggregated sums | Σ per-spoke shares/debt/deficit == asset totals (caught their M-03) | **none** — `consistency_rules` is single-op persistence only | **gap** |
| Hub P-06 solvency + share rate | external balance ≥ internal accounting; assets ≥ shares; rate monotone | `borrow_respects_reserves`, `claim_revenue_bounded_by_reserves`, `revenue_le_supplied_after_add_rewards`, index floor/monotone | **gap** on token-balance-vs-claims; note our supply index may drop by design on bad-debt socialization (floored, not monotone) |
| Hub P-07 additivity | split ops never beat single op | `pool/additivity_rules`, solvency round-trips | parity |
| Hub P-08 accrue integrity | idempotent, monotone, frame on other fields | `indexes_unchanged_when_no_time_elapsed`, `simulate_indexes_no_time_noop`, monotone-after-accrual | parity minus the other-fields frame |
| Hub P-09 view isomorphism | views == accrued state, same revert behavior | **none** (keeper + indexer consume our views) | **gap** |
| Hub P-10 temporal monotonicity | share rate/preview monotone over time; fee lemmas | `compound_interest_monotonic_in_time/_in_rate`, `supplier_rewards_conservation`, `supplier_rewards_plus_fee_equals_accrued_interest` | parity (no preview fns in our ABI) |
| Spoke P-01 frame + auth + pause | per-op single-user frame; paused/frozen semantics | `account_isolation_rules`, `market_guard_rules`, emode deprecated-blocks rules | partial — liquidation/clean_bad_debt not in frame rules |
| Spoke P-03 spoke-hub sums | Σ per-user == hub per-spoke totals | same gap as Hub P-05 | **gap** |
| Spoke P-04 health validity | health checked wherever HF can drop; HF<1 ⇒ ops only improve | `hf_safe_after_borrow/withdraw`, `supply_cannot_decrease_hf`, `ltv_borrow_bound_enforced` | **gap** on the universal below-threshold-only-improves rule |
| Spoke P-05 field integrity | noCollateralNoDebt; collateralFactor never non-zero→0; premium consistency | `no-collateral-no-debt.conf`, `no_collateral_account_cannot_borrow`, `ltv_less_than_liquidation_threshold` | partial — no param-transition rule; premium rules N/A until risk-premium lands |
| Spoke P-06 liquidation | healthy-cannot-liquidate, calc==realized, bonus bounds/monotonicity | `liquidation_rules` (strict decrease both legs, bonus derivation, seizure proportional, fee-on-bonus-only, HF=1 boundaries) | parity; ours stronger on derived-bonus, theirs adds bonus-monotone-in-HF |
| Spoke P-07 report deficit | deficit only when 1 collateral & debt > collateral | `clean_bad_debt_requires_qualification/_zeros_positions`, $5-band boundary rules | parity |
| Libraries P-01..P-07 | per-function CVL equivalence, ShareMath mono/additivity | `common/math_rules`, `controller/math_rules`, `tolerance_math_rules`; scaled-balance model instead of shares | parity in intent (equivalence-style vs identity/bounds-style) |
| TokenizationSpoke P-01..P-06 | ERC4626: round-trips never inflate, dustFavorsTheHouse, max* bounds, front-running safety, totalSupply == hub ledger | no analog | N/A today; template for V2 vaults |

## Where we exceed their scope

Oracle staleness/tolerance/dual-source compose policies (they scope oracle
quality out entirely), strategy operations (multiply, swap-debt,
swap-collateral, repay-with-collateral), flash-loan reentrancy guard, e-mode
transitions, isolation ceilings, i128-boundary numerics. Their scope is deeper
on conservation/aggregation; ours broader on risk machinery.

## Their findings as probes against our code

| Finding | Bug shape | Our exposure |
|---|---|---|
| Hub M-01 | fee-share rounding inconsistent with debt rounding → share rate dips | covered: `supplier_rewards_plus_fee_equals_accrued_interest`, half-up rounding-direction rules |
| Hub M-02 | value moved between buckets rounded more than once → total dips | low: single half-up rounding per reconstruction; bad-debt path floored (`bad_debt_socialization_keeps_supply_index_above_floor`) |
| Hub M-03 | re-registering an existing spoke overwrites live accounting | **unverified**: no rule that `create_market` on an existing asset panics |
| Hub L-01 | index accrual skipped when only premium debt exists | N/A on main; direct probe for the risk-premium branch settle gating |
| Hub L-02 | arbitrary transferFrom params fake deposits | N/A: Soroban `require_auth` + SAC transfer-from-caller model |
| Hub L-03 | views omit accrued fees | same as view-isomorphism gap below |
| Spoke M-01 | collateral factor non-zero→0 bricks liquidation (liquidation validates factor ≠ 0) | **unverified**: no param-transition rule; needs a check that liquidation of live debt survives any governance param update |
| Spoke L-01 | unchecked cast flips premium offset sign | N/A on main; probe for premium branch (`i256_no_overflow` pattern exists) |

## Recommended additions (priority order)

1. **Bounded-N conservation** (their sumOfSpoke*/userShareConsistency): with 2–3
   symbolic accounts, Σ scaled supply/debt == market totals after any op.
   Highest-signal missing family — it is what caught their M-03. Sunbeam has no
   CVL ghosts, so bounded-N in the harness is the feasible form.
2. **External-balance solvency** (their solvency_external): SAC
   balance(pool) ≥ supplied − borrowed + revenue; light form = per-op delta
   rule (token outflow ≤ accounting decrease) over the existing SAC summary.
3. **Below-threshold-only-improves**: for each entry point, HF<1 pre ⇒
   HF' ≥ HF. Formalizes the supply threshold-tightening fix (currently
   code-gated, not FV'd).
4. **`create_market` idempotency**: re-creating an existing market panics
   (cheap; mirrors Hub M-03).
5. **Param-transition safety**: liquidation threshold/LTV cannot go
   non-zero→0 while debt is live, or liquidation path provably tolerates 0
   (mirrors Spoke M-01).
6. **View isomorphism**: `bulk_get_sync_data` / account views equal
   post-accrual state at the same ledger (keeper and indexer rely on this).
7. **Frame rules for liquidation paths**: extend `account_isolation_rules` to
   `liquidate` and `clean_bad_debt` (their onlyOneUserDebtChanges).
8. **When risk-premium lands**: settle idempotency (twice-per-ledger == once),
   drawn==0 ⇒ premium==0, accrual not skipped when only premium debt exists
   (Hub L-01), checked casts (Spoke L-01).

Report quirk for the record: the Spoke report cites "C-01 in the companion
Libraries report" for `monotonicityOfDebtDecrease_collateralIncrease`, but the
Libraries report lists zero findings — the fix trail behind
`collateralToLiquidateValueLessThanDebtToLiquidate` ("Formally Verified After
Fix") is not documented anywhere in the published set.
