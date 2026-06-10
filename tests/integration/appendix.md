## Appendix — local memory budget profile

Soroban does not expose memory-meter consumption via RPC or explorers; the
numbers below come from the in-repo budget harness
(`cargo test -p test-harness --test meta budget_ -- --nocapture`,
native mock oracles — real deployed-wasm oracles cost more per feed).
Testnet per-tx caps: **400M CPU instructions / 40MB memory**.

| operation | CPU instructions | memory bytes | dominant cost |
|---|---|---|---|
| supply (1 asset) | 6,402,383 | 1,967,715 | MemAlloc ~32% |
| withdraw (1 asset, no debt) | 7,789,156 | 2,014,329 | MemAlloc ~31% |
| borrow (HF valuation) | 9,976,847 | 3,621,253 | MemAlloc |
| withdraw (1 asset, with debt → HF) | 11,895,789 | 3,677,005 | MemAlloc |
| swap_collateral (withdraw+swap+deposit) | 16,473,192 | 4,126,252 | MemAlloc |
| withdraw (5 collateral + 1 debt, double LTV+HF pass) | 24,321,480 | 8,731,169 | MemAlloc ~1.3MB/feed |

Memory grows ~1.3MB per distinct oracle-priced position in an HF-checked op;
the memory meter (40MB), not CPU (400M), is the binding budget for
many-position transactions — consistent with the on-chain
`Budget,ExceededLimit` frontier measured by the stress flow.
