# Property tests (proptest)

Randomized contract scenarios over full controller + pool WASM via `LendingTest`. Each property names an invariant; failures shrink to minimal cases stored in `*.proptest-regressions`.

Complements `tests/fuzz/` (libFuzzer byte-mutation campaigns). Proptest explores a fresh seed on each run; failures are reproducible from the reported and persisted seed, shrink to a minimal case, and replay automatically from committed regressions.

## Run

```bash
make proptest                          # use each property's tuned default
make proptest PROPTEST_CASES=256       # override every property for a deep run
make proptest-one TEST=prop_accounting_conservation PROPTEST_CASES=1000

PROPTEST_CASES=10000 cargo test --release -p test-harness --test fuzz -- --test-threads=1
cargo test -p test-harness --test fuzz prop_liquidation_matches_bigrational_reference -- --test-threads=1
```

`PROPTEST_CASES` overrides per-module defaults (see `config.rs`). Always use `--test-threads=1`.

## Modules

| File | Role |
|------|------|
| `config.rs` | Optional `PROPTEST_CASES` override wiring |
| `ops.rs` | Shared `LendingOp` alphabet and index capture for conservation |
| `strategy_helpers.rs` | Router allowance and flash-loan guard checks |
| `accounting_conservation.rs` | Pool accounting laws + index monotonicity |
| `privileged_auth_rejects.rs` | Deterministic owner and role auth matrices |
| `strategy_multiply_budget.rs` | `multiply` under Soroban budget limits |
| `strategy_router_invariants.rs` | HF, allowance, swap payload guards |
| `liquidation_vs_reference.rs` | Liquidation vs `BigRational` reference |

## Properties

| Property | Asserts | Catches |
|----------|---------|---------|
| `prop_accounting_conservation` | After random supply/borrow/repay/withdraw/advance/claim sequences: solvency inequality, supply/borrow/revenue conservation (±4 units), non-negative reserves, monotonic supply/borrow indexes | Accounting drift, revenue skim, index regression |
| `owner_only_endpoints_reject_unauthed_before_validation` | One exhaustive call matrix rejects every privileged controller endpoint before argument validation | Missing `only_owner` / caller auth gates |
| `governance_endpoints_reject_unauthed_before_validation` | Representative governance proposer and immediate-admin calls reject without auth | Missing proposer or owner gates |
| `prop_valid_multiply_fits_default_budget` | A valid multiply succeeds within a fresh default Soroban budget, remains healthy, and clears transient state | Strategy budget regression, vacuous invalid-mode coverage |
| `prop_multiply_succeeds_with_safe_hf_and_clean_router` | Valid multiply inputs always succeed with HF ≥ 1 WAD, zero router allowance, and a cleared flash guard | Strategy HF regression, allowance or guard leak |
| `prop_swap_collateral_conserves_position_delta` | Successful stablecoin swaps debit and credit the exact raw position amounts | Router or accounting delta drift |
| `empty_swap_payload_reverts_without_state_or_guard_leak` | Empty swap bytes reject atomically | Payload validation and rollback gaps |
| `prop_liquidation_matches_bigrational_reference` | Every generated account is liquidatable and in differential scope; production liquidation must succeed and match `reference::compute_liquidation` within documented ULP bounds | Liquidation math drift and silently skipped cases |

Flash-loan repayment with strict per-call auth is covered by deterministic tests in `tests/controller/flash_loan.rs`.

## Operation alphabet (`ops.rs`)

Weighted random sequences over two users (Alice, Bob) and three assets (USDC, ETH, WBTC):

- **Supply** — raw amount
- **Borrow** — small fraction of seeded collateral
- **Repay / Withdraw** — fraction of current balance (bps)
- **Advance** — time jump + keeper index sync
- **ClaimRevenue** — admin revenue claim per asset

Properties execute ops via `try_*` and assert invariants after every step regardless of success or failure. The seed book starts with live debt on both sides so repay, revenue, liquidation, and debt-swap operations do not depend on a lucky earlier borrow.
