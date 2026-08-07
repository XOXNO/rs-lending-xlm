# 0010. Flash loans repay by allowance, verified by exact balance assertions, with reentrancy blocked one layer up

Status: Accepted

## Context

A flash loan hands the pool's own reserves to arbitrary third-party code for the
duration of one invocation. Three failure classes dominate the threat model: the
receiver never repays; the receiver *appears* to repay by pushing tokens into the
pool mid-callback (a donation that would let tracked accounting and the live
balance drift apart); and the receiver re-enters position entrypoints while the
loan is outstanding, exploiting the transient hole in the pool's balance. The
custody split (see ADR 0013) compounds the stakes: the pool trusts declared
amounts everywhere else, so the flash path is the one place where the pool must
defend its own token balance. Finally, the pool deliberately contains no risk or
session state — all user-facing gating lives in the controller — so any
reentrancy defense must respect that ownership boundary rather than duplicate
state on both sides of it.

## Decision

The pool verifies flash loans purely by exact balance arithmetic, and repayment
is allowance-based only. `contracts/pool/src/ops/flash.rs::apply` requires the
market's `is_flashloanable` flag and sufficient reserves, requires the receiver
to be a deployed Wasm contract (`common/src/validation.rs::require_wasm_receiver`),
and computes `contracts/pool/src/ops/flash.rs::terms` against the pool's *live*
token balance — not tracked cash. It then:

1. transfers the principal out and asserts the balance equals `pre - amount`;
2. invokes `execute_flash_loan` on the receiver;
3. asserts the balance *still* equals `pre - amount` after the callback returns,
   so a receiver that pushes tokens back directly fails with
   `InvalidFlashloanRepay`;
4. collects repayment in `contracts/pool/src/ops/flash.rs::collect_repayment`:
   requires `allowance(receiver, pool) >= amount + fee`, executes
   `transfer_from(pool, receiver, pool, total)`, and asserts the final balance
   equals exactly `pre + fee`.

Only then is the fee booked (`::book_fee` credits cash and protocol revenue) and
the market cache committed — a failed loan leaves no state behind.

The pool holds no reentrancy guard. The controller wraps the pool call in
`contracts/controller/src/storage/session.rs::with_flash_guard`, which sets the
temporary-storage `SessionKey::FlashLoanOngoing` flag with a nesting-safe
restore, and `contracts/controller/src/risk/validation.rs::require_not_flash_loaning`
gates every position verb. The same guard wraps the swap router
(`contracts/controller/src/strategies/swap/route.rs::call_router_with_reentrancy_guard`)
and Blend externals, so all untrusted-callback surfaces share one flag. The full
entrypoint matrix is pinned by
`tests/test-harness/tests/meta/reentrancy_matrix.rs::test_all_state_changing_entries_reject_under_flash_loan_ongoing`.

## Alternatives

**Push-based repayment.** The receiver would transfer tokens back to the pool
during the callback and the pool would merely check its closing balance. This is
the common EVM pattern, but it cannot distinguish repayment from donation, and
it leaves the "did enough arrive" check entangled with whatever else moved the
balance mid-callback. The exact post-callback `pre - amount` assertion actively
rejects pushes, making the repayment channel unambiguous: allowance or nothing.

**A pool-level reentrancy mutex.** The pool could set its own busy flag around
the callback. That duplicates session state across the ownership boundary, and
still would not protect the controller's position verbs — the assets at risk are
priced and risk-checked in the controller, so the guard belongs where the
positions live. One flag, one layer, one matrix test.

**Debiting tracked `cash` for the principal.** Accounting the loan through the
pool's internal cash counter would keep tracked state consistent but verify
nothing about the real token balance, which is exactly what a malicious or
non-standard token perturbs. Asserting on live balances makes the token itself
part of the checked surface.

## Consequences

Conservation is exact and locally checkable: a flash loan either ends with the
pool holding precisely `pre + fee` or the whole transaction reverts, which keeps
the FLASH and ACCT domains of ../../reference/invariants.md provable in
isolation. The receiver ABI (`execute_flash_loan(initiator, asset, amount, fee,
pool, data)`) and the allowance-repayment convention are public contract surface;
changing either breaks every deployed receiver.

What it makes hard: receivers that cannot grant allowances (plain accounts) are
excluded by design — the Wasm-receiver requirement is a feature, not a
limitation. Donation-style repayment tooling from EVM ecosystems does not port.

What must stay true: every new state-changing controller entrypoint must call
`require_not_flash_loaning` (the reentrancy matrix test is the enforcement
mechanism — extend it with each new verb); the pool must never grow a second
inbound token path or a token operation between payout and repayment; and the
flash guard must keep wrapping every external callback surface, not just the
pool call (see ../threat-model.md for the untrusted-callee inventory).
