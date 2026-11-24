# Architecture

The controller manages lending accounts and risk checks. A central pool holds
tokens and keeps each market's accounting. Governance administers the core
contracts through delayed proposals and restricted immediate actions. This
page introduces the source architecture for contributors, integrators, and
auditors.

## Markets and accounts

A **hub** groups market books. A **hub asset** identifies one market by hub ID
and token address. Each market has its own cash book, supply shares, debt
shares, interest indexes, revenue shares, and rate parameters. An **index**
converts shares into an accrued claim or debt, so accrual does not update every
account.

A **spoke** defines which markets an account can use and their risk settings.
Account creation binds the spoke for the account's lifetime. Existing positions
store risk parameters, so a listing change does not immediately replace every
stored value; threshold updates and relevant position flows refresh them under
their risk checks.

An **account** holds supply and debt positions, one hub asset each, and is
represented by a position NFT. The NFT's owner controls the whole account,
collateral and debt included. Delegates need both an account grant and active
position-manager registration. The
[threat model](../explanation/threat-model.md#account-authority) explains NFT
approvals and delegate permissions.

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
| Curator | Registrar registry: hub and spoke ownership, registrar roles, cap grants, and forwarding of registrar calls to governance |

Governance's deployment helpers make governance the owner of the controller
and price aggregator. Run `deploy_controller` first: `deploy_price_aggregator`
then calls `set_price_aggregator` on the existing controller. In the other
order, a timelocked `SetPriceAggregator` operation connects them. The
controller deploys one pool and one position NFT with itself as their
authority. The pool has no ownership-transfer endpoint;
governance and controller ownership can transfer, which changes this authority
chain.

The curator lets third-party registrars run hubs, spokes, and markets through
governance, which owns the curator and stays the controller's owner. The
curator holds the governance `CURATOR`, `PROPOSER`, and `CANCELLER` roles:

- Immediate registrar calls (create hubs, spokes, and markets; tighten listing
  flags) go through `CURATOR`.
- New listings, listing edits, rate changes, and flag relaxations wait on the
  governance timelock, queued under `PROPOSER`.
- A registrar cancels a queued curator operation that names a hub or spoke it
  owns, under `CANCELLER`.
- The governance owner revokes `CURATOR` immediately with
  `revoke_role_immediate`. Queued operations and `PROPOSER` stay in place.

Registrar control, role bits, cap grants and the foreign-pause rule are in the
[curator README](../../contracts/curator/README.md).

The swap aggregator and XOXNO oracle each take a separate administrator at
construction. Their owner checks do not establish multisig custody, a
timelock, or common ownership with lending governance. The DeFindex adapter
has a fixed constructor configuration and no administrative rescue interface.

## Custody and accounting

The pool tracks cash in an accounting book. For supply, repayment, and
recapitalization, the controller measures the balance change of each transfer
into the pool and passes that amount; the pool credits the amount it is given.
A token that delivers less than requested therefore books the smaller amount,
and direct donations do not increase booked cash. Measurement cannot establish that an arbitrary token is
safe.

The same token can appear in multiple hubs. Its markets keep separate books
but share one physical pool balance, so a malicious token contract can affect
more than one market. See
[token assumptions](../explanation/threat-model.md#token-assumptions).

## Position lifecycle

1. **Supply:** tokens enter the pool and supply shares are credited. A
   third-party top-up cannot create a new asset position in another owner's
   account.
2. **Borrow or withdraw:** the caller needs owner-or-delegate authority. Risk
   and liquidity checks apply; a healthy account does not guarantee available
   cash.
3. **Accrue interest:** market indexes update supply claims and debt values.
4. **Repay or liquidate:** anyone can fund a repayment. An unhealthy account can
   be liquidated, subject to prices and listing restrictions.
5. **Resolve bad debt:** eligible cleanup converts remaining account supply to
   protocol revenue and writes off gross debt through the affected supply
   indexes, even when the account supplies the same market.

Liquidation can pay collateral tokens or credit net supply shares to an
authorized account in the same spoke, avoiding a collateral cash payout.
Receiver and listing restrictions still apply, and the protocol fee is
reclassified as pool revenue. The [formula reference](formulas.md) defines
valuation, rounding, and loss allocation; the [event reference](events.md)
defines gross and net amounts.

## Prices and external execution

Valuation-dependent operations collect required prices in an operation
context, and an invalid required price aborts the operation. A dual-source
configuration needs both sources to pass its checks; one usable source cannot
substitute for a failed one.

| Operation | Settlement |
|---|---|
| Cash flash loan | Sends principal to a contract receiver, then pulls principal plus fee through allowance with exact pool-balance checks; creates no account debt |
| Router strategy | Measures input and output changes, refunds controller-held unused input, then checks final account risk |
| Flash position | Creates debt without an origination fee, receives declared collateral from a callback, and leaves an open solvent account |

For a router strategy the controller authorizes one exact input-transfer
invocation, the router enforces the route's minimum output, and the controller
checks positive measured output and final account risk. These checks do not
guarantee route quality; the
[external integration section](../explanation/threat-model.md#routes-callbacks-and-external-integrations)
covers this boundary and callback assumptions.

## Administration and availability

Governance checks typed proposals before scheduling them, and target contracts
revalidate at execution. Immediate powers cover guardian actions, oracle
sanity-band tightening, curator forwarding, hot-role revocation, and owner
deployment bootstrap. Controller construction and upgrade leave the controller
paused.

Global pause preserves designated exit and recovery entrypoints. Listing flags
separately control entry, exits, and seizure. Price failures, listing
restrictions, cash shortages, storage archival, and resource limits can still
block an action.

Continue with the [threat model](../explanation/threat-model.md) for trust and
availability risks, [invariants](invariants.md) for safety properties, and
[endpoints](endpoints.md) for authorization and operation gates.
