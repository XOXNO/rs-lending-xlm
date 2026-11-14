# Transactions: builders and canonical lifecycle

Companion to [SKILL.md](SKILL.md); one lifecycle for scripts and frontends.
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

- `sourceSequence` comes from
  `(await server.getAccount(caller)).sequenceNumber()`.
- `accountNonce`: selected lending account id / position-NFT token id,
  unrelated to `sourceSequence`.
- `accountNonce: '0'` (or omitted) opens an account only in supply and
  multiply; `migrate_from_blend` uses `accountId: '0'`. Borrow, withdraw,
  repay, liquidation, `swap_debt`, `swap_collateral`, and
  `repay_debt_with_collateral` need an existing id.
- `spokeId`, `hubId`, `asset`: from the selected reserve/account.

Defaults: `fee: '100'` stroops, `timeoutSeconds: 300`. The XOXNO frontend uses
`fee: '100000'`, `timeoutSeconds: 300`; preparation adds the Soroban resource
fee.

## User builders and ABI intent

- `buildStellarSupplyTx` / `buildStellarSupplyBatchTx`: supply on a spoke;
  omitted/zero `accountNonce` opens an account.
- `buildStellarBorrowTx` / batch: existing account; optional `to` defaults to
  caller.
- `buildStellarWithdrawTx` / batch: existing account; `amount: '0'` withdraws
  all.
- `buildStellarRepayTx` / batch: existing account. No repay-all sentinel; a
  payment at or above the debt (rounded up) burns all debt shares and refunds
  the excess.
- `buildStellarLiquidateTx`: repays target debt; seizure mode `'Transfer'`,
  `{ Credit: 0 }`, or `{ Credit: existingId }`.
- `buildStellarFlashLoanTx`: `data` is hex or `Uint8Array`; never base64,
  which hex decoding may silently truncate.
- `buildStellarMultiplyTx`, `buildStellarSwapDebtTx`,
  `buildStellarSwapCollateralTx`, and
  `buildStellarRepayDebtWithCollateralTx`: see
  [strategies.md](strategies.md).
- `buildStellarMigrateFromBlendTx`: coordinates name the XOXNO destination;
  Blend assets are bare token addresses.

Controller signatures, option encoding, and return types:
[../xoxno-lending-contracts/abi.md](../xoxno-lending-contracts/abi.md). Methods
without a builder: verify the ABI, then call the exported
`buildTx(opts, method, params)` with ScVals from the host's
`@stellar/stellar-sdk`.

## Canonical lifecycle

1. Recheck the position-NFT owner and selected
   `(accountId, spokeId, hubId, asset)` coordinates.
2. Fetch the source account immediately before building.
3. Build unsigned XDR with its current sequence and fresh timebounds.
4. Prepare via host RPC with `prepareStellarBuiltTx`.
5. Sign **only the prepared XDR** with the matching network passphrase.
6. Verify the wallet signed it unchanged.
7. Persist network, signed envelope, and hash before sending.
8. Submit the signed envelope.
9. Poll the original hash after `PENDING`, `DUPLICATE`, or `TRY_AGAIN_LATER`.
10. Keep the outcome unknown after transport errors or polling limits.
11. Confirm ledger `SUCCESS` or `FAILED` for that hash.
12. After `SUCCESS`, reconcile returned account id, indexed positions, and NFT owner.

Always pass explicit `attempts` to `pollTransaction`; a bounded poll that
finds nothing does not prove failure.

The helper covers only a first submission; recovery uses the original record
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
helper cannot prove the input was prepared or the RPC matches the network; the
host enforces both. Signature presence and an unchanged hash are local checks,
not signer verification. Only ledger `SUCCESS` proves execution.

Keep the pending record through reloads and mobile app switching. Mark it
terminal after `SUCCESS`, `FAILED`, or a definitive first-send rejection;
keep rejection diagnostics. Failures before send submit nothing.

`UNKNOWN` (send transport failure, polling failure, or persistent `NOT_FOUND`)
never authorizes a new transaction:

1. Query the original hash on the recorded network's RPC.
2. To retry, resend the exact original signed XDR; keep its hash.
3. A retry `ERROR` does not prove the original failed.
4. Never sign a replacement while the original envelope is valid.
5. After expiry, resolve the original hash and reconcile retained history and
   account state before deciding on a replacement.
6. If history cannot prove the outcome, keep it unresolved; never auto-repeat.

XDR carries no network passphrase: recover on the stored network even if the
wallet switched. Expiry blocks future inclusion but does not prove
non-inclusion. Serialize transactions per source account to avoid sequence
races.

## Error interpretation

Catalog: [../xoxno-lending-troubleshooting/SKILL.md](../xoxno-lending-troubleshooting/SKILL.md).
Numeric codes overlap across controller, pool, router, NFT, governance, and
DEX contracts.

`invokedContractId` on `prepareStellarBuiltTx` prefixes a preparation error
with the id you pass, normally the **top-level** contract; a controller-tagged
strategy can fail in the pool, router, or a DEX. Resolve the emitter from
diagnostics, reading address and error from the same entry, and trace
propagated and handled nested failures (an earlier handled error can belong
to another contract). So:

- map a code only when diagnostics identify the emitter, or the top-level
  contract is proven to be it;
- otherwise leave nested failures unmapped;
- preserve raw diagnostic events and the numeric code;
- never attribute a pool or router failure to the controller because of the
  top-level tag.

`mapSorobanError(raw)` is a partial, contract-blind catalog; never call it just
because the top-level tag is the controller. `null` means unknown/unparsed,
not “no error.” Safe fallback:
`Nested contract error #N; emitting contract not identified` plus the raw
diagnostic for operators.

Completion checklist: [SKILL.md#completion-gates](SKILL.md#completion-gates).
