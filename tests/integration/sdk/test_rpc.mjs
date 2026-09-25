// Controlled RPC fixtures. No network, signing, or multiweek observation.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import {Account,Address,Keypair,Networks,Operation,SorobanDataBuilder,TransactionBuilder,rpc,xdr} from '@stellar/stellar-sdk';
import {buildStellarSupplyTx,prepareStellarTxXdr,mapSorobanError} from '@xoxno/sdk-js/stellar-lending';
assert.equal(process.versions.node.split('.')[0],'24');
for (const [name,version] of [['@xoxno/sdk-js','1.0.221'],['@stellar/stellar-sdk','16.3.0']]) {
  assert.equal(JSON.parse(readFileSync(new URL(`./node_modules/${name}/package.json`,import.meta.url),'utf8')).version,version);
}
assert.deepEqual(mapSorobanError('simulation failed: HostError: Error(Contract, #14)'),{
  code:14,name:'AmountMustBePositive',message:'The amount must be greater than zero.',
});
const caller=Keypair.fromRawEd25519Seed(Buffer.alloc(32,1)).publicKey();
const controller=Address.contract(Buffer.alloc(32,2)).toString();
const asset=Address.contract(Buffer.alloc(32,3)).toString();
const build=(sequence,accountNonce=0)=>buildStellarSupplyTx({network:'testnet',caller,sourceSequence:sequence,controllerAddress:controller},
  {asset,hubId:1,spokeId:1,amount:'10000001',accountNonce});
assert.equal(build('11','0').xdr,build('11',0).xdr);
const key=xdr.LedgerKey.contractData(new xdr.LedgerKeyContractData({contract:new Address(controller).toScAddress(),key:xdr.ScVal.scvLedgerKeyContractInstance(),durability:xdr.ContractDataDurability.persistent()}));
const restoreData=new SorobanDataBuilder().setReadWrite([key]).setResources(100000,200,400).setResourceFee('500').build();
const callData=new SorobanDataBuilder().setReadWrite([key]).setResources(200000,100,200).setResourceFee('300').build();
const success={latestLedger:100,minResourceFee:'300',transactionData:callData.toXDR('base64'),results:[{auth:[],xdr:xdr.ScVal.scvU64(xdr.Uint64.fromString('1')).toXDR('base64')}]};
const archived={...success,restorePreamble:{minResourceFee:'500',transactionData:restoreData.toXDR('base64')}};
assert(rpc.Api.isSimulationRestore(archived));
const server=new rpc.Server('https://rpc.invalid');
server.sendTransaction=()=>{throw new Error('offline fixture must never submit');};
// Restore transaction has its own sequence and footprint, prepared by native SDK.
const restore=new TransactionBuilder(new Account(caller,'10'),{fee:'100',networkPassphrase:Networks.TESTNET})
  .addOperation(Operation.restoreFootprint({})).setSorobanData(restoreData).setTimeout(60).build();
server.simulateTransaction=async()=>({...success,transactionData:restoreData.toXDR('base64'),minResourceFee:'500',results:[]});
const preparedRestore=TransactionBuilder.fromXDR(await prepareStellarTxXdr(server,restore.toXDR(),{network:'testnet'}),Networks.TESTNET);
assert.equal(preparedRestore.sequence,'11');
assert.equal(preparedRestore.operations[0].type,'restoreFootprint');
assert.equal(preparedRestore.toEnvelope().v1().tx().ext().sorobanData().toXDR('base64'),restoreData.toXDR('base64'));
assert.equal(preparedRestore.signatures.length,0);
// Refreshed source sequence after controlled restore; SDK owns all envelope data.
server.simulateTransaction=async()=>success;
const prepared=TransactionBuilder.fromXDR(await prepareStellarTxXdr(server,build('11').xdr,{network:'testnet',invokedContractId:controller}),Networks.TESTNET);
assert.equal(prepared.sequence,'12');
assert.equal(prepared.signatures.length,0);
assert.equal(prepared.toEnvelope().v1().tx().ext().sorobanData().toXDR('base64'),callData.toXDR('base64'));
for(const malformed of [{},null,{error:'restore failed'},{...success,transactionData:'invalid-base64'},{...success,results:[]}]) {
  server.simulateTransaction=async()=>malformed;
  await assert.rejects(()=>prepareStellarTxXdr(server,build('11').xdr,{network:'testnet',invokedContractId:controller}));
}
console.log('Controlled RPC restore preparation, refreshed sequence, malformed simulation checks passed');

