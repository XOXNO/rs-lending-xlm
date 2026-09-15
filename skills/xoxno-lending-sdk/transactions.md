# Transactions: builders and canonical lifecycle

Companion to [SKILL.md](SKILL.md). This is the single lifecycle used by
scripts and frontends. Every `buildStellar*Tx` call is synchronous and returns
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
  `server.getAccount(caller).sequenceNumber()`.
- `accountNonce` is the lending account id / position-NFT token id from a
  selected account. It is unrelated to `sourceSequence`.
- `accountNonce: '0'` may open an account only for supply/multiply paths that
  accept it. Borrow, withdraw, repay, liquidation, and strategy swaps require
  an existing id.
- `spokeId`, `hubId`, and `asset` come from the selected reserve/account
  coordinates. An account id does not encode a spoke.
- Builder amounts are decimal `i128` strings in token base units.

Defaults are `fee: '100'` stroops and `timeoutSeconds: 300`. Production wallet
flows commonly use `fee: '100000'` and 300-second timebounds; preparation adds
the Soroban resource fee.

## User builders and ABI intent

- `buildStellarSupplyTx` / `buildStellarSupplyBatchTx`: supply on a spoke;
  omitted/zero `accountNonce` opens an account.
- `buildStellarBorrowTx` / batch: borrow from an existing account; optional
  `to` defaults to caller.
- `buildStellarWithdrawTx` / batch: withdraw from an existing account.
  `amount: '0'` is the withdraw-all sentinel.
- `buildStellarRepayTx` / batch: repay an existing account. There is no
  repay-all sentinel; overpaying the ceiled debt closes shares and refunds
  excess.
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
Methods without a dedicated 1.0.214 builder can use exported `buildTx` with
ScVals encoded by the host's `@stellar/stellar-sdk`; verify the ABI first.

## Canonical lifecycle

1. Recheck the position-NFT owner and selected
   `(accountId, spokeId, hubId, asset)` coordinates.
2. Fetch the source account immediately before building.
3. Build unsigned XDR with its current sequence and fresh timebounds.
4. Prepare through the host RPC with `prepareStellarBuiltTx`.
5. Sign **only the prepared XDR** using the matching network passphrase.
6. Parse and retain the signed envelope; compute and persist its hash before
   sending.
7. Submit. `PENDING` and `DUPLICATE` both mean “poll the original hash.”
8. Confirm `SUCCESS` or `FAILED`. In Stellar SDK v16.0.0, the JSDoc says the
   `pollTransaction` default is 5 attempts, while the published ESM runtime
   uses `DEFAULT_GET_TRANSACTION_TIMEOUT = 30` as the default attempt count.
   Do not rely on that discrepancy: always pass `attempts` explicitly.
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

`UNKNOWN` includes polling `NOT_FOUND`, a polling transport exception, a
timeout, and send transport uncertainty. It is not failure and does not
authorize a rebuild:

1. Keep the exact signed envelope and original hash.
2. Query `getTransaction(originalHash)` again.
3. If submission may not have arrived, resubmit the **unchanged signed
   envelope**, then continue checking the same hash.
4. Never change sequence, fee, operations, or route while that envelope's
   timebounds remain valid.
5. Rebuild, re-prepare, and re-sign only after its max timebound has expired
   and the original hash has not reached `SUCCESS` or `FAILED`.

This prevents two distinct envelopes from both landing. A deterministic local
build/parse/signing error can be fixed before submission. A preparation
simulation error is also pre-submission and can be rebuilt after its cause is
fixed. `sendTransaction` transport errors and `NOT_FOUND` are uncertain
network observations and follow the retain/resubmit policy above. The same is
true of every exception thrown while polling: resume with the retained
envelope and original hash, never a replacement.

## Error interpretation

Use the contract-specific catalog in
[../xoxno-lending-troubleshooting/SKILL.md](../xoxno-lending-troubleshooting/SKILL.md).
Numeric codes overlap between controller, pool, router, NFT, governance, and
DEX contracts.

`invokedContractId` on `prepareStellarBuiltTx` tags only the **top-level**
contract invoked by the envelope. It does not identify a nested contract that
panicked. Therefore:

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
