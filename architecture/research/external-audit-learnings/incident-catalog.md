# Incident Catalog — Realized Exploits & Near-Misses

Lending-protocol incidents that actually happened (or were caught pre-exploit), with root cause and the lesson for an Aave-faithful Soroban lender. Audit-only findings (never exploited) live in `vuln-taxonomy.md`; their mapping to our code is in `self-audit-backlog.md`.

## Summary table

| Date | Protocol | Loss | Class | One-line root cause |
|------|----------|------|-------|---------------------|
| 2021-08 | Solend (Solana) | $16K (≈$2M at risk) | access | `UpdateReserveConfig` accepted an attacker-owned market as authority (forgeable admin) |
| 2022-03 | Cashio (Solana) | $52.8M | hf/validation | Minted against unvalidated fake collateral (account-type confusion) |
| 2022-06 | Solend (Solana) | $0 (governance) | access | SLND1: emergency power to seize a user's wallet, ratified by one whale vote in ~9h |
| 2022-06 | Solend (Solana) | near-miss | caps | No per-account borrow cap → whale position larger than DEX depth |
| 2022-07 | Crema (Solana) | $8.8M | flashloan | Forged tick account (missing owner check) amplified by flash loans |
| 2022 | Jet (Solana) | ~$25M at risk (caught) | hf/liquidation | Health loop `break`/`take_while` halted at first closed position slot |
| 2022-10 | Mango Markets (Solana) | ~$116M | oracle | Thin self-traded market pumped ~2,300%; unrealized PnL counted as collateral |
| 2022-11 | Solend (Solana) | $1.26M | oracle | USDH priced from a single low-liquidity/stale pool |
| 2022-11 | Aave v2 (CRV) | ~$1.6–1.8M bad debt | liquidation | Flat liquidation bonus → toxic-liquidation spiral past LTV=1/(1+i) |
| 2022 | Agave (Aave fork) | drain | reentrancy | ERC777-style token transfer hook re-entered lending logic |
| 2022 | Hundred Finance | drain | interest/reentrancy | Empty-market + reentrancy (first-depositor index manipulation class) |
| 2022 | Radiant Capital | ~$4.5M | interest | Empty/new-reserve first-depositor `liquidityIndex` inflation |
| 2023-06 | Morpho Aave-v3 Optimizer | ~$2.85M modeled (caught, $285K bounty) | interest | Per-block cached pool index + flash-loan-premium donation overvalued deposits |
| 2023-11 | Aave v2/v3 (stable rate) | incident → deprecated | interest | Stable-rate accounting bug; feature disabled protocol-wide |
| 2024-10 | Morpho Blue (PAXG/USDC) | $230K | oracle | Market oracle decimals configured 8/8 vs real 18/6 → collateral inflated 10¹² |
| 2025-04 | Morpho App | ~$2.6M near-miss (whitehat returned) | access | Frontend SDK approved the router instead of initiator-bound adapters |
| 2026-02 | Blend/YieldBlox (Soroban) | $10.8M | oracle | Reflector priced a token off a dead Stellar DEX; ~$4 trade moved it ~100x; adapter passed raw price through |

## The four most instructive incidents for us

### 1. Blend/YieldBlox — $10.8M oracle manipulation (Feb 2026) — **our nearest peer**
- **What:** Reflector derived a USTRY price from an illiquid Stellar DEX market. A ~$4–5 burner trade moved the price ~100x; Reflector's VWAP over a dead market gave ~51x effective. The oracle adapter took **no median, deviation, or sanity bound** and passed the raw last-price into HF math. $158K real collateral was valued at ~$16M; attacker borrowed 61.2M XLM + 1M USDC.
- **Critical detail:** Blend's $125K Code4rena audit + Certora formal verification had **proved** "cannot extract below min HF" — and the HF invariant *held* (1.35–1.47). It was computed on a poisoned price. Contract correctness and FV are necessary but **insufficient**; the oracle trust assumption was never validated against market structure.
- **Aftermath:** the ~$2M backstop (80/20 BLND:USDC LP) was undersized vs the ~$9M hole; residual socialized to suppliers via bToken collapse (bXLM 1.0→0.45, ~55% supplier loss). Recovery came only from a Stellar Tier-1 validator freeze — an off-protocol lever.
- **Lessons for us:** never price collateral off a thin order book; require liquidity/depth gates + median-of-N + max per-interval deviation + staleness + a circuit breaker *before* price reaches HF math; size any first-loss capital in stressed-dollar terms; don't design assuming a chain-freeze backstop. Our dual-source (Reflector+RedStone) tolerance bands + sanity bounds are the intended defense — the #1 self-audit question is whether they actually reject a 100x outlier and whether we can ever read off a thin source.
- Sources: rekt.news/yieldblox-rekt; halborn.com YieldBlox writeup; medium @cryip 10.8M analysis; bankless; github saariuslystoned/blnd-huntr.

### 2. Aave v2 CRV — toxic liquidation spiral (Nov 2022) — ~$1.6–1.8M bad debt
- **What:** Not price volatility — the liquidation *math*. A flat liquidation bonus `i` creates a "toxic frontier" at LTV = 1/(1+i) (~95.7% for a 4.5% bonus); above it, each liquidation seizes (1+i)·ΔB and *deterministically raises* the borrower's LTV, even at static price → self-reinforcing cascade → bad debt. One whale's ~$40M illiquid CRV short fed slippage into the spiral.
- **Lessons:** halt liquidations past the frontier, OR make the bonus dynamic (`i < 1/LTV − 1`, vanishing as LTV→1), OR scale the close factor toward 100% near insolvency; size borrow caps to real *exit* liquidity. Validates our **per-account derived bonus ceiling** (threshold·(1+bonus) ≤ 100%) — verify it actually prevents the frontier.
- Source: arxiv.org/abs/2212.07306 (Toxic Liquidation Spirals).

