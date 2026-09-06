# Architecture

For reviewers of custody, account authority, and cross-contract boundaries.
This describes the repository deployment path; deployed owners, contract hashes,
and configuration require separate verification.

## Components and authority

| Component | Responsibility and boundary |
|---|---|
| Governance | Typed administrative proposals, delayed execution, roles, and narrower immediate actions |
| Controller | Account state, authorization, position risk, liquidation, and strategies |
| Pool | Token custody and accounting keyed by hub asset; accounting mutations require its owner |
| Position NFT | Live account ownership; controller-authorized mint, burn, and upgrade; ordinary NFT transfers/approvals |
| Price aggregator | Governed source configuration and validated price reads; owner-authorized upgrades |
| XOXNO oracle | Authenticated signer submissions and threshold-based aggregation; separately supplied owner |
| Swap aggregator | Route execution, fees, and venue calls; separately supplied owner and upgrade authority |
| DeFindex adapter | Supply-only controller accounts keyed by authenticated vault address; fixed constructor configuration |

Governance's deployment helpers construct the controller and price aggregator
with governance as owner. The controller deploys its sole pool and NFT with
itself as authority. The pool exposes no separate ownership-transfer method.
The controller can transfer ownership, so this deployment wiring is not a
permanent guarantee against later authorized changes.

The router and XOXNO oracle accept their own administrator at construction.
Neither their address nor their owner gate establishes a multisig, a timelock,
or common ownership with lending governance. The adapter has no owner or
administrative rescue interface.

## Markets, accounts, and custody

A **hub asset** identifies a pool market: hub ID plus token address. Markets
have separate cash books, supply/debt shares, indexes, revenue shares, and rate
parameters. The same token in different hubs has separate accounting but
shares the pool's physical token balance. Isolation of books does not isolate
the effects of a malicious token contract.

A **spoke** supplies an account's listing and risk regime. Account creation
binds the spoke for the account's lifetime. Existing positions carry stored
risk parameters; changing listing configuration does not simply rewrite every
position. Threshold updates and relevant position flows refresh applicable stored values
under their risk gates.

Each account ID identifies a position NFT. The controller reads the NFT owner
when checking account authority. Transferring or approving that NFT therefore
concerns the whole lending position, including collateral and debt. Delegates
need both an account grant and an active position-manager registration. See
[account authority risks](../explanation/threat-model.md#account-authority).

The pool keeps an internal cash book rather than treating token balances as
its accounting state. Supply, repayment, and recapitalization credit measured
receipts. Direct donations do not automatically increase booked cash.
Measured receipt accommodates under-delivery on supported paths; it does not
make arbitrary token behavior safe. See [token assumptions](../explanation/threat-model.md#token-assumptions).

## Position lifecycle

1. Supply receives tokens and mints supply shares. Third-party top-ups cannot
   create a new asset position in another owner's account.
2. Borrow and withdrawal require owner-or-delegate authority. Relevant paths
   check account risk and pool liquidity; being healthy does not guarantee cash.
3. Interest accrues through indexes without iterating through individual accounts.
4. Repayment can be funded by another caller. Liquidation is available when
   the account meets its health gate, subject to prices and listing restrictions.
5. Transfer liquidation pays collateral tokens; credit liquidation moves net
   supply shares to an authorized same-spoke account and reclassifies the fee
   as pool revenue. Credit avoids collateral cash payout, not all failure modes.
6. Eligible bad-debt cleanup seizes remaining supply as revenue and writes off
   debt through the corresponding supply indexes. Same-market supply and debt
   are not netted first; the floor can leave a backing shortfall.

Detailed arithmetic and units belong in [formulas](formulas.md); event ordering
and gross/net amounts belong in [events](events.md).

## External work and valuation

Valuation-dependent controller paths fetch their required prices into a
per-operation context. Invalid required prices abort the operation rather than
substituting a permissive value. A configured dual-source price requires both
legs; it does not fall back to its one surviving leg.

Cash flash loans send principal to a contract receiver and pull principal plus
fee back through allowance, with exact pool-balance checks. They create no
lending account and do not use account strategy finalization.

Account strategies instead mutate positions and finish through account risk
checks. Router swaps settle measured input/output changes. The controller grants
one exact input-transfer invocation, not a token allowance, and refunds
unspent input still held by the controller. The router checks its payload
minimum against output after fees but before payout; its own residuals
follow a capped admin-revenue policy. The controller requires positive
measured output and final account risk, not an independent slippage bound.
Flash position creates zero-origination-fee debt, accepts declared measured
collateral from a callback, and must leave an open solvent position.

Guarded callback windows reject protected monetary reentry. Soroban failures
roll back the transaction. Neither property proves arbitrary external tokens,
price sources, vaults, or routers trustworthy.

## Administration and availability

Ordinary governance schedules typed operations after proposal-time checks;
target contracts retain execution-time validation. Restricted immediate paths
cover guardian actions, ORACLE sanity-band tightening, hot-role revocation,
and owner deployment bootstrap. Controller construction and upgrade leave it paused.

Global pause preserves designated exit/recovery entrypoints. Listing flags
have separate entry, exit, and seizure semantics. Price failures, paused debt,
`no_seize` collateral, cash limits, storage archival, and resource limits can
still prevent actions. See [threat model](../explanation/threat-model.md),
[invariants](invariants.md), and [endpoint gates](endpoints.md).
