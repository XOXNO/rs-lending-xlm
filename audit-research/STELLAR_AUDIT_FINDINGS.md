# Stellar / Soroban Smart Contract Audit Findings — Compendium

Compiled from public reports of reputable firms (Certora, OpenZeppelin via Code4rena, Runtime
Verification, Veridise, OtterSec, Halborn, Coinspect, CoinFabrik, Code4rena, Zellic).
Bias: lending / DeFi relevance for the `rs-lending-xlm` Soroban lending protocol.

> Evidence policy: every finding below was extracted by fetching the actual report PDF (via
> `WebFetch`) and converting it to text with `pdftotext`. Where a PDF could not be parsed, the
> report is listed in §1 but excluded from §2. No findings were fabricated.

---

## 1. Inventory of Audits Found

| # | Firm | Project | Type | Date | Report URL |
|---|------|---------|------|------|------------|
| 1 | Certora | **Slender** (lending on Soroban) | Manual review | 2024-05-22 | https://github.com/Certora/SecurityReports/blob/main/Reports/2024/05_22_2024_Slender-MR.pdf |
| 2 | Certora | **Blend V1** (Script3 lending) | Manual review | 2024-01-25 | https://github.com/Certora/SecurityReports/blob/main/Reports/2024/01_25_2024_Blend-MR.pdf |
| 3 | Certora (Sunbeam Prover) | **Blend V1** | Formal verification | 2025-01-30 | https://github.com/Certora/SecurityReports/blob/main/Reports/2025/01_30_2025_Blend_V1-FV.pdf |
| 4 | Code4rena + Certora FV | **Blend V2** (lending) | Crowdsource + FV | 2025-02 → 2025-03 | https://code4rena.com/reports/2025-02-blend-v2-audit-certora-formal-verification |
| 5 | Code4rena | **Blend V2 mitigation review** | Mitigation review | 2025-04 | https://code4rena.com/audits/2025-04-blend-v2-mitigation-review |
| 6 | Certora | **Huma Finance** (Soroban lending) | Manual + FV | 2024-07-04 | https://github.com/Certora/SecurityReports/blob/main/Reports/2024/07_04_2024_Huma-MR.pdf |
| 7 | Certora | **Aquarius AMM** (Stableswap + CPMM) | Manual review | 2024-12-29 | https://github.com/Certora/SecurityReports/blob/main/Reports/2024/12_29_2024_Aquarius-MR.pdf |
| 8 | Certora | **Reflector** (oracle) DAO & Subscription | FV + manual | 2024-10-10 | https://github.com/Certora/SecurityReports/blob/main/Reports/2024/10_10_2024_Reflector-FV-MR.pdf |
| 9 | Certora | **Cables** | Manual review | 2024-04-10 | https://github.com/Certora/SecurityReports/blob/main/Reports/2024/04_10_2024_Cables-MR.pdf |
| 10 | Veridise | **OrbitCDP** (Zenith CDP stablecoin) | Manual review | 2024-12-26 | https://veridise.com/wp-content/uploads/2025/04/VAR-Stellar-241216-Orbit-CDP-V1.pdf |
| 11 | Veridise | **Phoenix DEX** (Moonbite) | Manual review | 2024-05 | https://veridise.com/audits-archive/company/moonbite/phoenix-dex-2024-05-03/ |
| 12 | Veridise | **HiYield** (Lydia Labs yield) | Manual review | 2024-05 | https://veridise.com/audits-archive/company/lydia-labs/hiyield-2024-05-29/ |
| 13 | Veridise | **Wombat Exchange** (Stableswap) | Manual | 2024-12-24 | https://veridise.com/audits-archive/company/wombat/wombat-exchange-2024-12-24/ |
| 14 | Veridise | **Untangled Vault** | Manual | 2025-05-22 | https://veridise.com/audits-archive/company/untangled-finance/untangled-vault-2025-05-22/ |
| 15 | Veridise | **HOT Bridge locker** | Manual | 2025-09-01 | https://veridise.com/audits-archive/company/hot-dao/hot-bridge-2025-09-01/ |
| 16 | Veridise | **Stellar Timelock (57Blocks)** | Manual | 2024-08-06 | https://veridise.com/audits-archive/company/57blocks/stellar-timelock-contract-2024-08-06/ |
| 17 | OtterSec | **Soroswap Core** (AMM) | Manual | 2024-02-22 | https://github.com/soroswap/core/blob/main/audits/2024-02-22_soroswap_ottersec_audit.pdf |
| 18 | Runtime Verification | **Soroswap Aggregator** | Manual + FV | 2024 | https://docs.soroswap.finance/smart-contracts/soroswap-aggregator/audits |
| 19 | Runtime Verification | **StellarBroker** | Manual | 2025-04-28 | https://strapi-rv-bucket-01.s3.us-east-2.amazonaws.com/Stellar_Broker_6043e21a6c.pdf |
| 20 | Runtime Verification | **Trustless Work** | Manual | 2025-09-12 | https://amp.runtimeverification.com/public-report/trustless-work |
| 21 | Coinspect | **xCall / Balanced Soroban** (cross-chain) | Manual | 2024-10 → 2024-11 | https://github.com/balancednetwork/balanced-soroban-contracts/blob/main/Coinspect%20-%20Smart%20Contract%20Audit%20-%20ICON%20Foundation%20-%20xCall%20Soroban%20-%20Fix%20Review%20-%20v241106.pdf |
| 22 | Coinspect | **Tricorn Bridge** (Soroban) | Source code review | 2024-06 (v240725) | https://www.coinspect.com/doc/Coinspect%20-%20Source%20Code%20Audit%20-%20Soroban%20Tricorn%20Bridge%20v240725.pdf |
| 23 | Halborn | **Normal Finance** (Stellar AMM) | Manual | 2025-08-01 | https://www.halborn.com/audits/normal-finance/stellar-amm-6f9223 |
| 24 | Halborn | **Spiko** (Stellar contracts) | Manual | 2025-09-22 | https://www.halborn.com/audits/spiko/stellar-contracts-879885 |
| 25 | Halborn | **Crossmint** (Soroban) | Manual | 2025-08-22 | https://stellar.org/audit-bank/projects |
| 26 | Halborn | **zkCrossDEX** | Manual | 2025-09-05 | https://stellar.org/audit-bank/projects |
| 27 | Halborn | **Alula** | Manual | 2026-02-10 | https://stellar.org/audit-bank/projects |
| 28 | Code4rena + Zellic | **Reflector V3** (oracle) | Crowdsource + manual | 2025-10 → 2026-02 | https://code4rena.com/audits/2025-10-reflector-v3 |
| 29 | Code4rena | **LayerZero – Stellar endpoint** | Crowdsource | 2026-04 | https://code4rena.com/audits/2026-04-layerzero-stellar-endpoint |
| 30 | CoinFabrik | Scout-reviewed Soroban examples (incl. lending/AMM) | Internal + manual | 2024 | https://github.com/CoinFabrik/scout-soroban-examples |

