"""Validate committed RPC evidence using the pinned CLI's native XDR codec."""
import json
import re
import subprocess
import base64
import hashlib
import sys
from pathlib import Path


def cli(args, value):
    return subprocess.check_output(['stellar', *args], input=value, text=True, stderr=subprocess.PIPE, timeout=30).strip()


def decode(kind, value):
    return json.loads(cli(['xdr', 'decode', '--type', kind, '--output', 'json'], value))


def envelope_hash(envelope, passphrase):
    if set(envelope) not in ({'tx'}, {'tx_fee_bump'}):
        raise ValueError('unsupported transaction envelope')
    payload = {'network_id': hashlib.sha256(passphrase.encode()).hexdigest(),
               'tagged_transaction': {kind: value['tx'] for kind, value in envelope.items()}}
    encoded = cli(['xdr', 'encode', '--type', 'TransactionSignaturePayload'], json.dumps(payload))
    return hashlib.sha256(base64.b64decode(encoded, validate=True)).hexdigest()


def unwrap(envelope, outcome):
    """Keep fee-bump result status consistent with its inner transaction."""
    if set(envelope) == {'tx_fee_bump'}:
        result = outcome['result']
        if set(result) not in ({'tx_fee_bump_inner_success'}, {'tx_fee_bump_inner_failed'}):
            raise ValueError('missing fee-bump inner result')
        inner = next(iter(result.values()))['result']
        if ('tx_fee_bump_inner_success' in result) != ('tx_success' in inner['result']):
            raise ValueError('fee-bump status contradicts inner result')
        return envelope['tx_fee_bump']['tx']['inner_tx'], inner
    if set(envelope) != {'tx'} or any(key.startswith('tx_fee_bump_') for key in outcome['result']):
        raise ValueError('unsupported transaction envelope/result')
    return envelope, outcome


def verify(receipt, hash_, passphrase, status, contract=None, method=None):
    result = receipt['result']
    if receipt.get('jsonrpc') != '2.0' or receipt.get('id') != 1 or 'error' in receipt:
        raise ValueError('invalid receipt JSON-RPC envelope')
    if result['status'] != status or result['txHash'] != hash_ or type(result['ledger']) is not int or result['ledger'] <= 0:
        raise ValueError('invalid committed receipt identity/status/ledger')
    envelope = decode('TransactionEnvelope', result['envelopeXdr'])
    outcome = decode('TransactionResult', result['resultXdr'])
    meta=decode('TransactionMeta', result['resultMetaXdr'])
    # Classic and pre-apply failures may carry earlier metadata versions and
    # omit the optional RPC event list. Their decoded metadata must be empty of
    # contract events; a missing list must never suppress committed events.
    if 'v4' in meta:
        committed_events=[event for operation in meta['v4']['operations'] for event in operation['events']]
    elif 'v3' in meta:
        committed_events=(meta['v3']['soroban_meta'] or {}).get('events', [])
    elif set(meta) in ({'v0'}, {'v1'}, {'v2'}):
        committed_events=[]
    else:
        raise ValueError('unsupported transaction metadata')
    wire_events=[decode('ContractEvent',event) for operation in result.get('events', {}).get('contractEventsXdr', []) for event in operation]
    if committed_events!=wire_events:
        raise ValueError('RPC event list differs from committed metadata')
    actual_hash = envelope_hash(envelope, passphrase)
    inner_envelope, inner_outcome = unwrap(envelope, outcome)
    hashes = {actual_hash}
    if 'tx_fee_bump' in envelope:
        inner_hash = envelope_hash(inner_envelope, passphrase)
        if next(iter(outcome['result'].values()))['transaction_hash'] != inner_hash:
            raise ValueError('fee-bump result differs from inner transaction hash')
        # RPC accepts either the outer or inner hash when querying a fee bump.
        hashes.add(inner_hash)
    if hash_ not in hashes:
        raise ValueError('receipt envelope hash differs from submitted hash')
    transaction = inner_envelope['tx']['tx']
    successful = 'tx_success' in inner_outcome['result']
    if successful != (status == 'SUCCESS'):
        raise ValueError('receipt status contradicts decoded transaction result')
    if contract is not None:
        if not re.fullmatch(r'C[A-Z2-7]{55}', contract):
            raise ValueError('invalid invocation contract address')
        operations = transaction['operations']
        if len(operations) != 1:
            raise ValueError('expected one root invocation')
        invocation = operations[0]['body']['invoke_host_function']['host_function']['invoke_contract']
        if invocation['contract_address'] != contract or invocation['function_name'] != method:
            raise ValueError('receipt invocation differs from claimed contract/method')
    extension = transaction['ext']
    if extension == 'v0':
        return None
    if isinstance(extension, dict) and set(extension) == {'v1'}:
        return extension['v1']
    raise ValueError('unsupported transaction extension')


