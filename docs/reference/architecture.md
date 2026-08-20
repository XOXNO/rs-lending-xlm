# Architecture

## At a glance

XOXNO Lending is a Soroban lending protocol with a single custody pool,
per-market accounting, account-level risk configuration, governed prices, and
timelocked administration.

The central security rule is simple: only the controller may change pool
accounting, and the controller must establish authorization, price validity,
and post-operation solvency first.

## Components

| Component | Responsibility | Security boundary |
|---|---|---|
| Governance | Owns core protocol administration and schedules delayed changes | Timelock, roles, emergency ratchet |
| Controller | User-facing lending state machine | Auth, accounts, risk, liquidation, strategies |
| Pool | Custodies assets and tracks market accounting | Controller-only mutation and reserve guards |
| Price aggregator | Produces validated price snapshots | Source quality, tolerance, staleness, sanity |
| Swap aggregator | Executes supplied route payloads | Untrusted; settled by balance deltas |
| Oracle sources | Provide underlying observations | External availability and integrity boundary |
| Keeper | Calls permissionless maintenance actions | No privileged lending authority |
| Strategy adapter | Connects external vault flows to accounts | Normal account and solvency rules |

## Authority and custody

Governance owns the controller; the controller owns the pool. The controller is
therefore the only path from user intent to pool accounting.

Users authenticate their own actions. Borrowing and withdrawing additionally
require account-owner authority or a valid delegate. Permissionless actions
such as repayment, liquidation, recapitalization, and maintenance are designed
to reduce risk or restore liveness.

The pool does not infer risk from token balances. It keeps its own cash book
and market share totals. Inbound transfers are credited by the amount actually
received; outbound transfers are controlled by pool accounting.

## Markets and accounts

One pool contains many isolated markets. A market is keyed by hub asset and has
independent supply and debt indexes, supply and debt share totals, cash,
accrued revenue, and rate parameters. `get_reserves` reports that same cash
figure; reserves are not a separate quantity. The same token may appear in
distinct markets without combining their accounting.

An account is bound to one spoke at creation. The spoke supplies the risk
configuration used for every position in that account: collateral eligibility,
borrow eligibility, caps, LTV, threshold, liquidation terms, and halt flags.
The binding never changes.

### The account is an NFT

Each lending account is one token in the position-NFT collection. The
`account_id` is the NFT `token_id`, and the NFT holder is the account owner.
The controller mints the token when it creates an account (`create_account` in
`contracts/controller/src/account.rs`) and burns it when the account is
removed. The controller stores no owner address: it asks the NFT contract who
owns `account_id` every time it checks authority, so transferring the token
transfers the position. See
[position-nft/README.md](../../contracts/position-nft/README.md).

## Typical lifecycle

1. A supplier deposits an accepted hub asset. The controller measures receipt
   and the pool mints supply shares.
2. A borrower supplies collateral and borrows within the account’s spoke rules.
   The controller uses a complete price snapshot and rechecks solvency.
3. Interest updates indexes. Shares retain their quantity while their asset
   value changes with the relevant index.
4. A healthy user can repay or withdraw subject to authorization and risk
   gates. A third party can repay.
5. If health falls below one, a liquidator repays debt and receives discounted
   collateral. Any residual eligible bad debt can be socialized within that
   market’s supplier index.

## Price and risk flow

Every valuation-dependent mutation consumes a complete price snapshot. Source
failure, staleness, disagreement, or a failed sanity rule makes the operation
revert. The protocol favors halted risk-taking over a questionable price.

The controller evaluates collateral conservatively and debt conservatively,
then requires sufficient LTV and health factor after a risk-reducing or
risk-increasing operation as applicable.

## Controller event shapes an indexer must handle

These event shapes are easy to misread. None of them affects on-chain account
or pool state.

- Both legs of `swap_debt` carry `SwDebtR`: the borrow of the new debt and the
  repay of the old debt.
- When a liquidation also socializes dust bad debt, every
  `UpdatePositionBatchEvent` is published before `CleanBadDebtEvent`. Cleanup
  deletes the account after the batch.
- A liquidation using `SeizeMode::Credit` publishes **two**
  `UpdatePositionBatchEvent`s: the liquidated account's batch first, then the
  receiving account's. `SeizeMode::Transfer` publishes exactly one. Both
  batches precede any `CleanBadDebtEvent`.

Indexers key swap-debt opens on `SwDebtR`, must not assume bad-debt cleanup
precedes the position batch, and must not assume one liquidation produces one
position batch.

### `LiqSeize` is gross, `LiqCredit` is net

A share-credit liquidation moves collateral out of one account and into another,
and the two legs carry **different amounts** — the protocol fee is taken in
between. They therefore carry different action tags:

| Tag | Batch | Amount |
|---|---|---|
| `LiqSeize` | liquidated account | **gross** — protocol fee still inside |
| `LiqCredit` | share-credit receiver | **net** — fee already removed |

Measured on a representative fixture: gross `999_833_197_057`, fee
`59_982_992_641`, credited `939_850_204_416`. Summing both tags as if they were
the same quantity double-counts. The fee is **6.0% of the gross**, so reading
the gross figure as the liquidator's proceeds overstates those proceeds by
**6.4%**.

