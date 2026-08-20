# Runtime invariants

These are the properties an audit should try to falsify. Each must be enforced
on a live execution path, covered by tests, or specified formally. A passing
test or model is evidence, not a substitute for reviewing the deployed
configuration and integration assumptions.

## Authorization

### INV-AUTH-01 — One ownership chain

Governance controls the controller; the controller controls every
state-changing pool action. No user or external component can mutate pool
accounting directly.

### INV-AUTH-02 — Risk-reducing authority is explicit

Borrowing and withdrawing require the account owner or a delegate that is both
listed on the account and active as a position manager. Delegates cannot grant
or renew their own authority.

### INV-AUTH-03 — Permissionless actions do not create foreign risk

Third parties may repay, liquidate, recapitalize, and perform maintenance.
Third-party supply can only top up an already existing supply position. These
paths must not create an unwanted account slot or increase another user’s risk.

### INV-AUTH-04 — Emergency power only tightens

Immediate guardian power can pause and add restrictions. It cannot unpause or
clear a restriction. Reopening is timelocked.

### INV-AUTH-05 — Governance delay cannot be shortened

The governance delay is non-zero and subsequent updates can only increase it
within the supported domain.

## Accounting

### INV-ACCT-01 — Supply, revenue, and debt shares are non-negative

Revenue shares are a subset of supply shares. No operation may create negative
share totals or a treasury claim with no corresponding supplied shares.

### INV-ACCT-02 — Cash is the reserve book

Liquidity checks use tracked cash, not an incidental token balance. Donations
do not create lendable cash. Debits cannot make cash negative.

### INV-ACCT-03 — Credit equals measured receipt

Inbound supply, repayment, recapitalization, and strategy settlement credit
only tokens actually received. Requested transfer amounts are not accounting
evidence.

### INV-ACCT-04 — Backing shortfall blocks new supply

An underbacked market rejects new supply. Recapitalization can fill no more
than the shortfall and refunds excess without minting shares.

### INV-ACCT-05 — Positive value must change shares

Any positive operation whose share conversion produces zero shares reverts.
This prevents dust transfers from moving value without a matching book entry.

### INV-ACCT-06 — Revenue claims remain solvent

A revenue claim burns enough shares, respects cash and solvency limits, and
cannot pay a positive amount while burning zero entitlement.

## Interest and indexes

### INV-IDX-01 — Borrow index is monotone and bounded

Accrual cannot decrease debt value. The borrow index remains within its
configured maximum.

### INV-IDX-02 — Supply index is bounded

The supply index stays above a non-zero floor and below its configured maximum.

### INV-IDX-03 — Bad debt may lower the supply index

Supplier value is not monotone. Eligible socialized loss lowers only the
affected market’s supply index.

### INV-IDX-04 — Accrual is time-consistent

Zero elapsed time changes nothing. Long gaps are processed in bounded forward
chunks; time never moves backward.

### INV-IDX-05 — Accrued interest is fully assigned

Accrued borrower interest is accounted for as supplier reward or protocol
revenue, including conservative rounding remainders.

## Oracle and risk

### INV-ORACLE-01 — Valuation fails closed

Any missing, stale, invalid, or disagreeing required price prevents the
valuation-dependent mutation.

### INV-ORACLE-02 — A dual source requires both legs

One functioning leg is not a fallback. Accepted blended prices stay within the
two validated source prices.

### INV-ORACLE-03 — One transaction sees one snapshot

All risk calculations in a mutation use one coherent set of prices.

### INV-RISK-01 — Risk-increasing actions re-prove solvency

After the pool action, LTV, health factor, minimum collateral, caps, and
listing rules must all hold.

### INV-RISK-02 — Conservative valuation biases safety

Collateral is rounded down, debt is rounded up, and health factor is rounded
down.

### INV-RISK-03 — Risk configuration is coherent

LTV remains strictly below liquidation threshold. Liquidation bonus and fees
cannot consume more collateral than the protocol permits.

## Liquidation

### INV-LIQ-01 — Only unhealthy debt can be liquidated

