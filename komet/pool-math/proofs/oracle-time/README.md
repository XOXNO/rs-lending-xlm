# Oracle timestamp predicates: proved on the server

Feature `oracle-time` of the pool-math harness calls the production
`common::oracle::observation` predicates and compares them with independent
formulas over `u128`, for **every** `u64` input:

| Claim | Statement | Result |
|---|---|---|
| `test_stale_exact(now, feed, max)` | `is_stale(now, feed, max) == (now > feed + max)` | `PASSED`, 3 nodes, 540 rewrite steps, 233 s |
| `test_future_exact(now, feed)` | `is_future_at(now, feed) == (now + 60 <= u64::MAX && feed > now + 60)` | `PASSED`, 3 nodes, 428 steps, 225 s |
| `test_stale_wrong()` (false) | a feed exactly `max` old is stale | `FAILED` at the completed `false` assertion |
| `test_future_wrong()` (false) | a feed exactly at the skew boundary is in the future | `FAILED` at the completed `false` assertion |

Toolchain: [`komet/pipeline`](../../../pipeline/README.md) pins, extra module
[`pool-math-lemmas.k`](../../../pipeline/proofs/pool-math-lemmas.k) (every rule
Lean-backed; see [`TRUSTED-BASE.md`](../../../pipeline/TRUSTED-BASE.md)).
Records, checks and compressed graphs: [`server/`](server/summary.json).
