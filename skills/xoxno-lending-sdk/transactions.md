# Transactions: builders and canonical lifecycle

Companion to [SKILL.md](SKILL.md). This is the single lifecycle used by
scripts and frontends. Every builder on this page is synchronous and returns
`{ xdr }`: one unsigned, unprepared controller invocation.

## Builder options and identifiers

```ts
interface StellarLendingBuilderOptions {
  network: 'mainnet' | 'testnet'
  caller: string
  sourceSequence: string
  controllerAddress: string
  fee?: string
  timeoutSeconds?: number
}
```

- `sourceSequence` comes from
  `(await server.getAccount(caller)).sequenceNumber()`.
- `accountNonce` is the lending account id / position-NFT token id from a
  selected account. It is unrelated to `sourceSequence`.
- `accountNonce: '0'` (or omitted) opens an account only in supply and
  multiply. `migrate_from_blend` opens one with `accountId: '0'`. Borrow,
  withdraw, repay, liquidation, `swap_debt`, `swap_collateral`, and
  `repay_debt_with_collateral` require an existing id.
- `spokeId`, `hubId`, and `asset` come from the selected reserve/account
  coordinates. An account id does not encode a spoke.
- Builder amounts are decimal `i128` strings in token base units.

Defaults are `fee: '100'` stroops and `timeoutSeconds: 300`. The XOXNO
frontend uses `fee: '100000'` and `timeoutSeconds: 300`; preparation adds the
Soroban resource fee.

## User builders and ABI intent

- `buildStellarSupplyTx` / `buildStellarSupplyBatchTx`: supply on a spoke;
  omitted/zero `accountNonce` opens an account.
- `buildStellarBorrowTx` / batch: borrow from an existing account; optional
  `to` defaults to caller.
- `buildStellarWithdrawTx` / batch: withdraw from an existing account.
  `amount: '0'` is the withdraw-all sentinel.
- `buildStellarRepayTx` / batch: repay an existing account. There is no
  repay-all sentinel; a payment at or above the debt (rounded up) burns all
  debt shares and refunds any excess.
- `buildStellarLiquidateTx`: repay target debt and choose `'Transfer'`,
  `{ Credit: 0 }`, or `{ Credit: existingId }` seizure mode.
- `buildStellarFlashLoanTx`: `data` is hex or `Uint8Array`; do not pass
  base64, which may be silently truncated by hex decoding.
- `buildStellarMultiplyTx`, `buildStellarSwapDebtTx`,
  `buildStellarSwapCollateralTx`, and
  `buildStellarRepayDebtWithCollateralTx`: see
  [strategies.md](strategies.md).
- `buildStellarMigrateFromBlendTx`: migration coordinates refer to the XOXNO
  destination; Blend assets are bare token addresses.

The exact controller signatures, option encoding, and return types are in
[../xoxno-lending-contracts/abi.md](../xoxno-lending-contracts/abi.md).
Controller methods without a dedicated 1.0.214 builder can use the exported
`buildTx(opts, method, params)` with ScVals encoded by the host's
`@stellar/stellar-sdk`; verify the ABI first.

## Canonical lifecycle

1. Recheck the position-NFT owner and selected
   `(accountId, spokeId, hubId, asset)` coordinates.
2. Fetch the source account immediately before building.
3. Build unsigned XDR with its current sequence and fresh timebounds.
4. Prepare through the host RPC with `prepareStellarBuiltTx`.
5. Sign **only the prepared XDR** using the matching network passphrase.
6. Parse and retain the signed envelope; compute and persist its hash before
   sending.
7. Submit. `PENDING`, `DUPLICATE`, and `TRY_AGAIN_LATER` all mean “poll the
   original hash.”
8. Confirm `SUCCESS` or `FAILED`. Always pass `attempts` to
   `pollTransaction`: in Stellar SDK v16 the JSDoc states a default of 5
   attempts, but the runtime uses `DEFAULT_GET_TRANSACTION_TIMEOUT = 30`.
9. On `SUCCESS`, reconcile live state, indexed positions, returned account id,
   and current NFT owner.

