"""Normalize native-XLM balance deltas by actual committed network fees."""
import json
import re
import sys
from pathlib import Path
from receipts import decode


def spent(logs, address):
    total=0
    for path in logs.glob('*.receipt.json'):
        if not re.fullmatch(r'[0-9a-f]{64}\.receipt\.json',path.name): continue
        receipt=json.loads(path.read_text())['result']
        cache=path.with_suffix('.network-fee.json')
        if cache.exists():
            charge=json.loads(cache.read_text())
        else:
            envelope=decode('TransactionEnvelope',receipt['envelopeXdr'])
            result=decode('TransactionResult',receipt['resultXdr'])
            payer=(envelope['tx_fee_bump']['tx']['fee_source'] if 'tx_fee_bump' in envelope
                   else envelope['tx']['tx']['source_account'])
            charge=dict(payer=payer,fee=int(result['fee_charged']))
            if charge['fee']<0: raise ValueError('negative committed fee')
            cache.write_text(json.dumps(charge)+'\n')
        if charge['payer']==address: total+=charge['fee']
    return total


if __name__=='__main__':
    logs,address,raw=sys.argv[1:]
    print(int(raw)+spent(Path(logs),address))