ROUND_ORACLES = {'CCYOZJCOPG34LLQQ7N24YXBM7LL62R7ONMZ3G6WZAAYPB5OYKOMJRN63'}
OUTSIDE_FOOTPRINT = {'string': 'trying to access contract data key outside of the footprint'}


def is_round(key):
    return isinstance(key, dict) and set(key) == {'u64'} and str(key['u64']).isdigit()


def footprint_limit_failure(receipt, hash_, passphrase, contract, method):
    """Accept only a committed FAILED call that read a live oracle round outside its footprint."""
    verify(receipt, hash_, passphrase, 'FAILED', contract, method)
    result = receipt['result']
    envelope, outcome = unwrap(decode('TransactionEnvelope', result['envelopeXdr']),
                               decode('TransactionResult', result['resultXdr']))
    if outcome['result'].get('tx_failed') not in ([{'op_inner': {'invoke_host_function': 'trapped'}}],
                                                  [{'op_inner': {'invoke_host_function': 'resource_limit_exceeded'}}]):
        raise ValueError('failure is not a host-function trap or resource limit')
    limit = [{'error': {'storage': 'exceeded_limit'}}]
    events = [event['event']['body']['v0'] for event in
              (decode('TransactionMeta', result['resultMetaXdr']).get('v4') or {}).get('diagnostic_events') or []]
    if not any(event['topics'] == [{'symbol': 'host_fn_failed'}, *limit] for event in events):
        raise ValueError('failure is not a storage footprint limit')
    outside = [(event['data']['vec'][1]['address'], event['data']['vec'][2])
               for event in events if event['topics'] == [{'symbol': 'error'}, *limit]
               and isinstance(event['data'], dict) and event['data'].get('vec', [None])[0] == OUTSIDE_FOOTPRINT]
    if any(not is_round(key) for _, key in outside):
        raise ValueError('missing key is not an oracle round key')
    missing = {(address, json.dumps(key, sort_keys=True)) for address, key in outside}
    transaction = envelope['tx']['tx']
    declared = {(entry['contract_data']['contract'], json.dumps(entry['contract_data']['key'], sort_keys=True))
                for keys in transaction['ext']['v1']['resources']['footprint'].values()
                for entry in keys if 'contract_data' in entry}
    if not missing or any(address not in ROUND_ORACLES for address, _ in missing) or missing & declared:
        raise ValueError('failure is not a live oracle round outside the footprint')
    return transaction, missing


def footprint_drift(failed, failed_hash, passphrase, contract, method, succeeded=None, succeeded_hash=None):
    """Prove a retried call is the same call and only added the missing oracle round."""
    before, missing = footprint_limit_failure(failed, failed_hash, passphrase, contract, method)
    if succeeded is None:
        return
    verify(succeeded, succeeded_hash, passphrase, 'SUCCESS', contract, method)
    after = unwrap(decode('TransactionEnvelope', succeeded['result']['envelopeXdr']),
                   decode('TransactionResult', succeeded['result']['resultXdr']))[0]['tx']['tx']
    call = lambda tx: (tx['source_account'], tx['operations'][0]['body']['invoke_host_function']['host_function'])
    keys = lambda tx, kind: {json.dumps(entry, sort_keys=True) for entry in tx['ext']['v1']['resources']['footprint'][kind]}
    if call(before) != call(after):
        raise ValueError('retry is not the same call')
    if keys(before, 'read_write') != keys(after, 'read_write'):
        raise ValueError('retry changed the read-write footprint')
    moved = [json.loads(entry) for entry in keys(before, 'read_only') ^ keys(after, 'read_only')]
    if any(entry.get('contract_data', {}).get('contract') not in ROUND_ORACLES
           or entry['contract_data']['durability'] != 'temporary'
           or not is_round(entry['contract_data']['key']) for entry in moved):
        raise ValueError('retry moved a footprint entry that is not a live oracle round')
    added = {(entry['contract_data']['contract'], json.dumps(entry['contract_data']['key'], sort_keys=True))
             for entry in moved if json.dumps(entry, sort_keys=True) in keys(after, 'read_only')}
    if not missing <= added:
        raise ValueError('retry footprint lacks the missing oracle round')


