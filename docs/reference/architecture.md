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

One pool contains many isolated markets. A market is keyed by hub asset and
has independent supply and debt indexes, cash, revenue, rate parameters, and
reserves. The same token may appear in distinct markets without combining
their accounting.

An account is bound to one spoke at creation. The spoke supplies the risk
configuration used for every position in that account: collateral eligibility,
borrow eligibility, caps, LTV, threshold, liquidation terms, and halt flags.
The binding never changes.

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

## Controller events that differ from earlier main

Two observer-facing shapes changed in the controller flattening branch.
On-chain account and pool state are unchanged.

- `swap_debt` records the new-debt borrow as `SwDebtR`. Earlier main reused the
  `Multiply` action tag for that borrow through `borrow_for_strategy`. The repay
  leg was already `SwDebtR`.
- After a liquidation that also socializes dust bad debt, `UpdatePositionBatchEvent`
  is published before `CleanBadDebtEvent`. Earlier main published
  `CleanBadDebtEvent` first, then the position batch. The batch payload is the
  same. Cleanup still deletes the account after the batch.

Indexers should key swap-debt opens on `SwDebtR` as well as `Multiply`, and
should not assume bad-debt cleanup precedes the position batch.

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
