# Controller defense audit — shared findings report

This is the **authoritative shared report** for the controller defensive-protections
audit. The wave syntheses `findings/A101`–A110 are the detailed evidence kept
in the tree. The 100 per-scope primaries (A001–A100) were reviewed and then
left on the audit branch (`feat/controller-defense-audit-1735`, PR #134): they
are working notes, many under 2 KB, and their impact ranking was taken from
the syntheses, not from them (§9). Post-close passes:
[`RESIDUAL_REVALIDATION.md`](RESIDUAL_REVALIDATION.md) (A080 withdrawn) and
[`DRAIN-ANALYSIS.md`](DRAIN-ANALYSIS.md) (27 pool-outflow paths). An
independent re-derivation of this report's claims from source is folded into
§11.

**Corpus:** 110/110 manifest scopes filed. No production Rust was changed by
the audit; the branch it landed on also carries the Certora layer migration
and the script-runner sentinel fix (PR #133).

## 1. Verdict

Controller defenses on live paths are **strong** at auth, pause, flash
reentrancy, measured custody, Credit vs Transfer seize, pool-truth spoke
usage on healthy entry, and Cache memoization.

No filed finding demonstrates that the controller hands the pool an
unmeasured amount, credits custody it did not measure, or redirects another
account’s cash without INV-AUTH-02. That is a **controller-boundary** claim:
pool share and index arithmetic, directed rounding, and oracle aggregation
math are outside this corpus (§11.4).

**Protocol-total loss** requires a failed **deployment trust root** (controller
owner ≠ governance timelock, hostile price aggregator / XOXNO owner), not a
missing `transfer_amount_measured` in the present corpus.

**Highest-probability live-code extractable loss** is account-local strategy
slippage: the controller requires only `received > 0` on aggregator swaps
(A048 / A056).

## 2. Coverage

| Wave | IDs | Theme | Filed |
|---|---|---|---|
| 1 | A001–A020 | Auth / entry / pause / flash / keepers | 20/20 |
| 2 | A021–A040 | Storage mutations on user paths | 20/20 |
| 3 | A041–A060 | Money movement | 20/20 |
| 4 | A061–A075 | Untrustworthy input validation | 15/15 |
| 5 | A076–A085 | Spoke usage after pool calls | 10/10 |
| 6 | A086–A100 | Storage R/W + Cache | 15/15 |
| 7 | A101–A110 | Gap synthesis + impact | 10/10 |

Header census at close: **87 defended**, **20 partial** (including wave
syntheses that stay partial because they own residuals), **2 synthesis**,
**1 optimization-note**. No `critical` live-code hole under SAC + intended
ownership.

Later filings closed coverage holes that A101–A107 recorded while those
waves were still in flight. Those files keep their original text; this
report is the post-complete ranking. Addenda on A101–A104 / A106 / A110
point here.

## 3. Defended stacks (do not re-open without new evidence)

1. **Pause matrix** matches INV-HALT-01 (A001, A006).
2. **Owner-or-delegate** plus third-party supply slot rule (A003, A012, A057).
3. **Flash reentrancy guard** on monetary reentry; wasm receiver required
   (A007, A019, A030).
4. **Measured receipts** at the controller custody boundary; pool legs use
   received amounts, not caller claims (A041–A045, A058, A082).
5. **Spoke usage from pool `PoolPositionMutation`** indexes/amounts; persist
   after successful pool mutation (A076–A079, A081–A083, A078).
6. **Credit seize** is share absorb vs mint, fee-only usage exit (A052, A084).
7. **`finalize_position_flow`** batches usage → positions → events; solvency
   before persist on gated paths (A032, A033, A097).
8. **Cache constructors:** `Cache::new` renews controller instance TTL;
   `new_view` does not. Production mutators use `new`; views use `new_view`
   (A008, A034, A093).
9. **Memos:** spoke pin, success-only `verified_hubs`, ADR-0005 price snapshot,
   simulate-then-`put_market_index` overlay (A086–A091, A098, A099).
10. **Account load shapes** pair with safe `PositionSides` /
    `remove_if_empty` on live mutators (A096).

## 4. Leading residuals (ranked)

Impact units follow A106: **P** protocol-total, **M** ≤ market TVL,
**A** account-local, **C** contingent, **Z** fees/availability.

Post-close revalidation: [`RESIDUAL_REVALIDATION.md`](RESIDUAL_REVALIDATION.md).
**A080 is withdrawn** as a live production issue (persistent archive ≠ missing
row; entry always creates; money paths keep usage+positions in lockstep).

| Rank | Band | Residual | IDs | Kind | Ceiling | Verdict |
|---:|---|---|---|---|---|---|
| 1 | P0 | Controller owner must be governance; restore Sensitive floor | A009 | D / P | Entire book + NFT if mis-wired | VALID — deploy |
| 2 | P0 | Swap-aggregator + XOXNO oracle intended owners | threat-model, A056, A065 | D / P or A | Oracle: protocol-wide bad valuation; router: Σ strategy notionals | VALID — deploy |
| 3 | P0 | No controller quantitative `min_out` on strategy swaps | A048, A056, A101 | D / A | ≈ swapped / withdrawn notional; HF-clipped if debt remains | VALID — known design |
| 4 | P1 | Non-SAC / lying / rebasing tokens if listed | A055 | C / M | ≤ that market’s TVL | VALID — listing policy |
| 5 | P1 | `no_seize` not coupled to `frozen` / still allows supply | A064 | A→C / M | Liquidation stranding until force socialize | VALID — ADR-0008 design |
| 6 | P2 | Uncapped mutator / keeper Vec lengths | A062, A015 | Z | Attacker’s own fees | VALID — hygiene |
| 7 | P3 | Forgotten `put_market_index` on a *future* pool merge | A094, A098, A104 | A (tx-local) | Same-tx wrong HF/caps if new code omits the put | VALID — footgun only |
| 8 | P3 | Evidence density: PIN/CLOSE tests for residuals | A085, A108 | — | Does not create theft | VALID — tests |
| 9 | Z | `update_account_threshold` reverts the whole batch on one grandfathered account | A040, A102 | Z | Keeper availability only | VALID — see §11.3 |

**Withdrawn:** A080 missing-usage over-admission — not reachable via TTL archive
or healthy merges (`findings/A080-*.md` status **defended / info**).

**Not leading, but confirmed:** plant-stale liquidation DoS (A065); min-borrow
floor vs `BAD_DEBT_USD_THRESHOLD` desync (A067); `swap_debt` refinance-at-cap
UX (A066); unbound callback `Bytes` (A069); Certora harness override hygiene
(A035); spoke-usage key-family partials (A028); dead `pool_sync_data` memo
(A100).

A109 found **no material cross-agent fact conflicts**. Apparent tension is
scope framing (custody **defended** vs residual owned by A048/A056).

## 5. Max-loss bounds (A106)

| Tier | Practical ceiling under intended deploy + SAC listing |
|---|---|
| Single account | Strategy dust-out (S1) ≈ in-flight swap notional / excess HF |
| Single market | Listing desync (S3) or contingent socialize after A064 ≤ \(\mathrm{TVL}_m\) / \(D_{\mathrm{bad}}\) |
| Protocol | **Only** S4/S5 trust-root failure — not A055, A048, A056, or A064 alone. Bound holds at the controller boundary; pool share math and oracle aggregation are out of corpus (§11.4) |

Threat-model “unbounded loss” for slippage means unbounded relative to
**in-flight strategy notional**, not protocol share mint.

## 6. STRIDE residual recalibration (A107)

- **CONFIRM** Low on auth, flash guard, measured custody under SAC, pause,
  flag ratchet.
- **RAISE** Tamper.4 at the **controller** layer: positivity-only swap out
  is Medium likelihood for account-local dust-out, not “meet minimums → Low”.
- **WITHDRAW** earlier “ADD A080” capacity residual — not a live production
  hole after revalidation (persistent restore ≠ missing row).
- **REFRAME** Elevation.6 / INV-LIQ-04 wording so operators do not confuse
  account-local strategy drain with protocol insolvency, or ordinary-liquidate
  HF post-gates with bad-debt seize post-guards.

Threat-model Known gaps (A105): catalogue **largely confirmed**. Closed
router-input-measurement item stays closed. Newly surfaced for the next
threat-model revision: A080, A064 Option C, Vec caps, plant-stale, SAC
listing elevation, A094 footgun.

## 7. Theme summaries

### T1 Auth / entry (A001–A020)

Permissionless inventory matches live gating except documented keepers and
views. Owner-or-delegate holds on borrow/withdraw/strategies. Account=0 mint
and third-party supply slots are defended. Pause/guardian ratchet holds.
A009 is a **deploy gate**, not a missing `only_owner`. A010/A015/A062 are
hygiene (undeclared length, uncapped Vecs). A020 is a docs cross-check, not a
code hole.

### T2 Storage (A021–A040)

User-path writes go through listed hubs, `finalize_position_flow`, and
account/NFT coupling. TTL: mutators renew instance; views skip controller
instance bump (rent-grief defense) but still touch-renew user/shared keys.
A028/A035/A038 residuals are key-family documentation, harness-override
hygiene, and A094-class index persistence — not live wipes.

### T3 Money (A041–A060)

Custody measurement is the hard rule. Flash pullback, Credit absorb-vs-mint,
destination `to`, and directed rounding are defended. Residuals: G-SLIP
(`min_out` only in opaque aggregator payload) and G-LIST (non-SAC listing).
A042/A043/A060 later filings are **defended** (A060 partial on dust-band
ops), closing A101’s coverage holes without reopening L6–L10.

### T4 Validation (A061–A075)

Sign/zero/overflow, spoke/hub active, listing flags, oracle freshness on
valuation mutations, position limits, min-collateral floor, SeizeMode
exhaustiveness, flash refund allowlist, Blend migrate approval, and post-pool
HF gates hold. Leading in-wave residual: A064 G1. Vec caps and plant-stale
are Low. A069/A071/A073–A075 later filings did not add a new Critical.

### T5 Spoke usage (A076–A085)

Entry/cap/index/persist/isolation/pool-output reuse are defended. **A080**
was ranked medium mid-wave; **revalidation withdraws it as a live issue**
(first entry always creates; persistent archive restores; money paths
lockstep usage+positions). Credit fee-only exit (A084) remains intentional.
A079/A081/A083 later filings confirm A103’s provisionals as defended. A085
is evidence-partial only.

### T6–T7 Cache / read savings (A086–A100)

All Cache fields are used at the type level. Production call-site partition
for `new` vs `new_view` holds (18 mutator / 10 view). Event buffers are
append-only; coalesce is upstream payment aggregation + batch emit (A092).
In-tx market-index overlay is sound because ledger time is frozen (A098).
Dead-path hygiene: `pool_sync_data` is first-and-only (A100). Live hazard
remains A094’s future-merge footgun, not a current omitted `put`.

### T8 Gaps + impact (A101–A110)

Wave syntheses agree on ranking. A109: no material disagreements. A110 is
the remediation program (P0 deploy + `min_out`, P1 listing / Option C /
usage reconcile, P2 Vec caps + put-index checklist, P3 tests/docs). A108
names PIN vs CLOSE tests; do not clone existing `usage_*_tracks_scaled_delta`
or `RouterOverspend` cases.

## 8. Remediation program (from A110)

Do **not** “fix” intentional designs: aggregate-and-sum payments, ADR-0005
price snapshot, Credit fee-only usage, persist-after-pool, `new_view` rent
skip, missing-row exit no-op **until** product chooses a reconcile model.

| Rank | Band | Fix shape |
|---:|---|---|
| 1 | P0 | Restore Sensitive floor **and** `configs/networks.json` `timelock_min_delay_ledgers` (both tiers are 12 ledgers today, §11.1); verify on-chain owner = governance |
| 2 | P0 | Attest aggregator + XOXNO owners; no lone EOA on live XOXNO feeds |
| 3 | P0 | Controller `min_out` arg **or** decode+check vs measured Δ (mirror `flash_position`) |
| 4 | P1 | SAC-only listing; never `flashloanable` on non-exact tokens |
| 5 | P1 | ADR-0008 Option C: `no_seize ⇒ frozen` and/or block supply |
| 6 | P2 | Cap keeper/mutator Vec lengths (reuse view 256) |
| 7 | P3 | Review checklist: every *new* pool merge → `put_market_index` (+ `apply_leg_usage`) |
| 8 | P3 | PIN/CLOSE tests named in A108 for remaining design residuals |
| — | ~~P1~~ | ~~A080 usage reconcile~~ — **withdrawn** (not a live hole) |

## 9. Quality notes

- Early-wave stubs remain under 2 KB for some IDs (notably A080, A055, A094
  primaries). **Impact ranking for those IDs is taken from the deep syntheses**
  (A101, A103, A104, A106), not from stub length.
- Wave-7 files A101–A107 were drafted before the last Wave-3/5/6 IDs landed.
  Their “absent” rows are historical. Re-read this FINAL + the later primary
  files (A042, A043, A060, A069, A071, A073–A075, A079, A081, A083, A085,
  A088–A093, A095–A098, A100) before treating a synthesis hole as still open.
- Agents were forbidden from git operations after Wave 1 private-branch
  forks; the coordinator scooped findings onto `feat/controller-defense-audit-1735`.

## 10. How to use this report

| Need | Read |
|---|---|
| One-page ranking | §1, §4, §8 here |
| Quantified blast radius | A106, then A101–A104 |
| Threat-model / STRIDE updates | A105, A107 |
| Test/rule names | A108 |
| Ordered engineering backlog | A110 |
| Primary path evidence | `findings/AXXX-*.md` matching the residual ID |

## 11. Post-review corrections

An independent pass re-derived twelve of this report's code-level claims from
`main@a2afb21` (census, no-critical, R3, R5–R8, Credit absorb, the 18/10 Cache
split, measured custody, INV-AUTH-02 pinning). All twelve reproduced. Four
framing corrections came out of it and are applied above:

1. **R1 was half a fix.** `contracts/governance/src/timelock/mod.rs` resolves
   `Standard => min` and `Sensitive => min.max(TIMELOCK_SENSITIVE_MIN_DELAY_LEDGERS)`,
   and `configs/networks.json` sets `timelock_min_delay_ledgers: 12` on both
   networks. Restoring only the Sensitive constant leaves every Standard-tier
   op (`AddAssetToSpoke`, `EditAssetInSpoke`, `ConfigureAssetOracle`,
   `EditOracleTolerance`, `SetSpokeLiquidationCurve`, `UpgradeLiquidityPoolParams`,
   `MigrateController`, …) at ~72 s. `validate_delay_update` is ratchet-up
   only, so the raise is a one-way governance op that has to be scheduled.
2. **The PR was not docs-only.** See the header.
3. **A040 was dropped between file and ranking.** `keepers.rs`
   `update_account_threshold` asserts `hf >= THRESHOLD_UPDATE_MIN_HF_RAW`
   (1.05 WAD) inside the per-account loop, so one account in HF ∈ [1.0, 1.05)
   keeps its grandfathered risk tuple and reverts the whole permissionless
   batch. Now row 9 of §4.
4. **§1 and §5 over-reached.** The 110 scopes are controller-side. "No share
   mint" needs the pool's share/index math and directed rounding; the
   protocol-tier ceiling needs oracle aggregation. Both are named out of
   corpus above.

One listing-time footgun the corpus did not carry, recorded here so the next
pass starts from it: `params.asset_decimals` is trusted at `create_market`
(`contracts/pool/src/ops/market.rs`) and never compared with the token's own
`decimals()`. A wrong value mis-scales every cap and USD valuation for that
market. One cross-contract read at listing, or a check in the deploy
preflight, closes it. It belongs next to A055 in the listing policy.

One evidence-bar note stands as written: 24 thin (≤ 2 KB) `defended` files
back several of §3's "do not re-open" stacks. Stacks 2, 3, 4, 6 and 8 were
re-derived from source and hold; read §3 as "re-derive before relying on"
rather than closure.

Verified against the audit's own claims by the 2026-09-04 review: A080 is a
latent carve-out (every share writer updates usage with exact deltas; Credit
seize nets to `-fee` on both sides; an archived persistent row fails the
footprint, it does not read as `None`), and the harness now asserts
`usage == Σ scaled positions` per `(spoke, hub asset, side)` after every
composition snapshot and across a dedicated flow
(`tests/test-harness/tests/controller/spoke_usage_tracks_positions.rs`).

