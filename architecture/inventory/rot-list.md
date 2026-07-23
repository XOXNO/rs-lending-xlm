# Doc rot grep (P0)

Known mismatches to erase during alignment (code/tree is truth):

| Pattern | Where | Fix |
|---------|-------|-----|
| Truncated `/// position mutations…` | `contracts/pool/src/lib.rs` supply/borrow/withdraw leads | Full STYLE opening line |
| `xoxno-oracle-adapter` path/name | README, ADR 0003, SCF, SECURITY, keeper comments, harness helpers | `xoxno-oracle` / `contracts/xoxno-oracle` |
| `contracts/aggregator` | ADR 0005, SCF | `contracts/swap-aggregator` |
| Stale HF `1.02` / knee `0.51` in **product** comments | Prefer DEFAULT_* in controller/docs; harness test comments out of scope unless product-facing | Align or delete |
| Interface methods with only inherited trait blurbs | `interfaces/*` | Per-method STYLE docs |

Inventory artifact: `/opt/cursor/artifacts/protocol-docs/endpoint-inventory.md` (120 interface methods).
