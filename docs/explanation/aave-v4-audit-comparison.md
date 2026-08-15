# Aave V4 audit corpus vs. XOXNO Lending

**Date:** 2026-08-14
**Sources:** all 10 PDFs in `aave/aave-v4/audits` (Blackthorn ×2, Trail of Bits,
ChainSecurity ×3, Certora ×4), read in full.
**Our commit:** branch `controller-crazy-optimal`, `634dc8f8`.

Evidence labels used throughout: **Observed** = read directly from Aave's report
text or our source; **Inferred** = follows from that evidence but not reproduced;
**Unverified** = plausible, needs a test or proof.

---

## 1. Executive summary

The two protocols share a hub/spoke *vocabulary* but differ in three load-bearing
places, and every one of those differences moves whole classes of Aave findings
out of scope for us — while creating two classes that are ours alone.

**What the audit corpus actually contains.** Across all 10 reports: 0 critical,
0 high, 5 medium, ~14 low, ~20 informational, plus ~110 formally verified
properties. The mediums cluster into exactly three root causes:

1. **Risk-premium rounding** (ToB-7, CS-AAVE4-001, Blackthorn M-1) — a 2-wei
   premium recomputation running *after* the health-factor check.
2. **Vault share-price rounding** (CS-AAVE4-003, Certora Hub M-01/M-02,
   Blackthorn L-6) — `totalAssets/totalShares` non-monotonicity from fee and
   deficit rounding.
3. **Liquidation liveness** (CS-AAVE4-002/021, Certora Spoke M-01) — an
   unrelated paused or zero-CF reserve making a position unliquidatable.

We are **structurally immune to (1) and (2)** and **exposed to a wider-blast-radius
version of (3)**.

**Headline results:**

| | Count | Notes |
|---|---|---|
| Not applicable — design difference removes the class | 24 | Premium debt, vault share math, malicious-spoke trust, EIP-712 gateway |
| Applicable — same shape, we already mitigate | 9 | Several where our mitigation is stronger than Aave's fix |
| **Applicable — needs work** | **6** | Detailed in §4; two are genuinely ours-only |
| Informational / integration hygiene | ~9 | Event and view-surface sweep |

**The six that need work**, in priority order:

| # | Issue | Aave analogue | Status |
|---|---|---|---|
| A-1 | Paused collateral blocks liquidation of *every* account holding it | CS-AAVE4-002 (Med, fixed) | Widened by our pro-rata seizure |
| A-2 | `SpokeUsage` is a second accumulator that can drift from pool totals | *(no analogue — ours only)* | No reconciliation proof exists |
| A-3 | Permissionless `update_indexes` + chunked accrual → fee/index path dependence | CS-AAVE4-004 (Low, partially fixed) | Reachable, unproven |
| A-4 | Soroban CPU budget at max `PositionLimits` with dual-source oracle | CS note 8.1 (gas > reward) | Harder than EVM: no "pay more" escape |
| A-5 | No additivity ("anti-splitting") proofs | Certora Hub P-07 | Gap vs. their proven set |
| A-6 | No view/accrue isomorphism proofs | Certora Hub P-09/P-10 | Root cause of Blackthorn L-6 |

---

## 2. Architecture map

### 2.1 What "hub" and "spoke" mean in each system

This is the single most important thing to get right before comparing findings,
because the words do not denote the same objects.

