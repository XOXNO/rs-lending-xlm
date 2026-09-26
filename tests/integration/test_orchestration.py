#!/usr/bin/env python3
"""Real orchestrator/process-group cancellation, with network-free lane stubs."""
import hashlib
import json
import os
import shutil
import signal
import subprocess
import tempfile
import time
from pathlib import Path

HERE=Path(__file__).resolve().parent
STELLAR='''#!/bin/bash
echo "$*" >> "$STELLAR_CALLS"
case "$1 $2" in
    'keys address') [ -f "$STELLAR_CALLS.key" ] && echo GINSTALLER || exit 1;;
    'keys generate') touch "$STELLAR_CALLS.key";;
    'contract upload')
        n=$(grep -c -F -- "--wasm $4 " "$STELLAR_CALLS")
        if [ "$(basename "$4")" = "${FAIL_ONCE:-}" ] && [ "$n" = 1 ]; then echo TxInsufficientFee >&2; exit 1; fi
        if [ "$(basename "$4")" = "${WRONG_HASH:-}" ]; then printf '%064d\\n' 0; exit 0; fi
        python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$4";;
    *) exit 2;;
esac
'''
CURL='''#!/bin/bash
echo "curl $*" >> "$STELLAR_CALLS"
case "$*" in *horizon*) [ -z "${FUND_FAIL:-}" ] || exit 22;; esac
'''

def orchestrator(base,**extra):
    scripts=base/'scenarios';scripts.mkdir()
    for name in ['gate.py','resources.py','receipts.py','artifacts.py']:
        shutil.copy(HERE/name,base/name)
    shutil.copy(HERE/'scenarios/parallel_e2e.sh',scripts/'parallel_e2e.sh')
    (base/'env.sh').write_text('INTEG_DIR="$(cd "$HERE/.." && pwd)"\nNETWORKS_FILE=unused\nRUN_TS=fixture\n'
        'WASM_DIR="$INTEG_DIR/wasm"\nFIXTURE_WASM_DIR="$INTEG_DIR/fixtures"\nNET_ARGS=(--rpc-url stub)\n')
    (base/'no_network').write_text('#!/bin/bash\nexit 0\n');(base/'no_network').chmod(0o755)
    wasms={base/'wasm/controller.wasm':b'controller',base/'wasm/pool.wasm':b'pool',base/'fixtures/mock_oracle.wasm':b'oracle'}
    for path,body in wasms.items():
        path.parent.mkdir(exist_ok=True);path.write_bytes(body)
    (base/'wasm/candidate.json').write_text(json.dumps({'artifacts':{p.name:hashlib.sha256(b).hexdigest() for p,b in wasms.items() if p.parent.name=='wasm'}}))
    (base/'bin').mkdir();(base/'bin/stellar').write_text(STELLAR);(base/'bin/stellar').chmod(0o755)
    (base/'bin/curl').write_text(CURL);(base/'bin/curl').chmod(0o755)
    env=dict(os.environ,INTEG_DIR=str(base),E2E_LANES='agg',NODE_BIN=str(base/'no_network'),
        PATH=f"{base/'bin'}:{os.environ['PATH']}",STELLAR_CALLS=str(base/'calls'),**extra)
    return scripts,env,sorted(str(p) for p in wasms)

def uploads(base):
    calls=(base/'calls').read_text().splitlines()
    lane=[i for i,c in enumerate(calls) if c.startswith('lane ')]
    sent=[(i,c.split()[3]) for i,c in enumerate(calls) if c.startswith('contract upload')]
    assert all(i<min(lane,default=len(calls)) for i,_ in sent),calls
    return [w for _,w in sent],lane

for cancel in [False,True]:
    with tempfile.TemporaryDirectory() as directory:
        base=Path(directory)
        scripts,env,wasms=orchestrator(base,LANE_TIMEOUT='30s' if cancel else '1s',FAIL_ONCE='' if cancel else 'pool.wasm')
        (scripts/'full_e2e.sh').write_text('''#!/bin/bash
echo "lane $RUN_TS" >> "$STELLAR_CALLS"
run="$INTEG_DIR/runs/$RUN_TS"
mkdir -p "$run"
printf '{"selected_cases":["unfinished"]}' > "$run/metadata.json"
printf 'id\\tstatus\\tfirst_action\\tlast_action\\n' > "$run/cases.tsv"
printf 'partial evidence' > "$run/preserved.txt"
sleep 300 &
child=$!
echo "$child" > "$run/child.pid"
trap 'kill "$child" 2>/dev/null || true; wait "$child" 2>/dev/null; exit 130' TERM INT
wait "$child"
''')
        process=subprocess.Popen(['bash',str(scripts/'parallel_e2e.sh')],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        marker=base/'runs/fixture-agg/child.pid'
        for _ in range(1000):
            if marker.exists():break
            if process.poll() is not None:raise AssertionError(process.communicate())
            time.sleep(.02)
        assert marker.exists()
        if cancel:process.send_signal(signal.SIGTERM)
        out,err=process.communicate(timeout=15)
        assert process.returncode!=0,(out,err)
        run=marker.parent
        assert (run/'preserved.txt').read_text()=='partial evidence'
        assert 'incomplete' in (run/'cases.tsv').read_text(),err
        child=int(marker.read_text())
        try:os.kill(child,0)
        except ProcessLookupError:pass
        else:raise AssertionError(f'orphan child survives cancellation: {child}')
        sent,lane=uploads(base)
        assert sorted(set(sent))==wasms and len(lane)==1,sent
        assert sorted(sent)==sorted(wasms+([] if cancel else [str(base/'wasm/pool.wasm')])),sent
print('Timeout/cancellation regressions: children reaped and incomplete evidence preserved')
print('Each WASM installs once before any lane starts; a rejected upload retries')

with tempfile.TemporaryDirectory() as directory:
    base=Path(directory)
    scripts,env,wasms=orchestrator(base,LANE_TIMEOUT='1s',WRONG_HASH='pool.wasm')
    (scripts/'full_e2e.sh').write_text('#!/bin/bash\necho "lane $RUN_TS" >> "$STELLAR_CALLS"\n')
    done=subprocess.run(['bash',str(scripts/'parallel_e2e.sh')],env=env,capture_output=True,text=True,timeout=30)
    sent,lane=uploads(base)
    assert done.returncode!=0 and not lane and 'install of pool.wasm failed' in done.stderr,(done.stderr,sent)
    assert not (base/'runs/fixture-agg').exists()
print('A WASM hash mismatch at install stops the run before any lane starts')

with tempfile.TemporaryDirectory() as directory:
    base=Path(directory)
    scripts,env,wasms=orchestrator(base,LANE_TIMEOUT='1s',FUND_FAIL='1')
    (scripts/'full_e2e.sh').write_text('#!/bin/bash\necho "lane $RUN_TS" >> "$STELLAR_CALLS"\n')
    done=subprocess.run(['bash',str(scripts/'parallel_e2e.sh')],env=env,capture_output=True,text=True,timeout=60)
    sent,lane=uploads(base)
    assert done.returncode!=0 and not sent and not lane and 'installer wallet GINSTALLER was not funded' in done.stderr,done.stderr
    assert sum('friendbot' in c for c in (base/'calls').read_text().splitlines())==4
print('An unfunded installer wallet stops the run before any upload or lane')