// Published ESM/CJS entrypoints must request native leeway through the pinned
// host SDK's real RPC parser, preserving the returned fee and unsigned call.
const require=createRequire(import.meta.url);
const originalFetch=globalThis.fetch;
try {
  for(const sdk of [await import('@xoxno/sdk-js'),await import('@xoxno/sdk-js/stellar-lending'),
    require('@xoxno/sdk-js'),require('@xoxno/sdk-js/stellar-lending')]) {
    const built=build('11');
    let requests=0;
    globalThis.fetch=async(url,init)=>{
      assert.equal(String(url),'https://rpc.invalid/');
      const request=JSON.parse(init.body);
      assert.equal(request.method,'simulateTransaction');
      assert.equal(request.params.transaction,built.xdr);
      assert.deepEqual(request.params.resourceConfig,{instructionLeeway:20000000});
      requests++;
      return new Response(JSON.stringify({jsonrpc:'2.0',id:request.id,result:success}),{status:200});
    };
    const encoded=await sdk.prepareStellarTxXdr(new rpc.Server('https://rpc.invalid'),built.xdr,{network:'testnet'});
    const before=TransactionBuilder.fromXDR(built.xdr,Networks.TESTNET);
    const after=TransactionBuilder.fromXDR(encoded,Networks.TESTNET);
    assert.equal(requests,1);
    assert.equal(after.signatures.length,0);
    assert.equal(BigInt(after.fee),BigInt(before.fee)+300n);
    assert.deepEqual(after.toEnvelope().v1().tx().operations(),before.toEnvelope().v1().tx().operations());
    assert.equal(after.toEnvelope().v1().tx().ext().sorobanData().toXDR('base64'),callData.toXDR('base64'));
  }
} finally { globalThis.fetch=originalFetch; }
console.log('Published ESM/CJS root/lending entrypoints request native 20M leeway and preserve unsigned simulation resources');

// String and numeric zero both select the newly returned account identity.
const {receiptAccountId}=await import('./account.mjs');
for(const zero of [0,'0',0n]) {
  assert.equal(receiptAccountId({accountNonce:zero},7n),'7');
  assert.equal(receiptAccountId({accountId:zero},7n),'7');
}
for(const existing of [42,'42',42n]) {
  assert.equal(receiptAccountId({accountNonce:existing},undefined),'42');
  assert.equal(receiptAccountId({accountId:existing},undefined),'42');
}
assert.equal(receiptAccountId({accountNonce:'18446744073709551615'},undefined),'18446744073709551615');
for(const invalid of [undefined,null,0,'0',-1,'malformed']) {
  assert.throws(()=>receiptAccountId({accountNonce:'0'},invalid));
}
console.log('Receipt account identity: zero creation and existing IDs passed');

