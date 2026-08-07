# 0003. RAY-scaled shares with directed rounding that favors the protocol

Status: Accepted

## Context

Interest must accrue continuously to every position without touching every
position: per-position writes on accrual are unaffordable on-chain. The
standard answer is index-scaled shares, but shares introduce rounding at every
mint and burn, and integer rounding is a value channel — whichever side the
error falls on gains one unit per operation, repeatably. An adversary who can
choose operation sizes can farm any rounding direction that favors users, and
dust-sized operations whose share delta rounds to zero can move tokens while
moving no recorded claim. The rounding regime therefore has to be chosen per
operation, deliberately, and proven — not left to whatever division is
convenient at each call site.

## Decision

Positions and pool aggregates store scaled shares at 1e27 (`Ray`); an actual
balance is `scaled * index / RAY`. The rounding direction is fixed per
operation so the error always accrues to the protocol:

- supply mint rounds shares down —
  `common/src/rates/scaling.rs::calculate_scaled_supply` uses `div_floor`;
- supply burn rounds shares up —
  `common/src/rates/scaling.rs::calculate_scaled_supply_ceil` uses `div_ceil`;
- debt mint rounds shares up —
  `common/src/rates/scaling.rs::calculate_scaled_borrow` uses `div_ceil`;
- debt burn rounds shares down —
  `common/src/rates/scaling.rs::calculate_scaled_borrow_floor` uses
  `div_floor`.

Full-position closes resolve asymmetrically in the protocol's favor:
`common/src/rates/scaling.rs::resolve_withdrawal` pays the floor value of the
remaining shares, while `common/src/rates/scaling.rs::resolve_repay` charges
the ceil value and refunds any overpayment above it.

The pool rejects any value movement whose share delta rounds to zero:
`contracts/pool/src/ops/supply.rs::apply` fails with
`GenericError::SupplyRoundsToZeroShares` (with matching errors on the other
verbs), closing the channel where tokens move but recorded claims do not.

The directed mint/burn arithmetic is proven exactly, not just tested:
`certora/pool/spec/position_accounting_rules.rs` pins the rounding direction
of each operation.

## Alternatives

**Raw token amounts with per-position interest application.** Each position
would store a token amount and accrue interest on touch. This removes share
rounding but makes accrual O(positions) or lazily inconsistent, and every
read of a stale position understates debt. The index model accrues to all
positions in one aggregate write.

**Uniform half-up rounding everywhere.** One rounding rule is simpler to
implement and explain, but half-up leaks value to whichever side the half
falls on — an attacker sizes operations so the leak points at the protocol,
one unit at a time, at scale. Directed rounding makes the leak direction a
constant of the system rather than a function of attacker-chosen inputs.

**Allowing zero-share dust movements and cleaning up later.** Accepting
transfers that mint or burn zero shares keeps small operations frictionless,
but it decouples token flow from recorded claims — the exact property the
accounting invariants exist to guarantee. Cleanup jobs restore the books only
statistically; rejection restores them by construction.

## Consequences

Interest accrual is O(1) per market regardless of position count, and every
rounding error strengthens rather than weakens the books: the sum of what the
pool owes suppliers can only under-state, and the sum of what borrowers owe
can only over-state, relative to exact arithmetic — see
../../reference/invariants.md §INV-ACCT and §INV-IDX. Full closes leave no
residual claim behind.

What this makes hard: every integrator, off-chain balance calculation, and
differential fuzz reference must replicate the exact per-operation rounding
direction — approximating with a single rule produces off-by-one
disagreements. Operations too small to move one share revert rather than
succeed, which surfaces as failures on dust-sized supplies and repays. What
must stay true: no call site may swap a floor for a ceil (or vice versa)
without re-running the Certora position-accounting suite, since the proofs
pin the direction, not merely the magnitude.
