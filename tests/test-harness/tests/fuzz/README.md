# Property tests (proptest)

Randomized scenarios over the full controller + pool stack via `LendingTest`; complements the [libFuzzer campaigns](../../../fuzz/README.md). Each property names an invariant. Each run uses a fresh seed; failures reproduce from the reported seed and shrink. Minimized seeds are committed (`*.proptest-regressions` here, `../proptest-regressions/*.txt`) and replay every run; the two seed-adjusted cash properties persist none.

## Run

```bash
make proptest                          # use each property's tuned default
make proptest PROPTEST_CASES=256       # override every property for a deep run
make proptest-one TEST=prop_accounting_conservation PROPTEST_CASES=1000

PROPTEST_CASES=10000 cargo test --release -p test-harness --test fuzz -- --test-threads=1
cargo test -p test-harness --test fuzz prop_liquidation_matches_bigrational_reference -- --test-threads=1
```

`PROPTEST_CASES` overrides the per-property defaults in `config.rs`; zero is rejected. Deterministic `#[test]` functions (e.g. the auth matrices) ignore it. `make proptest-one` rejects an empty or unmatched filter. Use release builds for long runs. `make proptest` runs one property at a time. Set `PROPTEST_THREADS=<n>` to run `n` properties at a time; the cases of one property always run in sequence.

## Modules

| File | Role |
|------|------|
| `config.rs` | Optional `PROPTEST_CASES` override; regression-file persistence |
| `ops.rs` | Shared `LendingOp` alphabet; index capture for conservation |
| `strategy_helpers.rs` | Router allowance and flash-loan guard checks |
| `accounting_conservation.rs` | Pool accounting laws, index monotonicity, seed-adjusted cash conservation, token custody |
| `migrate_blend.rs` | MockBlend `migrate_from_blend` reconciliation; generated and deterministic rejects |
| `privileged_auth_rejects.rs` | Deterministic owner and role auth matrices |
| `strategy_multiply_budget.rs` | `multiply` under Soroban budget limits |
| `strategy_router_invariants.rs` | HF, allowance, swap payload guards |
| `liquidation_vs_reference.rs` | Liquidation vs `BigRational` reference |
| `whole_unit_liquidation.rs` | Whole-unit liquidation of sub-3-decimal collateral vs documented rules |

## Properties

"Clean end state" below means HF ≥ 1 WAD, zero router allowance and a cleared flash guard.

