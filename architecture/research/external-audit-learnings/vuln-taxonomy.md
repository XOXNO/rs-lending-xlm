# Lending Vulnerability Taxonomy

Ten classes, each with the recurring mechanism and real examples from the corpus. This is the shared lens used to classify all ~324 findings; `self-audit-backlog.md` maps each class onto our code.

---

## 1. Oracle
Manipulation, staleness, decimals/scale, dual-source divergence, exchange-rate/LST pricing, donation-to-price.

- **Thin-feed manipulation** — Mango ($116M, 2,300% pump), Solend USDH ($1.26M), **Blend/YieldBlox ($10.8M, ~$4 trade → 100x)**, Polter/Blizz. Lesson: never price collateral off a thin/single source; require depth + median-of-N + max per-interval deviation + a circuit breaker. *A VWAP over a dead market gives no protection.*
- **Decimals/scale misconfig** — Morpho PAXG ($230K, 8/8 vs 18/6 → 10¹²); Spark `CappedOracle` hardcoded 8 decimals; Aave fixed 1e36 scale truncates high-price low-decimal collateral to 0. Validate configured decimals against actual token decimals.
- **Stored-but-unenforced bound** — Sec3/Kamino `max_age_twap_seconds` configured but never read = silent fail-open. Audit every oracle config field is actually consulted.
- **Exchange-rate/LST blindness** — Spark `weETH` inflation via permissionless `eETH.burnShares`; exchange-rate feeds blind to market depeg; composite price passing the base feed's timestamp not the accumulator's freshness. Pair exchange-rate with a peg/market guard; carry `min(timestamp)` of all inputs.
- **Reverting oracle bricks the market** — Morpho: a deprecated/reverting feed blocks liquidations and withdrawals. Handle oracle failure gracefully, not as a hard revert.
- **eMode price applied to all reserves** — Aave PVE-006: category price used regardless of membership.
- **TWAP is not automatically safe** — UwU Lend ($19.3M): a flash-loaned spot trade moved a Curve pool *within* the averaging window. A TWAP/EMA window short enough to be flash-manipulated provides no protection; pair it with deviation bounds + a depth gate.
- **Auditor expectation:** staleness/heartbeat + deviation bounds + non-positive/zero rejection + fallback behavior defined; never derive sensitive values from ledger timestamp/sequence.

## 2. Interest / index accounting
Rounding direction, precision loss, index manipulation, share inflation, reserve-factor, accrual ordering, cache coherence.

