#!/usr/bin/env python3
"""Real orchestrator/process-group cancellation, with network-free lane stubs."""
import json
import os
import shutil
import signal
import subprocess
import tempfile
import time
from pathlib import Path

HERE=Path(__file__).resolve().parent
for cancel in [False,True]:
    with tempfile.TemporaryDirectory() as directory:
        base=Path(directory); scripts=base/'scenarios';scripts.mkdir()
        for name in ['gate.py','resources.py','receipts.py','artifacts.py']:
            shutil.copy(HERE/name,base/name)
        shutil.copy(HERE/'scenarios/parallel_e2e.sh',scripts/'parallel_e2e.sh')
        (base/'env.sh').write_text('INTEG_DIR="$(cd "$HERE/.." && pwd)"\nNETWORKS_FILE=unused\nRUN_TS=fixture\n')
        (base/'no_network').write_text('#!/bin/bash\nexit 0\n');(base/'no_network').chmod(0o755)
        (scripts/'full_e2e.sh').write_text('''#!/bin/bash
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
        env=dict(os.environ,INTEG_DIR=str(base),E2E_LANES='agg',LANE_TIMEOUT='30s' if cancel else '1s',NODE_BIN=str(base/'no_network'))
        process=subprocess.Popen(['bash',str(scripts/'parallel_e2e.sh')],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        marker=base/'runs/fixture-agg/child.pid'
        for _ in range(100):
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
print('Timeout/cancellation regressions: children reaped and incomplete evidence preserved')
