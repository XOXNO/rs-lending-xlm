# Transactions: builders and canonical lifecycle

Companion to [SKILL.md](SKILL.md). It gives one lifecycle for scripts and
frontends.
Every builder is synchronous and returns `{ xdr }`: one unsigned, unprepared
controller invocation.

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

Apply these rules to the identifiers:

- `sourceSequence` comes from
  `(await server.getAccount(caller)).sequenceNumber()`.
- `accountNonce` is the selected lending account id, which is also the
  position-NFT token id. It is unrelated to `sourceSequence`.
- `accountNonce: '0'`, or an omitted `accountNonce`, opens an account only in
  supply and multiply. `migrate_from_blend` uses `accountId: '0'`. Borrow, withdraw,
  repay, liquidation, `swap_debt`, `swap_collateral`, and
  `repay_debt_with_collateral` need an existing id.
- Take `spokeId`, `hubId`, and `asset` from the selected reserve or account.

The defaults are `fee: '100'` stroops and `timeoutSeconds: 300`. The XOXNO frontend uses
`fee: '100000'`, `timeoutSeconds: 300`; preparation adds the Soroban resource
fee.

## User builders and ABI intent

- `buildStellarSupplyTx` and `buildStellarSupplyBatchTx`: supply on a spoke.
  An omitted or zero `accountNonce` opens an account.
- `buildStellarBorrowTx` and its batch builder: existing account. The optional
  `to` defaults to the caller.
- `buildStellarWithdrawTx` and its batch builder: existing account. An amount
  of `i128::MAX` withdraws all. `'0'` fails with `AmountMustBePositive`.
- `buildStellarRepayTx` and its batch builder: existing account. There is no
  repay-all sentinel. A payment at or above the rounded-up debt burns all debt
  shares and refunds the excess.
- `buildStellarLiquidateTx`: repays target debt. Seizure mode is one of
  `'Transfer'`, `{ Credit: 0 }`, or `{ Credit: existingId }`.
- `buildStellarFlashLoanTx`: `data` is hex or `Uint8Array`. Never use base64,
  because hex decoding may silently truncate it.
- `buildStellarMultiplyTx`, `buildStellarSwapDebtTx`,
  `buildStellarSwapCollateralTx`, and
  `buildStellarRepayDebtWithCollateralTx`: see
  [strategies.md](strategies.md).
- `buildStellarMigrateFromBlendTx`: coordinates name the XOXNO destination.
  Blend assets are bare token addresses.

The [ABI reference](../xoxno-lending-contracts/abi.md) lists controller
signatures, option encoding, and return types. For a method without a builder,
verify the ABI. Then call the exported `buildTx(opts, method, params)` with
ScVals from the host's `@stellar/stellar-sdk`.

## Canonical lifecycle

Follow these steps in order:

1. Recheck the position-NFT owner and selected
   `(accountId, spokeId, hubId, asset)` coordinates.
2. Fetch the source account immediately before building.
3. Build unsigned XDR with its current sequence and fresh timebounds.
4. Prepare through the host RPC with `prepareStellarBuiltTx`.
5. Sign **only the prepared XDR** with the matching network passphrase.
6. Verify the wallet signed it unchanged.
7. Persist the network, the signed envelope, and the hash before you send.
8. Submit the signed envelope.
9. Poll the original hash after `PENDING`, `DUPLICATE`, or `TRY_AGAIN_LATER`.
10. Keep the outcome unknown after transport errors or polling limits.
11. Confirm ledger `SUCCESS` or `FAILED` for that hash.
12. After `SUCCESS`, reconcile the returned account id, the indexed positions,
    and the NFT owner.

Always pass explicit `attempts` to `pollTransaction`. A bounded poll that
finds nothing does not prove failure.

The helper covers only a first submission. Recovery uses the original record
and the policy below.