```ts
import { Transaction, TransactionBuilder, rpc } from '@stellar/stellar-sdk'

export type SubmitOutcome =
  | { hash: string; status: 'SUCCESS'; ledger: number }
  | { hash: string; status: 'FAILED'; resultXdr?: string }
  | { hash: string; status: 'UNKNOWN'; signedXdr: string }

export async function submitPreparedSigned(
  server: rpc.Server,
  signedXdr: string,
  passphrase: string,
): Promise<SubmitOutcome> {
  // Parsing/shape errors are deterministic local failures and should throw.
  const envelope = TransactionBuilder.fromXDR(signedXdr, passphrase)
  if (!(envelope instanceof Transaction)) {
    throw new Error('expected a prepared, signed transaction envelope')
  }

  const hash = envelope.hash().toString('hex')
  persistPending(hash, signedXdr)

  let sent: Awaited<ReturnType<typeof server.sendTransaction>>
  try {
    sent = await server.sendTransaction(envelope)
  } catch {
    // A transport exception does not reveal whether the RPC accepted it.
    return { hash, status: 'UNKNOWN', signedXdr }
  }

  if (sent.status === 'ERROR') {
    const detail = sent.errorResult?.toXDR('base64') ?? 'no errorResult'
    markTerminalRejected(hash, detail)
    throw new Error(`RPC rejected envelope before inclusion: ${detail}`)
  }

  // PENDING, DUPLICATE, and TRY_AGAIN_LATER all resolve by original hash.
  let result: Awaited<ReturnType<typeof server.pollTransaction>>
  try {
    result = await server.pollTransaction(hash, { attempts: 30 })
  } catch {
    // Poll transport failure says nothing about ledger status.
    return { hash, status: 'UNKNOWN', signedXdr }
  }
  if (result.status === rpc.Api.GetTransactionStatus.SUCCESS) {
    clearPending(hash)
    return { hash, status: 'SUCCESS', ledger: result.ledger }
  }
  if (result.status === rpc.Api.GetTransactionStatus.FAILED) {
    clearPending(hash)
    return {
      hash,
      status: 'FAILED',
      resultXdr: result.resultXdr?.toXDR('base64'),
    }
  }
  return { hash, status: 'UNKNOWN', signedXdr }
}

function persistPending(hash: string, signedXdr: string): void {
  // Use durable application storage in production.
  console.info('retain pending envelope', { hash, signedXdr })
}

function clearPending(hash: string): void {
  console.info('clear terminal envelope', hash)
}

function markTerminalRejected(hash: string, detail: string): void {
  // Replace the pending record with a durable terminal-rejection record.
  console.info('mark terminal RPC rejection', { hash, detail })
}
```

`UNKNOWN` covers three cases: `sendTransaction` throws, `pollTransaction`
throws, or polling ends at `NOT_FOUND`. It is not a failure and does not
authorize a rebuild:

1. Keep the exact signed envelope and original hash.
2. Query `getTransaction(originalHash)` again.
3. If submission may not have arrived, resubmit the **unchanged signed
   envelope**, then continue checking the same hash. Do this even when the
   resubmission returns `ERROR`: the original may already have applied.
4. Never change sequence, fee, operations, or route while that envelope's
   timebounds remain valid.
5. Rebuild, re-prepare, and re-sign only after its max timebound has expired
   and the original hash has not reached `SUCCESS` or `FAILED`.

This prevents two distinct envelopes from both landing. Errors before
submission (build, parse, signing, or preparation simulation) send nothing:
fix the cause, then rebuild.

## Error interpretation

Use the contract-specific catalog in
[../xoxno-lending-troubleshooting/SKILL.md](../xoxno-lending-troubleshooting/SKILL.md).
Numeric codes overlap between controller, pool, router, NFT, governance, and
DEX contracts.

`invokedContractId` on `prepareStellarBuiltTx` prefixes a preparation error
with the contract id you pass, normally the **top-level** contract. It does
not identify a nested contract that panicked. Therefore:

- map a code only when diagnostics identify the emitting contract, or when the
  top-level contract itself is proven to be the emitter;
- leave nested failures unmapped when diagnostics do not identify the emitter;
- preserve the raw diagnostic events and numeric code;
- never turn a top-level controller tag into controller attribution for a pool
  or router failure.

`mapSorobanError(raw)` is a partial, contract-blind lending catalog. `null`
means unknown/unparsed, not “no error.” A safe fallback is
`Nested contract error #N; emitting contract not identified` plus the raw
diagnostic for operators.

The canonical completion checklist is
[SKILL.md#completion-gates](SKILL.md#completion-gates).
