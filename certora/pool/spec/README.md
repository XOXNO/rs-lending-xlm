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
  caps; conservative reward distribution; accrued-interest split conservation.

position-accounting.conf
  Exact successful supply and borrow share minting from their indexes with
  conservative directed rounding (credits floor, debits ceil); partial/full
  withdrawal burns; partial/full repay burns, cash, and overpayment refund
  across every validated asset-decimal setting.

seize-settle-accounting.conf
  Borrow-side bad-debt share removal and supply-index write-down; deposit-side
  share transfer into protocol revenue; independently derived net-settle
  lesser-of math, cash invariance, and exact two-leg aggregate/position deltas.

fee-strategy-accounting.conf
  One positive production `global_sync` chunk, including reserve-fee and
  supplier-shortfall booking into protocol revenue; reward allocation and
  cash; liquidation withdrawal fee shares; strategy gross debt/net payout/fee;
  protocol revenue claim burns and cash.

flash-loan-accounting.conf
  Exact configured fee, payout/repayment balance targets, principal-plus-fee
  recovery identity, and successful fee booking into cash/revenue/supply.

pool-core-sanity.conf
  Concrete satisfy-only witnesses for every proof fixture family, including
  the floor-clamped seizure residual. Universal assertions and witnesses are
  intentionally never mixed in one config.

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
rule isolates one operation from interest accrual. The accrual integration rule
directly executes the exact production `global_sync` wrapper for one positive,
at-most-one-year chunk and checks timestamp advancement, both indexes, and
protocol-fee share booking.
Arbitrary multi-year `global_sync` loop completeness and arbitrary-length batch
induction remain out of scope unless the production ABI enforces a bound or the
Prover supports the needed loop invariant.

Supply-index monotonicity and its cap cover arithmetic-success paths across the
complete validated index band. Monotonicity is structurally enforced by the
production lower bound; separate reward-conservation rules check that the index
arithmetic cannot over-credit suppliers. Conservation is split for solver
tractability: the ordinary symbolic state band, a separate high-index band
through 200,000,000x that contains the confirmed rounding regression, and the
exact validated cap boundary. Net settlement expands the supply and debt
formulas without calling the two resolver helpers under test and spans every
validated asset-decimal setting. Supply-share burns round up; positive
settlements whose debt-share credit rounds to zero are rejected by production
and pinned by Rust regressions. The universal rule covers the remaining
successful domain.

Successful position-flow rules span asset decimals 0 through 27 and the
validated index caps, subject to explicit bounded amount/position fixtures and
arithmetic-success assumptions. Supply and repay credits round down and reject
positive zero-share results; borrow and withdrawal debits round up. The rules
also assert the independently valued shares cannot favor the caller. Concrete
Rust regressions pin the zero-credit rollback boundaries.

Net settlement, bad-debt seizure, liquidation-fee, strategy, revenue-claim,
and flash-fee transitions also span the full validated supply/borrow index
caps. The one-chunk accrual integration remains in its explicit 10x index band
because it symbolically executes compounding, both reward splits, and storage
together.

Bad-debt floor caveat
---------------------
The borrow-seizure rule proves the production formula:

  new_supply_index = max(proportional_write_down, SUPPLY_INDEX_FLOOR_RAW)

Thus exact proportionality holds only while the floor does not bind. A total
wipeout retains a small legacy claim at the floor. Every fresh supply checks
that aggregate claims remain covered by tracked cash plus outstanding debt,
including after accrual or rewards lift the index above the floor. The sanity
profile keeps an explicit witness for this exception instead of hiding it
behind the universal seizure rule.