def return_json(value):
    """Lossless CLI-shaped JSON for values returned by harness mutations."""
    if value == 'void':
        return None
    if not isinstance(value, dict) or len(value) != 1:
        raise ValueError('unsupported return value')
    kind, item = next(iter(value.items()))
    if kind in {'bool', 'u32', 'i32', 'u64', 'i64', 'u128', 'i128', 'u256', 'i256',
                'timepoint', 'duration', 'address', 'string', 'symbol', 'bytes'}:
        return item
    if kind == 'vec' and isinstance(item, list):
        return [return_json(entry) for entry in item]
    if kind == 'map' and isinstance(item, list):
        result = {}
        for entry in item:
            key = return_json(entry['key'])
            if type(key) is int:
                key = str(key)
            if not isinstance(key, str) or key in result:
                raise ValueError('unsupported or duplicate return map key')
            result[key] = return_json(entry['val'])
        return result
    raise ValueError('unsupported return value kind: '+kind)


def recover(receipt, hash_, passphrase, mode, *binding):
    """Recover only a verified, successful single host-function invocation."""
    contract, method = binding if mode == 'invoke' else (None, None)
    verify(receipt, hash_, passphrase, 'SUCCESS', contract, method)
    result = receipt['result']
    envelope, outcome = unwrap(decode('TransactionEnvelope', result['envelopeXdr']),
                               decode('TransactionResult', result['resultXdr']))
    transaction = envelope['tx']['tx']
    operations = transaction['operations']
    if len(operations) != 1:
        raise ValueError('expected one recovery operation')
    host = operations[0]['body']['invoke_host_function']['host_function']
    if mode == 'command' and list(binding[:4]) == ['stellar', 'contract', 'asset', 'deploy']:
        verb = 'asset'
        spec = binding[binding.index('--asset')+1]
        code, _, issuer = spec.partition(':')
        asset = 'native' if spec == 'native' else {
            'credit_alphanum4' if len(code) <= 4 else 'credit_alphanum12': {'asset_code': code, 'issuer': issuer}}
        if host != {'create_contract': {'contract_id_preimage': {'asset': asset}, 'executable': 'stellar_asset'}}:
            raise ValueError('receipt differs from requested asset contract deployment')
    elif mode == 'command':
        command = list(binding)
        if command[:2] != ['stellar', 'contract'] or command[2] not in {'deploy', 'upload'}:
            raise ValueError('unsupported recovery command')
        verb = command[2]
        options = command[3:command.index('--')] if '--' in command else command[3:]
        wasm = Path(options[options.index('--wasm')+1]).read_bytes()
        wasm_hash = hashlib.sha256(wasm).hexdigest()
        if verb == 'upload':
            if set(host) != {'upload_contract_wasm'} or bytes.fromhex(host['upload_contract_wasm']) != wasm:
                raise ValueError('receipt differs from requested WASM upload')
        else:
            if set(host) not in ({'create_contract'}, {'create_contract_v2'}):
                raise ValueError('receipt is not a completed contract deployment')
            if next(iter(host.values()))['executable'] != {'wasm': wasm_hash}:
                raise ValueError('receipt differs from requested deployment WASM')
    elif mode != 'invoke':
        raise ValueError('unsupported recovery mode')
    meta = decode('TransactionMeta', result['resultMetaXdr'])
    version = meta.get('v4', meta.get('v3'))
    if version is None or version['soroban_meta'] is None:
        raise ValueError('missing committed Soroban return value')
    value = version['soroban_meta']['return_value']
    events = ([e for op in version['operations'] for e in op['events']]
              if 'v4' in meta else version['soroban_meta']['events'])
    preimage = cli(['xdr', 'encode', '--type', 'InvokeHostFunctionSuccessPreImage'],
                   json.dumps({'return_value': value, 'events': events}))
    outcome = outcome['result']['tx_success']
    if len(outcome) != 1 or outcome[0]['op_inner']['invoke_host_function'] != {
            'success': hashlib.sha256(base64.b64decode(preimage, validate=True)).hexdigest()}:
        raise ValueError('committed return/events differ from successful result hash')
    if mode == 'command':
        if verb == 'upload' and value != {'bytes': wasm_hash}:
            raise ValueError('unexpected uploaded WASM return value')
        if verb in {'deploy', 'asset'} and (set(value) != {'address'} or not re.fullmatch(r'C[A-Z2-7]{55}', value['address'])):
            raise ValueError('unexpected deployed contract return value')
    return return_json(value)


if __name__ == '__main__':
    if sys.argv[1] == 'drift':
        failed, failed_hash, passphrase, contract, method, *retried = sys.argv[2:]
        footprint_drift(json.loads(Path(failed).read_text()), failed_hash, passphrase, contract, method,
                        *([json.loads(Path(retried[0]).read_text()), retried[1]] if retried else []))
        sys.exit(0)
    receipt_path, hash_, passphrase, mode, *binding = sys.argv[1:]
    print(json.dumps(recover(json.loads(Path(receipt_path).read_text()), hash_, passphrase, mode, *binding), separators=(',', ':')))
