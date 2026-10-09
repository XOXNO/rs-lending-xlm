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
    env=dict(os.environ,INTEG_DIR=str(base),E2E_LANES='agg-core',NODE_BIN=str(base/'no_network'),
        PATH=f"{base/'bin'}:{os.environ['PATH']}",STELLAR_CALLS=str(base/'calls'),**{'E2E_LANE_GAP':'0',**extra})
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
        marker=base/'runs/fixture-agg-core/child.pid'
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
    assert not (base/'runs/fixture-agg-core').exists()
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

with tempfile.TemporaryDirectory() as directory:
    base=Path(directory)
    scripts,env,wasms=orchestrator(base,LANE_TIMEOUT='30s')
    env['E2E_LANES']='agg-core liq-a'
    (scripts/'full_e2e.sh').write_text('#!/bin/bash\necho "run complete"\n')
    (scripts/'assert_green.sh').write_text('#!/bin/bash\nsleep 3\necho "GREEN $RUN_TS"\n')
    started=time.monotonic()
    done=subprocess.run(['bash',str(scripts/'parallel_e2e.sh')],env=env,capture_output=True,text=True,timeout=60)
    elapsed=time.monotonic()-started
    assert done.returncode==0,done.stderr
    assert done.stdout.index('GREEN fixture-agg-core')<done.stdout.index('GREEN fixture-liq-a'),done.stdout
    assert elapsed<5.5,elapsed
print('Lane gates run in parallel and report in lane order')

for lanes in ['agg','strategies','production']:
    with tempfile.TemporaryDirectory() as directory:
        base=Path(directory)
        scripts,env,wasms=orchestrator(base,LANE_TIMEOUT='1s')
        env['E2E_LANES']=lanes
        (scripts/'full_e2e.sh').write_text('#!/bin/bash\necho "lane $RUN_TS" >> "$STELLAR_CALLS"\n')
        done=subprocess.run(['bash',str(scripts/'parallel_e2e.sh')],env=env,capture_output=True,text=True,timeout=30)
        assert done.returncode==2 and f"unknown lane '{lanes}'" in done.stderr,done.stderr
        assert not (base/'calls').exists() and not (base/'runs').exists(),lanes
print('Lanes outside the release set exit 2 before any upload')

with tempfile.TemporaryDirectory() as directory:
    base=Path(directory)
    scripts,env,wasms=orchestrator(base,LANE_TIMEOUT='30s',E2E_LANE_STAGGER='2')
    env['E2E_LANES']='agg-core prod-full sdk stress prod-caller'
    stub='#!/bin/bash\necho "lane $RUN_TS $(python3 -c "import time; print(time.time())")" >> "$STELLAR_CALLS"\necho "run complete"\n'
    for name in ['full_e2e.sh','production.sh','sdk.sh']:(scripts/name).write_text(stub)
    (scripts/'assert_green.sh').write_text('#!/bin/bash\necho "GREEN $RUN_TS"\n')
    done=subprocess.run(['bash',str(scripts/'parallel_e2e.sh')],env=env,capture_output=True,text=True,timeout=60)
    assert done.returncode==0,done.stderr
    started={line.split()[1][len('fixture-'):]:float(line.split()[2]) for line in (base/'calls').read_text().splitlines() if line.startswith('lane ')}
    assert sorted(started)==['agg-core','prod-caller','prod-full','sdk','stress'],started
    assert max(started['prod-full'],started['prod-caller'],started['stress'])+1.5<=min(started['agg-core'],started['sdk']),started
    launched=[line.split("'")[1] for line in done.stderr.splitlines() if 'launching lane' in line]
    assert launched==['prod-full','stress','prod-caller','agg-core','sdk'],launched
    gated=[line.split()[1][len('fixture-'):] for line in done.stdout.splitlines() if line.startswith('GREEN ')]
    assert gated==['agg-core','prod-full','sdk','stress','prod-caller'],gated
print('Critical lanes start first; the rest start after E2E_LANE_STAGGER; gating keeps lane order')

with tempfile.TemporaryDirectory() as directory:
    base=Path(directory)
    scripts,env,wasms=orchestrator(base,LANE_TIMEOUT='30s',E2E_LANE_STAGGER='1')
    env['E2E_LANES']='agg-core prod-full'
    lane='''#!/bin/bash
run="$INTEG_DIR/runs/$RUN_TS"
mkdir -p "$run"
printf '{"selected_cases":["only"]}' > "$run/metadata.json"
printf 'id\\tstatus\\tfirst_action\\tlast_action\\n' > "$run/cases.tsv"
'''
    (scripts/'full_e2e.sh').write_text(lane+'echo "run complete"\n')
    (scripts/'production.sh').write_text(lane+'exit 1\n')
    (scripts/'assert_green.sh').write_text('#!/bin/bash\necho "GREEN $RUN_TS"\n')
    done=subprocess.run(['bash',str(scripts/'parallel_e2e.sh')],env=env,capture_output=True,text=True,timeout=60)
    assert done.returncode!=0,done.stderr
    assert "lane 'prod-full' FAILED — process did not exit cleanly (1)" in done.stderr and "lane 'agg-core' GREEN" in done.stderr,done.stderr
    assert 'incomplete' in (base/'runs/fixture-prod-full/cases.tsv').read_text()
    assert 'incomplete' not in (base/'runs/fixture-agg-core/cases.tsv').read_text()
print('After the critical-first reorder, each lane keeps its own exit code and incomplete mark')