`SeizeMode::Transfer` emits only `LiqSeize`, also gross — the fee is withheld
from the outbound transfer rather than from a second leg.

One tag cannot carry two senses: an indexer would have to know which account a
batch belongs to before it could read the numbers. That is why the receiver's
leg has its own tag.

Two further shapes on the receiver's batch:

- It is **supply-side only** (`PositionSides::SUPPLY`), so do not expect debt legs.
- It **omits any leg whose net credit is zero**, which is reachable when the fee
  consumes a one-share seizure whole.

### A `Credit(0)` account has no creation event

When the liquidator passes `Credit(0)`, the new account is announced only through
the second batch's `account_attributes`. There is no dedicated account-creation
event, so an indexer that discovers accounts from a creation event alone will
never see it. `liquidate` also returns the id to the caller.

### `LiquidationEvent.repaid_usd_wad` is the delivered repayment

It carries the repayment the pool actually received, valued after the tokens
moved: net of any overpayment refunded to the liquidator, and net of any
shortfall from a debt token that delivers less than it is sent. It therefore
agrees with the debt actually retired, which is also visible as the `LiqRepay`
deltas in the accompanying position batch.

The protocol explicitly supports a debt token that delivers less than it is
sent: the seizure is scaled down to match the measured receipt, and
`repaid_usd_wad` reports that same measured figure, never the planned one.

`LiquidationEvent` carries no seizure or protocol-fee figure at all; those are
the batch's `LiqSeize` and `LiqCredit` legs.

## Share-credit liquidation

Decision record: [ADR-0019](../explanation/decisions/0019-share-credit-liquidation.md).

`liquidate` takes a `SeizeMode`. `Transfer` is the classical path: the pool burns
the seized supply shares, debits cash, and pays the liquidator in underlying.
`Credit(account_id)` instead moves the seized shares to a controller account,
so the only token movement in the whole call is the liquidator's own repayment.
`Credit(0)` creates that account, owned by the liquidator and bound to the
liquidated account's spoke; `liquidate` returns the receiving account id, or `0`
in transfer mode.

The point is liveness. A seizure that must be paid in underlying can fail purely
because the market has no spare cash, exactly when liquidations matter most.
Moving shares needs none.

The pool barely participates. It tracks market totals, never per-account
positions, so moving supply shares between two controller accounts changes no
pool state: `supplied` and `cash` are untouched. The only pool interaction is the
protocol fee, booked through the existing deposit-side seize primitive, which
*reclassifies* existing shares into `revenue`. It deliberately does not use the
mint-based withdraw fee path — that path is correct only because cash equal to
the fee was withheld from an outbound transfer, and in credit mode nothing is
withheld, so minting would create supplier claims with no assets behind them.

Because scaled amounts are index-independent, a share credit is immune to index
drift between planning and application, which the transfer path is not.

`get_liquidation_estimate` takes the same parameter and reports the units the
chosen mode moves: asset units for `Transfer`, RAY-scaled supply shares for
`Credit`.

## What a liquidator must handle

The bonus is a function of the account's live health factor, so it is not fixed
at submission time. If another liquidator lands first and improves the account,
the bonus on arrival is lower than the estimate the transaction was built from.
The protocol guarantees only the account's base bonus as a floor; anything above
it depends on how unhealthy the position still is when the call executes.

Liquidators are therefore expected to enforce their own profitability and
slippage bounds — size against the base bonus as the worst case, and revert
rather than execute if the realised bonus is below expectation. Aave V4 reached
the same conclusion and documented it as the liquidator's responsibility.

Two further effects are worth pricing in:

- Rounding runs against the liquidator on the collateral leg, so very small
  positions can cost more in rounding than the bonus pays. The unprofitability
  threshold and the margin the minimum-collateral floor gives are derived in
  [numeric-bounds.md](numeric-bounds.md).
- A repayment small enough that its pro-rata seizure floors to zero asset units
  settles the debt and seizes nothing. Size the repayment against the seizure it
  is expected to produce, not just against the debt.

## External calls

The router, token contracts, price sources, flash receivers, and external
vaults are untrusted or partially trusted boundaries.

The controller limits router authority to a stated input, ignores returned
claims, measures balance deltas, and applies the normal final solvency gate.
Flash-loan repayment is checked by exact pool balances and guarded against
reentrant monetary actions.

## Emergency and governance model

The guardian can immediately make the protocol safer by pausing or tightening
listing flags. It cannot immediately reopen a market. Unpausing and relaxed
configuration require timelocked governance.

Governance operations are validated at proposal time, delayed, and bound to
their scheduled payload. Recovery operations have dedicated handling to avoid
a lost role permanently blocking governance.

## Audit checkpoints

- Verify the deployed ownership chain and every state-changing boundary.
- Treat market isolation, cash accounting, and measured receipt as one
  accounting system.
- Trace every route from external call to final balance and solvency check.
- Test emergency powers separately from timelocked reopening powers.
