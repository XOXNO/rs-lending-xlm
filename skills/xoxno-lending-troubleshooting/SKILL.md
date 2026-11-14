---
name: xoxno-lending-troubleshooting
description: 'Use when an XOXNO Lending call fails — Error(Contract, #NNN), a simulation or transaction reverts, authorization fails, submission returns a lifecycle status, an archived entry needs restoration, or an integration must be reproduced locally or on testnet.'
user-invocable: true
argument-hint: "[error string, hash, or test setup]"
---

# XOXNO Lending troubleshooting

## Decision tree

1. **Simulation contract error:** capture all diagnostic events;
   [identify the panicking contract](#identify-the-panicking-contract-first)
   before mapping the code.
2. **Archived footprint entries:** [Archived entries](#archived-entries).
3. **Simulation succeeded:** sign that exact prepared envelope; do not rebuild.
4. **Submission status, or `getTransaction` `NOT_FOUND`:**
   [submission lifecycle](#submission-lifecycle).
5. **Confirmed `FAILED`:** decode diagnostics, pairing code with panicking
   contract.
6. **Confirmed `SUCCESS`:** decode the return; reconcile events with state.

Catalogs: [errors](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/errors.md)
(variants and conditions) and
[events](https://github.com/XOXNO/rs-lending-xlm/blob/main/docs/reference/events.md)
(topics, payloads, units, ordering).

## Identify the panicking contract first

`Error(Contract, #N)` names a number, not a namespace. A lending call can
traverse controller, pool, price aggregator, oracle, token, position NFT,
router, and DEX contracts, whose codes overlap.

1. Extract only `Error(Contract, #N)`.
2. Find the code and emitting address in the same diagnostic event.
3. Trace handled and propagated errors: a caught nested error can precede the
   final failure. Never pair the code with the first error event or an
   adjacent log.
4. Match that address against the deployment manifest and invoked contracts.
5. Only then map `N` with the error reference.

Do not keep a "non-user/collision" set containing controller #101 or #114:
`HealthFactorTooHigh` #101 is legitimate when a liquidation target has no
debt or is no longer liquidatable; `CannotCleanBadDebt` #114 when
`clean_bad_debt` finds debt not above collateral or collateral above the dust
cap. Either can also collide with another contract's code.

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

Keep the original simulation and diagnostic records; humanized events are for
display only. An SDK helper that maps the number without the diagnostic
contract id gives a hint, never a final diagnosis.

## Submission lifecycle

Before the first send, durably persist the network, signed envelope XDR,
hash, sequence, timebounds, and application idempotency key.

| Observation | Required action |
|---|---|
| `ERROR` | Rejected by this RPC. Decode `errorResult` (transaction cause) and `diagnosticEvents` (contract cause); check the original hash before reusing the sequence. |
| `PENDING` | Poll the original hash. |
| `DUPLICATE` | Poll the original hash; no replacement, no second success. |
| `TRY_AGAIN_LATER` | Check the hash, retry the identical envelope; keep the record until resolved. |
| `getTransaction NOT_FOUND` | Not terminal; keep checking/resubmitting the original while timebounds are valid. |
| confirmed `FAILED` | Decode result and diagnostics; terminal. |
| confirmed `SUCCESS` | Verify return, events, post-state; terminal. |

A send status is never success. Check expiry by ledger close time; expiry
blocks future inclusion but does not prove non-inclusion. After expiry,
resolve the hash from retained or complete history and reconcile account and
token state; while unknown, keep the record and submit no replacement. After
a resolved failure or proven non-inclusion, fetch sequence, rebuild,
re-simulate, sign, and persist the replacement before sending.

`txBAD_SEQ` usually means another envelope used the sequence; `txTOO_LATE`,
expired timebounds. Neither justifies forgetting a still-live signed envelope.

## Archived entries

**On protocol 23 and later, simulate the invocation, assemble it with the
simulation result, and submit it; archived footprint entries are restored
inline and the submitter pays restoration rent.** Do not turn every
simulation restore response into a separate restore transaction.

A separate `restoreFootprint` is only for a hand-built footprint submitted
without simulation/assembly: confirm the restore, then re-simulate. For
dormant accounts the archived entry is often the position NFT owner record;
permissionless NFT `renew(token_id)` extends it proactively.

## Authorization ordering in calling contracts

A direct-invoking contract satisfies the controller's `caller.require_auth()`,
but nested pulls like `token.transfer(caller, pool, amount)` need exact
authorization. Do all views, balance and config reads first, then
`authorize_as_current_contract` for the exact transfer and invoke the
controller immediately. An outbound call in between can consume the "next
invocation" authorization and produce `Error(Auth, InvalidAction)`.

Contract liquidators must not repeat a token address across hub-specific debt
legs; see [unique debt-token addresses](../xoxno-lending-liquidations/SKILL.md#unique-debt-token-addresses-for-contract-liquidators).

## Common operational interpretations

- `SpokeNotFound`: take controller address and on-chain spoke id (not the
  config key) from the same network manifest; never spoke 0.
- `SpokeMismatch`: keep every leg in the account's stored spoke.
- Pause/freeze/no-seize: read the spoke-asset configuration against the
  [listing-flag table](../xoxno-lending-liquidations/SKILL.md#listing-flags).
- Router slippage or venue errors: identify the emitting contract, re-quote,
  rebuild route XDR, re-simulate the composed call.
- `InvalidPayments`: check the endpoint's rules for empty lists, length,
  duplicate assets, same-token legs, and swap-route shape.
- `expected a Transaction, got [object Object]`: rebuild the XDR with the same
  `@stellar/stellar-sdk` instance as the RPC server.
- Budget/storage failure after clean simulation: stale state or resources;
  re-simulate immediately before signing.

## Reproduction and completion

Use the narrowest harness target:

```bash
make test-match PATTERN=<substring>
```

`PATTERN=` is required; `MATCH=` is not recognised, and an empty `PATTERN`
exits with status 2.

Testnet: controller and pool addresses and on-chain hub and spoke ids from [`configs/networks.json`](https://github.com/XOXNO/rs-lending-xlm/blob/main/configs/networks.json),
asset addresses from
[`configs/testnet/markets.json`](https://github.com/XOXNO/rs-lending-xlm/blob/main/configs/testnet/markets.json) or
[addresses](../xoxno-lending/addresses.md). Asset identity is contract
address plus hub and spoke, never a display-symbol or search hit.

Completion checks:

- [ ] Final write was simulated with the same operations that were signed.
- [ ] Error number was mapped only after identifying the panicking contract.
- [ ] Original signed envelope/hash was persisted until its outcome was resolved.
- [ ] Success was confirmed for the original hash, not inferred from send.
- [ ] Expected events were reconciled with post-transaction state.
- [ ] Event/indexer cursor and application idempotency state were persisted.
