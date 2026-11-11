# Borrow-index identity and cap: proved on the server

Feature `index-identity` of the pool-math harness calls the production
`common::rates::update_borrow_index` with a symbolic `i128` argument:

| Claim | Statement | Result |
|---|---|---|
| `test_borrow_index_identity(old)` | for every `old` in `[RAY, MAX_BORROW_INDEX_RAY]`: `update_borrow_index(old, RAY) == old` | `PASSED`, 35 nodes, 5,217 rewrite steps, 1,798 s |
| `test_borrow_cap_sticky(factor)` | for every `factor` in `(RAY, 100*RAY]`: `update_borrow_index(MAX_BORROW_INDEX_RAY, factor) == MAX_BORROW_INDEX_RAY` | `PASSED`, 31 nodes, 5,121 steps, 1,343 s |
| `test_borrow_index_identity_wrong()` (false) | identity growth rounds `RAY + 1` down to `RAY` | `FAILED` at the completed `false` assertion |
| `test_borrow_cap_sticky_wrong()` (false) | doubling at the cap gives `2 * MAX_BORROW_INDEX_RAY` | `FAILED` at the completed `false` assertion |

The cap claim stops at `100*RAY` so that the product before the cap still fits
in an `i128` (10^38 < `i128::MAX`).

Toolchain: [`komet/pipeline`](../../../pipeline/README.md) pins, extra module
[`pool-math-lemmas.k`](../../../pipeline/proofs/pool-math-lemmas.k) (every rule
Lean-backed; see [`TRUSTED-BASE.md`](../../../pipeline/TRUSTED-BASE.md)).
Records, checks and compressed graphs: [`server/`](server/summary.json).