| Property | Asserts |
|----------|---------|
| `prop_accounting_conservation` | After each random `ops.rs` op: solvency inequality, supply/borrow/revenue conservation (±4 units), non-negative reserves, monotonic indexes, cleared flash guard, zero router allowance, controller leftover ≤ 4 units |
| `prop_seed_adjusted_cash_conservation_and_token_custody` | After each random `ops.rs` op: pool token balance equals accounting cash; seed-adjusted surplus within −1e9 raw RAY to 8 raw token units per op |
| `prop_seed_adjusted_cash_conservation_through_liquidation_and_bad_debt` | Open, accrue, crash USDC, one to three liquidations, claim revenue: same bounds after each step, including bad-debt socialization |
| `prop_migrate_blend_reconciles_same_asset` | Random USDC coll/supply/debt + cap buffer, new or existing hub account: Blend slots empty; hub supply and debt grow by the Blend positions (debt, not cap) within 4 units; HF ≥ 1; controller leftover ≤ 4; flash guard cleared; new account registered |
| `prop_migrate_blend_cap_too_low_reverts` | Cap below Blend liability reverts (mock health `#1`), no hub leftover, Blend position intact (no partial-repay-then-sweep) |
| `migrate_blend_rejects_empty_duplicate_unapproved_zero_cap` | Empty lists, duplicate cap, zero cap and unapproved Blend pool each revert with their own error |
| `owner_only_endpoints_reject_unauthed_before_validation` | Owner-only (`only_owner`; governance owns controller) and caller-auth controller endpoints reject unauthenticated calls before argument validation |
| `governance_endpoints_reject_unauthed_before_validation` | Governance `propose` (proposer), `deploy_controller` (owner) and `pause` (guardian) reject unauthenticated calls |
| `prop_valid_multiply_fits_default_budget` | Valid `multiply` fits the default Soroban budget (100M CPU instructions, 40 MiB memory) with a clean end state |
| `prop_multiply_succeeds_with_safe_hf_and_clean_router` | Valid multiply inputs always succeed with a clean end state |
| `prop_swap_collateral_conserves_position_delta` | Successful stablecoin swaps debit and credit exact raw position amounts |
| `empty_swap_payload_reverts_without_state_or_guard_leak` | Empty swap bytes reject atomically |
| `prop_liquidation_matches_bigrational_reference` | Every generated account is liquidatable and in differential scope (none silently skipped); `SeizeMode::Transfer` or `SeizeMode::Credit` liquidation succeeds and matches `reference::compute_liquidation` (repaid, seized, protocol fee) within ULP bounds |
| `prop_below_base_liquidation_matches_reference_and_never_loses_the_liquidator_money` | [Below-base cases](#below-base-cases) |
| `below_base_differential_holds_at_exact_cover` | Same checks at collateral = debt, offering 5%, 100%, 150% of it |
| `prop_whole_unit_liquidation_holds_the_documented_bounds` | P1 to P8 in [Whole-unit liquidation](#whole-unit-liquidation), including liveness outside the band |
| `wul_*` | Deterministic band, rule 1, rule 2 and residue edges ([Whole-unit edge cases](#whole-unit-edge-cases)) |

Deterministic tests in `tests/controller/flash_loan.rs` cover flash-loan repayment under strict per-call auth.

### Below-base cases

Band (`D <= C < D * (1 + base)`) and insolvent accounts, offers up to 1.5x debt. Each case: bonus within 1 bps and repayment within one debt-token unit of the reference; insolvent pull capped at the collateral-backed quote; liquidator receives at least what it spends (within 2 raw USDC units); a band partial never lowers coverage.

### Whole-unit edge cases

In `whole_unit_liquidation.rs`; each asserts units, `U`, `D`, `b` and HF. The first three share one setup: two $509.79 units, $351.00 debt, bonus 45.24%, LT 26%.

- `wul_two_units_inside_the_margin_band_refuse_every_offer_until_accrual`: every offer reverts until one day of accrual.
- `wul_two_units_above_the_margin_band_close_in_full_for_one_unit`: a debt-sized offer closes the debt for one unit.
- `wul_two_units_below_the_margin_band_sell_one_unit`: the offer sells one unit.
- `wul_one_unit_residue_at_cover_closes_in_full_at_zero_bonus`: one-unit residue at cover closes at bonus 0.
- `wul_listed_liqvid_two_units_are_healthy_at_the_unit_edge`: two units at the testnet LIQVID1039 listing stay healthy.

## Whole-unit liquidation

Collateral: a 0-, 1- or 2-decimal leg, unit price $0.50 to $100,000, 2 to 50,000 units, LT 25% to 90%, random curve. Debt: USDC plus an optional 6- or 18-decimal leg. The price falls into an HF band, below cover, or to the edge where one unit at `1 + b` equals the debt; debt-sized liquidations of the residue follow. A reverting setup liquidation fails the case.

`b` is the bonus; `R` is one base unit per repaid leg. Tolerance
`t = price / 2e18 + (1 + b) + 1`: `seizure_usd / price` rounds half up in WAD,
so seizure can gain half a token wei (worth `price / 2e18`) and cross a
whole-unit floor. For a 2-decimal $100,000 unit, `t` ≈ 5e6 raw WAD.

| Id | Property |
| --- | --- |
| P1 | Settled calls move whole units only |
| P2 | Liquidator receives ≤ `paid * (1 + b)` + `t` raw WAD; a full-debt repay may get one more unit; an insolvent call paying within `R` of the collateral-backed quote gets every unit held, plus up to `(1 + b) * R` |
| P3 | Partial plan: liquidator overpays ≤ `2 * (1 + b) * R + t` versus received units at `1 + b`; the unseized-fraction refund returns all payment no whole unit backs, less the kept amount's round-up |
| P4 | `C / D` does not fall on a solvent account, within 1e-12 |
| P5 | Debt falls; no debt stays without collateral |
| P6 | Pool units = account units + revenue |
| P7 | Execution = `get_liquidation_estimate` |
| P8 | A debt-sized offer on a solvent account reverts only inside the margin band; each accepted revert is `InvalidPayments` with a settled, empty estimate, so its classifying bonus equals the estimate's `bonus_rate_bps` |

The run prints outcome, skip and revert counts, largest excess and overcharge per path, and largest relative `C / D` drop.

## Operation alphabet (`ops.rs`)

Weighted random sequences over Alice, Bob, USDC, ETH and WBTC:

- **Supply** — 1 to 19,999 whole token units
- **Borrow** — 0.01 to 0.99 token units
- **Repay / Withdraw** — fraction of current balance (bps)
- **Advance** — time jump + keeper index sync
- **ClaimRevenue** — admin revenue claim per asset
- **Liquidate** — fraction (bps) of the target's ETH or WBTC debt, after forcing USDC to half a WAD
- **SwapDebt** — USDC debt into ETH via the mock router
- **FlashLoan** — flash loan to a receiver that repays
- **FlashPosition** — callback-multiply onto a new Multiply account
- **Multiply / SwapCollateral / Rdwc** — strategy legs via the mock router
- **MigrateBlend** — healthy same-asset USDC MockBlend position merged into the user's tracked default account (coll-only, or coll+debt with a 20% cap buffer)
- **Recapitalize** — LIQUIDATOR donates; only the measured shortfall applies, the rest is refunded
- **UpdateAccountThreshold** — keeper restamp of the user's default account
- **AddDelegate / RemoveDelegate** — CAROL added to or removed as a delegate of the user's default account
- **NftTransfer** — the default account NFT moves to the other user
- **Script** — in one invocation, a script-runner contract opens an account, runs 2 to 5 legs, then repays and withdraws everything; its wallet never ends richer

Ops run via `try_*`; invariants are checked after every step, success or failure. The seed book starts with live debt (Alice ETH, Bob USDC), so repay, revenue, liquidation and debt-swap ops need no lucky earlier borrow.