- **Cached index vs fresher reality** — Morpho Optimizer ($2.85M): per-block cached index + flash-loan-premium donation overvalued deposits. A cache is a freshness boundary, not a gas win.
- **Cache-writeback not enforced** — Blend H-01 (flash loan didn't `cache_reserve` → stale `d_supply`, negative supply, inflated rewards); BLRC-002 recommends a Rust drop-guard asserting writeback.
- **Rounding direction** — round AGAINST the user / toward solvency: HF floors, utilization ceils (Blend OS-SUG-01); mint divisors round up (Spark PSM-8.2); Certora/Kamino precision bug let redeem > deposit by storing-then-re-dividing a rate.
- **Divide-before-multiply / precision** — Scout, Spark PSM-001, Aave TOB-012; widen intermediates to avoid phantom overflow (Aave ABDK); `^` is XOR not power (Scout); division must revert on zero, not return 0 (SigmaPrime).
- **Virtual-share offset earning interest** — Morpho: inflation-defense virtual shares accrued interest → unrepayable bad debt + yield leakage.
- **Utilization from raw balance** — Aave v3.1 switched to internal `virtualUnderlyingBalance`; Blend computes util from pool balance (donation-manipulable). Use accounted reserves, not `token::balance`.
- **Accrue before mutate / before config change** — Aave (accrue index before validating; sync indexes before rate/reserve-factor change, esp. frozen assets); Blend M-05/M-13 (settle on pre-mutation snapshot). Guard `timestamp − last` underflow.
- **Discrete vs continuous accrual** — Blend BL-002: reward minted continuously but distributed in discrete cycles → stuck rewards. A continuous index avoids it.

## 3. Liquidations
Bad-debt accrual, self-liquidation, incentive miscalibration, dust/griefing, partial-liquidation rounding, auction timing.

- **Toxic liquidation spiral** — Aave CRV: flat bonus → frontier at LTV=1/(1+i) above which liquidation worsens LTV. Dynamic bonus / close-factor → 100% / halt past frontier.
- **Bad-debt socialization gameable** — Morpho OZ-H01 (only triggers at collateral==0, leave 1 wei to skip; callback withdraw before realization); Cantina 3.5.6 (round-up share conversion exceeds totals by 1 → plain-subtract **underflow reverts** cleanup); Blend M-03/M-09/M-17 (backstop withdrawal bricks auction; partial-fill not cleaned; bad debt blocked from socialization). **Our known stuck-bad-debt issue is this class.** Fix: saturating-subtract; partial socialization; guaranteed terminal transition.
- **Self-liquidation** — Blend OS-ADV-00 / V2-H-02 (filler==liquidatee uses stale read-after-write snapshot; mutating a position mid-liquidation underflows the fill). Reject liquidator==owner; saturating subtraction.
- **Seize rounding** — Morpho: dust repay (round-up reused for seize) over-seizes collateral; or seizedAssets-denominated repay rounds to 0 shares while still seizing. Round repaid-up-for-pay, down-for-seize.
- **Dust uneconomical liquidation** — Aave v3.3 / Blend sybil-dust: position too small to profitably liquidate (fee > bonus) → permanent bad debt; sybil-split one position into many sub-floor ones. Min-position-size + full-close path.
- **Same-block auction underflow** — Blend BLRC-018: `block − (block+1)` unsigned underflow → ~2³² payout. Use `checked_sub`, never rely on release overflow flags.
- **Self-liquidation via manufactured insolvency** — Euler ($197M): a donation primitive pushed the attacker's own position underwater, then they self-liquidated at an unbounded soft-liquidation discount. Any balance-increasing call (donation, reward injection, direct transfer) must be unable to manufacture insolvency or feed a self-liquidation; bound the discount and enforce `liquidator != owner`.
- **Stale flags / write-off attribution** — Aave: borrowing flag not cleared on full-close; bad-debt write-off attributed to liquidator not treasury; bounded grace period after unpause.

## 4. Health-factor / collateral math
LTV vs liquidation-threshold gap, e-mode misconfig, isolation bypass, decimals, on-behalf state keying.

- **On-behalf uses caller's state, not owner's** — Aave's most-repeated multi-auditor class (OZ C01/C02, TOB-007/015, PeckShield PVE-007): borrow/repay/eMode keyed off `msg.sender` not `onBehalfOf` → escape stricter tier, skip HF, evade same-block interest. Key everything off the debtor.
- **HF not revalidated on risk change** — Aave: switching between two non-zero eMode categories / `repayWithATokens` skipped the HF check; eMode LTV ignored when base LTV=0. Re-run HF after ANY tier change.
- **Empty-market / first-depositor inflation** — Radiant (~$4.5M), Hundred, Morpho 3.1.4 (first *borrower* inflates `totalBorrowShares` to overflow), Blend/Aquarius/PSM share inflation. Seed deposit + virtual offset + never go live empty + first-mint rounds toward the vault.
- **Bitmap/collateral query panics** — Aave ABDK CVF-300: collateral check reverts instead of returning false on zero data → DoS of HF/liquidation/withdraw. Empty config must return false, not trap.
- **Forced/dust collateral bricks withdrawal** — Spark CS-SPRKALM-019: dust-supply a soon-to-be-LTV-0 asset on a victim → can't withdraw unrelated collateral. Forced collateral must always be exitable.
- **Boundary operator** — Blend QA-05 / Aave 0.95 boundary: `>=` vs `>` at the HF threshold is load-bearing.

## 5. Reentrancy / callbacks
Token transfer hooks, flash-loan/strategy callbacks, cross-function reentrancy, TOCTOU.

- **Token hook reentrancy** — Agave (ERC777), Hundred Finance. Soroban custom (non-SAC) tokens can run code in transfer paths.
- **Flash-loan callback caches stale state** — Aave PVE-011 (`flashLoanSimple` cached `reserveCache` across the receiver callback, then overwrote in-callback updates). Re-read state after the external call; CEI + guard.
- **Auth-tree / sub-call inheritance** (Soroban-specific) — RV Soroswap-Aggregator A3 / StellarBroker: a top-level `require_auth` lets a downstream upgradeable contract inherit signer auth over *all* footprint assets. Allowlist callees; verify balance deltas; don't trust returned amounts.
- **Soroban cross-contract reentrancy surfaces in our code** — two real callback paths: the flash-loan receiver callback, and the `defindex-strategy` `deposit`/`withdraw`/`harvest` calls into the controller/pool. A callee re-entering the controller mid-`borrow_for_strategy` is the concern; defended by the `FlashLoanOngoing` guard + CEI (state from call-return values, not pre-call snapshots). Mapped in `self-audit-backlog.md` (strategy + flash rows).
- **FV scopes reentrancy out** — Aave & Blend Certora both excluded reentrancy from proofs. Proofs don't cover it; use guards + CEI.

## 6. Caps & limits
Borrow/supply cap bypass, isolation ceiling, position-limit/resource DoS, dust griefing.

- **Counter desync** — Aave OZ-H01/TOB-011: `isolationModeTotalDebt` decremented on repay but not on liquidation → ceiling ratchets shut → self-DoS. Update the counter on EVERY debt-reduction path via one shared helper.
- **Asymmetric check** — Blend H-03: utilization checked on borrow but not withdraw → util >100%. Withdraw (denominator-shrinking) needs the same gate.
- **Per-call vs cumulative** — Aave CVF-16: per-call cap bypassable by splitting borrows. Enforce against aggregate state.
- **Cap not a hard ceiling** — Morpho: accrued interest pushes exposure past cap; donation/supply-on-behalf bypasses cap=0. cap=0 must truly disable; disabling needs full de-listing.
- **Rounding / boundary / zero-semantics** — Aave: debt-ceiling increment rounds down (shave the cap); decimal-scaling reverts when reserve decimals < ceiling decimals; `<` vs `<=`; 0 = unlimited footgun (Spark deploy). Round debt UP, define the 0 meaning, check post-mutation.
- **Resource-limit DoS** — Blend BL-001/QA-02: per-asset IO in the health check scales with assets → large positions become **un-liquidatable**. Bound iterated positions/feeds; our 10+10 limit is a *safety* bound, and must hold on every path (Blend bypassed `MAX_POSITIONS` via the flash-loan path).
- **Exit paths must stay open** — Offside/Kamino: paused/obsolete status must still allow withdraw/recover; caps enforced symmetrically at enqueue and redeem.

## 7. Access control / governance
Privilege escalation, timelock bypass, two-phase ownership, role separation, upgrade safety, init.

- **Owner change leaves stale privilege** — Kamino ADV-02 (farm admin not migrated), Spark/Cantina (revoke silently no-ops if role absent). Migrate the full role set atomically; fail loud on revoke.
- **set_admin no-op / no signer check** — Aquarius ME-01 (ignored `new_admin`, re-set existing); Blend BLRC-010 (no new-admin `require_auth`); Scout `unnecessary-admin-parameter` (admin must come from storage, not a caller arg). Write the supplied address; require the new admin to accept.
- **Timelock bypass via instant upgrade** — Aquarius Certora H-01: fee/owner changes timelocked but `update_current_contract_wasm` had only `require_admin`. The upgrade path *itself* must be timelocked.
- **Timelock snapshot** — Morpho L-06: bumping a global mutable timelock retroactively revives expired proposals. Snapshot the delay per pending change.
- **Timelock doesn't save you if the queued action *is* the exploit** — Sonne ($20M): the attacker queued a malicious empty-market addition and waited out the delay. Proposals must be reviewable *as queued*; pair the delay with monitoring + the ability to cancel a pending malicious op.
- **Blind-signing / operational key management** — Radiant ($53M, Oct 2024): multisig signers approved a disguised `transferOwnership` on hardware wallets. Off-chain signer tooling and what signers can verify on-device are in scope; ownership transfers should be human-verifiable, and automated signers (keepers) must use least-privilege (role-scoped, never owner) keys.
- **Forgeable / unvalidated authority** — Solend-2021 ($16K): accepted an attacker-owned market as authority. `require_auth` the stored admin; bind state objects to their owning instance.
- **Governance can seize** — Solend SLND1, Mango self-vote. No role may target a specific account outside the permissionless flow; emergency powers need minimum windows + capture resistance.
- **Retroactive param change** — Aave OZ-N01: changing live LTV/threshold force-liquidates healthy positions. Timelock + the two-param gap mitigate; snapshot-per-position is the alternative.
- **Init / migration** — dedicated `IsInitialized` flag (not an incidental field); strictly-sequential upgrade revisions (no gaps); atomic deploy+init (no Soroban constructors → two-step is front-runnable); salt bound to admin.

## 8. Flash loans
Fee bypass, flash-enabled governance/oracle attacks, gate-skipping.

- **Flash path skips gates** — Blend M-01 (bypassed frozen-pool/per-reserve action controls); V2-CERT-M-01 (`MAX_POSITIONS` not checked on flash path). The flash/strategy path must replay every gate the normal borrow path enforces.
- **Unvalidated pool/counterparty** — Veridise Orbit VUL-003: unvalidated Blend pool address → arbitrary flash loans. Validate the counterparty contract identity.
- **Fee leg** — Aave fork checklist / Spark deploy: flash-loan premium mis-passed or set to 0 removes the manipulation deterrent. Charge the exact premium atomically; treat fee=0 as a conscious, compensated choice.
- **Amplifier** — Crema ($8.8M): flash loans amplified a single-tx state forgery. Flash/atomic paths must re-read authoritative state and enforce invariants on repayment.

## 9. Economic / market design
Utilization gaming, listing risk, depeg, share inflation, value-held-outside-contract, MEV/slippage.

- **Listing risk** — Blend/YieldBlox (illiquid USTRY listed as collateral), Aave GUNI/illiquid. Gate listing on liquidity/depth; conservative LTV/caps for thin assets.
- **Value held outside the contract** — Spark PSM-002 (High): funds in an external "pocket" excluded from `totalAssets()` → share mispricing/theft. **Directly relevant to our pools→vaults V2 direction** — count all externally-routed value in share valuation.
- **Share inflation / first-depositor** — Blend (pool + backstop), Aquarius, Morpho, Spark PSM-8.4. Seed dead shares / virtual offset / minimum first deposit; first-mint rounds toward the vault.
- **At-par vs redeemable gap** — MakerDAO D3M N03: externally-minted liquidity valued 1:1 at settlement while real redemption depends on host liquidity. Same class as our stuck-bad-debt — don't value external positions at par during emergency exit without a liquidity check.
- **MEV / slippage** — Aave JIT fee-distribution sandwich; Aquarius min-shares; Morpho no slippage bound. Index-drift between simulate and apply → entry points should accept min-out/max-in bounds. *(Stellar's lack of a public mempool reduces but doesn't eliminate this.)*
- **Reflexive first-loss capital** — Blend backstop (80/20 BLND:USDC) undersized exactly when needed. Size buffers in stressed-dollar terms; prefer uncorrelated denomination.
- **Interest-rate-model gaming** — the kink/jump-rate IRM is itself an attack surface: pushing utilization to swing borrow APR (to time liquidations or grief borrowers), or a misconfigured kink (slope2<slope1, unbounded max rate). Validate IRM params at the governance boundary (max combined rate, slope ordering); compute the rate from accounted reserves so a transient/donated balance can't move it.
- **Fee-on-transfer / measured delta** — Aave OZ-H03: mint from requested amount not measured received. Credit the balance delta, not the requested amount.

## 10. Soroban-platform
Storage durability/TTL, `require_auth` semantics, i128 overflow/rounding, SAC trust, resource limits, upgrade/deploy.

- **Unbounded data in Instance storage** — OtterSec Soroswap (HIGH), Veridise, Scout `dynamic-storage`/`vec-could-be-mapping`/`dos-unexpected-revert-with-vector`: Instance storage loads in full every invocation, ~64KB entry cap → DoS. Use Persistent per-key slots, never a growing `Vec`/`Map` in Instance.
- **Resource-limit DoS bricks liquidation** — Blend BL-001 (CRIT): per-asset reads in the health check scale with assets → un-liquidatable large positions. Hard-cap assets/position width; profile against an 80–90% budget high-water mark.
- **`require_auth` auth-tree** — sub-calls inherit signer auth over all footprint assets; a malicious sub-contract smuggles transfer auth. Allowlist callees; verify balance deltas.
- **Arithmetic** — `overflow-checks = true` in `[profile.release]` (Scout CRITICAL); `checked_*`/`saturating_*` at money sites regardless; `.pow()` not `^`; multiply-before-divide; division reverts (not 0) on zero divisor; unsigned ledger-seq deltas underflow.
- **Panic surface** — `unwrap`/`expect`/`assert!`/`panic!`/`Map::get` on a missing key all panic → whole-tx revert/DoS. Typed errors + safe accessors + existence checks.
- **TTL / eviction** — extend storage TTL on config writes (Blend V2-I-01b); off-chain keeper liveness for footprint TTL; allowance `live_until_ledger` ≠ entry TTL (Veridise). Don't equate logical expiry with storage TTL.
- **Deploy/init front-running** — no Soroban constructors → deploy+init are separate txs; init not enforced before use is hijackable (RV Aggregator A1); bind deploy salt to admin.
- **Events & token interface** — emit events AFTER security-relevant storage mutations; SEP-41 token-event compliance; scaled-balance events must match the realized index-scaled delta (indexer correctness).
- **Per-cross-call cost** — the binding constraint is memory per cross-contract CALL + per storage entry (our V2 accounting direction). FV "Verified" is conditional (unproven summaries, ≤2 loop unrolls, reentrancy/accrual scoped out).