Aggregator portals used during research:

- Soroban Security Portal: https://sorobansecurity.com/reports
- Stellar Audit Bank index: https://stellar.org/audit-bank/projects (53 projects)
- Veridise Soroban index: https://veridise.com/audits/soroban/
- Veridise Soroban checklist: https://veridise.com/blog/audit-insights/building-on-stellar-soroban-grab-this-security-checklist-to-avoid-vulnerabilities/

Reports listed but excluded from §2 because the PDF could not be reliably text-extracted in
this session: #3 Blend V1 FV, #5 Blend V2 mitigation, #9 Cables, #11/12/13/14/15/16 Veridise
project-specific reports (only summary pages were public), #18/19/20 Runtime Verification
private-PDF gates, #23–29 Halborn / Zellic / LayerZero (web pages exposed scope/summary only).

---

## 2. Per-Audit Findings (lending-relevant)

### 2.1 Certora — Slender (lending on Soroban) — May 2024
Repository `eq-lab/slender`, commit `93d1648`; fixes verified at `993fea5`. This is the closest
analogue to `rs-lending-xlm` in the public set: a Soroban over-collateralised lending pool with
SEP-40 oracle, s-tokens/d-tokens, liquidation, flash loans.

- **C-1 — Withdraw/transfer bypass `initial_health`** (Critical). `do_borrow` enforces
  `require_gte_initial_health`, but `withdraw` and `finalize_transfer` only check positive NPV
  via `require_good_position`. An attacker borrows then withdraws to leave the position at the
  edge, accumulating bad debt. Files: `borrow.rs`, `withdraw.rs`, `finalize_transfer.rs`.
- **C-2 — Transfer/burn-on-zero reverts liquidation** (Critical). SEP-41 `spend_balance` panics
  for non-positive amounts; rounded-down liquidation amounts (`liq_lp_amount`, `liq_comp_amount`)
  can be zero, blocking liquidation. Files: `liquidate.rs`. Fix: gate transfers/burns on `> 0`.
- **C-3 — Stellar resource limit blocks liquidation** (Critical). Health-check IO scales
  linearly with reserves; `liquidate` reverts above ~5 reserves (or 3 with TWAP). Fix: hard cap
  the number of active reserves per user (`pool_config.user_assets_limit`, set to 3).
- **C-4 — Sybil dust-positions create un-liquidatable bad debt** (Critical). Liquidation bonus
  is `% of collateral`; tiny positions are not worth the gas to liquidate. Fix: add
  `min_collat_amount` / `min_debt_amount` and check on every state-changing entry point.
- **C-5 — Rounding direction in withdraw drains funds** (Critical). `s_token_to_burn` uses
  `recip_mul_int` (round-down) instead of `recip_mul_int_ceil`. With `collat_coeff = 2`, a 4-token
  deposit yields 2 s-tokens, then withdrawing 3 tokens burns only 1 s-token, repeatable. Fix:
  rounding must always favour the protocol; refactored into `get_lp_amount(..., round_ceil)`.
- **H-1 — Liquidation bonus can reach 100%** (High). When NPV is very negative,
  `liq_bonus_percent = min(|npv%|, 100%)` zeros out `total_debt_to_cover_in_base`, letting the
  liquidator seize collateral while repaying zero debt.
- **H-2 — Unpause has no grace period** (High). Re-enabling the protocol immediately exposes
  underwater positions; recommend an Aave-style `PriceOracleSentinel`-equivalent grace window
  where deposit/repay/flash-loan are allowed but withdraw/transfer are not.
- **H-3 — TWAP assumes sorted `PriceData` vector** (High). `prices.get_unchecked(0)` is treated
  as the most recent timestamp, but SEP-40 does not require descending order. Reflector happens
  to comply, but a misbehaving / replacement oracle silently breaks pricing. Fix: explicitly
  sort or assert ordering. File: `price_provider.rs`.
- **M-2 — Division-before-multiplication precision loss** across `account_position.rs`,
  `deposit.rs`, `liquidate.rs`, `withdraw.rs`, `price_provider.rs`. Refactored into
  `get_compounded_balance` / `get_lp_amount` performing `mul` before `div`.
- **M-3 — Double decimal conversion via `FixedI128` (1e9 denominator)** loses precision when
  base asset decimals > 9. Fix: convert TWAP price directly to target precision.
- **M-4 — Centralisation: single admin, no RBAC** (Medium). Attempted RBAC pushed WASM > 64KB
  ledger-entry limit and was reverted; deferred until Soroban raises limits. Mitigated by
  multisig externally.
- **M-5 — No backup price feed** (Medium). Recommendation: fallback oracle.
- **M-6 — No stale-price check** (Medium). Fix added `min_timestamp_delta` and explicit error
  return when timestamp gap exceeds threshold.
