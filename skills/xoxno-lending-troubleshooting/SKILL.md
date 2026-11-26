---
name: xoxno-lending-troubleshooting
description: 'Use when an XOXNO Lending call fails: Error(Contract, #NNN), a simulation or transaction reverts, authorization fails, submission returns a lifecycle status, an archived entry needs restoration, or an integration must be reproduced locally or on testnet.'
user-invocable: true
argument-hint: "[error string, hash, or test setup]"
---

# XOXNO Lending troubleshooting

## Decision tree

Match your situation to one of these cases:

1. **Simulation contract error:** capture all diagnostic events. Then
   [identify the panicking contract](#identify-the-panicking-contract-first)
   before mapping the code.
2. **Archived footprint entries:** see [Archived entries](#archived-entries).
3. **Simulation succeeded:** sign that exact prepared envelope. Do not rebuild
   it.
4. **Submission status, or `getTransaction` `NOT_FOUND`:** follow the
   [submission lifecycle](#submission-lifecycle).
5. **Confirmed `FAILED`:** decode the diagnostics. Pair the code with the
   panicking contract.
6. **Confirmed `SUCCESS`:** decode the return. Reconcile events with state.

The [errors reference](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/errors.md)
lists variants and conditions. The
[events reference](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/events.md)
lists topics, payloads, units, and ordering.

## Identify the panicking contract first

`Error(Contract, #N)` names a number, not a namespace. A lending call can
traverse controller, pool, price aggregator, oracle, token, position NFT,
router, and DEX contracts, whose codes overlap.

Follow these steps:

1. Extract only `Error(Contract, #N)`.
2. Find the code and emitting address in the same diagnostic event.
3. Trace handled and propagated errors, because a caught nested error can
   precede the final failure. Never pair the code with the first error event or an
   adjacent log.
4. Match that address against the deployment manifest and invoked contracts.
5. Only then map `N` with the error reference.

Do not keep a "non-user or collision" set that contains controller #101 or
#114. `HealthFactorTooHigh` #101 is legitimate when a liquidation target has no
debt or is no longer liquidatable. `CannotCleanBadDebt` #114 is legitimate when
`clean_bad_debt` finds debt not above collateral or collateral above the dust
cap. Either code can also collide with another contract's code.

Host errors (`Error(Auth, ...)`, `Error(Storage, ...)`, budget errors, Wasm
traps) are not lending contract codes.

```ts
import { humanizeEvents, rpc, TransactionBuilder } from '@stellar/stellar-sdk'

const CONTRACT_CODE = /Error\(Contract,\s*#(\d+)\)/

export async function inspectSimulation(
  server: rpc.Server,
  transactionXdr: string,
  passphrase: string,
) {
  const tx = TransactionBuilder.fromXDR(transactionXdr, passphrase)
  const simulation = await server.simulateTransaction(tx)
  if (rpc.Api.isSimulationRestore(simulation)) {
    return { kind: 'restore' as const, preamble: simulation.restorePreamble, simulation }
  }
  if (!rpc.Api.isSimulationError(simulation)) {
    return { kind: 'ok' as const, simulation }
  }

  const events = humanizeEvents(simulation.events)
  const code = simulation.error.match(CONTRACT_CODE)?.[1]
  return {
    kind: 'error' as const,
    codeHint: code === undefined ? null : Number(code),
    // Determine the namespace from the complete diagnostic trace.
    contractId: null,
    events,
    diagnosticEvents: simulation.events,
    raw: simulation.error,
  }
}
```

Keep the original simulation and diagnostic records. Humanized events are for
display only. An SDK helper that maps the number without the diagnostic
contract id gives a hint, never a final diagnosis.

## Submission lifecycle

Before the first send, durably persist the network, signed envelope XDR,
hash, sequence, timebounds, and application idempotency key.

| Observation | Required action |
|---|---|
| `ERROR` | Rejected by this RPC. Decode `errorResult` (transaction cause) and `diagnosticEvents` (contract cause). Check the original hash before reusing the sequence. |
| `PENDING` | Poll the original hash. |
| `DUPLICATE` | Poll the original hash. Send no replacement. Count no second success. |
| `TRY_AGAIN_LATER` | Check the hash. Retry the identical envelope. Keep the record until resolved. |
| `getTransaction NOT_FOUND` | Not terminal. Keep checking or resubmitting the original while timebounds are valid. |
| confirmed `FAILED` | Decode result and diagnostics. Terminal. |
| confirmed `SUCCESS` | Verify return, events, and post-state. Terminal. |

A send status is never success. Check expiry by ledger close time. Expiry
blocks future inclusion but does not prove non-inclusion. After expiry,
resolve the hash from retained or complete history, and reconcile account and
token state. While the outcome is unknown, keep the record and submit no
replacement. After a resolved failure or proven non-inclusion, fetch the
sequence, rebuild, re-simulate, sign, and persist the replacement before you
send it.

`txBAD_SEQ` usually means another envelope used the sequence. `txTOO_LATE`
means expired timebounds. Neither justifies forgetting a still-live signed envelope.

## Archived entries

**On protocol 23 and later, simulate the invocation, assemble it with the
simulation result, and submit it. The transaction restores archived footprint entries
inline, and the submitter pays restoration rent.** Do not turn every
simulation restore response into a separate restore transaction.

Use a separate `restoreFootprint` only for a hand-built footprint submitted
without simulation or assembly. Confirm the restore, then re-simulate. For
dormant accounts, the archived entry is often the position NFT owner record.
The permissionless NFT `renew(token_id)` extends it proactively.

## Authorization ordering in calling contracts

A direct-invoking contract satisfies the controller's `caller.require_auth()`,
but nested pulls like `token.transfer(caller, pool, amount)` need exact
authorization. Do all views and balance and config reads first. Then
call `authorize_as_current_contract` for the exact transfer. Invoke the
controller immediately. An outbound call in between can consume the "next
invocation" authorization and produce `Error(Auth, InvalidAction)`.

Contract liquidators must not repeat a token address across hub-specific debt
legs. See [unique debt-token addresses](../xoxno-lending-liquidations/SKILL.md#unique-debt-token-addresses-for-contract-liquidators).

## Common operational interpretations

Apply these interpretations:

- `SpokeNotFound`: take the controller address and the on-chain spoke id, not
  the config key, from the same network manifest. Never use spoke 0.
- `SpokeMismatch`: keep every leg in the account's stored spoke.
- Pause, freeze, and no-seize: read the spoke-asset configuration against the
  [listing-flag table](../xoxno-lending-liquidations/SKILL.md#listing-flags).
- Router slippage or venue errors: identify the emitting contract. Re-quote.
  Rebuild the route XDR. Re-simulate the composed call.
- `InvalidPayments`: check the endpoint's rules for empty lists, length,
  duplicate assets, same-token legs, and swap-route shape.
- `expected a Transaction, got [object Object]`: rebuild the XDR with the same
  `@stellar/stellar-sdk` instance as the RPC server.
- Budget or storage failure after a clean simulation: the cause is stale state
  or resources. Re-simulate immediately before signing.

## Reproduction and completion

Use the narrowest harness target:

```bash
make test-match PATTERN=<substring>
```

`PATTERN=` is required. `MATCH=` is not recognised, and an empty `PATTERN`
exits with status 2.

For testnet, take the controller and pool addresses and the on-chain hub and spoke ids from [`configs/networks.json`](https://github.com/XOXNO/rs-lending-xlm/blob/main/configs/networks.json).
Take asset addresses from
[`configs/testnet/markets.json`](https://github.com/XOXNO/rs-lending-xlm/blob/main/configs/testnet/markets.json) or
[addresses](../xoxno-lending/addresses.md). Asset identity is the contract
address plus the hub and spoke, never a display symbol or search hit.

Before you finish, confirm each check:

- [ ] You simulated the final write with the same operations that you signed.
- [ ] You mapped the error number only after you identified the panicking contract.
- [ ] You persisted the original signed envelope and hash until you resolved its outcome.
- [ ] You confirmed success for the original hash and did not infer it from the send.
- [ ] You reconciled expected events with post-transaction state.
- [ ] You persisted the event or indexer cursor and the application idempotency state.
