"""Reconcile all candidate exports with contract-qualified required evidence."""
import json
import subprocess
import sys
from pathlib import Path
from artifacts import CONTRACTS

HERE=Path(__file__).resolve().parent


def check(directory=None):
    coverage=json.loads((HERE/'abi-coverage.json').read_text())
    cases={c['id']:c for c in json.loads((HERE/'cases.json').read_text())}
    controlled={c['id']:c for c in json.loads((HERE/'controlled-cases.json').read_text())}
    keys=[(c['contract'],c['method']) for c in coverage]
    if len(keys)!=len(set(keys)): raise ValueError('duplicate ABI mapping')
    for entry in coverage:
        if entry['contract'] not in CONTRACTS: raise ValueError('unknown ABI contract')
        if entry['execution']=='controlled-ledger':
            proof=controlled[entry['controlled_case']]
            if f"{entry['contract']}.{entry['method']}" not in proof['methods'] or not entry['scope']:
                raise ValueError('unjustified controlled ABI mapping')
        else:
            matches=[a for a in cases[entry['case']]['required_actions'] if a['label']==entry['label'] and a['method']==entry['root_method'] and a.get('contract')==entry['root_contract']]
            if not matches: raise ValueError(f'unbound ABI mapping: {entry}')
    if directory is not None:
        exports=set()
        for contract in CONTRACTS:
            spec=json.loads(subprocess.check_output(['stellar','contract','info','interface','--wasm',str(directory/f'{contract}.wasm'),'--output','json'],stderr=subprocess.PIPE,text=True))
            exports.update((contract,item['function_v0']['name']) for item in spec if 'function_v0' in item)
        if exports!=set(keys):
            raise ValueError(f'ABI coverage changed: missing={sorted(exports-set(keys))}; stale={sorted(set(keys)-exports)}')
    return len(keys)


if __name__=='__main__':
    print(f'ABI coverage: {check(Path(sys.argv[1]) if len(sys.argv)>1 else None)} contract-qualified exports mapped (runtime proof required separately)')
