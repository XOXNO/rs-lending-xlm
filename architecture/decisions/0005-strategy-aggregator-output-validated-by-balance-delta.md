# ADR 0005: Validate Strategy Aggregator Output by Balance Delta

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

The protocol exposes leveraged and rebalancing strategies (`multiply`,
`swap_collateral`, `swap_debt`, `repay_debt_with_collateral`) that route
through an external aggregator router. The aggregator accepts an
off-chain-built `AggregatorSwap` shape (multi-path, multi-hop, per-path
PPM splits) and is expected to produce at least `total_min_out` of the
output token in exchange for at most `total_in` of the input token.

Two failure modes have to be defended against without trusting the
router's bookkeeping:

1. The router pulls more input than committed.
2. The router returns less output than committed (or returns the wrong
   token entirely).

A naive design accepts the router's reported numbers. A more defensive
design treats the router as untrusted at the boundary and verifies the
controller's own token balances before and after the call.

## Decision

The controller measures what crossed the router boundary instead of trusting
router-reported amounts.

**Pre-call shape validation** (`contracts/controller/src/strategies/helpers.rs::validate_aggregator_swap`).
`amount_in` is not a field of `AggregatorSwap` (whose only scalar field is
`total_min_out`); it is the strategy's own measured leg delta — the withdrawn
or borrowed amount — passed into `swap_tokens` and bound to `BatchSwap.total_in`:

- `paths` is non-empty (else `InvalidPayments`).
- `amount_in > 0` and `total_min_out > 0` (else `AmountMustBePositive`).
- Each path has non-empty `hops` and `split_ppm > 0` (else `InvalidPayments`).
- `sum(split_ppm) == 1_000_000` (else `InvalidPayments`).
- First-hop `token_in` equals the strategy's input token; last-hop
  `token_out` equals the strategy's output token, on every path (else
  `WrongToken`).

**Pre-call balance snapshot** (`snapshot_swap_balances`): the controller
records its own balances of both the input and output tokens.

**On-call binding**: the on-the-wire `BatchSwap.sender` is forced to the
controller; `total_in` is the controller's committed input amount;
`referral_id = 0` disables router-side fee paths. The controller
pre-authorizes a single input-token pull from itself.

**Post-call delta verification**:

- `verify_router_input_spend` rejects any post-call input spend exceeding
  the committed `amount_in`. Underspend stays on the controller.
- `verify_router_output` rejects when the post-call output delta is
  below `total_min_out`.

Both delta checks currently surface their rejection as the generic
`GenericError::InternalError`.

**Reentrancy**: the router call runs inside the flash-loan single-flight
guard. `call_router_with_reentrancy_guard` sets `FlashLoanOngoing`
(`storage::set_flash_loan_ongoing`) for the duration of the call and clears it
only if it was not already set (`previously_set`), so a swap nested inside an
outer flash-loan flow keeps the guard live. Every position, strategy, and
router flow calls `require_not_flash_loaning`
(`contracts/controller/src/validation.rs`) on entry, so any such controller
mutation entered from the router callback path reverts with
`FlashLoanError::FlashLoanOngoing`. (Owner-/role-gated admin entrypoints —
config, access control, upgrades — and the owner-authenticated `renew_account`
TTL entrypoint are not on this guard.)

## Alternatives Considered

- **Trust router-reported amounts.** Rejected: a router bug or future
  ABI drift could silently misreport. Balance deltas are a check the
  router cannot lie about.
- **Per-hop on-chain validation.** Rejected: duplicates the router's job
  and fights the aggregator design. The controller does not need
  per-hop visibility, only the aggregate input and output deltas.
- **Quote-driven `amount_in` from off-chain.** Rejected: the controller
  uses its own withdrawal/borrow delta as `amount_in`, not the
  off-chain quote. Quote drift (off-chain price moved between quote
  and execution) is therefore irrelevant; the only post-condition is
  `total_min_out`.

## Consequences

Positive:

- The router is a black box: any output-token discrepancy surfaces as a
  controller-side revert with a clean error site.
- Strategy entrypoints share the same risk model as `supply` / `borrow` /
  `repay` because the only thing they trust about the router is the
  measured token delta.
- Reentry from a malicious router callback is blocked by the flash-loan
  single-flight flag.

Negative / accepted costs:

- Four `token.balance` reads per swap: both sides are snapshotted before the
  router call (`snapshot_swap_balances`) and both re-read after — input via
  `verify_router_input_spend`, output via `verify_router_output`. A `multiply`
  with a cross-token initial payment runs two swaps, doubling this.
- Off-chain integrators must build `AggregatorSwap` shapes that match
  the strategy's input and output tokens exactly; mismatches revert
  with `InvalidPayments` or `WrongToken`.

## References

- `SCF_BUILD_ARCHITECTURE.md` §11.1 (Strategies).
- `contracts/controller/src/strategies/helpers.rs::validate_aggregator_swap`
- `contracts/controller/src/strategies/helpers.rs::snapshot_swap_balances`
- `contracts/controller/src/strategies/helpers.rs::verify_router_input_spend`
- `contracts/controller/src/strategies/helpers.rs::verify_router_output`
- `contracts/controller/src/storage/instance.rs::set_flash_loan_ongoing`
- `contracts/controller/src/validation.rs::require_not_flash_loaning`
- `common/src/types/aggregator.rs` (`AggregatorSwap`, `BatchSwap`)