**Aave V4.** The Hub is a deployed contract holding all liquidity, running
share-price vault accounting, whitelisting spokes per asset and holding their
caps. A Spoke is a *separately deployed, upgradeable* contract that holds user
positions and is responsible for its own solvency. The Hub does not check spoke
solvency — it trusts spokes. (**Observed**, CS Jan-2026 §2.2.1.2: "the Hub does
not perform any solvency checks on Spokes or users… Only trusted Spokes should be
given access to the Hub's liquidity.")

**Ours.** The pool is a custody + per-market accounting contract with exactly one
owner, the controller ([contracts/pool/src/lib.rs:88](contracts/pool/src/lib.rs:88),
every mutator `#[only_owner]`). A "hub" is a `hub_id` namespace inside the market
key `HubAssetKey { hub_id, asset }`. A "spoke" is a `spoke_id` **configuration
record inside the controller** — `ControllerKey::Spoke(u32)`,
`ControllerKey::SpokeAsset(u32, HubAssetKey)`,
`ControllerKey::SpokeUsage(u32, HubAssetKey)`
([common/src/types/controller.rs:579](common/src/types/controller.rs:579)). An
account binds to one `spoke_id` at creation and never changes it (ADR-0009).

**Consequence:** our spokes are *data*, Aave's are *code*. There is no path by
which a spoke calls the pool. The entire Aave trust statement — "users using a
Spoke must not only trust the Hub but also all other Spokes connected to that Hub"
— has no counterpart here.

### 2.2 Side-by-side

| Dimension | Aave V4 | Ours | Same? |
|---|---|---|---|
| Supply accounting | Share-price vault: `totalAssets/totalShares`, virtual 1e6 assets + 1e6 shares | Index model: RAY supply index, scaled shares ([`cache/scale.rs`](contracts/pool/src/cache/scale.rs)) | **Different** |
| Debt accounting | `drawnShares` × `drawnIndex` (RAY) | `scaled_amount` × borrow index (RAY) | Same |
| Interest accrual | Linear per-call, compounding by frequency | Compound, chunked at `MAX_COMPOUND_DELTA_MS` ([`interest.rs:23`](contracts/pool/src/interest.rs:23)) | Different |
| Protocol fee | `realizedFees` stashed in assets, minted via permissioned `mintFeeShares` | Minted immediately as revenue shares; index shortfall routed to protocol ([`interest.rs:63`](contracts/pool/src/interest.rs:63)) | Different |
| Risk premium | Premium shares, offsets, `collateralRisk` sorting, `riskPremiumThreshold` | **None** | **Different** |
| Cap location | Hub, per `(assetId, spoke)`, whole asset units, `uint40` | Controller, per `(spoke_id, hub_asset)`, asset units → scaled at live index ([`spoke_usage.rs:178`](contracts/controller/src/spoke_usage.rs:178)) | **Different** |
| Spoke whitelist | Hub `addSpoke` + `spoke.active`; spokes fully trusted | No spoke ACL at the pool; pool trusts only the controller | **Different** |
| Bad debt | `deficit` counted **into** `totalAssets` so share price never falls; `eliminateDeficit` burns a whitelisted spoke's shares | **Direct socialization into the supply index**, floored ([`interest.rs:96`](contracts/pool/src/interest.rs:96)); `recapitalize` fills backing shortfall | **Different** |
| Liquidation target | Liquidator picks one `(collateral, debt)` pair | Multi-asset debt vector; **pro-rata seizure across all collateral** ([`math.rs:240`](contracts/controller/src/positions/liquidation/math.rs:240)) | **Different** |
| Bonus curve | `maxLiquidationBonus` per reserve, `healthFactorForMaxBonus` + `liquidationBonusFactor` per spoke, linear ramp | Same three parameters, same linear ramp ([`curve.rs:74`](contracts/controller/src/positions/liquidation/curve.rs:74)) | **Nearly identical** |
| Close sizing | `targetHealthFactor` + `DUST_LIQUIDATION_THRESHOLD` ($1000) | `liquidation_target_hf_wad` + `BAD_DEBT_USD_THRESHOLD` promotion to full close ([`curve.rs:147`](contracts/controller/src/positions/liquidation/curve.rs:147)) | Same idea |
| Dynamic risk config | `configKey` per position → mapping of dynamic configs | Risk tuple stored **inline** on the position; restamped on risk-increasing actions ([`risk/params.rs`](contracts/controller/src/risk/params.rs)) | Different mechanism, same goal |
| Oracle | Chainlink `latestRoundData`, **only `answer` used, timestamp not validated** | Dual-source primary+anchor, staleness + deviation + validity, fail-closed (ADR-0004/0005) | **Different** |
| Reentrancy | No guards; CEI violated in liquidation; hook tokens unsupported | Flash-loan flag gates monetary flows; measured balance deltas (ADR-0011/0013) | **Different** |
| Non-standard tokens | Assumed standard ERC20 | Credit = measured receipt everywhere (INV-ACCT-03) | **Different** |
| Per-user position cap | `MAX_USER_RESERVES_LIMIT`, added in v7 *after* the audits | `PositionLimits`, enforced on entry ([`validation.rs:60`](contracts/controller/src/risk/validation.rs:60)) | Same mechanism |
| Storage lifetime | N/A | Soroban TTL: persistence class + renewal (INV-STOR-01) | **Ours only** |

---

## 3. The three divergences that matter, with reasoning

### 3.1 Index model vs. share-price vault — **ours is better here, decisively**

Aave's Hub prices supply shares as `totalAssets / totalShares` where
`totalAssets = liquidity + swept + deficit + totalOwed − realizedFees − unrealizedFees`.
Every term in that expression is a rounding site, and the *same* expression is
evaluated by both view functions and the mutating `accrue()`. Five separate
findings across four firms are consequences of that one design choice:

- **Certora Hub M-01** — `getFeeShares` rounded inconsistently with `totalDebt`, so the share rate could decrease.
- **Certora Hub M-02** — `reportDeficit` moved assets from the debt bucket to the deficit bucket, both rounded up, so total assets *shrank*.
- **CS-AAVE4-003 / Blackthorn L-6** — unrealized fee shares are in the *view* denominator but only materialize in `accrue()`; the "donation" of one block reappears as a minted fee share the next, so the share price visibly drops.
- **CS-AAVE4-004** — `feesAmount` rounds down before the fee percentage is applied, so calling `accrue()` every second rounds the fee to zero. Quantified at ~$30k/yr on a $1M WBTC book.
- **Blackthorn M-2** *(acknowledged, won't fix)* — with `VIRTUAL_ASSETS = VIRTUAL_SHARES = 1e6` and 5-decimal high-value assets, up to **90% of yield** is permanently stranded on dead shares.

Our supply index is not derived from a balance ratio. It is advanced by
`update_supply_index(supplied, old_index, supplier_rewards)` and moved down only
by explicit socialization. **Inferred:** this makes us immune to the entire class
— there is no first-depositor inflation attack because donating tokens to the pool
does not move the index (INV-ACCT-02: "Donations do not create lendable cash"),
and there are no virtual shares to strand yield on.

The cost of our choice is that **the supply index is not monotone** (ADR-0012,
INV-IDX-03). Aave deliberately preserved monotonicity by parking bad debt in
`deficit` inside `totalAssets`. That is a real integration advantage for them:
ERC-4626 wrappers and external accounting systems can assume a non-decreasing
share price. We cannot offer that guarantee, and `docs/reference/architecture.md`
correctly warns consumers.

**Judgement:** ours is better on correctness and attack surface; theirs is better
for naive integrators. Given that we have no tokenized spoke and no ERC-4626
surface, we are trading away a benefit we do not currently use. Keep our design;
keep the warning loud.

### 3.2 Deficit tracker vs. direct socialization — **ours is better on liveness, worse on loss localization in time**

Aave's `_evaluateDeficit` reports a deficit only when the liquidated collateral is
emptied **and** `activeCollateralCount <= 1`, where "active" means *any non-zero
balance*. Two firms independently found the same griefing vector (ToB-AAVE-1
Medium, Blackthorn L-3): supply 1 wei of a second collateral and deficit reporting
is blocked forever. Aave **accepted the risk** on both, arguing positions can be
progressively liquidated down to one collateral — at a loss to the liquidator.
ChainSecurity then found a second route to the same state (CS-AAVE4-008): a
config-key overflow letting an existing collateral factor be set to 0, producing a
position with debt and `activeCollateralCount == 0`.

Our gate is **value-based, not count-based**:

```rust
// contracts/controller/src/positions/liquidation/curve.rs:25
pub(crate) fn is_socializable_bad_debt(total_debt: Wad, total_collateral: Wad) -> bool {
    total_debt > total_collateral && total_collateral <= Wad::from(BAD_DEBT_USD_THRESHOLD)
}
```

**Inferred:** we are structurally immune to ToB-AAVE-1 and Blackthorn L-3. Adding
1 wei of a second collateral adds a negligible *value*, which does not lift
`total_collateral` above the threshold. This is precisely the fix Trail of Bits
recommended ("treat collateral below a protocol dust threshold as inactive") and
Aave declined — we already ship it.

We are also immune to CS-AAVE4-008's consequence: `total_collateral` is
`sum_supply_usd`, the *unweighted* USD value
([`risk/totals.rs:44`](contracts/controller/src/risk/totals.rs:44)), so setting a
liquidation threshold to zero cannot hide collateral from the bad-debt gate the
way a zero collateral factor hid it from Aave's `activeCollateralCount`.

The residual: an attacker *can* hold `total_collateral` just above
`BAD_DEBT_USD_THRESHOLD` to block the dust-gated path. Our escape hatch is
`force_socialize_bad_debt` ([`lib.rs:715`](contracts/controller/src/lib.rs:715)),
owner-gated with the looser `Insolvent` gate (`debt > collateral`, no dust cap).
So the liveness dependency is on governance rather than on liquidator economics —
strictly better than Aave's "liquidate at a loss" answer, but it is a dependency
and belongs in the operations runbook.

The genuine cost of direct socialization: the loss lands on whoever is supplying
at that instant, with no insurance layer in between. Aave's deficit sits on the
books until an umbrella spoke burns shares to cover it, which spreads the loss
across a designated backstop rather than across incumbent suppliers. **If a
backstop/insurance module is ever added, revisit this.** Until then, direct
socialization is the honest accounting: the loss exists and the index says so.

### 3.3 Caps and whitelist in the controller, not the pool — **ours is better, at a real cost**

Aave puts `addCap`/`drawCap` and the spoke whitelist in the Hub because spokes are
untrusted external contracts. Even so, Blackthorn L-5 showed the caps did **not**
contain a malicious spoke: it could call `Hub.add(assetId, amount, from=victim)`
against any user who had approved the Hub, then `Hub.remove()` to reset its own
cap usage and repeat. Aave's fix (PR #955) was to move approvals from the Hub to
the Spoke — i.e. to stop relying on the cap as a containment boundary at all.
Certora Hub L-02 found the same root cause independently, and Certora Hub M-03
found that re-calling `addSpoke()` on an existing spoke silently zeroed its
accounting.

Our pool has one owner and no spoke concept, so:

- Blackthorn L-5 / Certora Hub L-02: **not applicable.** No external party can invoke pool mutators; `transfer_amount_measured` always pulls from the authorized caller.
- Certora Hub M-03: our analogue is guarded — `add_asset_to_spoke` asserts the asset is not already in the spoke ([`config/asset.rs:22`](contracts/controller/src/config/asset.rs:22)) and `create_market` panics on a duplicate hub-asset pair.

Two additional wins from putting caps at the controller:

- **Cap granularity.** CS-AAVE4-018 flags that Aave's caps are in whole asset units (`$1` for USDC vs `$100k` for a BTC derivative) and `uint40`-bounded. Ours are `i128` asset units validated against the asset's decimal domain (`require_cap_within_asset_domain`), and converted to scaled shares at the live index via `calculate_scaled_cap`, so interest growth cannot make a configured cap ambiguous (ADR-0015). Strictly better.
- **Zero means zero.** ADR-0015 makes a zero cap admit nothing. Aave uses `MAX_ALLOWED_SPOKE_CAP` as the "unlimited" sentinel, which is how CS-AAVE4-023 (fee receiver silently given an unlimited add cap) happened.

**The cost, stated honestly:** Aave can ship a new risk regime as a new Spoke
contract without touching the Hub. We cannot — a new regime is a controller
upgrade. We have bought a much smaller trust surface with reduced modularity.
For a protocol of our size that is the right trade; it stops being right if we
ever want third-party spokes, and at that point the whole Aave cap/whitelist
design becomes the reference again.

### 3.4 Where the missing risk premium leaves us

Removing premium debt deletes 8 findings outright (§4, class A) including the
root cause of *both* Trail of Bits mediums and one of three ChainSecurity
mediums. It also deletes: `riskPremiumThreshold` config-DoS (ToB-3, CS-022),
`percentMulUp` threshold slack (CS-025), the `Array.sort()` 170-collateral stack
overflow (CS-012 — we have no `collateralRisk` sorting), and the unsafe
`int` cast in `getPremiumDelta` (Certora Spoke L-01).

The premium mechanism is the single largest complexity source in Aave V4 —
ChainSecurity's own maturity note says it "introduces significant complexity
across multiple components, requiring careful coordination between Hub premium
tracking and Spoke user premium accounting." Not having it is a straightforward
win on audit surface. What we give up is risk-priced borrowing: a borrower against
volatile collateral pays the same rate as one against a stablecoin. We compensate
partially through per-asset LTV/threshold and per-spoke regimes, which is coarser
but has no accounting machinery behind it.

**Judgement:** correct call at our maturity. If premium pricing is ever added,
treat ToB-AAVE-7 and CS-AAVE4-001 as the specification: keep premium in full
precision (token decimals + RAY), and never let a debt-mutating step run *after*
the health-factor gate.

---

## 4. Finding-by-finding triage

### Class A — Risk premium (8 findings): **not applicable**

ToB-AAVE-3, ToB-AAVE-7, CS-AAVE4-001, CS-AAVE4-022, CS-AAVE4-025, Blackthorn M-1,
Certora Spoke L-01, and the premium half of CS-AAVE4-002.

**But the *pattern* behind ToB-AAVE-7 must be checked in our code.** Their bug was
ordering: `_refreshAndValidateUserPosition` (health check) ran *before*
`_notifyRiskPremiumUpdate` (which could add 2 wei of debt), so a user at exactly
HF = 1 passed validation and was instantly liquidatable.

Our analogue is `enforce_post_pool_solvency`, which restamps first and validates
second — the correct order:

```rust
// contracts/controller/src/positions/mod.rs:89
pub(crate) fn enforce_post_pool_solvency(...) -> bool {
    let restamped = risk::restamp_listed_supply_ltv(cache, account);
    validation::require_post_pool_risk_gates(env, cache, account);
    restamped
}
```

**Unverified:** that *nothing* in `finalize_position_flow` or any strategy tail
mutates a value-bearing field after the gate. Needs a formal rule (§5, V-1).

Separately, our `update_account_threshold` keeper
([`keepers.rs:79`](contracts/controller/src/keepers.rs:79)) is the function most
analogous to `updateUserRiskPremium` — a third party can restamp another account's
risk tuple. We gate it correctly and *more* strictly than Aave: liquidation
parameters only tighten if the account still clears **HF ≥ 1.05** afterwards
(`apply_gated_liquidation_params` → `clears_min_hf`,
[`risk/params.rs:66`](contracts/controller/src/risk/params.rs:66)), and the whole
call reverts if the post-update HF is below 1.05. Aave has no such gate; this is
one of the clearest places where our design is ahead of their post-fix state.

### Class B — Vault share-price accounting (7 findings): **not applicable, one analogue survives**

CS-AAVE4-003, CS-AAVE4-004, Blackthorn M-2, Blackthorn L-6, Certora Hub M-01,
M-02, L-03, plus CS Mar-2026 note 8.3.

Reasoning in §3.1. **One analogue does survive:** CS-AAVE4-004's "accrue often to
round fees to zero" shape is reachable against us. `update_indexes` is
permissionless (`caller.require_auth()` only,
[`keepers.rs:18`](contracts/controller/src/keepers.rs:18)), and our accrual is
*chunked* at `MAX_COMPOUND_DELTA_MS`, so the number and size of chunks is
caller-influenced. Our rounding residual goes the opposite way to Aave's — the
part of supplier rewards that cannot lift the index is captured as protocol
revenue via `supply_index_reward_shortfall` — so frequent accrual should favor the
*treasury*, not drain it. **Unverified.** This is finding A-3; see §5, V-3.

### Class C — Deficit / bad-debt reporting (4 findings): **structurally immune, one residual**

ToB-AAVE-1, Blackthorn L-3, CS-AAVE4-008, CS-AAVE4-021. Reasoning in §3.2.
Residual: the `BAD_DEBT_USD_THRESHOLD` boundary can be straddled; escape hatch is
owner-gated. Existing coverage: `bad_debt_socialization_threshold_boundary` in
`certora/controller/spec/`. Add the adversarial straddle case (§5, V-7).

### Class D — Liquidation liveness under pause (2 findings): **applicable, wider blast radius — finding A-1**

CS-AAVE4-002 was a Medium: a user borrows 1 wei of every reserve, one asset is
later paused in the Hub, and `refreshPremium` reverts, so *all* liquidations of
that user revert. CS-AAVE4-021 was the same shape via `reportDeficit`. Aave fixed
both by allowing those two calls through on paused spokes.

Ours is the same *shape* with a **larger blast radius**, and the reason is our
pro-rata seizure. `build_liquidation_plan` enforces pause flags on every payment
asset *and* on every seized collateral asset:

```rust
// contracts/controller/src/positions/liquidation/plan.rs
for entry in seized_collaterals.iter() {
    enforce_spoke_asset_flags(env, cache, account.spoke_id,
                              &entry.hub_asset, FreezePolicy::AllowOnExit);
}
```

`FreezePolicy::AllowOnExit` still asserts `!paused`
([`positions/mod.rs:317`](contracts/controller/src/positions/mod.rs:317)). Because
seizure is pro-rata across *all* collateral rather than a single liquidator-chosen
asset, pausing one asset makes **every account holding a material amount of it**
unliquidatable — not just accounts the liquidator would have targeted there.

ADR-0008 states this is intentional: "A paused debt asset cannot be repaid or
liquidated until governance resolves the condition." That is a defensible policy
for a *debt* asset whose price is untrustworthy. It is much harder to defend for a
*collateral* asset that merely happens to be in the account, and Aave's audit
history is direct evidence that the pattern is severe enough to fix.

Options, with trade-offs:

1. **Keep and document.** Add an operations runbook entry: pausing a widely held
   collateral halts liquidation protocol-wide for holders. Cheapest, honest,
   leaves the exposure.
2. **Skip paused collateral instead of reverting.** Non-trivial: dropping a
   position changes `total_collateral` and therefore `proportion_seized`,
   `weighted_coll`, the bonus bounds, and the HF math. Would need the whole
   snapshot recomputed over the eligible subset, and the seizure would then
   over-concentrate on unpaused assets. Do not do this without a full re-derivation.
3. **Split the flag.** Distinguish "paused as debt" (blocks repay legs) from
   "paused as collateral" (blocks entry but permits seizure). Closest to Aave's
   fix, which was precisely to let the liquidation-critical calls through.

**Recommendation:** (1) now, evaluate (3) next. Do not attempt (2) casually.

Note also that `enforce_spoke_asset_flags` is a no-op when the asset has no cached
spoke config — a *delisted* asset does not block exits. That is deliberate and
exit-safe, and worth an explicit test so it is not "fixed" later by accident.

### Class E — Liquidation under liquidity crunch (3 findings): **partly ahead, one gap**

- **Blackthorn L-9** recommended repaying debt *before* seizing collateral so a same-asset liquidation can use the freed liquidity. Aave **declined**, arguing repay-first bumps the supply share price via rounding donations, adding friction for liquidators. **We already repay first** — `process_liquidation` runs `apply_liquidation_repayments` then `apply_liquidation_seizures` ([`liquidation/mod.rs:56`](contracts/controller/src/positions/liquidation/mod.rs:56)) — **and Aave's objection does not transfer to us**, because in an index model a repayment does not move the supply index at all. We get the benefit they declined, without the cost they cited. **(Inferred; worth a same-asset regression test.)**
- **CS-AAVE4-005 / Blackthorn L-8**: Aave added `receiveShares` so a liquidator can take collateral shares when the Hub is cash-short. We have no equivalent. We do have a *preventive* measure Aave lacks: `require_liquidation_buffer` reserves `LIQUIDATION_BUFFER_BPS` of supplied value against ordinary borrow draws ([`guards.rs:33`](contracts/pool/src/guards.rs:33)). **Inferred:** the buffer covers the common case (utilization-driven crunch) but not a crunch caused by large withdrawals or by seizing more than the buffer. Evaluate a share-receipt path; low priority while the buffer holds.
- **CS-AAVE4-010** (bonus slippage between submit and execution): applies to us identically — our bonus is also a function of live HF. Liquidator-side concern; document it, as Aave did.

### Class F — Malicious-spoke trust (4 findings): **not applicable**

Blackthorn L-5, Certora Hub L-02, Certora Hub M-03, and Aave's standing trust
statement. Reasoning in §3.3. Add regression tests pinning the guards that make
this true (§5, V-8).

### Class G — Unbounded iteration / resource exhaustion (3 findings): **applicable in Soroban terms — finding A-4**

- **CS-AAVE4-012** (quicksort stack overflow at 170 collaterals): not applicable — no sorting.
- **Blackthorn L-1** (OOG in account-data refresh): applicable in principle, but we enforce `PositionLimits` on every entry path, whereas Aave only added `MAX_USER_RESERVES_LIMIT` in v7 *after* these findings.
- **CS note 8.1** (liquidation gas cost may exceed reward): this is the one that matters, and it is **harder on Soroban than on EVM**. On EVM a liquidator can pay more gas; on Soroban, exceeding the CPU/memory budget is a hard failure with no escape. `calculate_account_risk_totals` loops every position *and* resolves a dual-source price per distinct asset. **Unverified:** that a worst-case account at `max_supply_positions + max_borrow_positions`, all distinct assets, all dual-source, fits the budget for `liquidate` — the longest path, since it calls the totals routine twice (plan + post-totals) plus seizure and repayment batches.

This is the single most consequential unproven property in the comparison, because
failure mode is "position can never be liquidated". See §5, V-4.

### Class H — Rounding vs. small positions (2 items): **applicable, quantify**

CS Mar-2026 note 8.4 gives a closed form for the position value below which
liquidation is unprofitable: `V < L_round / (b·(1−f))`, worked to ~4.4¢ for
WBTC-debt/ETH-collateral. Our rounding is also against the liquidator (seizure
floors via `to_asset_floor`, debt ceils via `unscale_borrow_ceil`), so the same
formula applies with our own rounding-site count. We are better protected than
Aave because `MinBorrowCollateralUsd` puts a floor on position size and Aave has
no minimum debt size — but we should instantiate the formula for our supported
decimal range rather than assume. See §5, V-6.

### Class I — Governance parameter changes blocking user actions (3 items): **applicable, same as theirs**

ToB-AAVE-3 and CS-AAVE4-022 are premium-specific (not applicable), but **CS note
8.2 applies to us verbatim**: a paused asset keeps accruing interest while
repayment is disabled, so users are forced to accrue debt they cannot pay down.
ADR-0008 makes this an explicit choice. Aave's mitigation is operational ("pausing
is not expected to be long-lasting; the Spoke owner can update the interest rate
to mitigate"). Adopt the same operational guidance — we have the same lever via
`upgrade_liquidity_pool_params`.

Our `set_spoke_asset_flags` ratchet ([`config/asset.rs:148`](contracts/controller/src/config/asset.rs:148))
is stronger than Aave's equivalent: flags can only tighten, and relaxation requires
timelocked governance (ADR-0007). Blackthorn L-7 (freezing an asset does not
freeze *future* spokes) has no analogue because our flags live on the
`(spoke_id, hub_asset)` row itself, not on a snapshot iterated at freeze time.

### Class J — Events and integration surface (~9 informational): **sweep recommended**

ToB-AAVE-4 (`liquidatedCollateral` documented as liquidator proceeds but emitted
as gross seizure), CS-AAVE4-006 (premium delta and deficit clearing not emitted),
CS-AAVE4-011 (spoke verbs don't return share amounts), CS-AAVE4-013 (ambiguous
revert reasons), CS-AAVE4-019/024/028 (missing reverse lookups; wrong answers on
uninitialized state), CS-AAVE4-031 (rounded return values overstate the deficit).

We already document two event-shape divergences for indexers in
`docs/reference/architecture.md` (`SwDebtR` vs `Multiply`, and
`UpdatePositionBatchEvent` ordering before `CleanBadDebtEvent`), which is exactly
the right instinct. The specific thing to check: our `SeizeEntry` carries `amount`
and `protocol_fee` separately — confirm the emitted event and its doc comment
agree on whether `amount` is gross or net of fee. That is ToB-AAVE-4 exactly.
See §5, V-9.

### Class K — Signature gateway / position managers (3 findings): **mostly not applicable**

ToB-AAVE-6, CS-AAVE4-007, CS-AAVE4-015 are all EIP-712 gateway issues. Soroban's
native auth framework replaces that surface. Our analogues are the
`PositionManager(Address)` registry and per-account `Delegates` (`MAX_DELEGATES = 16`);
INV-AUTH-02 already states delegates cannot grant or renew their own authority.
CS-AAVE4-007's lesson — batched intents can be front-run into partial fulfilment —
is worth keeping in mind for multi-leg strategy entrypoints.

### Class L — Type bounds (1 note): **reproduce the analysis for our domain**

CS note 8.5 bounds `drawnIndex` (uint120 → ~70 years at 30% APR), `premiumOffset`
(int200), and caps (uint40 → ~1 trillion units). Our values are `i128` throughout
with RAY scaling, and INV-IDX-01/02 assert configured index maxima. Do the same
arithmetic for our domain and write it down: max borrow index reachable given
`MAX_COMPOUND_DELTA_MS` chunking and the rate cap, and the largest asset balance
representable before `Ray` multiplication overflows.

### Class M/N — AccessManagerEnumerable, TokenizationSpoke: **not applicable**

CS-AAVE4-026/029/030 concern an enumerable access manager we do not use. The
TokenizationSpoke reports concern an ERC-4626 wrapper we do not have. **One
transferable lesson:** `maxDeposit()` overestimated capacity because
`totalAssets()` rounded down and was subtracted from the cap. If
`contracts/defindex-strategy` exposes any max/preview pair, check for the same
rounding-direction mismatch.

---

## 5. Verification plan

We already have a strong base: `certora/` with ~150 rules across controller, pool
and price-aggregator; `tests/fuzz` with 7 targets; `cargo-mutants` split into 14
scoped targets; miri; per-crate coverage. The plan below is expressed as
*additions to existing files*, ordered by the risk each retires.

### Priority 1 — retire findings A-1 through A-4

**V-1 — No value-bearing mutation after the solvency gate** *(retires the ToB-AAVE-7 pattern)*
`certora/controller/spec/health_rules.rs`. For every verb that calls
`enforce_post_pool_solvency`, assert that the account's `total_debt` and
`weighted_collateral` at the end of the transaction equal the values the gate
observed. This is the rule that would have caught ToB-AAVE-7 and it generalizes
past the premium mechanism that caused it.

**V-2 — Liquidation liveness under pause** *(quantifies A-1)*
`contracts/controller/tests/positions/` integration test. Construct an unhealthy
account with N collaterals; pause one; assert the current behavior (whole
liquidation reverts) and pin it with an explicit named test so the policy is
visible rather than emergent. Add the delisted-asset counterpart asserting exits
still work. Then decide between options (1)/(3) in §4-D with the test as evidence.

**V-3 — Accrual path independence** *(retires A-3, the CS-AAVE4-004 analogue)*
`certora/pool/spec/` + a fuzz target in `tests/fuzz/fuzz_targets/rates_and_index.rs`.
Property: for any time span T and any partition of T into chunks, the terminal
`(supply_index, borrow_index, revenue)` differs from the single-chunk result by at
most a bounded epsilon, **and the frequent-caller partition never yields a larger
supplier claim than the single-chunk one**. The directional half is the security
property; the epsilon half is the correctness property.

**V-4 — Worst-case resource budget** *(retires A-4)*
`contracts/controller/tests/` with Soroban budget assertions. Build an account at
exactly `max_supply_positions + max_borrow_positions`, all distinct assets, all
dual-source oracles, then measure CPU and memory for `liquidate`,
`get_liquidation_estimate`, and `clean_bad_debt`. Assert headroom against the
ledger limit. Wire into CI so a future price-path change cannot silently make
worst-case accounts unliquidatable.

**V-5 — Spoke usage reconciliation** *(retires A-2, ours-only)*
`certora/controller/spec/spoke_rules.rs`. This is our analogue of Certora Hub P-05
(`sumOfSpokeSupplyShares` et al.), and it is *more* necessary for us than for
Aave: their per-spoke shares are the Hub's own record, whereas our `SpokeUsage` is
a **second accumulator maintained by a separate code path**
([`spoke_usage.rs`](contracts/controller/src/spoke_usage.rs)). Any verb that
mutates a position but misses its `apply_leg_usage` call silently corrupts cap
enforcement, in either direction: under-count lets a spoke exceed its cap,
over-count locks out legitimate supply. Property: for every
`(spoke_id, hub_asset)`, `SpokeUsage` equals the sum of scaled positions over
accounts bound to that spoke — preserved by supply, withdraw, borrow, repay,
liquidation seizure, liquidation repayment, bad-debt cleanup, and every strategy
leg. Aave has no equivalent finding because they have no equivalent duplication;
this is a class we invented and must therefore prove ourselves.

### Priority 2 — close the gap against Aave's proven property set

**V-6 — Additivity / anti-splitting** *(Certora Hub P-07; retires A-5)*
`certora/controller/spec/math_rules.rs` and `certora/pool/spec/`. Aave proved
`addAdditivity`, `drawAdditivity`, `restoreAdditivity`, `reportDeficitAdditivity`:
doing an operation in two steps is never more beneficial to the caller than doing
it in one. We have roundtrip-error rules but no splitting rules. Two targets:

- Pool level: supply, withdraw, borrow, repay, net-settle.
- **Liquidation level — the important one.** CS-AAVE4-009 is a splitting attack: when `f · LB > HF_pre`, each partial liquidation *worsens* HF and earns a larger bonus next time. Aave fixed it by a *configuration constraint* (`f · maxLB ≤ HF_maxBonus`) enforced off-chain by risk governance. **We fix it at runtime instead**: `max_hf_preserving_bonus_bps` clamps the bonus to `HF/proportion_seized − 1`, and `normalize_repayment_plan` raises `FullCloseRequired` when a partial repayment cannot preserve HF ([`math.rs:182`](contracts/controller/src/positions/liquidation/math.rs:182)). Ours is the stronger construction — it does not depend on governance parameter discipline — but it is currently justified by tests, not by a proof. Add: *N sequential partial liquidations never extract more collateral than one liquidation of the summed amount.*

**V-7 — Bad-debt gate adversarial boundary** *(strengthens class C)*
Extend `bad_debt_socialization_threshold_boundary`: an attacker who tops
`total_collateral` to `BAD_DEBT_USD_THRESHOLD + 1` blocks dust socialization, and
`force_socialize_bad_debt` still admits. Pins the escape hatch as load-bearing.

**V-8 — Pool trust-boundary regressions** *(pins class F)*
`certora/pool/spec/lifecycle_rules.rs`: no pool entrypoint can move tokens from an
address other than the controller-authorized payer (Certora Hub L-02 analogue);
`create_market` on an existing key reverts and `add_asset_to_spoke` on an existing
asset reverts (Certora Hub M-03 analogue).

**V-9 — View/accrue isomorphism** *(Certora Hub P-09/P-10; retires A-6)*
`certora/controller/spec/index_rules.rs` and `certora/pool/spec/`. Two families:

- **Isomorphism:** every view returns the same value whether or not `update_indexes` ran first, and reverts under the same conditions. Directly targets Blackthorn L-6 / Certora Hub L-03. Applies to `get_health_factor`, `is_liquidatable`, `get_liquidation_estimate`, `get_market_index`.
- **Time monotonicity without accrual:** Aave proved 10 such rules. Ours: `get_market_index` and `get_liquidation_estimate` must be monotone in time when no accrual is invoked, so a keeper cannot change a user's apparent risk merely by choosing when to call.

### Priority 3 — hygiene

**V-10 — Event/doc consistency sweep** *(ToB-AAVE-4 class)*
Audit every liquidation and bad-debt event against its doc comment, focusing on
gross-vs-net semantics of `SeizeEntry.amount` relative to `protocol_fee`. Extend
`contracts/controller/tests/events.rs`.

**V-11 — Zero-parameter liquidation regression** *(Certora Spoke M-01 analogue)*
Assert a position whose collateral has `liquidation_threshold == 0` is still
liquidatable at zero bonus and still socializable. **Inferred** from
`max_bonus_for_threshold` returning 0 and `try_liquidation_at_target` remaining
solvable when `proportion_seized == 0`; needs to be a test, since this is the
exact shape Certora found in Aave's Spoke.

**V-12 — Type-bound write-up** *(CS note 8.5 analogue)*
Document max reachable borrow index given the rate cap and chunking, and the
largest representable asset balance before `Ray` multiplication overflows. Add
boundary tests at those values.

**V-13 — Small-position profitability** *(CS note 8.4 analogue)*
Instantiate `V < L_round / (b·(1−f))` for our supported decimal range; confirm
`MinBorrowCollateralUsd` sits above it for every listed pair.

### Methods worth adopting from the auditors

- **Trail of Bits** ran `slither-mutate` and `necessist`, and found `asset.accrue()` could be deleted entirely with all Hub tests still passing. Our `cargo-mutants` targets are the right equivalent — but per [[maintenance-agent-runner-lessons]], `mutants-diff` is vacuous on harness-only diffs, so V-1..V-9 must be checked with the scoped targets (`mutants-pool-interest`, `mutants-controller-positions`), not the diff target.
- **Trail of Bits' Slither access-control script** (report appendix E) checks that every configurator function is `onlyOwner`, every state-changing function it calls is `restricted`, and every `restricted` function is reachable from the configurator. Our equivalent: a CI check that every controller mutator is `#[only_owner]`, timelocked, or explicitly listed as permissionless-by-design. Cheap, catches a whole bug class, and we have the exception list already implied by INV-AUTH-03.
- **Certora's modularity trick**: prove Hub properties, then verify the Spoke against a *symbolic* Hub satisfying only those properties. We can do the same — verify the controller against a symbolic pool constrained by the proven pool invariants — which should cut solver time on the controller specs substantially.

---

## 5b. Execution status (2026-08-14)

### Shipped

| Item | Result |
|---|---|
| A-1 pause split | `no_seize` flag + `FreezePolicy::SeizureLeg`; exactly two seizure-leg call sites moved. Pausing a collateral listing no longer halts liquidation of everyone holding it. ADR-0008 rewritten. |
| Share-credit liquidation | `SeizeMode::{Transfer, Credit(u64)}`. Verified: a cash-starved market reverts `Transfer` and clears the identical liquidation under `Credit`. Pool source untouched — the existing deposit-side seize primitive sufficed. |
| A-3 accrual leak | **Measured, property holds.** Frequent accrual never leaves suppliers worse off; deltas are +$5.55/yr supplier, +$1.23/yr treasury on a $1M 7dp book, funded by borrowers. Aave's equivalent was −$30,000/yr. Fee capture stays at exactly 1000 bps at every cadence; theirs collapsed to 0. Residual ≈0.5 ray per accrual, decimals-invariant. |
| V-12 / V-13 | `docs/reference/numeric-bounds.md`: index ceiling 11 y at 200% APR / 70 y at 30%; balance ceiling ~170.14 bn whole tokens, decimal-independent; dust-liquidation threshold cleared by ≥33× for every listed pair. |
| Item 18 access-control CI | `scripts/check_access_control.py` + declared-permissionless file, wired into `make` and CI. 196 entrypoints, **zero genuine gaps**; failure path tested 10/10. |
| V-7 bad-debt straddle | **Proven**, including a negative control against vacuity. |
| Item 17 | Verified not applicable — no ERC-4626 surface exists. Dropped. |

### Written but NOT proven

~40 rules across V-1, V-5, V-6, V-8, V-9 are compile-verified only; no solver
verdict exists. Queue order by author-reported vacuity risk: S6
`market_duplicate_create_reverts` (an assert-false rule passes on *any* panic),
then S3a `net_settle` and `repay` additivity, then the rest. Two of S2's eight
assert rules are **vacuous today** — supply and repay have no post-pool solvency
gate, contrary to this document's earlier claim.

### Toolchain limitation found while starting the proof pass

The pool **lifecycle** module cannot be proven by the local prover, and not for a
reason any configuration change fixes. Two independent failures stack:

1. **The loop does not converge at any bound.** Swept `loop_iter` over
   3/8/16/24/25/26/27/28/29/30/32: wherever the prover completes it reports an
   *unwinding condition* whose message tracks the bound itself — "higher than 27"
   at 27, "higher than 32" at 32. The loop is effectively unbounded under the
   current harness, so raising `loop_iter` only moves the complaint. These are
   **not** property violations.
2. **The prover crashes at some bounds regardless** — `Expected 3892314112 to be
   an Int` at 8, 28 and 29, but not at neighbouring values. Non-monotonic, so it
   is a distinct prover bug rather than a consequence of (1).

`market_create_writes_zeroed_state` and `market_duplicate_create_reverts` are
therefore **neither verified nor refuted**, and cannot be made so by tuning. The
fix has to be either a harness change that bounds whatever is being iterated, or
a prover-side fix.

This is pre-existing, not caused by this work: `pool-lifecycle.conf` is unmodified
from HEAD and already specified `loop_iter = 28`, and removing the newly added
rule does not change the outcome. The prover itself is healthy — the controller
boundary module verifies at the same bound.

Do **not** work around it with `--optimistic_loop`; that is unsound and the repo
deliberately restricts it to a single conf. The routes are the hosted prover (a
different version) or a support report — the error text asks for one.

Related: `artifacts/wasm/certora/` was found stale by a day. Every agent that
verified anything built its own artifact in a throwaway worktree. A stale
artifact makes *new* rules fail loudly ("invalid entry point") but lets
*pre-existing* rules verify green against old code — the profile runner should
refuse to run against artifacts older than their sources.

### New findings from executing the plan

| Finding | Impact |
|---|---|
| Pool views are not accrual-isomorphic (`views.rs` loads a `Cache` without accruing) | The Blackthorn L-6 / Certora L-03 shape, in our code. Integrator-facing only — the controller reads through `get_bulk_indexes`, which projects. Direction asserted: a stale read never overstates debt. |
| `certora/controller/harness/storage.rs:77` models the controller's index source as the **stale** stored index | Latent: any future rule on that path would prove the wrong semantics. The harness encodes the bug the suite exists to exclude. |
| Nothing bounds an asset's **unit** value | With 3 decimals and a $1M/unit token, a $5 full close seizes nothing. A listing-admission constraint, documented in numeric-bounds.md; no listed asset is within four orders of magnitude. |
| `apply_entry` inserts a zero usage row, `apply_exit` returns without writing when absent | Unreachable today; pinned as a rule rather than a comment. |
| Splitting `net_settle` settles *more* asset units than one call | Not a caller gain — the extra unit is paid for in ceil-rounded collateral. The naive additivity bound is false and was deliberately not asserted. |

### Corrections to this document's own claims

Executing the plan falsified four statements made above: supply and repay do not
call the post-pool solvency gate; the naive `net_settle` additivity bound is
false; the proposed accrual fuzz property is false in the general parameter
domain (deferring a fee mint *over*-credits suppliers); and spoke usage under
share-credit seizure nets to **−fee**, not zero, because the fee shares leave the
account system entirely. Without that fee exit, usage would ratchet up forever
and `remove_asset_from_spoke` — which requires zero usage — would become
permanently unreachable.

## 6. Bottom line

On the three axes where we diverge — index accounting, direct socialization, and
controller-side caps with no external spokes — **our design is the safer one**,
and in each case the Aave audit corpus contains findings that exist *because* of
the choice we did not make. Two of our mitigations (value-based bad-debt gating,
runtime bonus clamping) are the fixes auditors recommended and Aave declined or
pushed into governance policy.

The exposure runs the other way in exactly two places. Pro-rata seizure, which
buys us immunity from the dust-collateral griefing that Aave accepted as a
Medium, costs us a wider halt radius when a collateral asset is paused (A-1).
And `SpokeUsage` as a second accumulator is a consistency obligation Aave simply
does not have (A-2) — it is the price of moving caps out of the custody contract,
and it is currently unproven.

Neither is a vulnerability today. Both are properties we are relying on without
having proved, which is precisely the gap the verification plan closes.
