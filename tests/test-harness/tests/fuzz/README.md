# Property tests (proptest)

Randomized contract scenarios over the full controller + pool stack via `LendingTest`. Each property names an invariant; failures shrink to minimal cases stored in `*.proptest-regressions` (the two seed-adjusted cash properties persist none).

Complements `tests/fuzz/` (libFuzzer byte-mutation campaigns). Proptest explores a fresh seed on each run; failures are reproducible from the reported and persisted seed, shrink to a minimal case, and replay automatically from committed regressions.

## Run

```bash
make proptest                          # use each property's tuned default
make proptest PROPTEST_CASES=256       # override every property for a deep run
make proptest-one TEST=prop_accounting_conservation PROPTEST_CASES=1000

PROPTEST_CASES=10000 cargo test --release -p test-harness --test fuzz -- --test-threads=1
cargo test -p test-harness --test fuzz prop_liquidation_matches_bigrational_reference -- --test-threads=1
```

`PROPTEST_CASES` overrides the per-property defaults of the randomized properties
(see `config.rs`); the deterministic `#[test]` functions, such as the two auth
matrices, ignore it.

Run this suite serially. `make proptest` pins `--test-threads=1`; the rest of the
harness suite runs at libtest's default of one thread per core.

## Modules

| File | Role |
|------|------|
| `config.rs` | Optional `PROPTEST_CASES` override and regression-file persistence |
| `ops.rs` | Shared `LendingOp` alphabet and index capture for conservation |
| `strategy_helpers.rs` | Router allowance and flash-loan guard checks |
| `accounting_conservation.rs` | Pool accounting laws, index monotonicity, seed-adjusted cash conservation and token custody |
| `migrate_blend.rs` | MockBlend `migrate_from_blend` reconciliation + generated and deterministic rejects |
| `privileged_auth_rejects.rs` | Deterministic owner and role auth matrices |
| `strategy_multiply_budget.rs` | `multiply` under Soroban budget limits |
| `strategy_router_invariants.rs` | HF, allowance, swap payload guards |
| `liquidation_vs_reference.rs` | Liquidation vs `BigRational` reference |
| `whole_unit_liquidation.rs` | Whole-unit liquidation of a collateral leg below 3 decimals vs the documented rules |

## Properties