```ts
import { Transaction, TransactionBuilder, rpc } from '@stellar/stellar-sdk'
import {
  STELLAR_NETWORK_PASSPHRASE,
  type StellarNetwork,
} from '@xoxno/sdk-js/stellar-lending'

export type PendingRecord = {
  network: StellarNetwork
  hash: string
  signedXdr: string
}

export async function submitPreparedSigned(
  server: rpc.Server,
  network: StellarNetwork,
  preparedXdr: string,
  signedXdr: string,
  persistPending: (record: PendingRecord) => Promise<void>,
) {
  const passphrase = STELLAR_NETWORK_PASSPHRASE[network]
  const prepared = TransactionBuilder.fromXDR(preparedXdr, passphrase)
  const signed = TransactionBuilder.fromXDR(signedXdr, passphrase)
  if (!(signed instanceof Transaction) || signed.signatures.length === 0
      || !signed.hash().equals(prepared.hash())) {
    throw new Error('Wallet must sign the prepared transaction unchanged')
  }
  const record = { network, hash: signed.hash().toString('hex'), signedXdr }
  await persistPending(record)

  let sent: Awaited<ReturnType<typeof server.sendTransaction>>
  try {
    sent = await server.sendTransaction(signed)
  } catch {
    return { ...record, status: 'UNKNOWN' as const }
  }
  if (sent.status === 'ERROR') {
    return { ...record, status: 'REJECTED' as const, result: sent }
  }
  try {
    const result = await server.pollTransaction(record.hash, { attempts: 30 })
    if (result.status === 'SUCCESS' || result.status === 'FAILED') {
      return { ...record, status: result.status, result }
    }
  } catch {
    // A polling error cannot prove whether the transaction landed.
  }
  return { ...record, status: 'UNKNOWN' as const }
}
```

The caller supplies durable storage and the selected network's RPC. The
helper cannot prove the input was prepared or the RPC matches the network. The
host enforces both. Signature presence and an unchanged hash are local checks,
not signer verification. Only ledger `SUCCESS` proves execution.

Keep the pending record through reloads and when the user switches mobile apps.
Mark it terminal after `SUCCESS`, `FAILED`, or a definitive first-send
rejection. Keep rejection diagnostics. Failures before send submit nothing.

`UNKNOWN` means a send transport failure, a polling failure, or a persistent
`NOT_FOUND`. It never authorizes a new transaction. Follow these steps:

1. Query the original hash on the recorded network's RPC.
2. To retry, resend the exact original signed XDR. Keep its hash.
3. Do not treat a retry `ERROR` as proof that the original failed.
4. Never sign a replacement while the original envelope is valid.
5. After expiry, resolve the original hash and reconcile retained history and
   account state before deciding on a replacement.
6. If history cannot prove the outcome, keep it unresolved. Never auto-repeat.

XDR carries no network passphrase, so recover on the stored network even if the
wallet switched. Expiry blocks future inclusion but does not prove
non-inclusion. Serialize transactions per source account to avoid sequence
races.

## Error interpretation

The [troubleshooting skill](../xoxno-lending-troubleshooting/SKILL.md) holds the
error catalog. Numeric codes overlap across controller, pool, router, NFT, governance, and
DEX contracts.

`invokedContractId` on `prepareStellarBuiltTx` prefixes a preparation error
with the id you pass. This id is normally the **top-level** contract. A
controller-tagged strategy can fail in the pool, router, or a DEX. Resolve the
emitter from the diagnostics. Read the address and the error from the same
entry. Trace propagated and handled nested failures, because an earlier handled
error can belong to another contract. Apply these rules:

- Map a code only when diagnostics identify the emitter, or when the top-level
  contract is proven to be the emitter.
- Otherwise leave nested failures unmapped.
- Preserve raw diagnostic events and the numeric code.
- Never attribute a pool or router failure to the controller because of the
  top-level tag.

`mapSorobanError(raw)` is a partial, contract-blind catalog. Never call it only
because the top-level tag is the controller. `null` means unknown or unparsed,
not "no error." The safe fallback is
`Nested contract error #N; emitting contract not identified` plus the raw
diagnostic for operators.

See [Completion gates](SKILL.md#completion-gates) for the completion checklist.
