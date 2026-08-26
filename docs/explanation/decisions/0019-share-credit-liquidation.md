# 0019. A liquidator may take collateral as shares

**Status:** Accepted

**Implemented by:** common/src/types/controller.rs (`SeizeMode`), contracts/controller/src/positions/liquidation/mod.rs (`process_liquidation`, `resolve_seize_receiver`), contracts/controller/src/positions/liquidation/apply.rs (`apply_liquidation_share_credit`, `credit_supply_shares`, `record_share_credit_updates`, `require_credit_position_limit`), contracts/controller/src/positions/liquidation/math.rs (`split_seized_shares`), contracts/pool/src/cache/shares.rs (`absorb_supply_as_revenue`), contracts/controller/src/events/mod.rs (`LiqSeize`, `LiqCredit`), contracts/controller/src/events/position.rs (`account_attributes`); specs certora/controller/spec/spoke_rules.rs, certora/controller/spec/account_isolation_rules.rs (`liquidation_does_not_change_other_account_positions`); tests contracts/controller/tests/events.rs.

## Decision

`liquidate` takes a `SeizeMode`. `Transfer` is the classical path: the pool
burns the seized supply shares, debits cash, and pays the liquidator in
underlying. `Credit(account_id)` instead moves the seized shares to a controller
account the liquidator controls, so the only token movement in the whole call is
the liquidator's own repayment. `Credit(0)` creates that account; `liquidate`
returns the receiving account id, or `0` in transfer mode.

The reason is liveness. A seizure that must be paid in underlying can fail
purely because the market has no spare cash — precisely when liquidations matter
most. Moving shares needs none.

## Settlement model

The pool tracks market totals and never per-account positions, so moving supply
shares between two controller accounts changes **no pool state**: `supplied` and
`cash` are both untouched. Solvency is preserved trivially rather than by
argument.

The single pool interaction is the protocol fee, booked through the existing
deposit-side seize primitive, which **reclassifies** existing shares into
`revenue`.

**It must not use the mint-based withdraw fee path.** That path is correct only
because cash equal to the fee was withheld from an outbound transfer. In credit
mode nothing is withheld, so minting would create supplier claims with no assets
behind them. This is the single easiest way to get this feature wrong, and it
fails silently — the arithmetic still balances locally while the market quietly
becomes underbacked.

Because scaled amounts are index-independent, a share credit is immune to index
drift between planning and application. The transfer path is not.

## Binding and admission

- **Same spoke.** The receiving account must be bound to the liquidated
  account's spoke. Supply positions carry their risk tuple inline, so moving one
  into another spoke's account would import a foreign risk regime and break
  ADR-0009.
- **Risk tuple on arrival.** A receiver with no position in that asset is
  stamped from the *current* listing; one that already holds the asset keeps its
  own tuple and simply grows, exactly as an ordinary supply behaves. The
  liquidated account's stale tuple is never imported.
- **No entry gate.** A seizure is not a new supply, so `is_collateralizable`
  does not block it. Only `no_seize` gates the leg (ADR-0008).
- **Position limits are enforced**, not bypassed. The liquidator chooses the
  receiver, so a revert is actionable — pass `Credit(0)` for a fresh account.
  Bypassing the bound would let accounts grow past the size the worst-case
  liquidation resource budget is sized for.

## Cap usage nets to minus the fee

The account-to-account move is usage-neutral by construction — same spoke, same
asset — and is deliberately **not** run through the supply-cap check, so a
liquidation cannot fail because the receiving spoke is at its cap.

The fee is different. Those shares leave the account system entirely into pool
revenue, so summed over both accounts the spoke's usage falls by exactly the
fee, and the implementation books that with an explicit exit. Without it, usage
would ratchet upward by every fee ever taken and `remove_asset_from_spoke` —
which requires zero usage — would eventually become unreachable.

The V-5 usage invariant must therefore be stated as a **sum over affected
accounts**, not per-account, and its liquidation form asserts `−fee`, not zero.

## Event contract

A credit-mode liquidation writes two accounts, so it publishes **two**
`UpdatePositionBatchEvent`s: the liquidated account first, the receiver second.
Their collateral legs carry different tags because they carry different amounts:

- `LiqSeize` — the liquidated account's debit, **gross** of the protocol fee, in
  both seize modes.
- `LiqCredit` — the receiver's credit, **net** of that fee.

So the fee is `LiqSeize.amount - LiqCredit.amount`; in transfer mode it is
withheld from the outbound transfer instead of appearing as a second leg.

An earlier revision tagged both legs `LiqSeize`. One tag cannot carry two senses
without forcing an observer to know which account a batch belongs to before it
can read the number, and an observer that skipped that step overstated
liquidator proceeds by the fee — 6.0% on a representative fixture.

Two shapes that surprise integrators: the receiver's batch is **supply-side
only**, and it **omits any leg whose net credit is zero**, which is reachable
when the fee consumes a one-share seizure whole. A `Credit(0)` account is
announced only through that batch's `account_attributes` — there is no
account-creation event.

## What this costs

`liquidation_does_not_change_other_account_positions` previously held
unconditionally. Credit mode makes one liquidation write two accounts, so the
rule now names both principals and asserts no *third* account changes. That is a
genuine weakening of a strong invariant and is the price of the feature; it was
not loosened to "some other account may change".

## Guarantees

- A liquidation cannot fail for want of cash when the liquidator accepts shares.
- Pool `supplied` and `cash` are unchanged by the share transfer; only `revenue`
  moves, by exactly the summed fee.
- Seized shares are conserved: `seized == fee + credited`, asserted in code.
- A receiver never inherits the liquidated account's risk parameters.
- Gross and net are distinguishable on-chain without knowing which account a
  batch belongs to.

## Auditor focus

Check that the fee path reclassifies rather than mints. Check the conservation
identity under under-delivery, where the seizure is scaled down after the split
is computed. Check that the same-spoke rule cannot be bypassed through account
creation, and that the usage exit fires on every path that books a fee. Model an
indexer that reads only `LiqSeize` and confirm the tag split is what stops it
over-counting.
