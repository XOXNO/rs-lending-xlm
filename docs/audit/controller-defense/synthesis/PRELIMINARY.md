# Preliminary synthesis — superseded

**This file is a mid-wave snapshot.** The authoritative shared report is
[`FINAL.md`](FINAL.md).

Keep this document only as history of ranking during incomplete coverage.
Do not treat the “Remaining A021–…” paragraph or the Wave-6 hole lists in
A101–A107 as current.

Historical leading residuals (confirmed by FINAL / A101–A110, then
**revalidated** in `RESIDUAL_REVALIDATION.md`):

| ID | Issue | Impact quantification | Revalidation |
|---|---|---|---|
| A080 | `apply_exit` no-op if usage row missing | Spoke caps can under-count → temporary over-admission | **WITHDRAWN** — not a live hole |
| A055 | Non-SAC / rebasing tokens if listed | Market-wide desync → bad debt ≤ market TVL | VALID — listing |
| A009 | Owner must be timelock (deploy) | If mis-wired, instant admin compromise | VALID — deploy |
| A094 | Future leg forgetting `put_market_index` | Wrong HF/caps within a tx; footgun | VALID — footgun only |
| A048/A056 | No quantitative `min_out` on strategy swaps | Router compromise can drain swapped notional | VALID — known design |
| A064 | `no_seize` not coupled to `frozen` | Can strand liquidations until force_socialize | VALID — ADR-0008 |
| A062/A015 | No hard length cap on mutator/keeper Vecs | Fee-funded compute DoS only | VALID — hygiene |
