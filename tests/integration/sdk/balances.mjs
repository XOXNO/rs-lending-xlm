// Explicit absence proof for classic SAC trustlines; never error-to-zero.
import assert from 'node:assert/strict';
import {writeFileSync} from 'node:fs';
import {Asset,Keypair,Networks,xdr} from '@stellar/stellar-sdk';

export async function trustlineBalances(url, contract, code, issuer, addresses, fetcher=fetch) {
  const asset = new Asset(code,issuer);
  assert.equal(asset.contractId(Networks.TESTNET),contract,'SAC identity does not match classic asset');
  assert.equal(new Set(addresses).size,addresses.length);
  assert(!addresses.includes(issuer),'issuer balance uses SAC view');
  const keys=addresses.map(address=>xdr.LedgerKey.trustline(new xdr.LedgerKeyTrustLine({
    accountId:Keypair.fromPublicKey(address).xdrAccountId(),
    asset:xdr.TrustLineAsset.fromXDR(asset.toXDRObject().toXDR())
  })));
  // Check the wire response before SDK parsing can turn missing/null entries
  // into an empty list and incorrectly prove an absent trustline.
  const response=await fetcher(url,{method:'POST',headers:{'Content-Type':'application/json'},
    body:JSON.stringify({jsonrpc:'2.0',id:1,method:'getLedgerEntries',params:{keys:keys.map(key=>key.toXDR('base64'))}})});
  assert(response.ok, `trustline HTTP ${response.status}`);
  const wire=await response.json();
  assert.equal(wire.jsonrpc,'2.0'); assert.equal(wire.id,1);
  assert(!Object.hasOwn(wire,'error'),'trustline RPC error');
  const result=wire.result;
  assert(Number.isSafeInteger(result.latestLedger)&&result.latestLedger>0&&result.latestLedger<=0xffffffff&&Array.isArray(result.entries));
  const entries=new Map();
  const expected=new Set(keys.map(k=>k.toXDR('base64')));
  for(const entry of result.entries) {
    const ledgerKey=xdr.LedgerKey.fromXDR(entry.key,'base64');
    const key=ledgerKey.toXDR('base64');
    assert(expected.has(key)&&!entries.has(key),'unknown or duplicate trustline response');
    const line=xdr.LedgerEntryData.fromXDR(entry.xdr,'base64').trustLine();
    assert.equal(line.accountId().toXDR('base64'),ledgerKey.trustLine().accountId().toXDR('base64'));
    assert.equal(line.asset().toXDR('base64'),ledgerKey.trustLine().asset().toXDR('base64'));
    const balance=line.balance().toString(); assert(/^\d+$/.test(balance));
    entries.set(key,balance);
  }
  return Object.fromEntries(addresses.map((address,i)=>[address,{
    balance:entries.get(keys[i].toXDR('base64'))??'0',
    trustline_present:entries.has(keys[i].toXDR('base64')), ledger:result.latestLedger
  }]));
}

if(process.argv[1]?.endsWith('/balances.mjs')) {
  const [url,contract,code,issuer,destination,...addresses]=process.argv.slice(2);
  assert.equal(process.versions.node.split('.')[0],'24');
  writeFileSync(destination,JSON.stringify(await trustlineBalances(url,contract,code,issuer,addresses),null,2));
}