- **M-7 — No min/max sanity prices ("circuit breaker")** (Medium). Fix added
  `min_sanity_price_in_base` / `max_sanity_price_in_base` computed off-chain.
- **L-1 — Config setter validation gaps** (Low). `set_ir_params` only checks
  `initial_rate <= PERCENTAGE_FACTOR`, `max_rate > PERCENTAGE_FACTOR`,
  `scaling_coeff < PERCENTAGE_FACTOR`; missing `initial_rate <= max_rate`, `scaling_coeff > 0`.
  `set_initial_health` and `set_flash_loan_fee` lack 0..=`PERCENTAGE_FACTOR` bounds.
- **I-2 — Use `10i128.checked_pow` instead of `10i128.pow`** (Informational). Prevents silent
  overflow when computing decimal scalers.

### 2.2 Certora — Blend V1 (Script3) — January 2024
Lending protocol composed of emitter / backstop / liquidity pool. Code3 reviewed live repo at
commit `5a64f8b9a8472b232f9fefe861b2638993b894c8`.

- **BL-001 — "Dead-end" flows under Soroban resource limits** (Critical). Health-check on a
  user holding many assets exceeds CPU/IO budget; the user becomes un-liquidatable. Fix: cap
  supported assets per pool; profile every flow against 80–90 % of resource limit.
- **BL-002 — BLND reward loss between weekly emission cycles** (Medium). Emitter mints 1 BLND/s
  continuously; backstop only distributes in discrete weekly windows. If the cycle is updated
  late, BLND for the gap period gets stuck. Fix: drive emissions from a time-coupled index.
- **BL-003 — Incomplete pool-type validation in backstop deposit** (High). Backstop deposit
  does not verify the pool address was instantiated by the canonical factory; depositors can
  point at fake pools.
- **BL-004 — Reward-zone slot race lets pools enter without time lock** (Info). The 5-day lock
  is not applied when new slots open every 97 days.
- **BL-005 — `fill_user_liq_auction` missing self-check** (Low). Filler may equal the
  liquidated user.
- **BL-006 — Reactivity constant bounds disagree with whitepaper** (Info). Whitepaper:
  0.00001 – 0.001; code: only `> 0.0005` is enforced.
