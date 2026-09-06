# Architecture

The controller manages lending accounts and risk checks. A central pool holds
tokens and maintains each market's accounting. Governance administers the core
contracts through delayed proposals and restricted emergency actions.

This page introduces the source architecture for contributors, integrators,
and auditors. Deployment review must also verify contract code, owners, and
configuration.

## Markets and accounts

A **hub** groups market books. A **hub asset** identifies one market by hub ID
and token address. Each market
has its own cash book, supply shares, debt shares, interest indexes, revenue
shares, and rate parameters. An **index** converts shares into an accrued claim
or debt without updating every account when interest accrues.

A **spoke** defines which markets an account can use and their risk settings.
Account creation binds the spoke for the account's lifetime. Existing positions
store risk parameters; a listing change does not immediately replace every
stored value. Threshold updates and relevant position flows refresh applicable
values under their risk checks.

An **account** holds supply and debt positions and is represented by a position
NFT. Each position records supplied assets or debt for one hub asset. The NFT's owner controls the whole account, including its collateral and
debt. Delegates require both an account grant and active position-manager
registration. The [threat model](../explanation/threat-model.md#account-authority)
explains NFT approvals and delegate permissions.

## Components and authority

| Component | Responsibility |
|---|---|
| Governance | Typed administrative proposals, delayed execution, roles, and restricted immediate actions |
| Controller | Account state, authorization, position risk, liquidation, and strategies |
| Pool | Token custody and market accounting; mutations require its owner |
| Position NFT | Account ownership, holder/approved transfers, and controller-authorized mint, burn, and upgrade |
| Price aggregator | Source configuration, validated price reads, and owner-authorized upgrades |
| XOXNO oracle | Authenticated signer submissions and threshold-based aggregation |
| Swap aggregator | Route execution, fees, and venue calls |
| DeFindex adapter | Supply-only controller accounts keyed by authenticated vault address |

Governance's deployment helpers make governance the owner of the controller
and price aggregator. The controller deploys one pool and one position NFT
with itself as their authority. The pool has no separate ownership-transfer
endpoint. The controller can transfer ownership, so authorized changes can
alter this authority chain.

The swap aggregator and XOXNO oracle each accept a separate administrator at
construction. Their owner checks do not establish multisig custody, a timelock,
or common ownership with lending governance. The DeFindex adapter has fixed
constructor configuration and no administrative rescue interface.

## Custody and accounting

The pool tracks cash in an accounting book. Supply, repayment, and
recapitalization credit the tokens actually received; direct donations do not
automatically increase booked cash. Receipt measurement supports specific
under-delivery behavior but cannot establish that an arbitrary token is safe.

The same token can appear in multiple hubs. Its markets keep separate books
but share one physical pool balance. A malicious token contract can therefore
affect more than one market. See [token assumptions](../explanation/threat-model.md#token-assumptions).

## Position lifecycle

1. **Supply:** tokens enter the pool and supply shares are credited. A third-party
   top-up cannot create a new asset position in another owner's account.
2. **Borrow or withdraw:** the caller needs owner-or-delegate authority. Risk
   and liquidity checks apply; a healthy account does not guarantee available cash.
3. **Accrue interest:** market indexes update supply claims and debt values.
4. **Repay or liquidate:** repayment can be funded by another caller. An
   unhealthy account can be liquidated subject to prices and listing restrictions.
5. **Resolve bad debt:** eligible cleanup converts remaining account supply to
   protocol revenue and writes off debt through the affected supply indexes.
   It processes gross debt even when the account supplies the same market.

Liquidation can pay collateral tokens or credit net supply shares to an
authorized account in the same spoke. Share credit avoids a collateral cash
payout; receiver and listing restrictions still apply. The protocol fee is
reclassified as pool revenue. The [formula reference](formulas.md) defines valuation,
rounding, and loss allocation. The [event reference](events.md) defines gross
and net amounts.

## Prices and external execution

Valuation-dependent operations collect required prices in an operation context.
An invalid required price aborts the operation. A dual-source configuration
requires both sources to pass its checks; one usable source cannot substitute
for a failed source.

Cash flash loans and account strategies have different settlement rules:

| Operation | Settlement |
|---|---|
| Cash flash loan | Sends principal to a contract receiver, then pulls principal plus fee through allowance with exact pool-balance checks; creates no account debt |
| Router strategy | Measures input and output changes, refunds controller-held unused input, then checks final account risk |
| Flash position | Creates debt without an origination fee, receives declared collateral from a callback, and leaves an open solvent account |

The controller authorizes one exact input-transfer invocation for a router
strategy. The router enforces the route's minimum output; the controller checks
positive measured output and final account risk. These checks do not guarantee
route quality. The [external integration section](../explanation/threat-model.md#routes-callbacks-and-external-integrations)
covers this boundary and callback assumptions.

## Administration and availability

Governance checks typed proposals before scheduling them; target contracts also
validate at execution. Immediate powers cover guardian actions, oracle sanity-band
tightening, hot-role revocation, and owner deployment bootstrap. Controller
construction and upgrade leave the controller paused.

Global pause preserves designated exit and recovery entrypoints. Listing flags
separately control entry, exits, and seizure. Price failures, listing restrictions,
cash shortages, storage archival, and resource limits can still block an action.

Continue with the [threat model](../explanation/threat-model.md) for trust and
availability risks, [invariants](invariants.md) for safety properties, and
[endpoints](endpoints.md) for authorization and operation gates.
