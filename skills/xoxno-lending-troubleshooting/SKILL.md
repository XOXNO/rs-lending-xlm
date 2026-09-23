---
name: xoxno-lending-troubleshooting
description: 'Use when an XOXNO Lending call fails — Error(Contract, #NNN), a simulation or transaction reverts, authorization fails, submission returns a lifecycle status, an archived entry needs restoration, or an integration must be reproduced locally or on testnet.'
user-invocable: true
argument-hint: "[error string, hash, or test setup]"
---

# XOXNO Lending troubleshooting

## Decision tree

1. **Did simulation fail?**
   - If it returned a contract error, capture the full diagnostic events and
     identify the panicking contract before mapping the numeric code.
   - If it reports archived footprint entries, follow
     [Archived entries](#archived-entries).
   - If simulation succeeds, preserve that exact simulated/prepared envelope;
     do not rebuild different operations before signing.
2. **Was submission rejected with `ERROR`?**
   Decode `errorResult` for the transaction-level cause and
   `diagnosticEvents` for any contract cause. Refresh sequence only after the
   submitted envelope is known not to have been accepted.
3. **Was submission `PENDING`, `DUPLICATE`, or `TRY_AGAIN_LATER`?**
   Persist the original signed envelope and hash. Poll the hash and resubmit
   only that identical envelope while it remains live. Do not build a
   replacement yet.
4. **Does `getTransaction(hash)` return `NOT_FOUND`?**
   This is not a terminal failure. Keep polling the hash and resubmit the
   original envelope until it confirms or its timebounds expire. Rebuild only
   after expiry or another definitive terminal result.
5. **Did the transaction confirm `FAILED`?**
   Decode its diagnostic events, again pairing code with panicking contract.
6. **Did it confirm `SUCCESS`?**
   Verify the original hash, decode the return value, and reconcile expected
   events with current state. A send status alone is not success.

Canonical catalogs:

- All error variants and conditions:
  [docs/reference/errors.md](../../docs/reference/errors.md)
- All event topics, payloads, units, and ordering:
  [docs/reference/events.md](../../docs/reference/events.md)

This skill keeps only operational interpretation and failure-handling rules.

## Identify the panicking contract first

`Error(Contract, #N)` names a number, not an error namespace. A lending call
can traverse the controller, pool, price aggregator, oracle, token, position
NFT, router, and DEX contracts, and their codes overlap.

1. Extract only `Error(Contract, #N)`.
2. Locate the diagnostic `error` event and its emitting `contractId` (or the
   adjacent `contract:C…` diagnostic log).
3. Match that address against the deployment manifest and invoked contracts.
4. Only then map `N` using the canonical error reference.

Do not maintain a "non-user/collision" set containing controller #101 or #114:

- controller `HealthFactorTooHigh` #101 legitimately occurs when a liquidation
  target has no debt or is no longer liquidatable;
- controller `CannotCleanBadDebt` #114 legitimately occurs on the
  permissionless `clean_bad_debt` path when debt does not exceed collateral
  or collateral is above the dust cap.

Either number can also collide with another contract. The emitting contract,
not assumptions about reachability, decides the namespace.

Host errors such as `Error(Auth, ...)`, `Error(Storage, ...)`, budget errors,
and Wasm traps are not lending contract codes.

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
    return { kind: 'restore' as const, preamble: simulation.restorePreamble }
  }
  if (!rpc.Api.isSimulationError(simulation)) {
    return { kind: 'ok' as const, simulation }
  }

  const events = humanizeEvents(simulation.events)
  const diagnostic = events.find(
    (event) => event.type === 'diagnostic' && event.topics[0] === 'error',
  )
  const code = simulation.error.match(CONTRACT_CODE)?.[1]
  return {
    kind: 'error' as const,
    code: code === undefined ? null : Number(code),
    contractId: diagnostic?.contractId ?? null,
    events,
    raw: simulation.error,
  }
}
```

An SDK helper that maps the raw number without the diagnostic contract id is a
hint only. Never present its lending name as final diagnosis.

## Submission lifecycle

Before the first send, durably persist:

- the signed envelope XDR;
- its transaction hash;
- sequence and timebounds;
- the operation's application idempotency key.

Interpret statuses as follows:

| Observation | Required action |
|---|---|
| `ERROR` | Decode the transaction result and diagnostics. It was rejected by this RPC submission. Check the original hash before deciding sequence reuse is safe. |
| `PENDING` | Poll the original hash. |
| `DUPLICATE` | Poll the original hash; do not submit a replacement or show another success. |
| `TRY_AGAIN_LATER` | Check the original hash, then retry the identical signed envelope; preserve it until terminal or expired. |
| `getTransaction NOT_FOUND` | Keep checking/resubmitting the original envelope while its timebounds remain valid. |
| confirmed `FAILED` | Decode result and diagnostics; terminal for this hash. |
| confirmed `SUCCESS` | Verify return, events, and post-state; terminal for this hash. |

After the original envelope expires, verify it is still unconfirmed, then fetch
the current account sequence, rebuild, re-simulate, re-sign, persist the new
hash, and submit. This prevents duplicate effects from two distinct envelopes.

`txBAD_SEQ` commonly means another envelope consumed the sequence. `txTOO_LATE`
means the original timebounds expired. Neither justifies forgetting another
still-live signed envelope.

## Archived entries

Use one rule:

**On protocol 23 and later, simulate the invocation, assemble it with the
simulation result, and submit it; archived footprint entries are restored
inline and the submitter pays restoration rent.**

A separate `restoreFootprint` transaction is only for a footprint constructed
by hand and submitted without the normal simulation/assembly path. If an
integration deliberately does that, submit the explicit restore, wait for its
confirmation, and re-simulate the invocation against the restored state.

For dormant lending accounts, the archived entry is often the position NFT
owner record. The permissionless NFT `renew(token_id)` can proactively extend
it. Do not turn every simulation restore response into a separate restore
transaction.

## Authorization ordering in calling contracts

The controller's `caller.require_auth()` is satisfied when the calling
contract is the direct invoker. Nested pulls such as
`token.transfer(caller, pool, amount)` still require exact authorization.

Perform all controller views, token balances, configuration reads, and
lookups first. Then call `authorize_as_current_contract` for the exact nested
transfer and invoke the controller immediately. Any outbound contract call
between those steps can consume the "next invocation" authorization position
and produce `Error(Auth, InvalidAction)`.

For contract liquidators, do not repeat a token address across hub-specific
debt payment legs. `LiquidationEstimate` refunds name only the token address,
not the hub. Authorization binds the token `transfer` arguments, not the hub
id, so the contract cannot derive and authorize each repeated-address leg.

## Common operational interpretations

- `SpokeNotFound`: resolve the controller address and the on-chain spoke id
  (not the config key) from the same network manifest; never use spoke 0.
- `SpokeMismatch`: load the account's stored spoke and keep all account legs in
  that spoke.
- Pause/freeze/no-seize failures: read the exact spoke-asset configuration.
  Seizure is pro-rata across every collateral, so one `no_seize` collateral
  blocks the whole liquidation.
- Router slippage or venue errors: identify the router/venue contract, fetch a
  fresh quote, rebuild route XDR, and re-simulate the composed call.
- `InvalidPayments`: check the exact endpoint's rules for empty lists, list
  length, duplicate assets, same-token legs, and swap-route shape.
- `expected a Transaction, got [object Object]`: rebuild the XDR through the
  same `@stellar/stellar-sdk` instance used by the RPC server before
  preparation.
- Budget/storage failure after clean simulation: ledger state changed or
  resource estimates became stale; re-simulate the still-unsubmitted
  operation immediately before signing.

## Reproduction and completion

Use the narrowest harness target:

```bash
make test-match PATTERN=<substring>
```

`PATTERN=` is required. `MATCH=` is not recognised, and the target exits with
status 2 when `PATTERN` is empty.

For testnet, resolve the controller and pool addresses and the on-chain hub
and spoke ids from `configs/networks.json`, and asset addresses from
`configs/testnet/markets.json` or [addresses](../xoxno-lending/addresses.md).
Asset identity is the contract address plus explicit hub and spoke context,
never the first display-symbol or search result.

Completion checks:

- [ ] Final write was simulated with the same operations that were signed.
- [ ] Error number was mapped only after identifying the panicking contract.
- [ ] Original signed envelope/hash was persisted through terminal or expiry.
- [ ] Success was confirmed for the original hash, not inferred from send.
- [ ] Expected events were reconciled with post-transaction state.
- [ ] Event/indexer cursor and application idempotency state were persisted.
