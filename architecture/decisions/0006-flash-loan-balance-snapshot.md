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
- Token contracts can be SAC-wrapped or SEP-41 with non-standard
  semantics (fee-on-transfer, rebasing). Trusting reported amounts from
  the token contract is risky.

## Decision

Settle flash loans by snapshotting the pool's own token balance on the
way out and verifying the post-repay balance against that snapshot.
Implementation in `pool/src/lib.rs::flash_loan_begin` and
`pool/src/lib.rs::flash_loan_end`, orchestrated by
`controller/src/flash_loan.rs::process_flash_loan`.

**Begin** (`flash_loan_begin`):

1. `verify_admin` (controller-only).
2. `interest::global_sync` then `cache.has_reserves(amount)`.
3. Snapshot the pool's token balance into instance storage at
   `FLASH_LOAN_PRE_BALANCE`.
4. Transfer `amount` to the receiver.

**Callback** (controller side): the controller invokes
`execute_flash_loan(initiator, asset, amount, fee, data)` on the
receiver. The receiver runs arbitrary logic and pre-authorizes the
pool's pull of `amount + fee`.

**End** (`flash_loan_end`):

1. `verify_admin`, non-negative fee (`NegativeFlashLoanFee` otherwise).
2. Pull `amount + fee` from receiver to the pool.
3. Read back `FLASH_LOAN_PRE_BALANCE`; missing snapshot reverts with
   `InvalidFlashloanRepay` (i.e., `end` cannot be called without `begin`).
4. Remove the snapshot key.
5. Assert `balance_after >= pre_balance + fee`. Below the floor reverts
   `InvalidFlashloanRepay`.
6. Record the fee as protocol revenue
   (`interest::add_protocol_revenue`).

**Controller-side guards** (`controller/src/flash_loan.rs`):

- `caller.require_auth()`.
- `require_market_active(asset)`, `is_flashloanable`, `amount > 0`.
- Set `FlashLoanOngoing = true` for the entire call window
  (single-flight reentrancy guard, also used by strategies).
- Compute `fee` from `flashloan_fee_bps` (capped at `MAX_FLASHLOAN_FEE_BPS`
  = 500 bps at listing time).

## Alternatives Considered

- **Internal-balance bookkeeping (EVM-style).** Rejected: requires
  trusting the token contract's reported amounts in `transfer`.
  Fee-on-transfer or rebasing tokens would silently break the invariant.
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

- The repayment floor is enforced by the pool's own observation of its
  token balance, which is the strongest available signal.
- The design transparently handles fee-on-transfer and other token
  oddities — over-credit is counted, under-credit reverts.
- The `FLASH_LOAN_PRE_BALANCE` key is removed at the end of `end`, so
  it cannot accumulate or leak between invocations.
- One single-flight guard covers both flash loans and strategies (ADR
  0005).

Negative / accepted costs:

- Two extra `token.balance` reads per flash loan.
- Receivers must implement `execute_flash_loan(initiator, asset, amount,
  fee, data)` exactly as specified and pre-authorize the pool's pull.
- The pool emits no fine-grained event from the snapshot; observers must
  rely on `FlashLoanEvent` from the controller.

## References

- `SCF_BUILD_ARCHITECTURE.md` §11.2 (Flash Loans).
- `pool/src/lib.rs::{flash_loan_begin, flash_loan_end}`
- `controller/src/flash_loan.rs::process_flash_loan`
- `common/src/constants.rs::MAX_FLASHLOAN_FEE_BPS`
