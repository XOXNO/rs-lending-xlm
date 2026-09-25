#!/usr/bin/env python3
"""Native offline XDR regression using a public testnet receipt (no keys)."""
import json
import subprocess
from copy import deepcopy
from pathlib import Path
from receipts import verify
r=json.loads(Path(__file__).with_name('fixtures').joinpath('receipt.json').read_text())
args=(r['result']['txHash'],'Test SDF Network ; September 2015','SUCCESS','CDDVDGITCUZTHQSYFRV37AC3263SMK2FZYYVJ74EBLESTFFF5UOVODZW','deploy_controller')
assert verify(r,*args)['resources']['instructions'] > 0
for field,value in [('envelopeXdr','AA=='),('resultXdr','AA=='),('resultMetaXdr','AA=='),('ledger',0),('status','FAILED')]:
    broken=deepcopy(r); broken['result'][field]=value
    try: verify(broken,*args)
    except (ValueError,KeyError,subprocess.CalledProcessError): pass
    else: raise AssertionError('malformed receipt accepted: '+field)
for changed in [(args[0],'wrong network',*args[2:]),(*args[:-1],'withdraw')]:
    try: verify(r,*changed)
    except ValueError: pass
    else: raise AssertionError('wrong network/method accepted')
print('Native receipt decoding and identity regressions passed')
# Fee adjustment uses actual charged fees exactly once, preserving raw reads.
import tempfile
from network_fees import spent
from receipts import decode
with tempfile.TemporaryDirectory() as directory:
    logs=Path(directory)
    (logs/(args[0]+'.receipt.json')).write_text(json.dumps(r))
    (logs/'sdk_duplicate.receipt.json').write_text('{}')
    payer=decode('TransactionEnvelope',r['result']['envelopeXdr'])['tx']['tx']['source_account']
    charged=int(decode('TransactionResult',r['result']['resultXdr'])['fee_charged'])
    assert spent(logs,payer)==charged
    assert spent(logs,payer)==charged
    assert spent(logs,'unrelated')==0

broken=deepcopy(r); broken['result']['events']['contractEventsXdr']=[]
try: verify(broken,*args)
except ValueError: pass
else: raise AssertionError('suppressed event bytes accepted')

# Native codec round-trips for legal metadata/result variants. These are
# synthetic structural fixtures, not claims of successful live execution.
from receipts import cli
def encode(kind, value):
    return cli(['xdr', 'encode', '--type', kind], json.dumps(value))

failed=deepcopy(r)
failed['result']['status']='FAILED'
failed['result']['resultXdr']=encode('TransactionResult', {
    'fee_charged':'100', 'result':{'tx_failed':[{'op_inner':{'invoke_host_function':'trapped'}}]}, 'ext':'v0'})
failed['result']['resultMetaXdr']=encode('TransactionMeta', {'v0':[]})
failed['result'].pop('events')
assert verify(failed,args[0],args[1],'FAILED',args[3],args[4])['resources']['instructions']>0
try: verify(failed,*args)
except ValueError: pass
else: raise AssertionError('failed decoded outcome accepted as success')

classic=deepcopy(r)
envelope=decode('TransactionEnvelope',classic['result']['envelopeXdr'])
envelope['tx']['tx']['operations']=[{'source_account':None,'body':{'manage_data':{'data_name':'offline-audit','data_value':None}}}]
envelope['tx']['tx']['ext']='v0'
classic['result']['envelopeXdr']=encode('TransactionEnvelope',envelope)
classic['result']['txHash']=cli(['tx','hash','--network-passphrase',args[1]],classic['result']['envelopeXdr'])
classic['result']['resultXdr']=encode('TransactionResult',{
    'fee_charged':'100','result':{'tx_success':[{'op_inner':{'manage_data':'success'}}]},'ext':'v0'})
classic['result']['resultMetaXdr']=encode('TransactionMeta',{'v0':[{'changes':[]}]})
classic['result'].pop('events')
assert verify(classic,classic['result']['txHash'],args[1],'SUCCESS') is None
classic['result']['events']=r['result']['events']
try: verify(classic,classic['result']['txHash'],args[1],'SUCCESS')
except ValueError: pass
else: raise AssertionError('injected classic contract events accepted')
print('Classic and failed receipt metadata regressions passed')

