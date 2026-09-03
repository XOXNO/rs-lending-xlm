# Preliminary synthesis (wave 1 + deep themes)

## Agent execution note

Async subagents are capped at **10 concurrent**. Wave-1 explore agents also
forked private `feat/a0xx-*` branches; findings were consolidated onto
`feat/controller-defense-audit-1735`. Later waves must follow
`shared/COORDINATION.md` (no git branch ops).

## Coverage so far

Findings present under `findings/` for A001–A020 (auth/entry), plus deep
dives A041, A055 (money), A076–A077, A080, A082, A084 (spoke usage), A086,
A094, A099 (cache/opts). Remaining A021–A075, A078–A079, A081, A083, A085,
A087–A093, A095–A098, A100–A110 continue in subsequent waves.

## Defenses that look strong

1. Pause matrix matches INV-HALT-01 (A001).
2. Owner-or-delegate + third-party supply slot rule (A003, A012).
3. Flash reentrancy guard on monetary reentry (A007, A019).
4. Measured token receipts at controller custody boundary (A041, A016, A082).
5. Spoke usage deltas from pool `PoolPositionMutation` indexes/amounts (A077).
6. Flag ratchet / guardian tighten-only (A006).
7. Cache memoization with spoke pin + post-leg index refresh (A086, A094).

## Leading residuals (undefended / partial) with impact

| ID | Issue | Impact quantification |
|---|---|---|
| A080 | `apply_exit` no-op if usage row missing | Spoke caps can under-count → temporary over-admission up to that spoke's cap headroom; no direct theft; supplier risk only if over-admission later goes bad |
| A055 | Non-SAC / rebasing tokens if listed | Market-wide desync → bad debt socialized to that market's suppliers (≤ market TVL) |
| A009 | Owner must be timelock (deploy) | If mis-wired, instant admin = full parameter/upgrade compromise |
| A094 | Future leg forgetting `put_market_index` | Wrong HF/caps within a tx; footgun for new code |
| A048/A056 | Controller has no quantitative `min_out` on strategy swaps (only `received > 0`; min-out lives in opaque aggregator payload) | Router compromise can drain swapped notional up to post-swap solvency; known aggregator trust-root gap |
| A062/A015 | No hard length cap on mutator payment Vecs / keeper asset lists (views use 256) | Fee-funded compute DoS only; money paths still aggregate/limit positions |
| Threat-model | Aggregator + XOXNO oracle standalone owners | Immediate upgrade/sweep/price authority outside governance |

## Next waves

- Storage mutation maps A021–A040
- Money movement completeness A042–A054, A056–A060
- Validation A061–A075
- Spoke usage remaining A078–A085
- Cache remaining A087–A100
- Gap synthesis A101–A110
