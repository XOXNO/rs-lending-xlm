Pool-core formal verification
=============================

Purpose
-------
This directory verifies the pool's essential scaled-share, index, cash, loss,
and protocol-fee accounting. It deliberately excludes controller solvency,
oracle behavior, governance, and broad view smoke checks.

The pool stores market aggregates, not account maps. Therefore these proofs
show that each returned position delta equals the corresponding market-total
delta. Proving that the controller persists every returned account delta is a
separate cross-contract obligation.

Six focused jobs
----------------
rate-index-accounting.conf
  Validated utilization kink monotonicity and boundaries; rate cap; deposit
  rate bound; compound-factor lower bound; borrow/supply index monotonicity and
  caps; accrued-interest split conservation.

position-accounting.conf
  Exact supply and borrow share minting from their indexes; partial/full
  withdrawal burns; partial/full repay burns, cash, and overpayment refund.

seize-settle-accounting.conf
  Borrow-side bad-debt share removal and supply-index write-down; deposit-side
  share transfer into protocol revenue; net-settle cash invariance and exact
  two-leg aggregate/position deltas.

fee-strategy-accounting.conf
  Reward allocation and cash; liquidation withdrawal fee shares; strategy
  gross debt/net payout/fee; protocol revenue claim burns and cash.

flash-loan-accounting.conf
  Exact configured fee, payout/repayment balance targets, principal-plus-fee
  recovery identity, and successful fee booking into cash/revenue/supply.

pool-core-sanity.conf
  Concrete satisfy-only witnesses for every proof fixture family. Universal
  assertions and witnesses are intentionally never mixed in one config.

Proof boundary
--------------
Position, strategy, and claim rules call accounting helpers used verbatim by
the production ABI before its external SAC transfer/refund. This avoids a fake
proof caused by an unresolved token contract while preserving production code
identity. The flash job proves the exact balance targets consumed by the real
endpoint and its persisted fee transition. It does not prove arbitrary SAC,
callback, allowance, reentrancy, or Soroban rollback behavior; those require a
sound external-call model.

Each operation fixture sets last_timestamp to the current ledger time, so the
rule isolates one operation from interest accrual. Accrual primitives are
proved compositionally in rate-index-accounting.conf. Arbitrary multi-year
global_sync loop completeness and arbitrary-length batch induction remain out
of scope unless the production ABI enforces a bound or the Prover supports the
needed loop invariant.

Bad-debt floor caveat
---------------------
The borrow-seizure rule proves the production formula:

  new_supply_index = max(proportional_write_down, SUPPLY_INDEX_FLOOR_RAW)

Thus exact proportionality holds only while the floor does not bind. A total
wipeout retains a small legacy claim at the floor; this is a known production
behavior, not hidden by the spec.
