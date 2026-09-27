import assert from 'node:assert/strict';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { Account, Address, Keypair, Operation, TransactionBuilder, hash, nativeToScVal, scValToNative, xdr } from '@stellar/stellar-sdk';

const script = join(dirname(fileURLToPath(import.meta.url)), 'sign_auth.mjs');
const passphrase = 'Test SDF Network ; September 2015';
const owner = Keypair.random();
const channel = Keypair.random();
const contract = Address.contract(Buffer.alloc(32, 7)).toString();
const invocation = (fn = 'create_liquidity_pool', args = [nativeToScVal(1, { type: 'u32' })]) => new xdr.InvokeContractArgs({
  contractAddress: Address.fromString(contract).toScAddress(), functionName: fn, args });
const authorized = (call, subInvocations = []) => new xdr.SorobanAuthorizedInvocation({
  function: xdr.SorobanAuthorizedFunction.sorobanAuthorizedFunctionTypeContractFn(call), subInvocations });
const addressEntry = (address = owner.publicKey(), root = authorized(invocation())) => new xdr.SorobanAuthorizationEntry({
  credentials: xdr.SorobanCredentials.sorobanCredentialsAddress(new xdr.SorobanAddressCredentials({
    address: Address.fromString(address).toScAddress(), nonce: xdr.Int64.fromString('42'),
    signatureExpirationLedger: 0, signature: xdr.ScVal.scvVoid() })),
  rootInvocation: root });
const sourceEntry = () => new xdr.SorobanAuthorizationEntry({
  credentials: xdr.SorobanCredentials.sorobanCredentialsSourceAccount(), rootInvocation: authorized(invocation()) });
const envelope = (auth, count = 1) => {
  const builder = new TransactionBuilder(new Account(channel.publicKey(), '10'), { fee: '100', networkPassphrase: passphrase });
  for (let i = 0; i < count; i += 1) {
    builder.addOperation(Operation.invokeHostFunction({ func: xdr.HostFunction.hostFunctionTypeInvokeContract(invocation()), auth }));
  }
  return builder.setTimeout(0).build().toEnvelope();
};

const work = mkdtempSync(join(tmpdir(), 'sign-auth-'));
const sign = (auth, secret = owner.secret(), expiry = '12345', count = 1) => {
  const file = join(work, 'sim.xdr');
  writeFileSync(file, `${envelope(auth, count).toXDR('base64')}\n`);
  const result = spawnSync(process.execPath, [script, file, owner.publicKey(), expiry, passphrase], { input: secret, encoding: 'utf8' });
  assert(!result.stderr.includes(secret), 'secret leaked to stderr');
  return result;
};
const rejects = (auth, message, secret, expiry, count) => {
  const result = sign(auth, secret, expiry, count);
  assert.notEqual(result.status, 0, message);
  assert.equal(result.stdout, '', message);
  assert.match(result.stderr, new RegExp(message), result.stderr);
};

try {
  const input = [sourceEntry(), addressEntry()];
  const result = sign(input);
  assert.equal(result.status, 0, result.stderr);
  const output = xdr.TransactionEnvelope.fromXDR(result.stdout.trim(), 'base64');
  const op = output.v1().tx().operations()[0].body().invokeHostFunctionOp();
  const [source, signed] = op.auth();
  assert.equal(source.toXDR('base64'), input[0].toXDR('base64'));
  const credentials = signed.credentials().address();
  assert.equal(credentials.signatureExpirationLedger(), 12345);
  assert.equal(signed.rootInvocation().toXDR('base64'), input[1].rootInvocation().toXDR('base64'));
  const payload = network => hash(xdr.HashIdPreimage.envelopeTypeSorobanAuthorization(new xdr.HashIdPreimageSorobanAuthorization({
    networkId: hash(Buffer.from(network)), nonce: credentials.nonce(), signatureExpirationLedger: 12345,
    invocation: signed.rootInvocation() })).toXDR());
  const [signature] = scValToNative(credentials.signature());
  assert.deepEqual(Buffer.from(signature.public_key), owner.rawPublicKey());
  assert(owner.verify(payload(passphrase), Buffer.from(signature.signature)));
  assert(!owner.verify(payload('Public Global Stellar Network ; September 2015'), Buffer.from(signature.signature)));
  op.auth(input);
  assert.equal(output.toXDR('base64'), envelope(input).toXDR('base64'));
  console.log('The owner entry is signed for the given expiry and passphrase; source entries pass through unchanged');

  rejects([addressEntry(Keypair.random().publicKey())], 'auth entry address is not the owner');
  rejects([addressEntry(contract)], 'auth entry address is not the owner');
  rejects([addressEntry(undefined, authorized(invocation('set_oracle')))], 'root invocation differs from the operation');
  rejects([addressEntry(undefined, authorized(invocation(undefined, [nativeToScVal(2, { type: 'u32' })])))], 'root invocation differs from the operation');
  rejects([addressEntry(undefined, authorized(invocation(), [authorized(invocation('transfer'))]))], 'root invocation has sub-invocations');
  rejects([sourceEntry()], 'no owner auth entry to sign');
  rejects([], 'no owner auth entry to sign');
  rejects([addressEntry()], 'the secret does not belong to the owner', Keypair.random().secret());
  rejects([addressEntry()], 'expected exactly one operation', undefined, undefined, 2);
  for (const expiry of ['0', '', '1e6', '4294967296']) rejects([addressEntry()], 'invalid expiry ledger', undefined, expiry);
  const v2 = addressEntry();
  v2.credentials(xdr.SorobanCredentials.sorobanCredentialsAddressV2(v2.credentials().address()));
  rejects([v2], 'address not set');
  console.log('Foreign addresses, other invocations, sub-invocations, no owner entry, a foreign secret, extra operations, bad expiries and V2 credentials are refused');
} finally {
  rmSync(work, { recursive: true, force: true });
}
