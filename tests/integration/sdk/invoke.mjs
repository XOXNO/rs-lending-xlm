import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { setTimeout as delay } from 'node:timers/promises';
import { execFileSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { Keypair, rpc, TransactionBuilder, Networks, scValToNative } from '@stellar/stellar-sdk';
import * as sdk from '@xoxno/sdk-js/stellar-lending';
import { receiptAccountId } from './account.mjs';

assert.equal(process.versions.node.split('.')[0], '24', 'SDK lane requires Node 24');
for (const [name,version] of [['@xoxno/sdk-js','1.0.221'],['@stellar/stellar-sdk','16.0.1']]) {
  assert.equal(JSON.parse(readFileSync(new URL(`./node_modules/${name}/package.json`,import.meta.url),'utf8')).version,version);
}
const [builder, argsJSON, evidence] = process.argv.slice(2);
const key = Keypair.fromSecret(readFileSync(0, 'utf8').trim());
const caller = key.publicKey();
const server = new rpc.Server(process.env.RPC_URL);
const controllerAddress = process.env.CONTROLLER;
const json = value => JSON.stringify(value, (_, v) => typeof v === 'bigint' ? v.toString() : v, 2);
// Observe the native RPC boundary; return the exact response to the preparer.
const simulate = server.simulateTransaction.bind(server);
let simulation = 0;
server.simulateTransaction = async (...args) => {
  const prefix = `${evidence}.simulation-${++simulation}`;
  let response;
  try { response = await simulate(...args); }
  catch (error) {
    writeFileSync(`${prefix}.error.txt`, String(error));
    throw error;
  }
  writeFileSync(`${prefix}.json`, json(response) ?? 'null');
  if (response?.transactionData) {
    try {
      const data = response.transactionData;
      const encoded = typeof data === 'string' ? data : data.build().toXDR('base64');
      writeFileSync(`${prefix}.transaction-data.xdr`, encoded);
      const resources = JSON.parse(execFileSync('stellar', ['xdr','decode','--type','SorobanTransactionData','--output','json'], {input:encoded,encoding:'utf8'}));
      writeFileSync(`${prefix}.resources.json`, json({latestLedger:response.latestLedger,minResourceFee:response.minResourceFee,...resources}));
    } catch (error) {
      // Malformed evidence must still reach the SDK's own validation unchanged.
      writeFileSync(`${prefix}.resources-error.txt`, String(error));
    }
  }
  return response;
};
const account = await server.getAccount(caller);
const options = {network: 'testnet', caller, sourceSequence: account.sequenceNumber(), controllerAddress, fee: '1000000'};
assert.equal(typeof sdk[builder], 'function', `published builder missing: ${builder}`);
const built = sdk[builder](options, JSON.parse(argsJSON));
writeFileSync(`${evidence}.unsigned.xdr`, built.xdr);
let prepared;
try {
  prepared = await sdk.prepareStellarTxXdr(server, built.xdr, {network: 'testnet', invokedContractId: controllerAddress});
} catch (error) {
  const mapped = sdk.mapSorobanError(String(error));
  writeFileSync(`${evidence}.prepare-error.json`, json({error: String(error), mapped}));
  if (process.env.EXPECT_ERROR) {
    assert.equal(mapped?.name, process.env.EXPECT_ERROR);
    console.log(json({expectedError: mapped}));
    process.exit(0);
  }
  throw error;
}
assert(!process.env.EXPECT_ERROR, 'expected contract error did not occur');
writeFileSync(`${evidence}.prepared.xdr`, prepared);
if (process.env.DELAY_SIGN_SECONDS) await delay(Number(process.env.DELAY_SIGN_SECONDS) * 1000);
const transaction = TransactionBuilder.fromXDR(prepared, Networks.TESTNET);
transaction.sign(key);
const hash = transaction.hash().toString('hex');
writeFileSync(`${evidence}.hash`, hash);
// Submit exactly once. Every uncertain result reconciles this same hash.
try {
  const sent = await server.sendTransaction(transaction);
  writeFileSync(`${evidence}.submission.json`, json(sent));
} catch (error) {
  writeFileSync(`${evidence}.submission-error.txt`, String(error));
}
let receipt;
for (let attempt = 0; attempt < 30; attempt++) {
  try { receipt = await server.getTransaction(hash); }
  catch (error) { writeFileSync(`${evidence}.poll-${attempt}.txt`, String(error)); }
  if (receipt) writeFileSync(`${evidence}.receipt.json`, json(receipt));
  if (receipt?.status === 'SUCCESS' || receipt?.status === 'FAILED') break;
  await delay(2000);
}
// Save the RPC wire receipt as well as the SDK's decoded receipt. The common
// gate checks the same committed resources for both CLI and SDK transactions.
// Save failures too, before any success assertion can discard their evidence.
const logs = dirname(evidence);
let wire;
try {
  const response = await fetch(process.env.RPC_URL, {method:'POST', headers:{'Content-Type':'application/json'},
    body:JSON.stringify({jsonrpc:'2.0',id:1,method:'getTransaction',params:{hash}})});
  wire = await response.json();
  writeFileSync(join(logs, `${hash}.receipt.json`), json(wire));
  assert(response.ok, `receipt HTTP ${response.status}`);
} catch (error) {
  writeFileSync(`${evidence}.receipt-error.txt`, String(error));
  throw error;
}
assert.equal(receipt?.status, 'SUCCESS', `unconfirmed/failed transaction ${hash}`);
assert(receipt.returnValue && receipt.resultMetaXdr && receipt.envelopeXdr, 'incomplete receipt');
assert.equal(wire.jsonrpc, '2.0');
assert.equal(wire.id, 1);
assert(!wire.error);
assert.equal(wire.result.txHash, hash);
assert.equal(wire.result.status, 'SUCCESS');
const envelope = JSON.parse(execFileSync('stellar', ['xdr','decode','--type','TransactionEnvelope','--output','json'], {input:wire.result.envelopeXdr,encoding:'utf8'}));
function resourceData(value) {
  if (!value || typeof value !== 'object') return;
  if (value.resources) return value;
  for (const child of Object.values(value)) { const data = resourceData(child); if (data) return data; }
}
const resources = resourceData(envelope);
assert(resources, 'committed resource declaration missing');
writeFileSync(join(logs, `${hash}.resources.json`), json(resources));
execFileSync('python3', [new URL('../resources.py', import.meta.url).pathname,
  join(dirname(logs),'network-limits.json'),join(logs,`${hash}.resources.json`),join(logs,`${hash}.receipt.json`)]);
const events = receipt.events.contractEventsXdr.flat().map(event => {
  const body = event.body().v0();
  return sdk.decodeStellarLendingEvent(body.topics().map(v => v.toXDR('base64')), body.data().toXDR('base64'));
}).filter(Boolean);
assert(events.length > 0, 'SDK decoded no lending events');
const value = scValToNative(receipt.returnValue);
const args = JSON.parse(argsJSON);
const accountId = receiptAccountId(args, value);
const position = events.find(e => e.topic === 'position:batch_update' && e.data.accountId === accountId);
assert(position, `missing decoded position event for account ${accountId}`);
assert.equal(position.data.accountAttributes.owner, caller);
const expectedAction = {buildStellarSupplyTx:'supply',buildStellarBorrowTx:'borrow',buildStellarRepayTx:'repay',buildStellarWithdrawTx:'withdraw'}[builder];
if (expectedAction) {
  const update = position.data.updates.find(u => u.action === expectedAction && u.asset === args.asset && u.hubId === args.hubId);
  assert(update, `missing ${expectedAction} event for requested market`);
  assert(BigInt(update.amount) > 0n);
  if (expectedAction === 'supply' || expectedAction === 'borrow') assert.equal(update.amount, args.amount);
  if (expectedAction === 'repay') assert(BigInt(update.amount) <= BigInt(args.amount));
}
const result = {hash, value, events};
writeFileSync(`${evidence}.result.json`, json(result));
console.log(json(result));
