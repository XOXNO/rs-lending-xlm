# Preliminary synthesis — superseded

**This file is a mid-wave snapshot.** The authoritative shared report is
[`FINAL.md`](FINAL.md).

Keep this document only as history of ranking during incomplete coverage.
Do not treat the “Remaining A021–…” paragraph or the Wave-6 hole lists in
A101–A107 as current.

Historical leading residuals (confirmed by FINAL / A101–A110):

| ID | Issue | Impact quantification |
|---|---|---|
| A080 | `apply_exit` no-op if usage row missing | Spoke caps can under-count → temporary over-admission up to that spoke's cap headroom; no direct theft |
| A055 | Non-SAC / rebasing tokens if listed | Market-wide desync → bad debt socialized to that market's suppliers (≤ market TVL) |
| A009 | Owner must be timelock (deploy) | If mis-wired, instant admin = full parameter/upgrade compromise |
| A094 | Future leg forgetting `put_market_index` | Wrong HF/caps within a tx; footgun for new code |
| A048/A056 | No quantitative `min_out` on strategy swaps | Router compromise can drain swapped notional up to post-swap solvency |
| A064 | `no_seize` not coupled to `frozen` | Can strand liquidations until force_socialize |
| A062/A015 | No hard length cap on mutator/keeper Vecs | Fee-funded compute DoS only |
