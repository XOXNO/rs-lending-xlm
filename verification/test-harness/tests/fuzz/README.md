# Property tests (proptest)

Randomized contract scenarios over full controller + pool WASM via `LendingTest`. Each property names an invariant; failures shrink to minimal cases stored in `*.proptest-regressions`.

Complements `verification/fuzz/` (libFuzzer byte-mutation campaigns). Proptest runs are deterministic, CI-friendly, and replay committed regressions automatically.

## Run

```bash
make proptest PROPTEST_CASES=256
make proptest-one TEST=prop_accounting_conservation PROPTEST_CASES=1000

PROPTEST_CASES=10000 cargo test --release -p test-harness --test fuzz -- --test-threads=1
cargo test -p test-harness --test fuzz prop_liquidation_matches_bigrational_reference -- --test-threads=1
```

`PROPTEST_CASES` overrides per-module defaults (see `config.rs`). Always use `--test-threads=1`.

## Modules

| File | Role |
|------|------|
| `config.rs` | `PROPTEST_CASES` env wiring |
| `ops.rs` | Shared `LendingOp` alphabet and index capture for conservation |
| `strategy_helpers.rs` | Router allowance and flash-loan guard checks |
| `accounting_conservation.rs` | Pool accounting laws + index monotonicity |
| `privileged_auth_rejects.rs` | Owner and role auth gates |
| `strategy_multiply_budget.rs` | `multiply` under Soroban budget limits |
| `strategy_router_invariants.rs` | HF, allowance, swap payload guards |
| `adversarial_aggregator.rs` | Zero-output aggregator rejection |
| `liquidation_vs_reference.rs` | Liquidation vs `BigRational` reference |

## Properties

| Property | Asserts | Catches |
|----------|---------|---------|
| `prop_accounting_conservation` | After random supply/borrow/repay/withdraw/advance/claim sequences: solvency inequality, supply/borrow/revenue conservation (±4 units), non-negative reserves, monotonic supply/borrow indexes | Accounting drift, revenue skim, index regression |
| `prop_owner_only_endpoints_reject_unauthed` | Privileged controller endpoints reject calls with no mocked auth | Missing `only_owner` / `only_role` gates |
| `prop_wrong_role_rejected` | Caller with one role cannot invoke endpoints gated on another | Cross-role privilege escalation |
| `prop_strategy_under_budget` | `multiply` either succeeds with live HF, fails cleanly, or panics only with budget-related messages | Soroban budget blow-up, opaque panics |
| `prop_multiply_leverage_hf_safe` | Successful multiply: HF ≥ 1 WAD, zero ETH router allowance, flash guard cleared; failed multiply leaves no dangling account | Strategy HF regression, allowance leak |
| `prop_strategy_swap_collateral_balance_delta` | Empty swap payload rejected; valid payload yields USDT collateral and zero allowance | Router payload validation gaps |
| `prop_short_aggregator_rejected` | Aggregator that transfers input but returns zero output is rejected; flash guard cleared | Swap output verification bypass |
| `prop_liquidation_matches_bigrational_reference` | Production liquidation USD and per-asset seizure match `reference::compute_liquidation` within documented ULP bounds (bad-debt scenarios filtered out) | Liquidation math drift |

Flash-loan repayment with strict per-call auth is covered by deterministic tests in `tests/controller/flash_loan.rs`.

## Operation alphabet (`ops.rs`)

Weighted random sequences over two users (Alice, Bob) and three assets (USDC, ETH, WBTC):

- **Supply** — raw amount
- **Borrow** — small fraction of seeded collateral
- **Repay / Withdraw** — fraction of current balance (bps)
- **Advance** — time jump + keeper index sync
- **ClaimRevenue** — admin revenue claim per asset

Properties execute ops via `try_*` and assert invariants after every step regardless of success or failure.