with tempfile.TemporaryDirectory() as directory:
    base=Path(directory)
    scripts,env,wasms=orchestrator(base,LANE_TIMEOUT='60s',E2E_LANE_STAGGER='29')
    env['E2E_LANES']='agg-core stress'
    (scripts/'full_e2e.sh').write_text('''#!/bin/bash
echo "lane $RUN_TS" >> "$STELLAR_CALLS"
run="$INTEG_DIR/runs/$RUN_TS"
mkdir -p "$run"
printf '{"selected_cases":["unfinished"]}' > "$run/metadata.json"
printf 'id\\tstatus\\tfirst_action\\tlast_action\\n' > "$run/cases.tsv"
sleep 300 &
child=$!
echo "$child" > "$run/child.pid"
trap 'kill "$child" 2>/dev/null || true; wait "$child" 2>/dev/null; exit 130' TERM INT
wait "$child"
''')
    process=subprocess.Popen(['bash',str(scripts/'parallel_e2e.sh')],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    marker=base/'runs/fixture-stress/child.pid'
    for _ in range(1000):
        if marker.exists():break
        if process.poll() is not None:raise AssertionError(process.communicate())
        time.sleep(.02)
    time.sleep(.5)
    cancelled=time.monotonic()
    process.send_signal(signal.SIGTERM)
    out,err=process.communicate(timeout=15)
    assert process.returncode==130 and time.monotonic()-cancelled<10,(process.returncode,err)
    assert 'other lanes start in 29s' in err and "launching lane 'agg-core'" not in err,err
    for _ in range(100):
        if subprocess.run(['pgrep','-fx','sleep 29'],capture_output=True).returncode==1:break
        time.sleep(.02)
    else:raise AssertionError('the stagger sleep outlives the cancelled orchestrator')
    assert not (base/'runs/fixture-agg-core').exists() and 'lane fixture-agg-core' not in (base/'calls').read_text()
    assert 'incomplete' in (base/'runs/fixture-stress/cases.tsv').read_text()
print('Cancellation during the stagger delay stops at once, reaps started lanes and never starts the rest')

for bad in ['-1','x','12345']:
    with tempfile.TemporaryDirectory() as directory:
        base=Path(directory)
        scripts,env,wasms=orchestrator(base,LANE_TIMEOUT='1s',E2E_LANE_STAGGER=bad)
        done=subprocess.run(['bash',str(scripts/'parallel_e2e.sh')],env=env,capture_output=True,text=True,timeout=30)
        assert done.returncode==2 and 'invalid E2E_LANE_STAGGER' in done.stderr and not (base/'calls').exists(),done.stderr
print('An invalid E2E_LANE_STAGGER exits 2 before any upload')

with tempfile.TemporaryDirectory() as directory:
    base=Path(directory)
    scripts,env,wasms=orchestrator(base,LANE_TIMEOUT='30s',E2E_LANE_STAGGER='2',E2E_LANE_GAP='1')
    env['E2E_LANES']='agg-core prod-full sdk stress'
    stub='#!/bin/bash\necho "lane $RUN_TS $(python3 -c "import time; print(time.time())")" >> "$STELLAR_CALLS"\necho "run complete"\n'
    for name in ['full_e2e.sh','production.sh','sdk.sh']:(scripts/name).write_text(stub)
    (scripts/'assert_green.sh').write_text('#!/bin/bash\necho "GREEN $RUN_TS"\n')
    done=subprocess.run(['bash',str(scripts/'parallel_e2e.sh')],env=env,capture_output=True,text=True,timeout=60)
    assert done.returncode==0,done.stderr
    started={line.split()[1][len('fixture-'):]:float(line.split()[2]) for line in (base/'calls').read_text().splitlines() if line.startswith('lane ')}
    order=['prod-full','stress','agg-core','sdk']
    gaps=[started[b]-started[a] for a,b in zip(order,order[1:])]
    assert gaps[0]>=0.9 and gaps[1]>=1.9 and gaps[2]>=0.9,(gaps,started)
print('E2E_LANE_GAP spaces every launch; the stagger replaces the gap before the first non-critical lane')

for bad in ['-1','x','1000']:
    with tempfile.TemporaryDirectory() as directory:
        base=Path(directory)
        scripts,env,wasms=orchestrator(base,LANE_TIMEOUT='1s',E2E_LANE_GAP=bad)
        done=subprocess.run(['bash',str(scripts/'parallel_e2e.sh')],env=env,capture_output=True,text=True,timeout=30)
        assert done.returncode==2 and 'invalid E2E_LANE_GAP' in done.stderr and not (base/'calls').exists(),done.stderr
print('An invalid E2E_LANE_GAP exits 2 before any upload')

with tempfile.TemporaryDirectory() as directory:
    base=Path(directory)
    scripts,env,wasms=orchestrator(base,LANE_TIMEOUT='30s')
    (scripts/'full_e2e.sh').write_text('#!/bin/bash\nmkdir -p "$INTEG_DIR/runs/$RUN_TS"; echo "{}" > "$INTEG_DIR/runs/$RUN_TS/metadata.json"; echo "run complete"\n')
    (scripts/'assert_green.sh').write_text('#!/bin/bash\necho started > "$INTEG_DIR/runs/$RUN_TS.gate-started"\nsleep 6\necho passed > "$INTEG_DIR/runs/$RUN_TS.gate-passed"\n')
    process=subprocess.Popen(['bash',str(scripts/'parallel_e2e.sh')],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    lane=env['E2E_LANES'].split()[0]
    started=base/f'runs/fixture-{lane}.gate-started'
    for _ in range(1000):
        if started.exists(): break
        if process.poll() is not None: raise AssertionError(process.communicate())
        time.sleep(.02)
    assert started.exists()
    process.send_signal(signal.SIGTERM)
    process.communicate(timeout=30)
    assert process.returncode!=0
    time.sleep(7)
    assert not (base/f'runs/fixture-{lane}.gate-passed').exists(), 'a gate survived cancellation'
print('Cancellation during gating stops the gate processes before marking the run incomplete')
