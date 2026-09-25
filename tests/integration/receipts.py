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
    if mode == 'command':
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
        if verb == 'deploy' and (set(value) != {'address'} or not re.fullmatch(r'C[A-Z2-7]{55}', value['address'])):
            raise ValueError('unexpected deployed contract return value')
    return return_json(value)


if __name__ == '__main__':
    receipt_path, hash_, passphrase, mode, *binding = sys.argv[1:]
    print(json.dumps(recover(json.loads(Path(receipt_path).read_text()), hash_, passphrase, mode, *binding), separators=(',', ':')))