# Recovery binds the metadata value/events to the native success result hash.
from receipts import recover, return_json
assert recover(r,args[0],args[1],'invoke',args[3],args[4]) == 'CCTE6RN6NK4KWLXL22UBL5DGF3XZ4LT5XBLN4AVOKIKLWVF6D7U7UQFB'
assert return_json({'map':[{'key':{'symbol':'amount'},'val':{'i128':'1000000000000000001'}},
    {'key':{'symbol':'tuple'},'val':{'vec':['void',{'bool':True},{'bytes':'abcd'}]}}]}) == {
    'amount':'1000000000000000001','tuple':[None,True,'abcd']}
positions={'map':[{'key':{'symbol':'collateral'},'val':{'map':[
    {'key':{'u32':0},'val':{'i128':'1000000000000000001'}}]}}]}
assert return_json(decode('ScVal',encode('ScVal',positions))) == {'collateral':{'0':'1000000000000000001'}}
for kind in ['missing','changed','error']:
    bad=deepcopy(r); meta=decode('TransactionMeta',bad['result']['resultMetaXdr'])
    if kind=='missing': meta['v4']['soroban_meta']=None
    else: meta['v4']['soroban_meta']['return_value'] = {'u64':'7'} if kind=='changed' else {'error':{'contract':1}}
    bad['result']['resultMetaXdr']=encode('TransactionMeta',meta)
    try: recover(bad,args[0],args[1],'invoke',args[3],args[4])
    except (ValueError,KeyError): pass
    else: raise AssertionError('invalid recovered value accepted: '+kind)
try: recover(failed,args[0],args[1],'invoke',args[3],args[4])
except ValueError: pass
else: raise AssertionError('failed transaction recovered')

# A deploy command cannot recover the address from an unrelated invocation or
# an upload-only success. Synthetic valid deployment uses native XDR encoding.
with tempfile.TemporaryDirectory() as directory:
    wasm=Path(directory)/'fixture.wasm'; wasm.write_bytes(b'offline-wasm-fixture')
    import hashlib
    wasm_hash=hashlib.sha256(wasm.read_bytes()).hexdigest()
    command=['stellar','contract','deploy','--wasm',str(wasm)]
    try: recover(r,args[0],args[1],'command',*command)
    except ValueError: pass
    else: raise AssertionError('invoke accepted as deployment')
    deployed=deepcopy(r); envelope=decode('TransactionEnvelope',deployed['result']['envelopeXdr'])
    host=envelope['tx']['tx']['operations'][0]['body']['invoke_host_function']
    host['host_function']={'create_contract_v2':{
        'contract_id_preimage':{'address':{'address':envelope['tx']['tx']['source_account'],'salt':'00'*32}},
        'executable':{'wasm':wasm_hash},'constructor_args':[]}}
    deployed['result']['envelopeXdr']=encode('TransactionEnvelope',envelope)
    deployed['result']['txHash']=cli(['tx','hash','--network-passphrase',args[1]],deployed['result']['envelopeXdr'])
    assert recover(deployed,deployed['result']['txHash'],args[1],'command',*command).startswith('C')
    host['host_function']={'upload_contract_wasm':wasm.read_bytes().hex()}
    deployed['result']['envelopeXdr']=encode('TransactionEnvelope',envelope)
    deployed['result']['txHash']=cli(['tx','hash','--network-passphrase',args[1]],deployed['result']['envelopeXdr'])
    try: recover(deployed,deployed['result']['txHash'],args[1],'command',*command)
    except ValueError: pass
    else: raise AssertionError('upload-only receipt accepted as deployment')
    meta=decode('TransactionMeta',deployed['result']['resultMetaXdr'])
    meta['v4']['soroban_meta']['return_value']={'bytes':wasm_hash}
    deployed['result']['resultMetaXdr']=encode('TransactionMeta',meta)
    import base64
    preimage=encode('InvokeHostFunctionSuccessPreImage',{'return_value':{'bytes':wasm_hash},
        'events':meta['v4']['operations'][0]['events']})
    outcome=decode('TransactionResult',deployed['result']['resultXdr'])
    outcome['result']['tx_success'][0]['op_inner']['invoke_host_function']['success']=hashlib.sha256(base64.b64decode(preimage)).hexdigest()
    deployed['result']['resultXdr']=encode('TransactionResult',outcome)
    upload=['stellar','contract','upload','--wasm',str(wasm)]
    assert recover(deployed,deployed['result']['txHash'],args[1],'command',*upload)==wasm_hash
    wasm.write_bytes(b'wrong candidate')
    try: recover(deployed,deployed['result']['txHash'],args[1],'command',*upload)
    except ValueError: pass
    else: raise AssertionError('wrong upload candidate accepted')
print('Verified return recovery and deployment operation binding passed')
