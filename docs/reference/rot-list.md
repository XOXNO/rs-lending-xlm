# Doc rot checklist

Erase mismatches when code or the tree changes. Code is truth.

| Pattern | Where to check | Fix |
|---------|----------------|-----|
| Truncated `///` openings | `contracts/*/src` public mutators | Full opening line per [doc-style](./doc-style.md) |
| `xoxno-oracle-adapter` / bare `aggregator/` paths | Comments, mermaid, skills | `xoxno-oracle` / `swap-aggregator` |
| “Immediate pause/**unpause**” | Skills, ops notes | Pause may be immediate (GUARDIAN); **unpause is timelocked** |
| Missing `is_flashloanable` / `flashloan_fee` | Deploy configs, reading/flash skills | Fields live on `MarketParams` / `InterestRateModel` |
| Stale HF `1.02` / knee `0.51` in product docs | Prefer `DEFAULT_*` in controller | Align or delete |
| Interface methods with only inherited blurbs | `interfaces/*` | Per-method docs per doc-style |

Inventory: [endpoint-inventory.md](./endpoint-inventory.md).
