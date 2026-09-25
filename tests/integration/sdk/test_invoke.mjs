// Exercise the real invocation and shell recorder against offline RPC fixtures.
import assert from 'node:assert/strict';
import {mkdtempSync,readFileSync,writeFileSync,appendFileSync,mkdirSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {Account,Address,Keypair,SorobanDataBuilder,rpc,xdr} from '@stellar/stellar-sdk';

const here=dirname(fileURLToPath(import.meta.url));
const key=Keypair.fromRawEd25519Seed(Buffer.alloc(32,1));
const controller=Address.contract(Buffer.alloc(32,2)).toString();
const asset=Address.contract(Buffer.alloc(32,3)).toString();
const mode=process.env.SDK_INVOKE_FIXTURE;
if (mode) {
  const trace=message=>appendFileSync(join(process.env.LOG_DIR,'trace'),message+'\n');
  const data=new SorobanDataBuilder().setResources(200000,100,200).setResourceFee('300');
  const raw={latestLedger:100,minResourceFee:'300',transactionData:data.build().toXDR('base64'),
    results:[{auth:[],xdr:xdr.ScVal.scvVoid().toXDR('base64')}]};
  const simulation=mode==='simulation-error'
    ? {latestLedger:100,error:'HostError: Error(Contract, #14)'}
    : mode==='native' ? rpc.parseRawSimulation(raw) : raw;
  const before=JSON.stringify(simulation);
  rpc.Server.prototype.getAccount=async()=>new Account(key.publicKey(),'11');
  rpc.Server.prototype.simulateTransaction=async function() {
    assert(this instanceof rpc.Server);
    trace('simulate');
    return simulation;
  };
  rpc.Server.prototype.sendTransaction=async transaction=>{
    trace('send');
    assert.equal(JSON.stringify(simulation),before,'observer changed simulation');
    const hash=transaction.hash().toString('hex');
    if (mode==='send-error') throw new Error('uncertain submission');
    return {status:'PENDING',hash};
  };
  rpc.Server.prototype.getTransaction=async hash=>{
    trace('poll');
    return {status:'FAILED',txHash:hash,ledger:101};
  };
  globalThis.fetch=async (url,request)=>{
    assert.equal(url,'https://rpc.invalid');
    const {method,params}=JSON.parse(request.body);
    assert.equal(method,'getTransaction');
    trace('wire');
    return {ok:true,json:async()=>({jsonrpc:'2.0',id:1,result:{status:'FAILED',txHash:params.hash,ledger:101}})};
  };
} else {
  assert.equal(process.versions.node.split('.')[0],'24');
  for (const fixture of ['failed','send-error','simulation-error','native']) {
    const run=mkdtempSync(join(tmpdir(),'sdk-invoke-'));
    try {
      const logs=join(run,'logs');
      mkdirSync(logs);
      writeFileSync(join(run,'actions.tsv'),'seq\tphase\tlabel\tstatus\tfn\thash\tinstructions\tread_bytes\twrite_bytes\tresource_fee\tnote\n');
      const result=spawnSync('bash',['-c',`
        set -euo pipefail
        source "$INTEG_DIR/lib/core.sh"
        source "$INTEG_DIR/flows/sdk.sh"
        stellar() { printf '%s\\n' "$FIXTURE_SECRET"; }
        sdk_inv fixture buildStellarSupplyTx "$FIXTURE_ARGS"
      `],{encoding:'utf8',env:{...process.env,SDK_INVOKE_FIXTURE:fixture,
        NODE_OPTIONS:`--import=${import.meta.url}`,NODE_BIN:process.execPath,
        INTEG_DIR:dirname(here),RUN_DIR:run,LOG_DIR:logs,ACTIONS_TSV:join(run,'actions.tsv'),PHASE:'sdk',
        RPC_URL:'https://rpc.invalid',CONTROLLER:controller,ALICE:'fixture',EXPECT_ERROR:'',
        FIXTURE_SECRET:key.secret(),FIXTURE_ARGS:JSON.stringify({asset,hubId:1,spokeId:1,amount:'10000001',accountNonce:0})}});
      assert.equal(result.status,1,result.stderr);
      const action=readFileSync(join(run,'actions.tsv'),'utf8').trimEnd().split('\n')[1].split('\t');
      const proof=readFileSync(join(run,'evidence.tsv'),'utf8').trimEnd().split('\n')[1].split('\t');
      assert.equal(action[3],'FAIL');
      assert.equal(action[4],'supply');
      assert.equal(proof[2],controller);
      const sim=JSON.parse(readFileSync(join(logs,'fixture.simulation-1.json'),'utf8'));
      assert.equal(sim.latestLedger,100);
      const trace=readFileSync(join(logs,'trace'),'utf8').trim().split('\n');
      assert.equal(trace.filter(call=>call==='simulate').length,1);
      if (fixture==='simulation-error') {
        assert.match(sim.error,/#14/);
        assert.equal(action[5],'');
        assert.equal(proof[1],'simulation');
        assert.deepEqual(trace,['simulate']);
      } else {
        const resources=JSON.parse(readFileSync(join(logs,'fixture.simulation-1.resources.json'),'utf8'));
        assert.equal(resources.latestLedger,100);
        assert.equal(resources.minResourceFee,'300');
        assert.equal(resources.resources.instructions,200000);
        assert.equal(proof[1],'transaction',readFileSync(join(logs,'fixture.err'),'utf8'));
        assert.match(action[5],/^[0-9a-f]{64}$/);
        assert.equal(readFileSync(join(logs,'fixture.hash'),'utf8'),action[5]);
        const receipt=JSON.parse(readFileSync(join(logs,`${action[5]}.receipt.json`),'utf8'));
        assert.equal(receipt.result.status,'FAILED');
        assert.equal(receipt.result.txHash,action[5]);
        assert.deepEqual(trace,['simulate','send','poll','wire']);
        if (fixture==='send-error') assert.match(readFileSync(join(logs,'fixture.submission-error.txt'),'utf8'),/uncertain submission/);
      }
    } finally { rmSync(run,{recursive:true,force:true}); }
  }
  console.log('SDK simulation evidence, failed wire receipt, single submission, and sticky shell failure checks passed');
}