| Property | Asserts | Catches |
|----------|---------|---------|
| `prop_accounting_conservation` | After each op of a random `ops.rs` sequence: solvency inequality, supply/borrow/revenue conservation (±4 units), non-negative reserves, monotonic indexes, cleared flash guard, zero router allowance, controller leftover ≤ 4 units | Accounting drift, revenue skim, index regression, strategy residue, guard leak |
| `prop_seed_adjusted_cash_conservation_and_token_custody` | After each op of a random `ops.rs` sequence: the pool token balance equals its accounting cash, and the seed-adjusted surplus stays between −1e9 raw RAY and 8 raw token units per op | Cash leak, over-collection, custody drift |
| `prop_seed_adjusted_cash_conservation_through_liquidation_and_bad_debt` | Open, accrue, crash the USDC price, attempt one to three liquidations, claim revenue: the same custody and surplus bounds hold after each step | Cash leak through liquidation or bad-debt socialization |
| `prop_migrate_blend_reconciles_same_asset` | Random USDC coll/supply/debt + cap buffer, new or existing hub account: Blend slots empty, hub supply and debt grow by the Blend positions (debt, not the cap) within 4 units, HF ≥ 1, controller leftover ≤ 4, flash guard cleared | Refund/cap confusion, leftover dust, unregistered new account |
| `prop_migrate_blend_cap_too_low_reverts` | Cap below Blend liability reverts (mock health `#1`) with no hub leftover and Blend position intact | Partial-repay-then-sweep hole |
| `migrate_blend_rejects_empty_duplicate_unapproved_zero_cap` | Empty lists, a duplicate cap, a zero cap and an unapproved Blend pool each revert with their own error | Missing migration input checks |
| `owner_only_endpoints_reject_unauthed_before_validation` | A call matrix over the owner-only and caller-auth controller endpoints rejects each unauthenticated call before argument validation | Missing `only_owner` / caller auth gates (governance owns controller) |
| `governance_endpoints_reject_unauthed_before_validation` | Governance `propose` (proposer role), `deploy_controller` (owner) and `pause` (guardian) reject unauthenticated calls | Missing proposer, owner or guardian gates |
| `prop_valid_multiply_fits_default_budget` | A valid `multiply` stays within the default Soroban CPU (100M instructions) and memory (40 MiB) budget, ends with HF ≥ 1 WAD, zero router allowance and a cleared flash guard | Strategy budget regression |
| `prop_multiply_succeeds_with_safe_hf_and_clean_router` | Valid multiply inputs always succeed with HF ≥ 1 WAD, zero router allowance, and a cleared flash guard | Strategy HF regression, allowance or guard leak |
| `prop_swap_collateral_conserves_position_delta` | Successful stablecoin swaps debit and credit the exact raw position amounts | Router or accounting delta drift |
| `empty_swap_payload_reverts_without_state_or_guard_leak` | Empty swap bytes reject atomically | Payload validation and rollback gaps |
| `prop_liquidation_matches_bigrational_reference` | Every generated account is liquidatable and in differential scope; liquidation in `SeizeMode::Transfer` or `SeizeMode::Credit` succeeds and matches `reference::compute_liquidation` (repaid, seized, protocol fee) within the ULP bounds | Liquidation math drift and silently skipped cases |
| `prop_below_base_liquidation_matches_reference_and_never_loses_the_liquidator_money` | Band (`D <= C < D * (1 + base)`) and insolvent accounts, offers up to 1.5x the debt: bonus within 1 bps and repayment within one debt-token unit of the reference, insolvent pull capped at the collateral-backed quote, liquidator receives at least what it spends (within 2 raw USDC units), a band partial never lowers coverage | Band and insolvent quote drift, liquidator loss |
| `below_base_differential_holds_at_exact_cover` | The same checks at collateral equal to debt, with offers of 5%, 100% and 150% of the debt | Exact-cover boundary |
| `prop_whole_unit_liquidation_holds_the_documented_bounds` | A 0-, 1- or 2-decimal collateral leg ($0.50 to $100,000 a unit, 2 to 50,000 units, LT 25% to 90%, random curve) borrows USDC and an optional 6- or 18-decimal leg. The price falls into an HF band, below cover, or onto the edge where one unit at `1 + b` equals the debt. Debt-sized liquidations of the residue follow. Each settled call moves whole units only (P1). The liquidator receives at most `paid * (1 + b)`, plus one unit when it repays all debt, or plus `(1 + b)` times one base unit per debt leg (P2). A plan that leaves debt charges at least `units * U / (1 + b)` less one base unit per debt leg (P3). `C / D` does not fall on a solvent account, within 1e-12 (P4). Debt falls, and no debt stays without collateral (P5). Pool units equal account units plus revenue (P6). Execution equals `get_liquidation_estimate` (P7). A debt-sized offer on a solvent account reverts only inside the margin band (P8). The run prints the outcome and revert counts | Whole-unit rounding, rule 1 and rule 2 drift, liquidator over-reward, estimate drift, liveness gaps outside the band |
| `wul_*` | Deterministic unit-edge cases: inside the margin band every offer reverts until one day of accrual; above it a debt-sized offer closes the debt for one unit; below it the offer sells one unit; a one-unit residue at cover closes at bonus 0; two units at the testnet LIQVID1039 listing stay healthy at the unit edge | Band, rule 1, rule 2 and residue boundaries |

Flash-loan repayment with strict per-call auth is covered by deterministic tests in `tests/controller/flash_loan.rs`.

## Operation alphabet (`ops.rs`)

Weighted random sequences over two users (Alice, Bob) and three assets (USDC, ETH, WBTC):

- **Supply** — 1 to 19,999 whole token units
- **Borrow** — 0.01 to 0.99 token units
- **Repay / Withdraw** — fraction of current balance (bps)
- **Advance** — time jump + keeper index sync
- **ClaimRevenue** — admin revenue claim per asset
- **Liquidate** — fraction of the target's ETH or WBTC debt (bps), after forcing
  the USDC price down to half a WAD
- **SwapDebt** — rotate a user's USDC debt into ETH through the mock router
- **FlashLoan** — flash loan to a receiver that repays
- **FlashPosition** — callback-multiply onto a new Multiply account
- **Multiply / SwapCollateral / Rdwc** — strategy legs through the mock router
- **MigrateBlend** — healthy same-asset USDC MockBlend position merged into the user's already-tracked default account (coll-only or coll+debt with a 20% cap buffer)
- **Recapitalize** — LIQUIDATOR donates to a market; only the measured shortfall is applied and the rest is refunded
- **UpdateAccountThreshold** — keeper restamp of the user's default account
- **AddDelegate / RemoveDelegate** — CAROL added to or removed from the user's default account as a delegate
- **NftTransfer** — the default account NFT moves to the other user
- **Script** — a script-runner contract opens an account, runs 2 to 5 legs, then repays and withdraws everything in one invocation; its wallet never ends richer

Properties execute ops via `try_*` and assert invariants after every step regardless of success or failure. The seed book starts with live debt for both users (Alice ETH, Bob USDC) so repay, revenue, liquidation, and debt-swap operations do not depend on a lucky earlier borrow.
