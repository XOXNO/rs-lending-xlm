import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { Address, Keypair, authorizeEntry, xdr } from '@stellar/stellar-sdk';

const [file, owner, expiryText, passphrase] = process.argv.slice(2);
assert.match(expiryText ?? '', /^[1-9][0-9]{0,8}$/, 'invalid expiry ledger');
const keypair = Keypair.fromSecret(readFileSync(0, 'utf8').trim());
assert.equal(keypair.publicKey(), owner, 'the secret does not belong to the owner');

const envelope = xdr.TransactionEnvelope.fromXDR(readFileSync(file, 'utf8').trim(), 'base64');
const operations = envelope.v1().tx().operations();
assert.equal(operations.length, 1, 'expected exactly one operation');
const op = operations[0].body().invokeHostFunctionOp();
const call = op.hostFunction().invokeContract().toXDR('base64');

const auth = [];
for (const entry of op.auth()) {
  if (entry.credentials().switch().name === 'sorobanCredentialsSourceAccount') {
    auth.push(entry);
    continue;
  }
  assert.equal(Address.fromScAddress(entry.credentials().address().address()).toString(), owner, 'auth entry address is not the owner');
  const root = entry.rootInvocation();
  assert.equal(root.function().contractFn().toXDR('base64'), call, 'root invocation differs from the operation');
  assert.equal(root.subInvocations().length, 0, 'root invocation has sub-invocations');
  auth.push(await authorizeEntry(entry, keypair, Number(expiryText), passphrase));
}
assert(auth.some(entry => entry.credentials().switch().name !== 'sorobanCredentialsSourceAccount'), 'no owner auth entry to sign');
op.auth(auth);
process.stdout.write(`${envelope.toXDR('base64')}\n`);