- **BLRC-002 — Reserve cache/load is error-prone** (recommendation). Reserves are read into a
  cache; writeback requires explicit `cache_reserve(write=true)`. Suggests a drop-guard pattern
  to make missing writebacks impossible (highly relevant — same pattern likely exists in
  `rs-lending-xlm`'s `Cache`).
- **BLRC-003 — Numeric enum discriminants are compared directly** (recommendation). E.g.
  `build_actions_from_request` matches numeric request types instead of semantic enums.

### 2.3 Code4rena + Certora FV — Blend V2 — Feb–Mar 2025
Crowdsource audit + first Rust formal-verification contest in DeFi (Certora Sunbeam). Scope
focused on backstop (`withdrawal.rs`, `user.rs`, `deposit.rs`, `fund_management.rs`, `pool.rs`)
plus the full pool. Final report (from Code4rena reports portal) yielded:

- **H-01 — Stale `d_supply` after flash-loan adds liability**. Flash-loan path adds liability
  via `from_state.add_liabilities()` but omits `pool.cache_reserve(reserve)`; subsequent
  requests on the same asset see stale `d_supply`, breaking
  `Σuser_liabilities == d_supply`.
- **H-02 — Cross-recipient emission claim drift**. `backstop::emissions::execute_claim()`
  when `from != to` calls `user_balance.add_shares(to, ...)` without first
  `update_emissions(&to)`; the recipient's emission index is uninitialised and emissions can
  be stolen.
- **H-03 — Utilization can exceed 100 % via withdrawals**. `apply_borrow` enforces
  `require_utilization_below_max()`; `apply_withdraw` / `apply_withdraw_collateral` do not.
  Large withdrawals shrink the denominator and push utilization above 100 %.
- **M-01 — Flash loans bypass frozen pool/reserve status**. `execute_submit_with_flash_loan`
  skips `pool.require_action_allowed()` and `reserve.require_action_allowed()`.
- **M-02 — Premature utilization check blocks valid flash-loan repayments**. The check fires
  immediately after liability is added, before the borrower has a chance to repay; should be
  at the end of request processing.
- **M-03 — Bad-debt auction stuck when backstop tokens withdrawn during fill**. Auction lot
  scales 0 → 200 then plateaus; if backstop tokens are withdrawn mid-auction, fill reverts and
  the auction record is undeletable. Fix: block backstop withdraw while bad-debt auction
  exists, or cap lot at fill time.
- **M-04 — Duplicate reserves inflate auctions**. `create_interest_auction_data` and
  `create_bad_debt_auction_data` iterate the caller-supplied reserve list without dedup;
  duplicates multiply credited value.
- **M-05 — `draw` / `donate` skip `update_rz_emis_data`** breaking reward-zone emissions.
- **M-06 — Pools removed from reward zone keep getting tokens** that were already allocated.
- **M-07 — Gulped emissions lost when reserve has `d_supply == 0`** (division-by-zero / bad
  branch).
- **M-08 — Ungulped emissions destroyed when pool exits reward zone**.
- **M-09 — Partial bad-debt-auction fill on code default leaves orphaned auction record**.
- **M-10 — Division-before-multiplication in `convert_to_shares` DoS at low backstop supply**.
- **M-11 — Fee vault drained below zero on defaults**.
- **M-12 — Repeated triggers dilute emission period**.
- **M-13 — `set_backstop_take_rate` does not accrue interest first**, so the new rate is
  applied against stale `backstop_credit`.
- **M-14 — Flash-loan inflation pushes pool to 100 % utilization** even transiently.
- **M-15 — APR-cap boundary condition extracts excess fees** at `util == max_util`.
- **M-16 — Reward-zone removal blocks historical gulp** of pre-removal accruals.
- **M-17 — Bad debt blocked from socialising to backstop** under edge cases.
- **M-18 — Interest auctions enable inflation attack on backstop vault shares** — flash-mint
  deposit then withdraw against unprotected share ratio.
- **L-02 — `get_market()` DoS** when too many reserves (unbounded iteration).
- **L-03 — Interest auction cannot be created at exactly 200 USDC** (boundary off-by-one).
- **L-05 — User with 1 : 1 collateral-to-liability ratio is still liquidatable** (off-by-one
  in health check).

### 2.4 Certora — Huma (Soroban tranche lending) — July 2024
Huma is on-chain private credit (junior/senior tranches, redemption epochs, pool lifecycle).
Only Low + Informational severities were found (25 issues), but many are directly relevant:

- **L-01 — `initialize` can be front-run** because public init lacks authorization. Anyone
  watching the mempool can call it on a fresh deployment. Fix: bundle deploy+init in a factory
  contract, or guard with deterministic address + auth.
- **L-02 — Round-down in `cancel_redemption_request()`** can zero out recovered principal:
  `lrr.principal_requested() * shares / lrr.num_shares_requested()` rounds toward zero.
- **L-03 — Round-down in `withdraw_after_pool_closure()`** leaves dust in the pool
  permanently.
- **L-04 — `enable_pool` / `disable_pool` / `close_pool` allow forbidden state transitions**.
  A closed pool can be re-enabled, contradicting the `PoolStatus::Closed` doc-comment.
- **L-05 — Underlying token decimals not bounded**. `10_u128.pow(token.decimals())` overflows
  for huge decimals; `refresh_late_fee` rounds to zero with very small decimals. Recommend
  6 ≤ decimals ≤ 18.
- **L-06 — Removing and re-adding a lender wipes their `DepositRecord`**, resetting
  `last_deposit_time` and bypassing the withdrawal lockout.
- **L-07 — `unprocessed_amount` is scaled inconsistently** in `process_epoch()`: first sum is
  unscaled, second sum divides by `DEFAULT_DECIMALS_FACTOR`; if no senior tranche, the event
  fires unscaled.
- **L-10 — Duplicate error codes** across crates (101 used in two modules) hinders triage.
- **L-11 — `TrancheVault.initialize` missing `index ∈ {0,1}` check** (Solidity version had it).

### 2.5 Certora — Aquarius AMM — December 2024
AMM (constant-product + Curve-style Stableswap). High-severity findings demonstrate Soroban
patterns that DeFi protocols repeatedly get wrong.

- **H-01 — Admin time-locked fee/ownership change is defeated by `upgrade`**. The pool ships
  with `commit_new_fee` / `apply_new_fee` / `commit_transfer_ownership` time-locks, but
  `upgrade(new_wasm_hash)` has no delay — an admin can simply replace the WASM and bypass the
  notice period. Fix: time-lock the upgrade itself; allow emergency admin to skip only for
  vulnerability hotfixes.
- **H-02 — `get_d()` Newton–Raphson lacks divergence guard**. Curve's reference reverts after
  255 rounds; Aquarius silently exits with whatever last value `d` had, which can be far from
  the invariant root. Fix: revert on non-convergence.
- **H-03 — Stableswap hard-codes 7 decimals** for every asset, breaking capital efficiency
  whenever assets have different SEP-41 decimals.
- **H-04 — Stableswap amplification `a` parameter mis-interpreted** versus the whitepaper
  (`ann = a * n^n` vs `a * n` confusion).
- **M-01 — Fee-on-transfer / rebasing / deflationary tokens** break the internal-accounting
  invariant on which Soroban AMMs rely.
- **M-02 — Soroban `require_auth` phishing surface** (originally raised by OtterSec on
  Soroswap). A scam-token contract can craft an auth payload that lets it drain not only the
  scam balance but the user's other assets within the same authorization tree.
- **M-03 — Single admin role violates least-privilege**.
- **M-04 — Inflation attack via `donate_admin_fees`**. Internal-reserve accounting normally
  defeats first-depositor inflation, but `donate_admin_fees` rewrites
  `reserves[i] = token.balance(contract)`, re-syncing internal to external — an attacker who
  controls (or coerces) the operations admin can deploy the classic ERC-4626 share inflation.
- **L-01 — Loss of funds on token-address migration**.
- **L-02 — Privileged address can be set to invalid values without two-step confirmation**.
- **L-03 — Empty salt used in token creation** (predictable contract address).
- **L-04 — Fee-fraction check is hard-coded** rather than parameterised.

### 2.6 Certora — Reflector (oracle) DAO + Subscription — October 2024
Reflector is the principal SEP-40 price feed used by Slender, Blend, Orbit, etc.

- **L-01 — All-powerful admin** in Subscription and DAO contracts (acknowledged, won't fix).
- **L-02 — Missing events** on `config`, `set_fee`, `update_contract`, `update_dao_balance`,
  `set_dao_balance`, `create_ballot`.
- **L-03 — Admin/token setter has no two-step confirmation or address-shape validation**.
- **L-04 — Unchecked arithmetic** (`balance + amount`, `deposit * 75 / 100`, `-refunded`,
  `-burn_amount`); fixed with `checked_*` and explicit errors.
- **L-05 — `unlock(developer, operators)` accepts duplicate operator entries**, letting an
  admin pay the same address multiple shares.

### 2.7 Veridise — OrbitCDP (Soroban CDP stablecoin pegged via Blend) — December 2024
OrbitCDP lets users mint stablecoins against XLM collateral and integrates the Blend lending
pool for flash-loan-based liquidations and peg-keeping.

- **V-OBT-VUL-001 — Centralisation risk** (High). Heavy reliance on centralised actors; needs
  multi-sig + RBAC.
- **V-OBT-VUL-002 — Pegkeeper does not share profits with protocol** (High). Missing fee
  distribution.
- **V-OBT-VUL-003 — Pegkeeper accepts an arbitrary Blend pool address** (High). Allows the
  caller to take a flash loan from any Blend pool because the pool reference is not
  whitelisted.
- **V-OBT-VUL-004 — Pegkeeper can finish with an open debt position** (High). Liquidation flow
  is allowed to complete without closing the borrowed flash-loan position.
- **V-OBT-VUL-005 — Admin transfer is single-step, irreversible** (High). Recommend
  two-step + timelock.
- **V-OBT-VUL-006 — Insufficient sanity checks when adding a stablecoin** (Medium).
- **V-OBT-VUL-007 — OUSD pegged to USDC instead of USD** (High). Oracle base mis-config.
- **V-OBT-VUL-008 — Pegkeeper does not validate the callback function name** (Medium).
- **V-OBT-VUL-009 — Liquidator has no slippage protection** during swap into stablecoin
  (High).
- **V-OBT-VUL-010 — Unbounded data written to instance storage** (Medium). The Soroban-class
  bug: instance storage loads on every call, and growth eventually exceeds the per-tx ledger
  entry size.
- **V-OBT-VUL-011 — Maintainability issues** (Low).

### 2.8 OtterSec — Soroswap Core (AMM) — December 2023, published Feb 2024
First public Soroban DeFi audit; established several patterns later cited by every subsequent
audit.

- **OS-SWP-ADV-00 — Unbounded data in instance storage** (High). `add_pair_to_all_pairs`
  pushes pair addresses into a `Vec` stored under instance storage; permissionless
  `create_pair` lets anyone fill the 64 KB instance budget and DoS the factory. Recommendation:
  use persistent storage **and** key per-item (not a single big Vec).
- **OS-SWP-ADV-01 — `burn` does not update `total_shares`** (Low). LP-token burn diverges
  internal supply from balances.
- **OS-SWP-ADV-02 — Fee rounding-down in `swap`** (Low). `fee = amount * 3 / 1000` rounds
  fees to zero on small swaps.
- **OS-SWP-SUG-00 — Integer overflow** in arithmetic on `i128`/`u128` constants.
- **OS-SWP-SUG-01 — Missing address check** when creating pairs (allows duplicate / mirrored
  pairs).
- **OS-SWP-SUG-02 — `require_auth` token-transfer phishing** (informational). Origin of the
  Soroban auth-tree drainage class re-cited by Aquarius M-02.

### 2.9 Coinspect — Tricorn Bridge (Soroban side) — June/July 2024
Bridge between EVM and Stellar via Soroban contracts.

- **TRI-001 — Adversaries can modify bridge parameters to steal funds** (High). Privileged
  setters reachable without proper auth segregation.
- **TRI-002 — Storage unlimited growth halts contract operations** (High). Classic Soroban
  storage growth issue.
- **TRI-003 — Insufficient authorization validation lets adversaries steal bridge-out funds**
  (High).
- **TRI-004 — Resource exhaustion due to inefficient storage layout** (Medium).
- **TRI-005 — Fee-on-transfer tokens cause unexpected losses** (Medium).
- **TRI-006 — Inconsistent storage TTL handling** (Medium). Mixed `extend_ttl` semantics
  across modules.
- **TRI-007 — Lack of adversarial / integration tests** (Medium).
- **TRI-008 — Backend can process duplicate Bridge events** (Medium). No idempotency on
  cross-chain replay.
- **TRI-009 — Platform admin front-runs fee updates** (Medium).
- **TRI-010 — Unsupported `uint256` token value bridge operation** (Low).
- **TRI-011 — Bridge-in operations cannot handle high amounts due to `i128` overflow** (Low).
- **TRI-012 — Using old Soroban SDK version**.

### 2.10 Coinspect — xCall (Balanced/ICON GMP on Soroban) — Oct–Nov 2024
- **XCL-001 — Anyone can prevent updates to sources/destinations on xCall Manager** (High).
  Update authorization is gated on a flag the attacker can flip.
- **XCL-002 — Lack of privilege segregation** (High → kept open).
- **XCL-003 — Asset manager returns info for non-existing token addresses** (High; fixed).
- **XCL-004 — Anyone can write token data for arbitrary tokens** (Medium).
- **XCL-005 — Unsafe integer casting** (e.g. `as i128` truncation).
- **XCL-006 — Anyone can trigger rollbacks without authorization** (Medium).
- **XCL-007 — Anyone can drain asset manager token holdings** (High with PoC).
- **XCL-010 — Time-diff conversion to seconds with wrong units**.
- **XCL-011 — Zero-value deposits allowed**.
- **XCL-013 — Same error code reused for multiple issues** (testing/debug hazard).
- **XCL-015 — `deposit` does not enforce destination address**.

### 2.11 Veridise — Soroban Security Checklist (audit-insights blog)
Not an audit but a distilled checklist Veridise publishes alongside its Soroban audits:

- Validate `Vec<T>` / `Map<K,V>` after they cross the host boundary (they collapse to `Val`
  and can fail back-conversion later, halting the contract).
- Replace bare `panic!` with `panic_with_error!` so fuzzers can distinguish expected vs
  unexpected panics.
- `createimport!` does not enforce dependency declarations — contracts can deploy with stale
  dependency hashes without test-time errors.
- Instance storage is loaded **on every invocation** — never store unbounded data there;
  spread persistent data across keyed slots to avoid the ledger-entry-size cap.

### 2.12 Halborn — Normal Finance (Stellar AMM) — Aug 2025
From Halborn's public case study (full PDF not exposed):

- Audit emphasised AMM invariant enforcement, fee/residue accounting, NAV-aligned share
  valuation during rebalancing, insurance coverage caps, and revenue withdrawal procedure.
- Halborn researched Stellar's native handling of reentrancy and recommended **tracking entry
  and exit for every user-facing contract call** so cross-contract flows stay observable —
  i.e. an explicit `non_reentrant`-style guard at every public entry, because Soroban does not
  inherently prevent re-entry via cross-contract calls.

---

## 3. Cross-Cutting Taxonomy — Recurring Soroban Bug Classes

Ranked by frequency across the corpus.

1. **Storage misuse (TTL + bucket + unbounded growth).** Slender M-6 stale price, OrbitCDP
   VUL-010, Soroswap ADV-00, Tricorn TRI-002/TRI-006, Aquarius L-03. Pattern: use of
   `instance()` storage for permissionless / unbounded data; missing `extend_ttl`; persistent
   storage stored as a single big `Vec`. Veridise checklist makes the same point.
2. **Authorization gaps & phishing-prone `require_auth`.** Huma L-01 unauthenticated `init`;
   xCall XCL-001/004/006/007; Tricorn TRI-001/TRI-003; Aquarius M-02; Soroswap SUG-02.
   `require_auth` for the *caller* is not enough — entry points must also validate target
   addresses and dedupe replay (idempotency).
3. **Rounding direction / division-before-multiplication.** Slender C-5 / M-2 / M-3; Soroswap
   ADV-02; Blend V2 M-10; Huma L-02/L-03. Rule: every share/asset conversion in a lending
   protocol must round in the direction that hurts the user, not the protocol.
4. **Share inflation / first-depositor attack.** Aquarius M-04 (via `donate_admin_fees`);
   Blend V2 M-18 (backstop vault auctions). Internal accounting alone is not enough if any
   path can resync internal reserves to external balances.
5. **Resource-limit-induced DoS / un-liquidatable positions.** Slender C-3, Blend BL-001,
   Blend V2 L-02 (`get_market()` DoS), Tricorn TRI-004. Soroban's per-tx CPU/IO budget is
   tight; any flow whose cost scales with N reserves can become un-callable.
6. **Health-check inconsistency between entry points.** Slender C-1 (withdraw vs borrow),
   Blend V2 H-03 (utilization check missing on withdraw), Blend V2 L-05 (1:1 ratio edge), and
   the broader pattern of "borrow enforces invariant X but withdraw / transfer / liquidate
   does not".
7. **Oracle handling (TWAP ordering, staleness, sanity bounds, fallback).** Slender H-3 (TWAP
   ordering), M-5 / M-6 / M-7. SEP-40 only specifies the price *shape*, not freshness or
   ordering — every consumer must enforce.
8. **Liquidation incentives / dust positions.** Slender C-4 (no min position), Slender H-1
   (100% bonus → zero debt repaid), Blend V2 L-03 (200-USDC boundary).
9. **Interest-accrual ordering bugs.** Blend V2 M-13 (`set_backstop_take_rate` without prior
   accrual), H-01 (flash-loan path skips `cache_reserve` so `d_supply` is stale).
10. **Pause / lifecycle state machine errors.** Slender H-2 (no grace period after unpause),
    Huma L-04 (`Closed → On` transition allowed despite docs), Blend V2 M-01 (flash loans
    bypass frozen-pool checks).
11. **Reentrancy via cross-contract calls.** Halborn (Normal) recommends explicit entry/exit
    tracking on every external call because Soroban does not guarantee non-reentry; Blend
    BLRC-002 cache/writeback pattern is in the same family.
12. **Centralisation & upgrade bypass.** Aquarius H-01 (upgrade defeats timelocked fee
    change), Slender M-4, Reflector L-01, OrbitCDP VUL-001, xCall XCL-002.
13. **Token-decimal assumptions.** Aquarius H-03 (hard-coded 7 decimals), Huma L-05 (no
    decimal bounds), Slender M-3 (double conversion via fixed 1e9 denom loses precision when
    base decimals > 9).
14. **Unsafe arithmetic.** Reflector L-04 (unchecked +/−/*), Slender I-2 (`10i128.pow` not
    `checked_pow`), Tricorn TRI-011 (`i128` overflow on bridge-in), xCall XCL-005 (unsafe
    `as` casts), Soroswap SUG-00.
15. **Idempotency / replay.** Tricorn TRI-008 (duplicate cross-chain events), xCall XCL-006
    (anyone triggers rollback).
16. **Missing events.** Reflector L-02; Aquarius L-06; xCall (multiple).

---

## 4. Verification Checklist Tailored to `rs-lending-xlm`

Each item lists the **audit finding(s)** that motivate it, and the **module/file** to inspect.
Treat as a review pass over the lending protocol; every "yes" is the answer the protocol
should produce.

### 4.1 Math & rounding — `math/`, `pool/state/`, `pool/liquidation/`
- [ ] Every share→asset / asset→share conversion rounds in the **protocol's favour** (round
  shares **up** on burn/withdraw, round assets **down** on credit; round shares **down** on
  mint, round assets **up** on debt). Motivation: Slender C-5, Soroswap ADV-02, Huma L-02/03.
- [ ] No `recip_mul_int(...)` (round-down) is used on a withdraw/burn path; use the `_ceil`
  variant. Motivation: Slender C-5 fix.
- [ ] All Ray / Wad / fixed-point chains multiply **before** dividing
  (`a.mul_int(b)?.div_int(c)?` not `a.div_int(c)?.mul_int(b)?`). Motivation: Slender M-2,
  Blend V2 M-10.
- [ ] No `i128::pow` / `u128::pow` without `checked_pow`. Motivation: Slender I-2.
- [ ] All `+ - * /` on user-supplied or accruing values use `checked_*` and return a typed
  error, never `unwrap`. Motivation: Reflector L-04, Tricorn TRI-011, xCall XCL-005.
- [ ] Fixed-point conversion does not lose precision when base-asset decimals > 9 (avoid the
  `FixedI128(1e9)` round-trip). Motivation: Slender M-3.

### 4.2 Oracle — `oracle/`, `price_provider`, `pricing/`
- [ ] TWAP code does **not** assume the SEP-40 `Vec<PriceData>` is sorted; explicitly sort or
  assert by `timestamp`. Motivation: Slender H-3.
- [ ] Every price read enforces a **staleness bound** (`ledger.timestamp() - price.timestamp
  ≤ max_staleness`) with a typed error path, not a panic. Motivation: Slender M-6.
- [ ] Every price read enforces **min/max sanity bounds** computed off-chain. Motivation:
  Slender M-7.
- [ ] A **fallback oracle** exists (or the protocol pauses cleanly when primary returns
  stale/None). Motivation: Slender M-5.
- [ ] Oracle integration code reverts cleanly on a malformed `Vec<PriceData>` (no
  `get_unchecked`). Motivation: Slender H-3, Veridise checklist on `Vec<Val>` boundary.

### 4.3 Interest-rate model — `interest_rate/`, `pool/accrue/`, `reserve/`
- [ ] `IRParams` setters validate `initial_rate <= max_rate`, `scaling_coeff > 0`, and the
  whole struct fits in `[0, PERCENTAGE_FACTOR]`. Motivation: Slender L-1.
- [ ] Every administrative knob that affects interest rate calls `reserve.accrue()` (or the
  equivalent `cache_reserve` write-back) **before** applying the new value. Motivation:
  Blend V2 M-13.
- [ ] Utilization is checked **on every state-changing entry**, not just `borrow`. Withdraw,
  withdraw-collateral, flash-loan, transfer must all gate against `max_utilization` (or
  recheck final-state utilization). Motivation: Blend V2 H-03, M-01, M-02, M-14.
- [ ] Flash-loan flow caches the reserve **after** adding liabilities and **before**
  processing the next request on the same asset. Motivation: Blend V2 H-01.
- [ ] Reactivity / IR bounds match the whitepaper exactly (not just one side). Motivation:
  Blend BL-006.

### 4.4 Health / NPV / position invariants — `pool/account/`, `health/`
- [ ] `initial_health` (or equivalent collateralisation threshold) is enforced on borrow,
  **and** on withdraw, withdraw-collateral, transfer of s-tokens, and `set_as_collateral`.
  Motivation: Slender C-1.
- [ ] Health check does not have an off-by-one allowing a 1 : 1 collateral/liability position
  to liquidate. Motivation: Blend V2 L-05.
- [ ] Minimum collateral and minimum debt thresholds (`min_collat_amount`, `min_debt_amount`)
  are enforced on every entry point that can leave a residue — borrow, repay, withdraw,
  set_as_collateral, transfer. Motivation: Slender C-4.
- [ ] Number of active reserves per user is **hard-capped** (Slender chose 3) and matches
  current Soroban CPU/IO budget — every flow profiled at ≤ 80–90 % of the resource limit.
  Motivation: Slender C-3, Blend BL-001, Blend V2 L-02.

### 4.5 Liquidation — `pool/liquidation/`, `auction/`
- [ ] Liquidation bonus is bounded so the repaid debt **never** rounds to zero (i.e. liquidator
  must repay > 0 debt to receive > 0 collateral). Motivation: Slender H-1.
- [ ] Transfers/burns of zero amount are short-circuited so they cannot revert liquidation.
  Motivation: Slender C-2.
- [ ] Liquidator address ≠ liquidated address (defensive even if reachable today).
  Motivation: Blend BL-005.
- [ ] Auction creation deduplicates `lot` / reserve lists. Motivation: Blend V2 M-04.
- [ ] Bad-debt auctions cannot be left orphaned: either block withdrawals from the backstop
  while an auction exists, or cap `lot_amount` at fill time, and always clear the auction
  record on default. Motivation: Blend V2 M-03, M-09, M-17.
- [ ] Liquidation swap path has explicit slippage protection (`min_amount_out`). Motivation:
  OrbitCDP VUL-009.
- [ ] After unpause, lenders get an explicit grace period during which liquidation is
  disabled but deposit / repay / flash-loan are allowed. Motivation: Slender H-2.

### 4.6 Storage — `storage/`, `pool/storage/`, instance vs persistent vs temporary
- [ ] No permissionless write goes into `env.storage().instance()` — instance is for small,
  per-contract config only. Anything user-driven uses `persistent()` (or `temporary()` for
  short-lived data). Motivation: Soroswap ADV-00, OrbitCDP VUL-010, Veridise checklist.
- [ ] No single persistent slot holds an unbounded `Vec` / `Map`; data is keyed per item.
  Motivation: same.
- [ ] Every persistent / instance read path calls `extend_ttl(...)` with documented thresholds
  consistent across modules (`*_BUMP_AMOUNT` constants). Motivation: Tricorn TRI-006.
- [ ] Temporary storage TTLs cover the worst-case latency between deposit and the next user
  interaction. Motivation: same.
- [ ] Reserve / cache pattern: every code path that mutates an in-memory `Reserve` either
  writes it back via the cache or is statically proven not to mutate. Consider a drop-guard.
  Motivation: Blend BLRC-002.

### 4.7 Authorisation, init, upgrade — `lib.rs`, `admin/`, `governance/`
- [ ] `initialize` either requires `require_auth` of a deterministic deployer **or** is
  bundled with deployment via a factory so it cannot be front-run. Motivation: Huma L-01.
- [ ] Admin / role transfer is two-step (`commit_*` → `apply_*` with a timelock). Motivation:
  OrbitCDP VUL-005, Reflector L-03, Aquarius (general).
- [ ] Privileged config setters that are time-locked are not bypassable by `upgrade` —
  upgrade itself must be time-locked, with a separate emergency-admin path. Motivation:
  Aquarius H-01.
- [ ] RBAC: at least `admin` (upgrade), `risk_admin` (rate/health params), `pause_admin`
  (freeze), `emergency` (skip timelock for hotfix). Motivation: Slender M-4, Aquarius M-03,
  Reflector L-01.
- [ ] Every public function explicitly chooses between `require_auth(addr)` and
  `require_auth_for_args(addr, args)`; tokens passed in as arguments are validated
  (e.g. by calling a known SEP-41 method or against a registry) before any auth tree is
  built, to avoid the cross-token auth-drainage phishing class. Motivation: OtterSec SUG-02,
  Aquarius M-02.

### 4.8 Token assumptions — `token/`, `sac/`, integration boundary
- [ ] Underlying token `decimals()` is bounded (e.g. 6 ≤ d ≤ 18) and the bound is checked at
  reserve-init. Motivation: Huma L-05.
- [ ] Fee-on-transfer / rebasing tokens are explicitly disallowed (documented and tested) or
  the protocol uses pre/post-balance accounting rather than the transferred amount.
  Motivation: Aquarius M-01, Tricorn TRI-005.
- [ ] Stellar Asset Contract (SAC) wrappers are treated as untrusted external code (panics
  must not poison protocol state).
- [ ] No assumption that the asset's decimal scaling factor fits in `i64`; use `i128` and
  `checked_pow`. Motivation: Tricorn TRI-011, Huma L-05.

### 4.9 Pause / lifecycle — `pool/status/`, `state_machine/`
- [ ] `PoolStatus` transitions are a documented state machine; illegal transitions
  (`Closed → On`, etc.) revert. Motivation: Huma L-04.
- [ ] Flash loans honour `pool.require_action_allowed()` and `reserve.require_action_allowed()`
  exactly like normal borrow. Motivation: Blend V2 M-01.

### 4.10 Reentrancy / cross-contract — every public entry
- [ ] Every user-facing entry has an explicit non-reentrant guard tracking entry/exit (Soroban
  does not enforce this for cross-contract calls). Motivation: Halborn Normal recommendation.
- [ ] Any callback target supplied by the user (e.g. flash-loan receiver, oracle, swap
  adapter) is **whitelisted or whitelisted-by-class**, never an arbitrary `Address`.
  Motivation: OrbitCDP VUL-003 / VUL-008.

### 4.11 Idempotency & events — every entry, every storage write
- [ ] Bridge-style / oracle-callback-style entries dedupe by deterministic ID and reject
  duplicates. Motivation: Tricorn TRI-008.
- [ ] Every state-changing function emits an event with the relevant amounts (post-scaling,
  not pre-scaling). Motivation: Reflector L-02, Huma L-07/L-08.
- [ ] Error codes are unique per condition (no dual-purpose codes that hinder testing).
  Motivation: Huma L-10, xCall XCL-013.

### 4.12 Panics vs `Error` returns — `errors.rs`, every `?`
- [ ] No bare `panic!()` in production paths — only `panic_with_error!` so fuzzers
  distinguish. Motivation: Veridise checklist.
- [ ] No `.unwrap()` / `.expect()` outside tests / documented startup-only invariants.
- [ ] All math errors return a typed `Error` (`MathOverflowError`, `LiquidateMathError`, …)
  not a generic `Error::Unknown`. Motivation: pattern observed in Slender fixes.
- [ ] Try-style fallible patterns (`try_*`) wrap every cross-contract call that the protocol
  must remain solvent across (e.g. partial oracle outage, token panic during liquidation).

### 4.13 Inflation & first-depositor — `share_token/`, `s_token/`, vault math
- [ ] No path lets an admin or anyone resync internal accounting to external balances
  (e.g. no `donate` / `skim` / `sync` that updates `reserves[i] = balance(self)`).
  Motivation: Aquarius M-04.
- [ ] First-depositor inflation mitigated by (a) internal accounting that never trusts
  external balance, (b) minimum-shares-on-first-deposit ("dead shares"), or (c) virtual
  shares offset. Motivation: Blend V2 M-18, Aquarius M-04.

### 4.14 Resource budget testing — `tests/`, fuzz/
- [ ] Worst-case flows (liquidation of a user with the maximum number of reserves + flash
  loan + auction fill) are exercised under a CPU/IO watermark at ≤ 80–90 % of mainnet
  budget. Motivation: Blend BL-001 strategic recommendation.
- [ ] Adversarial / negative tests exist for every reachable `panic_with_error!`. Motivation:
  Tricorn TRI-007.

---

### Sources of each finding referenced above

All findings cited in §2 and §4 trace to specific PDFs in §1. Where a Soroban-class pattern
is recurring (storage misuse, oracle staleness, rounding), the canonical citations are:

- Slender (Certora, 2024-05): C-1 … M-7, L-1, I-2.
- Blend V1 (Certora, 2024-01): BL-001 … BL-006, BLRC-002.
- Blend V2 (C4 + Certora, 2025-02–03): H-01 H-02 H-03; M-01 … M-18; L-02 L-03 L-05.
- Huma (Certora, 2024-07): L-01 … L-11.
- Aquarius (Certora, 2024-12): H-01 … H-04, M-01 … M-04, L-01 … L-06.
- Reflector (Certora, 2024-10): L-01 … L-05.
- OrbitCDP (Veridise, 2024-12): V-OBT-VUL-001 … 011.
- Soroswap (OtterSec, 2023-12): OS-SWP-ADV-00 … 02, SUG-00 … 02.
- Tricorn Bridge (Coinspect, 2024-06): TRI-001 … TRI-014.
- xCall (Coinspect, 2024-10): XCL-001 … XCL-016.
- Veridise Soroban checklist (blog, 2024).
- Halborn Normal Finance (case-study summary, 2025-08).

Anything not in this list is *not* asserted in this document.
