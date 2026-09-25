#!/usr/bin/env python3
"""Check declared transaction resources against both captured network limits."""
import base64
import json
import re
import sys
from pathlib import Path

if not __debug__:
    raise RuntimeError('release verification requires Python assertions; unset PYTHONOPTIMIZE')


def check(limits, data, receipt):
    r = data['resources']; footprint = r['footprint']
    usage = dict(instructions=int(r['instructions']),
                 disk_read_bytes=int(r.get('disk_read_bytes',r.get('read_bytes'))),
                 write_bytes=int(r['write_bytes']),
                 read_entries=len(footprint['read_only'])+len(footprint['read_write']),
                 write_entries=len(footprint['read_write']),
                 tx_bytes=len(base64.b64decode(receipt['envelopeXdr'],validate=True)),
                 event_bytes=sum(len(base64.b64decode(e,validate=True)) for op in receipt['events']['contractEventsXdr'] for e in op))
    passphrases={'testnet':'Test SDF Network ; September 2015','mainnet':'Public Global Stellar Network ; September 2015'}
    for name in ['testnet','mainnet']:
        snapshot=limits[name]
        if type(snapshot['ledger']) is not int or snapshot['ledger']<=0 or snapshot['network']['passphrase']!=passphrases[name]:
            raise ValueError('invalid network limit provenance')
        if type(snapshot['network']['protocolVersion']) is not int or snapshot['network']['protocolVersion']<1 or snapshot['version']['protocolVersion']!=snapshot['network']['protocolVersion'] or not snapshot['version']['version']:
            raise ValueError('invalid protocol version provenance')
        cap=snapshot['limits']
        for field in ['txMaxInstructions','txMaxDiskReadBytes','txMaxWriteBytes','txMaxDiskReadEntries','txMaxWriteLedgerEntries','txMaxSizeBytes','txMaxContractEventsSizeBytes','txMemoryLimit']:
            if not re.fullmatch(r'[1-9][0-9]*',str(cap[field])):
                raise ValueError(f'invalid positive limit {field}')
        if any(type(value) is not int or value<0 for value in usage.values()):
            raise ValueError('invalid measured resource amount')
        assert usage['instructions']*10 <= int(cap['txMaxInstructions'])*9, f'{name}: <10% instruction headroom'
        for resource, field in [('disk_read_bytes','txMaxDiskReadBytes'),('write_bytes','txMaxWriteBytes'),('read_entries','txMaxDiskReadEntries'),('write_entries','txMaxWriteLedgerEntries'),('tx_bytes','txMaxSizeBytes'),('event_bytes','txMaxContractEventsSizeBytes')]:
            assert 0 <= usage[resource] <= int(cap[field]), f'{name}: {resource} exceeds {field}'
    # Successful execution proves the testnet hard memory cap. If mainnet's
    # cap is lower, that evidence is insufficient: require another measurement.
    assert int(limits['testnet']['limits']['txMemoryLimit']) <= int(limits['mainnet']['limits']['txMemoryLimit']), 'mainnet memory proof unavailable'
    return usage


if __name__=='__main__':
    limits,data,receipt=map(lambda p:json.loads(Path(p).read_text()),sys.argv[1:])
    print(json.dumps(check(limits,data,receipt['result'])))
