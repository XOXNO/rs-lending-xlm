# ADR 0006: Flash-Loan Settlement by Pool Balance Snapshot

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

A flash loan must guarantee that, by the end of the transaction, the pool
holds at least the principal plus the fee it had before the loan
started. EVM designs typically maintain bookkeeping (decrement an
internal balance, then check it back at the end). On Soroban the model
is different in ways that affect the safest implementation:

- The transaction is a single host invocation; any panic reverts the
  envelope.
- Authorization is explicit per `require_auth` and scoped to the
  invocation tree, so the receiver must pre-authorize the pool's pull
  during the callback rather than via persistent allowances.
- Listed assets use SAC-compatible token semantics. The pool still verifies
  its own token balance before and after the callback so repayment is proven
  by custody, not by a receiver return value.

## Decision

Settle flash loans by snapshotting the pool's own token balance on the
way out and verifying the post-repay balance against that snapshot.
Implementation in `contracts/pool/src/lib.rs::flash_loan`, orchestrated by
`contracts/controller/src/strategies/flash_loan.rs::process_flash_loan`.

**Pool execution** (`flash_loan`):

1. `#[only_owner]` (controller-only).
2. Reject negative fee (`NegativeFlashLoanFee` otherwise).
3. `interest::global_sync` then `cache.has_reserves(amount)`.
4. Snapshot the pool's token balance in a local variable.
5. Transfer `amount` to the receiver.
6. Invoke `execute_flash_loan(initiator, asset, amount, fee, pool, data)` on
   the receiver. `data` is opaque to the controller and pool.
7. Require the pool balance to still equal `pre_balance - amount`, rejecting
   push-style or wrong-asset settlement.
8. Pull `amount + fee` from the receiver with SAC `transfer_from`, where the
   pool is the spender approved by the receiver during the callback.
9. Assert `balance_after == pre_balance + fee`; any mismatch reverts with
   `InvalidFlashloanRepay`.
10. Record the fee as protocol revenue
   (`interest::add_protocol_revenue`).

**Controller-side guards** (`contracts/controller/src/strategies/flash_loan.rs`):

- `caller.require_auth()`.
- `require_market_active(asset)`, `is_flashloanable`, `amount > 0`.
- Require the receiver to be a deployed Wasm contract.
- Set `FlashLoanOngoing = true` for the entire call window
  (single-flight reentrancy guard, also used by strategies).
- Compute `fee` from `flashloan_fee_bps` (capped at `MAX_FLASHLOAN_FEE_BPS`
  = 500 bps at listing time).

## Alternatives Considered

- **Internal-balance bookkeeping (EVM-style).** Rejected: requires
  trusting the token contract's reported amounts in `transfer`.
  Fee-on-transfer or rebasing tokens would silently break the invariant.
- **Controller-owned callback with pool begin/end.** Rejected: it routes
  repayment through the controller and forces temporary pool storage even
  though the pool owns both funds and fee accounting.
- **Receiver returns repayment via callback return value.** Rejected:
  trusting a return value from arbitrary user code as the settlement
  signal collapses the threat model.
- **Persistent token allowance for the pool.** Rejected: doesn't fit
  Soroban's per-invocation auth model and creates a standing
  approval surface.
- **No protocol-level reentrancy guard, rely on per-call auth.** Rejected:
  the `FlashLoanOngoing` flag provides defense in depth and also
  protects strategy router calls.

## Consequences

Positive:

- The repayment invariant is enforced by the pool's own observation of its
  token balance, which is the strongest available signal.
- No controller token custody is needed in the flash-loan repayment path.
- No temporary pool storage key is needed; the snapshot is local to the pool
  invocation.
- One single-flight guard covers both flash loans and strategies (ADR
  0005).

Negative / accepted costs:

- A few extra `token.balance` reads per flash loan.
- Receivers must implement `execute_flash_loan(initiator, asset, amount,
  fee, pool, data)` exactly as specified and pre-authorize the pool's pull.
- The pool emits no fine-grained event from the local snapshot; observers must
  rely on `FlashLoanEvent` from the controller.

## References

- `SCF_BUILD_ARCHITECTURE.md` §11.2 (Flash Loans).
- `contracts/pool/src/lib.rs::flash_loan`
- `contracts/controller/src/strategies/flash_loan.rs::process_flash_loan`
- `common/src/constants/` (`MAX_FLASHLOAN_FEE_BPS`)