// Missing trustline is proven by a valid ledger response, never by a failed view.
const {trustlineBalances}=await import('./balances.mjs');
const {Asset}=await import('@stellar/stellar-sdk');
const issuer=Keypair.fromRawEd25519Seed(Buffer.alloc(32,4)).publicKey();
const classic=new Asset('TEST',issuer), sac=classic.contractId(Networks.TESTNET);
const url='https://rpc.invalid';
const wireResponse=result=>({ok:true,json:async()=>({jsonrpc:'2.0',id:1,result})});
const noLines=async()=>wireResponse({latestLedger:100,entries:[]});
const balances=(fetcher,contract=sac,addresses=[caller])=>trustlineBalances(url,contract,'TEST',issuer,addresses,fetcher);
assert.deepEqual(await balances(noLines),{
  [caller]:{balance:'0',trustline_present:false,ledger:100}
});
const present=async(actualUrl,request)=>{
  assert.equal(actualUrl,url);
  const body=JSON.parse(request.body);
  assert.equal(body.method,'getLedgerEntries');
  assert.equal(body.params.keys.length,1);
  return wireResponse({latestLedger:101,entries:[{key:body.params.keys[0],xdr:xdr.LedgerEntryData.trustline(
    new xdr.TrustLineEntry({accountId:Keypair.fromPublicKey(caller).xdrAccountId(),
      asset:xdr.TrustLineAsset.fromXDR(classic.toXDRObject().toXDR()),balance:xdr.Int64.fromString('1000000000000000001'),
      limit:xdr.Int64.fromString('9223372036854775807'),flags:1,ext:new xdr.TrustLineEntryExt(0)})
  ).toXDR('base64')}]});
};
assert.equal((await balances(present))[caller].balance,'1000000000000000001');
for(const malformed of [{},null,{latestLedger:0,entries:[]},{latestLedger:1e40,entries:[]},
  {latestLedger:1,entries:null},{latestLedger:1},{latestLedger:1,entries:{}},
  {latestLedger:1,entries:[{key:'invalid',xdr:'invalid'}]}]) {
  await assert.rejects(()=>balances(async()=>wireResponse(malformed)));
}
// Exercise the actual SDK parser boundary that used to erase malformed entries.
const normalizingServer=new rpc.Server(url);
for(const raw of [{latestLedger:100},{latestLedger:100,entries:null}]) {
  normalizingServer._getLedgerEntries=async()=>raw;
  assert.deepEqual((await normalizingServer.getLedgerEntries()).entries,[]);
  await assert.rejects(()=>balances(async()=>wireResponse(raw)));
}
for(const wire of [null,{},
  {jsonrpc:'2.0',id:1,error:{code:-1,message:'unavailable'}},
  {jsonrpc:'2.0',id:1,error:null,result:{latestLedger:1,entries:[]}},
  {jsonrpc:'1.0',id:1,result:{latestLedger:1,entries:[]}},
  {jsonrpc:'2.0',id:2,result:{latestLedger:1,entries:[]}}]) {
  await assert.rejects(()=>balances(async()=>({ok:true,json:async()=>wire})));
}
await assert.rejects(()=>balances(async()=>({ok:false,status:503})));
await assert.rejects(()=>balances(async()=>{throw new Error('transport failed')}));
await assert.rejects(()=>balances(async()=>({ok:true,json:async()=>{throw new Error('invalid JSON')}})));
for(const mutate of [
  result=>result.entries.push(result.entries[0]),
  result=>{ const entry=xdr.LedgerEntryData.fromXDR(result.entries[0].xdr,'base64');
    entry.trustLine().accountId(Keypair.fromPublicKey(issuer).xdrAccountId()); result.entries[0].xdr=entry.toXDR('base64'); },
  result=>{ const key=xdr.LedgerKey.fromXDR(result.entries[0].key,'base64');
    key.trustLine().accountId(Keypair.fromPublicKey(issuer).xdrAccountId()); result.entries[0].key=key.toXDR('base64'); },
  result=>{ const entry=xdr.LedgerEntryData.fromXDR(result.entries[0].xdr,'base64');
    entry.trustLine().balance(xdr.Int64.fromString('-1')); result.entries[0].xdr=entry.toXDR('base64'); }
]) {
  await assert.rejects(()=>balances(async(...request)=>{
    const wire=await (await present(...request)).json(); mutate(wire.result);
    return {ok:true,json:async()=>wire};
  }));
}
await assert.rejects(()=>balances(noLines,controller));
await assert.rejects(()=>balances(noLines,sac,[caller,caller]));
console.log('Explicit trustline absence, raw malformed-response rejection, and numeric balances passed');
