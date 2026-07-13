# ADR 0005: Validate Strategy Aggregator Output by Balance Delta

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

The protocol exposes leveraged and rebalancing strategies (`multiply`,
`swap_collateral`, `swap_debt`, `repay_debt_with_collateral`) that route
through an external aggregator router. The aggregator accepts an
off-chain-built opaque swap payload and is expected to enforce endpoint tokens,
slippage, referrals, and route execution in exchange for at most `total_in` of
the input token.

Two failure modes have to be defended against without trusting the
router's bookkeeping:

1. The router pulls more input than committed.
2. The router returns no output, sends the wrong token, or misreports output.

A naive design accepts the router's reported numbers. A more defensive
design treats the router as untrusted at the boundary and verifies the
controller's own token balances before and after the call.

## Decision

The controller measures what crossed the router boundary instead of trusting
router-reported amounts.

**Pre-call strategy-bound validation** (`contracts/controller/src/strategies/swap/route.rs::validate_strategy_swap`).
`StrategySwap` is opaque `Bytes`; `amount_in` is the strategy's own
measured leg delta, the withdrawn or borrowed amount, passed into
`swap_tokens` and bound to `execute_strategy.total_in`. The controller
validates:

- `amount_in > 0` (else `AmountMustBePositive`).
- swap bytes are non-empty (else `InvalidPayments`).
- `token_in` and `token_out` are the strategy-known assets selected by the
  lending flow, not values decoded from the swap payload.

The route itself is opaque `Bytes` owned by the aggregator/router ABI. The
controller does not decode or validate paths, hops, pools, venues, or splits.

**Pre-call balance snapshot** (`contracts/controller/src/strategies/swap/balances.rs::snapshot_swap_balances`): the controller
records its own balances of both the input and output tokens.

**On-call binding**: the controller passes itself as `sender` and its own
measured `amount_in` as `total_in`. The aggregator requires auth from that
sender and can pull at most that amount (enforced on the controller side by
pre-authorization of exactly `amount_in` plus post-call overspend check). The
opaque swap XDR is forwarded unchanged; the controller never decodes routes.

**Post-call delta verification** (in `balances.rs`):

- `verify_router_input_spend` rejects if the router spent more input than the
  committed `amount_in` (overspend protection).
- `refund_router_underspend` refunds any unspent input back to the provided
  refund recipient (leftover stays with the caller; does not stay trapped in
  the router).
- `verify_router_output` computes the output delta and rejects if it is not
  strictly positive (`NoSwapOutput`).

These checks use strategy-specific errors (`StrategyError::RouterOverspend`,
`StrategyError::NoSwapOutput`) rather than a generic internal error.

**Reentrancy**: the router call runs inside the flash-loan single-flight
guard. `call_router_with_reentrancy_guard` sets `FlashLoanOngoing`
(`storage::set_flash_loan_ongoing`) for the duration of the call and clears it
only if it was not already set (`previously_set`), so a swap nested inside an
outer flash-loan flow keeps the guard live. Every position, strategy, and
router flow calls `require_not_flash_loaning`
(`contracts/controller/src/risk/validation.rs`) on entry, so any such controller
mutation entered from the router callback path reverts with
`FlashLoanError::FlashLoanOngoing`. Governance/controller admin entrypoints
and the owner-authenticated `renew_account` TTL entrypoint are not on this
guard.

## Alternatives Considered

- **Trust router-reported amounts.** Rejected: a router bug or future
  ABI drift could misreport. Balance deltas are a check the
  router cannot lie about.
- **Per-hop on-chain validation.** Rejected: duplicates the router's job
  and fights the aggregator design. The controller does not need per-hop
  visibility, only the aggregate input and output deltas.
- **Quote-driven `amount_in` from off-chain.** Rejected: the controller
  uses its own withdrawal/borrow delta as `amount_in`, not the
  off-chain quote. Quote drift (off-chain price moved between quote
  and execution) is therefore irrelevant to input authorization; slippage is
  enforced by the aggregator payload.

## Consequences

Positive:

- The router is a black box: any output-token discrepancy surfaces as a
  controller-side revert with a clean error site.
- Strategy entrypoints share the same risk model as `supply` / `borrow` /
  `repay` because they trust the measured token delta, not router bookkeeping.
- Reentry from a malicious router callback is blocked by the flash-loan
  single-flight flag.

Negative / accepted costs:

- Four `token.balance` reads per swap: both sides are snapshotted before the
  router call (`snapshot_swap_balances`) and both re-read after: input via
  `verify_router_input_spend`, output via `verify_router_output`. A `multiply`
  with a cross-token initial payment runs two swaps, doubling this.
- Off-chain integrators must build route bytes that the aggregator can decode.
  Lending will not reject malformed route internals before the router call; it
  enforces the strategy-owned input amount, selected assets, and positive
  output delta.

## References

- `SCF_BUILD_ARCHITECTURE.md` §12 (Strategies).
- `contracts/controller/src/strategies/swap/route.rs` (validate + call with reentrancy guard)
- `contracts/controller/src/strategies/swap/balances.rs` (snapshot + verify_input_spend + refund_underspend + verify_output)
- `contracts/controller/src/strategies/swap/auth.rs` (pre-authorize exactly the committed input)
- `contracts/controller/src/storage/instance.rs::set_flash_loan_ongoing`
- `contracts/controller/src/risk/validation.rs::require_not_flash_loaning`
- `common/src/errors.rs` (`StrategyError::RouterOverspend`, `StrategyError::NoSwapOutput`)
- `common/src/types/aggregator.rs` (`StrategySwap`)
- Aggregator side (`contracts/aggregator/src/vault.rs`): the router itself tracks real balance deltas internally using an invocation-local vault; the controller performs an independent outer verification on its own token balances.

The decision (treat router output as untrusted and verify controller-measured deltas) remains the implemented approach.
