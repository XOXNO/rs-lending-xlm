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
Controller methods without a dedicated 1.0.228 builder can use the exported
`buildTx(opts, method, params)` with ScVals encoded by the host's
`@stellar/stellar-sdk`; verify the ABI first.

## Canonical lifecycle

1. Recheck the position-NFT owner and selected
   `(accountId, spokeId, hubId, asset)` coordinates.
2. Fetch the source account immediately before building.
3. Build unsigned XDR with its current sequence and fresh timebounds.
4. Prepare through the host RPC with `prepareStellarBuiltTx`.
5. Sign **only the prepared XDR** using the matching network passphrase.
6. Verify that the wallet signed the prepared transaction without changing it.
7. Persist the original network, signed envelope, and hash before sending.
8. Submit the signed envelope.
9. Poll the original hash after `PENDING`, `DUPLICATE`, or `TRY_AGAIN_LATER`.
10. Keep the outcome unknown after transport errors or polling limits.
11. Confirm ledger `SUCCESS` or `FAILED` for that hash.
12. After `SUCCESS`, reconcile the returned account id, indexed positions, and current NFT owner.

Always pass an explicit `attempts` value to `pollTransaction`. A bounded poll
that finds no transaction is not proof of failure.

The helper below is for the first submission of a newly signed transaction.
Recovery must use the original record and the policy below; do not treat a retry
rejection as proof that the original failed.

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

The caller supplies durable storage and the RPC for the selected network.
The helper cannot prove that input XDR was prepared or that the RPC matches the
network; the host must enforce both. A signature's presence and an unchanged
hash are local checks, not cryptographic verification of the signer identity.
Only ledger `SUCCESS` proves execution.

Keep the pending record through reloads and mobile app switching. Mark it
terminal after `SUCCESS`, `FAILED`, or a definitive first-send rejection.
Retain rejection diagnostics. Local parsing, preparation, or signing failures
before send do not submit anything.

`UNKNOWN` means send failed in transport, polling failed, or lookup stayed
`NOT_FOUND`. It does not authorize a new transaction:

1. Query the original hash on the recorded network's RPC.
2. If retrying submission, send the exact original signed XDR and keep its hash.
3. A retry `ERROR` does not prove that the original failed.
4. Do not sign a replacement while the original envelope remains valid.
5. After expiry, resolve the original hash and reconcile retained transaction
   history and account state before deciding whether a replacement is needed.
6. If available history cannot prove the outcome, keep the operation unresolved.
   Do not automatically repeat it.

XDR does not contain a network passphrase. Recovery uses the stored network even
if the wallet has switched. Expiry prevents future inclusion; it does not prove
that the transaction was never included. Serialize transactions from one Stellar
source account to avoid sequence races.

## Error interpretation

Use the contract-specific catalog in
[../xoxno-lending-troubleshooting/SKILL.md](../xoxno-lending-troubleshooting/SKILL.md).
Numeric codes overlap between controller, pool, router, NFT, governance, and
DEX contracts.

`invokedContractId` on `prepareStellarBuiltTx` prefixes a preparation error
with the contract id you pass, normally the **top-level** contract. Resolve
the actual emitter from diagnostics. Read its address and error from the same
entry. An earlier handled error can belong to another contract. Therefore:

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