Liquidation requires live debt and health factor below one; it is
permissionless, and an account owner may liquidate its own account. The one
remaining identity guard is receiver-side, not caller-side: in `Credit` seize
mode, the receiving account cannot be the liquidated account itself
(`requested != account_id`, `SelfLiquidationNotAllowed` = error #133) — crediting
seized collateral back to the account it was seized from would undo the
seizure.

### INV-LIQ-02 — Repayment and seizure stay coupled

Repayment is bounded by the close policy, excess is refunded, and seizure
never exceeds the current position.

### INV-LIQ-03 — Under-delivery reduces seizure

When a token transfer delivers less than planned, collateral seizure scales down
with the measured receipt.

### INV-LIQ-04 — Bad-debt socialization is explicit and total

Residual debt is socialized only after its gates hold; debt removal, loss
allocation, and post-condition checks are one atomic result.

## Halts, caps, and storage

### INV-HALT-01 — Global pause blocks new risk

Global pause blocks risk-increasing actions while preserving safe exits where
the listing permits them.

### INV-HALT-02 — Frozen and paused differ

Frozen prevents new exposure but permits exits. Paused blocks all activity on
the listing, including an affected liquidation leg.

### INV-HALT-03 — Caps are literal and exit-safe

Zero cap admits nothing. Entry paths enforce usage at the live index; exits do
not consume a cap or underflow its usage.

### INV-STOR-01 — Persistent state has lifecycle discipline

Account and market records use their intended persistence lifetime, renew when
read or written, and remove empty account state without leaving reachable
orphaned authority.

### INV-STOR-02 — NFT TTL renewal is asymmetric with account renewal

The position NFT's own instance (controller address, collection metadata, the
sequential id counter) renews to the protocol's instance TTL on every `mint`
and `burn` — i.e. on every controller account create/delete. But OZ's
`owner_of` renews only the per-token persistent `Owner(token_id)` entry by
OZ's own 30-day default, not the controller's 120-day per-user renewal
window.

Two renewal paths close the gap: `renew_account` on the controller extends
the account's NFT `Owner` entry to the same 120-day window as the
controller's own entries (via the NFT's `renew` entrypoint), and
`position-nft::renew(token_id)` itself is permissionless — anyone, including
a keeper or liquidation bot, may extend any live token's `Owner` entry at
any time (a TTL extension moves no state and cannot shorten a lifetime).

The residual asymmetry: an account whose owner only *passively* touches
`owner_of` (ordinary user actions, no `renew_account`) refreshes the entry
by 30 days per touch, so a position idle for 30–120 days can still let its
`Owner` entry archive while controller state is live — requiring a
`RestoreFootprint` on the NFT contract's owner entry before any controller
op, including liquidation, proceeds. Bots should prefer calling
`position-nft::renew` proactively on positions they monitor and must handle
restore-then-liquidate as the fallback. See
`docs/explanation/threat-model.md` (Controller ↔ Position NFT boundary) and
the `building-lending-liquidation-bots` skill.

## Flash loans and strategies

### INV-FLASH-01 — Flash repayment is exact

Pool balances are checked around the callback. Repayment is allowance-pulled
and includes the exact required fee.

### INV-FLASH-02 — Monetary reentrancy is blocked

The flash callback, router call, and external strategy paths share protection
against entering protected monetary flows recursively.

### INV-STRAT-01 — Router authority is narrowly scoped

The router cannot pull more input than approved. Its return values are not
trusted.

### INV-STRAT-02 — Strategy settlement is measured and solvent

Swaps must produce measured output, return residue to the rightful caller, and
finish behind the same risk gates as ordinary account operations.

### INV-STRAT-03 — Flash position cannot round-trip to a closed account

`flash_position` mints strategy debt with no flash fee and never repays that
debt in the same call. The receiver's only protocol-side settlement is a
measured collateral deposit onto the same account, followed by ordinary
solvency gates. After a successful call the account must still hold that
debt **and** at least one supply position (`FlashPositionClosed` otherwise).
It must not become a free cash flash loan.
