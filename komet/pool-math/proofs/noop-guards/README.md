# No-op guards: proved on the server

Feature `noop-guards` of the pool-math harness calls the production
`common::rates` helpers and checks that their zero-input guards return the
input unchanged, for **every** symbolic `i128` input:

| Claim | Statement | Result |
|---|---|---|
| `test_supply_zero_inputs(supplied, old, rewards)` | `supplied == 0 \|\| rewards == 0` implies `update_supply_index(supplied, old, rewards) == old` | `PASSED`, 7 nodes, 1,074 rewrite steps, 91 s |
| `test_compound_zero_delta(rate)` | `compound_interest(rate, 0) == RAY` | `PASSED`, 3 nodes, 463 steps, 318 s |
| `test_supply_zero_inputs_wrong()` (false) | zero supply grows the index to `RAY + 1` | `FAILED` at the completed `false` assertion |
| `test_compound_zero_delta_wrong()` (false) | `compound_interest(1, 0) == RAY + 1` | `FAILED` at the completed `false` assertion |

Not packed: `test_wad_try_mul_negative` (a negative operand makes
`Wad::try_mul` return `None`) ended `blocked`. Z3 returned unknown on a
path condition over the 64-bit words of the two `i128` operands. Its false
control failed as required, but it is left out until the positive claim
proves.

Toolchain: [`komet/pipeline`](../../../pipeline/README.md) pins, extra module
[`pool-math-lemmas.k`](../../../pipeline/proofs/pool-math-lemmas.k) (every rule
Lean-backed; see [`TRUSTED-BASE.md`](../../../pipeline/TRUSTED-BASE.md)).
Records, checks and compressed graphs: [`server/`](server/summary.json).
