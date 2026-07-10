# ADR 0006: Flash-Loan Settlement by Pool Balance Snapshot

- Status: Accepted
- Date: 2026-05-05
- Deciders: XOXNO Lending contract team
- Supersedes: none

## Context

A flash loan must guarantee that, by the end of the transaction, the pool
holds at least the principal plus the fee it had before the loan
started. EVM designs maintain bookkeeping (decrement an
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

**Pool execution** (`flash_loan(initiator, receiver, amount, fee, data)`,
`contracts/pool/src/lib.rs`):

1. `#[only_owner]` (controller-only).
2. `require_positive_amount(amount)` then `require_nonneg_amount(fee)`; both
   reject with `GenericError::AmountMustBePositive`.
3. `load_synced_cache` (runs `renew_pool_instance` then
   `interest::global_sync`), then `cache.require_reserves(amount)` (rejects
   with `CollateralError::InsufficientLiquidity`).
4. `require_wasm_receiver(receiver)`: the pool re-checks the receiver is a
   deployed Wasm contract.
5. Snapshot `pre_balance = token.balance(pool)`; derive
   `expected_after_payout = pre_balance - amount` and
   `expected_after_repay = pre_balance + fee`.
6. Transfer `amount` to the receiver, then assert the balance equals
   `expected_after_payout` (`InvalidFlashloanRepay`) before the callback.
7. Invoke `execute_flash_loan(initiator, asset, amount, fee, pool, data)` on
   the receiver. `data` is opaque to the controller and pool.
8. Assert the balance *again* equals `expected_after_payout`
   (`InvalidFlashloanRepay`); the callback must not change the pool's balance
   before settlement.
9. Verify the receiver authorized at least `amount + fee` allowance for the
   pool, then call
   `transfer_from(spender=pool, from=receiver, to=pool, amount + fee)`. The pool
   is the direct contract invoker and therefore authenticates its own spender
   leg; the receiver authorizes the debit during the callback through the token
   allowance.
10. Assert `balance_after == expected_after_repay`; any mismatch reverts with
    `InvalidFlashloanRepay`.
11. Convert the fee to RAY (`Ray::from_asset(fee, asset_decimals)`), record it
    as protocol revenue (`interest::add_protocol_revenue_ray`), add the fee to
    the market's internal `cash`, and persist the asset state.

**Controller-side guards** (`contracts/controller/src/strategies/flash_loan.rs::process_flash_loan`;
the entrypoint signature is `flash_loan(caller, asset, amount, receiver, data)`):

- `#[when_not_paused]` on the entrypoint.
- `caller.require_auth()`.
- `require_not_flash_loaning()` rejects re-entry with
  `FlashLoanError::FlashLoanOngoing` (the single-flight guard shared with the
  strategy and router entrypoints).
- `require_positive_amount(amount)`, `require_market_active(asset)`, and
  `is_flashloanable` (`FlashLoanError::FlashloanNotEnabled` otherwise).
- Require the receiver to be a deployed Wasm contract.
- Set `FlashLoanOngoing = true` for the entire call window.
- Compute `fee` from `flashloan_fee` via `flash_loan_fee`, which floors a
  positive-rate dust fee up to 1 unit. The rate is capped at
  `MAX_FLASHLOAN_FEE_BPS` = 500 bps at listing time
  (`FlashLoanError::StrategyFeeExceeds`).

## Alternatives Considered

- **Internal-balance bookkeeping (EVM-style).** Rejected: requires
  trusting the token contract's reported amounts in `transfer`.
  Fee-on-transfer or rebasing tokens would break the invariant.
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
  fee, pool, data)` with the expected ABI and pre-authorize the pool's pull.
- The pool emits no fine-grained event from the local snapshot; observers must
  rely on `FlashLoanEvent` from the controller.

## References

- `SCF_BUILD_ARCHITECTURE.md` §11 (Flash Loans).
- `contracts/pool/src/lib.rs::flash_loan`
- `contracts/pool/src/interest.rs::add_protocol_revenue_ray`
- `contracts/controller/src/strategies/flash_loan.rs::process_flash_loan`
- `contracts/controller/src/risk/validation.rs::require_not_flash_loaning`
- `common/src/constants/shared.rs` (`MAX_FLASHLOAN_FEE_BPS` = 500)
- `common/src/errors.rs` (`FlashLoanError::{InvalidFlashloanRepay, FlashloanNotEnabled, StrategyFeeExceeds, FlashLoanOngoing}`)