### 3. Morpho Aave-v3 Optimizer — cached index + flash-loan donation (Jun 2023) — ~$2.85M (caught)
- **What:** Morpho cached the underlying Aave liquidity index once per block; an attacker inflated the *live* aToken index by repeatedly taking flash loans whose premiums donate to the reserve, then supplied against the stale cached index, over-crediting the position. Reported by a whitehat; fixed by removing index caching ($285K bounty).
- **Lessons:** a per-block / per-tx index cache is a **freshness-correctness boundary, not a gas win**. Direct caution for our `bulk_get_sync_data` / `prefetch_market_indexes` batching: a snapshotted index read against fresher reality (donations, premium/risk-premium accrual) within a tx reproduces this exact bug.
- Source: morpho.mirror.xyz vulnerability report; Spearbit review.

### 4. Morpho Blue PAXG/USDC — oracle decimals (Oct 2024) — $230K
- **What:** A permissionless market's oracle was configured with Base/Quote decimals 8/8 while real decimals are PAXG=18, USDC=6 (12-decimal gap). SCALE_FACTOR normalized wrong, overvaluing PAXG 10¹². $350 collateral borrowed $230K.
- **Lessons:** validate configured oracle decimals against on-chain token decimals at config time; keep absolute supply/borrow caps + min-borrow-collateral as defense-in-depth so an oracle/decimal error cannot mint unbounded borrow power.
- Source: blog.verichains.io Morpho oracle; medium coinmonks decoding-morphoblues-230k.

## Cross-incident patterns

- **Oracle manipulation is the #1 realized killer** (Mango, Solend×2, Blend, Polter/Blizz). Always traces to: collateral priced off a thin/single/stale source with no deviation/depth gate. FV of the HF math does not help.
- **Account/state-confusion → over-borrow or infinite mint** (Solend-2021, Cashio, Crema, Jet): trusting caller-supplied accounts/collateral without verifying owner/type/identity. Soroban analog: validate every caller-supplied `Address` against governance-registered state; read authoritative state from your own storage keyed by trusted IDs.
- **Governance can be the backdoor** (Solend SLND1, Mango self-vote): no role may seize/freeze/force-close a *specific* account outside the permissionless flow; emergency/upgrade powers need timelocks + minimum windows + capture resistance.
- **Forks get hurt in the code they added, not the inherited core** (Spark, Agave, Radiant): custom oracle adapters, savings-rate pricing, and value held outside the main contract are where bugs live. "Matches the reference" gives zero assurance for the parts you changed.
- **The off-chain tx-builder is in scope** (Morpho App): correct contracts were still drainable because the frontend approved the wrong contract. Our keeper/frontend auth-tree construction is security-critical.

## Additional incidents (completeness round)

Added after a completeness-critic pass; each carries a lesson not covered above.

| Date | Protocol | Loss | Class | New lesson |
|------|----------|------|-------|-----------|
| 2023-03 | Euler Finance | ~$197M | liquidation/economic | A donation primitive can be used to *manufacture* one's own insolvency, then self-liquidate at the (unbounded) soft-liquidation discount. |
| 2024-05 | Sonne Finance | ~$20M | hf/access | Empty-market first-depositor inflation executed *through the governance timelock* — the malicious market-add was queued and the delay waited out. |
| 2024-06 | UwU Lend | ~$19.3M | oracle | A TWAP/weighted oracle whose window is short enough to be moved by a flash-loaned spot trade is **not** protection. |
| 2024-10 | Radiant Capital | ~$53M | access/operational | Multisig signers **blind-signed** a malicious `transferOwnership` hidden in a routine batch on hardware wallets (malware showed benign data). |
| 2022-04 | Inverse Finance | ~$15.6M | oracle | (Reinforces Mango/Blend) collateral priced off a manipulable on-chain TWAP. |

- **Euler ($197M):** `donateToReserves` + flawed donation/liquidation accounting let the attacker inflate reserves to push *their own* position underwater, then self-liquidate capturing an unbounded discount. → Lesson for us: any balance-increasing call (donation, `add_rewards`, direct transfer) must be unable to manufacture insolvency or feed a self-liquidation. We have no `donateToReserves` and `add_rewards` is REVENUE-role-gated, but the *principle* — bound self-liquidation and never let a donation move HF inputs — is why our `cash`-based accounting (not `token.balance`) and `liquidator != owner` check matter. Adds the self-liquidation-via-donation mechanic the catalog otherwise lacked.
- **Sonne ($20M):** a Compound-v2 fork; the exploit was a classic empty-market first-depositor share-inflation, but delivered by *queuing the malicious new-market proposal through the timelock and waiting out the delay*. → Lesson: **a timelock does not save you if the queued action itself is the exploit.** Directly relevant since we lean on timelocks as a primary governance defense — new-market/parameter proposals must be reviewable *as queued*, and markets must never go live empty (seed deposit / virtual offset).
- **UwU Lend ($19.3M):** manipulated an oracle that read a Curve pool whose spot the attacker moved with flash loans, within the averaging window. → Lesson: **TWAP windows short enough to be flash-manipulated aren't protection** — a sharp caveat against over-trusting our Reflector TWAP read mode; pair TWAP with deviation bounds + a depth/liquidity gate.
- **Radiant ($53M, Oct 2024):** not a contract bug — a key-management/operational compromise where hardware-wallet signers approved a disguised ownership transfer. → Lesson: **operational key management and what signers can actually verify on-device is in audit scope.** Maps onto our keeper signer + the two-phase ownership-transfer surface (the keeper must use a KEEPER-scoped key, never owner; ownership transfers should be human-verifiable, not blind-signed).
