init_run() {
    if [ -e "$RUN_DIR" ] && [ "${E2E_RESUME:-0}" != 1 ]; then
        echo "run directory exists; choose a fresh RUN_TS or explicit E2E_RESUME=1" >&2
        exit 1
    fi
    umask 077
    mkdir -p "$RUN_DIR" "$LOG_DIR" "$RUN_DIR/private"
    export XDG_CONFIG_HOME="$RUN_DIR/private"
    export STELLAR_NO_CACHE=true
    if [ ! -f "$RUN_DIR/cases.tsv" ]; then
        printf 'id\tstatus\tfirst_action\tlast_action\n' > "$RUN_DIR/cases.tsv"
    fi
    python3 - "$INTEG_DIR" "$RUN_DIR" "${E2E_LANE:?}" "${INSTRUCTION_LEEWAY:-20000000}" "$NETWORKS_FILE" "$RPC_URL" "$NETWORK_PASSPHRASE" "$RUN_TS" <<'PYMETA'
import hashlib, json, subprocess, sys, os
from datetime import datetime, timezone
from pathlib import Path
base, run, lane, leeway, config, rpc, passphrase, run_id = sys.argv[1:]
manifest = json.loads((Path(base) / 'cases.json').read_text())
metadata = dict(lane=lane, selected_cases=[c['id'] for c in manifest if lane in c['lanes']],
    source_sha=subprocess.check_output(['git','rev-parse','HEAD'], text=True).strip(),
    instruction_leeway=int(leeway), cli_version=subprocess.check_output(['stellar','--version'], text=True).strip(),
    configuration_sha256=hashlib.sha256(Path(config).read_bytes()).hexdigest(),
    case_manifest_sha256=hashlib.sha256((Path(base)/'cases.json').read_bytes()).hexdigest(),
    sdk_lock_sha256=hashlib.sha256((Path(base)/'sdk/package-lock.json').read_bytes()).hexdigest(),
    sdk_version='1.0.221', stellar_sdk_version='16.3.0', network='testnet', rpc_url=rpc,
    network_passphrase=passphrase, run_id=run_id,
    workflow_run_id=os.environ.get('GITHUB_RUN_ID'), workflow_run_attempt=os.environ.get('GITHUB_RUN_ATTEMPT'))
path = Path(run)/'metadata.json'
if path.exists():
    previous = json.loads(path.read_text())
    if {key: value for key, value in previous.items() if key != 'started_at'} != metadata:
        raise SystemExit('resume identity changed; choose a fresh run')
else:
    metadata['started_at'] = datetime.now(timezone.utc).isoformat()
    path.write_text(json.dumps(metadata,indent=2)+'\n')
PYMETA
    [ "$?" -eq 0 ] || exit 1
    if [ ! -f "$ACTIONS_TSV" ]; then
        printf 'seq\tphase\tlabel\tstatus\tfn\thash\tinstructions\tread_bytes\twrite_bytes\tresource_fee\tnote\n' > "$ACTIONS_TSV"
    fi

    python3 "$INTEG_DIR/artifacts.py" check "$WASM_DIR" || exit 1
    if [ -f "$RUN_DIR/candidate.json" ]; then
        cmp -s "$WASM_DIR/candidate.json" "$RUN_DIR/candidate.json" || { echo "resume candidate changed" >&2; exit 1; }
    fi
    cp "$WASM_DIR/candidate.json" "$RUN_DIR/candidate.json"
    if [ -f "$WASM_DIR/controlled.json" ]; then
        cp "$WASM_DIR/controlled.json" "$RUN_DIR/controlled.json" || exit 1
        cp "$WASM_DIR/controlled-tests.log" "$RUN_DIR/controlled-tests.log" || exit 1
    fi
    if [ -f "$RUN_DIR/network-limits.json" ]; then
        : # Resume retains the original captured limits and ledger provenance.
    elif [ -n "${E2E_LIMITS_FILE:-}" ]; then
        cp "$E2E_LIMITS_FILE" "$RUN_DIR/network-limits.json" || exit 1
    else
        "${NODE_BIN:-node}" "$INTEG_DIR/sdk/limits.mjs" "$NETWORKS_FILE" "$RUN_DIR/network-limits.json" || exit 1
    fi
    if [ -e "$RUN_DIR/active-case" ] || [ -e "$RUN_DIR/active-attempt.json" ] || [ -e "$RUN_DIR/interruption.json" ]; then
        echo "interrupted case/attempt cannot be replayed safely; start a fresh run" >&2
        exit 1
    fi
    if ! awk -F'\t' 'NR>1 && $2!="pass" {exit 1}' "$RUN_DIR/cases.tsv"; then
        echo "failed/incomplete cases cannot be resumed; start a fresh run" >&2
        exit 1
    fi
    [ -f "$STATE_ENV" ] && source "$STATE_ENV"
    PHASE="${PHASE:-init}"
    python3 "$INTEG_DIR/gate.py" summary "$RUN_DIR" running || exit 1
}

phase() {
    PHASE="$1"
    log "===== PHASE: $PHASE ====="
}

log() {
    printf '[%s] %s\n' "$(date +%H:%M:%S)" "$*" >&2
}

save_state() {
    local key="$1" value="$2"
    if [ -n "${E2E_JOB:-}" ]; then
        printf '%s=%q\n' "$key" "$value" >> "$E2E_JOB_DIR/state.part" || return 1
        eval "$key=\$value"
        return 0
    fi
    group_guard "save_state $key" || return 1
    touch "$STATE_ENV"
    grep -v "^${key}=" "$STATE_ENV" > "$STATE_ENV.tmp" 2>/dev/null || true
    printf '%s=%q\n' "$key" "$value" >> "$STATE_ENV.tmp"
    mv "$STATE_ENV.tmp" "$STATE_ENV"
    eval "$key=\$value"
}

next_seq() {
    if [ -n "${E2E_JOB:-}" ]; then
        echo $(( $(wc -l < "$E2E_JOB_DIR/actions.part") + 1 ))
    else
        echo $(( $(wc -l < "$ACTIONS_TSV") ))
    fi
}

job_log() {
    printf '%s\n' "$LOG_DIR/$1${E2E_JOB:+.j$E2E_JOB}"
}

group_violation() {
    log "refused while group ${GROUP_DIR##*/} is open: $1"
    printf '%s\n' "$1" >> "$GROUP_DIR/violations"
    return 1
}

group_guard() {
    [ -z "${GROUP_DIR:-}" ] || [ -n "${GROUP_REPLAYING:-}" ] || group_violation "$1"
}

group_marker_open() {
    [ -f "$RUN_DIR/active-attempt.json" ] && jq -e 'has("group")' "$RUN_DIR/active-attempt.json" >/dev/null 2>&1
}

job_refuse() {
    log "refused in group job $E2E_JOB: $1 [$2]"
    record "$2" FAIL "$1" "" "" "" "" "" "$1 is serial-only; refused in group job $E2E_JOB"
}

record() {
    local seq
    if [ -n "${E2E_JOB:-}" ]; then
        { printf '%q ' "$PHASE" "$1" "$2" "$3" "${4:-}" "${5:-}" "${6:-}" "${7:-}" "${8:-}" "${9:-}" "${10:-}" "${11:-}"; echo; } \
            >> "$E2E_JOB_DIR/actions.part"
        return
    fi
    group_guard "record $1" || return 1
    seq=$(($(wc -l < "$ACTIONS_TSV")))
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$seq" "$PHASE" "$1" "$2" "$3" "${4:-}" "${5:-}" "${6:-}" "${7:-}" "${8:-}" "${9:-}" >> "$ACTIONS_TSV"
    if [ ! -f "$RUN_DIR/evidence.tsv" ]; then
        printf 'seq\texecution\tcontract\n' > "$RUN_DIR/evidence.tsv"
    fi
    printf '%s\t%s\t%s\n' "$seq" "${10:-assertion}" "${11:-}" >> "$RUN_DIR/evidence.tsv"
}

# Keep every CLI attempt separate, including attempts interrupted before receipt
# polling finishes. actions.tsv remains the stable action/result interface.
begin_attempt() {
    local marker="$RUN_DIR/active-attempt.json"
    if [ -n "${E2E_JOB:-}" ]; then
        marker="$E2E_JOB_DIR/active.json"
    else
        group_guard "begin_attempt $1" || return 1
        ! group_marker_open || { record "$1" FAIL "$2" "" "" "" "" "" "an unfinished group marker is still active"; return 1; }
    fi
    python3 - "$RUN_DIR" "$PHASE" "$marker" "$@" <<'PYBEGIN'
import json, sys
from datetime import datetime, timezone
from pathlib import Path
run, phase, marker, label, method, sequence, attempt, out, err, contract = sys.argv[1:]
root = Path(run)
Path(out).write_text('')
Path(err).write_text('')
item = dict(action_seq=int(sequence), phase=phase, label=label, method=method,
    attempt=int(attempt), contract=contract or None, started_at=datetime.now(timezone.utc).isoformat(),
    stdout=str(Path(out).relative_to(root)), stderr=str(Path(err).relative_to(root)))
path = Path(marker)
temporary = path.with_suffix('.json.tmp')
temporary.write_text(json.dumps(item)+'\n')
temporary.replace(path)
PYBEGIN
}

record_attempt() {
    local label="$1" fn="$2" sequence="$3" attempt="$4" rc="$5" hash="$6" out="$7" err="$8" contract="${9:-}"
    local ordinal=1 sink="$RUN_DIR/attempts.jsonl" marker="$RUN_DIR/active-attempt.json"
    if [ -n "${E2E_JOB:-}" ]; then
        sink="$E2E_JOB_DIR/attempts.part" marker="$E2E_JOB_DIR/active.json"
    else
        group_guard "record_attempt $label" || return 1
    fi
    [ ! -f "$sink" ] || ordinal=$(( $(wc -l < "$sink") + 1 ))
    local prefix
    prefix="$(job_log "$label").s$sequence.a$attempt.t$ordinal"
    cp "$out" "$prefix.out" && cp "$err" "$prefix.err" || return 1
    python3 - "$RUN_DIR" "$PHASE" "$label" "$fn" "$sequence" "$attempt" "$rc" "$hash" "$prefix" "$contract" "$ordinal" "$sink" "$marker" "${E2E_JOB:-}" <<'PYATTEMPT'
import json, sys
from datetime import datetime, timezone
from pathlib import Path
run, phase, label, method, sequence, attempt, rc, hash_, prefix, contract, ordinal, sink, marker, job = sys.argv[1:]
root = Path(run)
item = dict(id=None if job else int(ordinal), action_seq=int(sequence), phase=phase, label=label, method=method,
    attempt=int(attempt), cli_exit=int(rc), hash=hash_ or None, contract=contract or None,
    observed_at=datetime.now(timezone.utc).isoformat(),
    stdout=str(Path(prefix+'.out').relative_to(root)), stderr=str(Path(prefix+'.err').relative_to(root)),
    receipt='logs/'+hash_+'.receipt.json' if hash_ else None)
active = Path(marker)
if active.exists():
    item['started_at'] = json.loads(active.read_text())['started_at']
with Path(sink).open('a') as stream:
    stream.write(json.dumps(item)+'\n')
active.unlink(missing_ok=True)
PYATTEMPT
}

run_summary() {
    awk -F'\t' 'NR>1 {c[$4]++} END {for (k in c) printf "  %s: %d\n", k, c[k]}' "$ACTIONS_TSV" >&2
}

finish_run() {
    local rc="$1"
    [ -z "${GROUP_DIR:-}" ] || group_abort
    if [ "$rc" -ne 0 ]; then
        python3 "$INTEG_DIR/gate.py" mark-incomplete "$RUN_DIR" "scenario exit $rc" || true
    fi
    python3 "$INTEG_DIR/gate.py" summary "$RUN_DIR" completed "$rc" || true
    write_report
    run_summary
}

die() {
    local label="$1" msg="$2"
    [ -n "${E2E_JOB:-}" ] || [ -z "${GROUP_DIR:-}" ] || group_abort
    log "FATAL [$label]: $msg"
    record "$label" FAIL fatal "" "" "" "" "" "$msg"
    exit 1
}

kill_tree() {
    local sig="$1" pid="$2" child
    kill -STOP "$pid" 2>/dev/null || return 0
    for child in $(pgrep -P "$pid" 2>/dev/null); do kill_tree "$sig" "$child"; done
    kill "$sig" "$pid" 2>/dev/null
    kill -CONT "$pid" 2>/dev/null
    return 0
}

job_alive() {
    ps -o stat= -p "$1" 2>/dev/null | grep -qv '^Z'
}

group_begin() {
    local name="$1" width="${2:-}" mode="${3:-}"
    [ -z "${E2E_JOB:-}" ] || { job_refuse group_begin "group_$name"; return 1; }
    [ -z "${GROUP_DIR:-}" ] || { group_violation "group_begin $name"; return 1; }
    ! group_marker_open || { record "group_$name" FAIL group "" "" "" "" "" "an unfinished group marker is still active"; return 1; }
    if [[ ! "$name" =~ ^[A-Za-z0-9_]+$ || ! "$width" =~ ^[1-9][0-9]?$ ]] || { [ -n "$mode" ] && [ "$mode" != reads ]; }; then
        record "group_$name" FAIL group "" "" "" "" "" "group refused: width '$width', mode '$mode'"
        return 1
    fi
    if [ "$mode" != reads ] && [ -z "${CHANNELS:-}" ]; then
        record "group_$name" FAIL group "" "" "" "" "" "write group refused: no channel sources in CHANNELS"
        return 1
    fi
    [ "$mode" != reads ] || group_ledger_floor "$name" || return 1
    local dir
    mkdir -p "$RUN_DIR/jobs" && dir=$(mktemp -d "$RUN_DIR/jobs/$name.XXXXXX") \
        || { record "group_$name" FAIL group "" "" "" "" "" "cannot create the group directory"; return 1; }
    python3 - "$RUN_DIR" "$PHASE" "$name" "$(next_seq)" "${dir#"$RUN_DIR"/}" <<'PYGROUP' \
        || { rm -rf "$dir"; record "group_$name" FAIL group "" "" "" "" "" "cannot write the group marker"; return 1; }
import json, sys
from datetime import datetime, timezone
from pathlib import Path
run, phase, name, sequence, group = sys.argv[1:]
item = dict(action_seq=int(sequence), phase=phase, label='group:'+name, method='group', attempt=1, contract=None,
    started_at=datetime.now(timezone.utc).isoformat(), stdout=group, stderr=group+'/signing.err', group=group)
path = Path(run)/'active-attempt.json'
temporary = path.with_suffix('.json.tmp')
temporary.write_text(json.dumps(item)+'\n')
temporary.replace(path)
PYGROUP
    GROUP_DIR="$dir" GROUP_LAST="$dir" GROUP_ID="${dir##*.}" GROUP_NAME="$name" GROUP_WIDTH="$width" GROUP_N=0 GROUP_SUBSHELL="$BASH_SUBSHELL"
    GROUP_SRCS=(${CHANNELS:-}) GROUP_PIDS=() GROUP_BATCH=()
}

group_ledger_floor() {
    local label="${1}_ledger_floor" floor latest='' n start left
    floor=$(python3 - "$LOG_DIR" <<'PYFLOOR'
import json, re, sys
from pathlib import Path
floor = 0
for path in Path(sys.argv[1]).iterdir():
    if re.fullmatch(r'[0-9a-f]{64}\.receipt\.json', path.name):
        result = json.loads(path.read_text()).get('result')
        if isinstance(result, dict) and result.get('status') in ('SUCCESS', 'FAILED'):
            if type(result.get('ledger')) is not int or result['ledger'] <= 0:
                raise SystemExit(f'{path.name}: final receipt without a ledger')
            floor = max(floor, result['ledger'])
print(floor)
PYFLOOR
    ) || { log "ASSERT FAIL [$label]: unreadable receipt ledgers"; record "$label" FAIL assert '' '' '' '' '' 'unreadable receipt ledgers in the lane logs'; return 1; }
    start=$SECONDS
    for ((n = 0; n <= 30; n++)); do
        [ "$n" -eq 0 ] || sleep 1
        left=$((start + 30 - SECONDS))
        [ "$left" -gt 0 ] || break
        latest=$(latest_ledger "$left") || latest=''
        if [[ "$latest" =~ ^[0-9]{1,18}$ ]] && [ "$latest" -ge "$floor" ]; then
            record "$label" ok assert '' '' '' '' '' "latest=$latest floor=$floor"
            return
        fi
    done
    log "ASSERT FAIL [$label]: RPC ledger ${latest:-unknown} below the lane floor $floor after $((SECONDS - start)) s"
    record "$label" FAIL assert '' '' '' '' '' "latest=${latest:-unknown} below floor=$floor after $((SECONDS - start)) s"
    return 1
}

group_spawn() {
    local dir src='' pid
    [ -z "${E2E_JOB:-}" ] || { job_refuse group_spawn "group_spawn_$1"; return 1; }
    [ -n "${GROUP_DIR:-}" ] || { record "group_spawn_$1" FAIL group "" "" "" "" "" "no open group"; return 1; }
    [ "$BASH_SUBSHELL" = "$GROUP_SUBSHELL" ] || {
        log "refused: group_spawn $1 in a subshell of group ${GROUP_DIR##*/}"
        printf '%s\n' "group_spawn $1 in a subshell" >> "$GROUP_DIR/untracked"
        return 1
    }
    GROUP_N=$((GROUP_N + 1))
    dir="$GROUP_DIR/$GROUP_N"
    mkdir "$dir" && : > "$dir/actions.part" || { group_violation "group_spawn $1: cannot create $dir"; return 1; }
    [ "${#GROUP_SRCS[@]}" -eq 0 ] || src="${GROUP_SRCS[$(( (GROUP_N - 1) % ${#GROUP_SRCS[@]} ))]}"
    (
        exec 8>"$dir/alive" && python3 -c 'import fcntl; fcntl.flock(8, fcntl.LOCK_EX | fcntl.LOCK_NB)' || exit 1
        E2E_JOB="$GROUP_ID-$GROUP_N" E2E_JOB_DIR="$dir" E2E_SRC="$src"; "$@"; echo "$?" > "$dir/rc"
    ) \
        </dev/null >"$dir/stdout" 2>"$dir/stderr" &
    GROUP_PIDS+=("$!") GROUP_BATCH+=("$!")
    if [ "${#GROUP_BATCH[@]}" -ge "$GROUP_WIDTH" ]; then
        for pid in "${GROUP_BATCH[@]}"; do wait "$pid" 2>/dev/null || :; done
        GROUP_BATCH=()
    fi
}

group_gaps() {
    python3 - "$GROUP_DIR" "$GROUP_N" <<'PYGAPS'
import fcntl, os, sys
from pathlib import Path
group, count = Path(sys.argv[1]), int(sys.argv[2])
jobs = sorted(int(p.name) for p in group.iterdir() if p.name.isdigit())
gaps = [] if jobs == list(range(1, count + 1)) else [f'job directories {jobs} differ from the {count} spawned jobs']
for n in jobs:
    alive = group/str(n)/'alive'
    if alive.exists():
        fd = os.open(alive, os.O_RDONLY)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            gaps.append(f'job {n} left a live process')
        finally:
            os.close(fd)
path = group/'untracked'
gaps += path.read_text().splitlines() if path.exists() else []
print('; '.join(gaps))
PYGAPS
}

group_end() {
    local n dir rc pid first kept='' marker gaps unfinished=''
    [ -z "${E2E_JOB:-}" ] || { job_refuse group_end group_end; return 1; }
    [ -n "${GROUP_DIR:-}" ] || { record group_end FAIL group "" "" "" "" "" "no open group"; return 1; }
    for pid in ${GROUP_BATCH[@]+"${GROUP_BATCH[@]}"}; do wait "$pid" 2>/dev/null || :; done
    GROUP_BATCH=()
    gaps=$(group_gaps) || gaps='cannot check the job directories'
    first=$(next_seq)
    GROUP_REPLAYING=1
    for ((n = 1; n <= GROUP_N; n++)); do
        dir="$GROUP_DIR/$n"
        cat "$dir/stderr" >&2
        group_replay "$dir" || record "group_${GROUP_NAME}_replay_$n" FAIL group "" "" "" "" "" "job journal replay failed"
        rc=$(cat "$dir/rc" 2>/dev/null) || rc=''
        [ "$rc" = 0 ] || record "group_${GROUP_NAME}_job_$n" FAIL group '' '' '' '' '' "job exited ${rc:-without status}"
    done
    [ ! -s "$GROUP_DIR/violations" ] || record "group_${GROUP_NAME}_parent_writes" FAIL group '' '' '' '' '' \
        "$(wc -l < "$GROUP_DIR/violations" | tr -d ' ') parent writes refused while the group was open: $(tr '\n' ';' < "$GROUP_DIR/violations")"
    [ -z "$gaps" ] || { kept=1; record "group_${GROUP_NAME}_untracked" FAIL group '' '' '' '' '' "$gaps"; }
    for marker in "$GROUP_DIR"/*/active.json; do
        [ ! -e "$marker" ] || { dir="${marker%/active.json}"; unfinished="$unfinished ${dir##*/}"; }
    done
    [ -z "$unfinished" ] || { kept=1; record "group_${GROUP_NAME}_unfinished" FAIL group '' '' '' '' '' \
        "an attempt did not finish in job(s)$unfinished; the group marker is kept"; }
    unset GROUP_REPLAYING
    if [ -n "$kept" ]; then
        log "group ${GROUP_DIR##*/}: journals incomplete; keeping the group marker"
        unset GROUP_DIR; GROUP_PIDS=()
        return 1
    fi
    rm -f "$RUN_DIR/active-attempt.json"
    unset GROUP_DIR; GROUP_PIDS=()
    awk -F'\t' -v first="$first" 'NR>1 && $1>=first && ($4=="FAIL" || $4=="UNEXPECTED-OK") {exit 1}' "$ACTIONS_TSV"
}

group_abort() {
    local pid end=$((SECONDS + 20)) alive
    for pid in ${GROUP_BATCH[@]+"${GROUP_BATCH[@]}"}; do kill_tree -TERM "$pid"; done
    while :; do
        alive=''
        for pid in ${GROUP_BATCH[@]+"${GROUP_BATCH[@]}"}; do job_alive "$pid" && alive="$alive $pid"; done
        [ -n "$alive" ] && [ "$SECONDS" -lt "$end" ] || break
        command sleep 0.2
    done
    for pid in $alive; do kill_tree -KILL "$pid"; done
    for pid in ${GROUP_BATCH[@]+"${GROUP_BATCH[@]}"}; do wait "$pid" 2>/dev/null || :; done
    log "group ${GROUP_DIR##*/} aborted; its marker is kept"
    unset GROUP_DIR; GROUP_PIDS=() GROUP_BATCH=()
}

group_out() {
    cat "$GROUP_LAST/$1/stdout"
}

group_each() {
    local name="$1" width="$2" items="$3" item
    shift 3
    group_begin "$name" "$width" || return 1
    while IFS= read -r item; do
        [ -z "$item" ] || group_spawn "$@" "$item"
    done <<<"$items"
    group_end
}

group_replay() {
    local dir="$1" line ps key value
    python3 - "$RUN_DIR" "$LOG_DIR" "$dir" "$GROUP_ID-${dir##*/}" "$(next_seq)" <<'PYREPLAY' || return 1
import json, shutil, sys
from pathlib import Path
run, logs, job_dir, job, base = sys.argv[1:]
part, sink = Path(job_dir)/'attempts.part', Path(run)/'attempts.jsonl'
count = len(sink.read_text().splitlines()) if sink.exists() else 0
labels = []
with sink.open('a') as stream:
    for line in part.read_text().splitlines() if part.exists() else []:
        item = json.loads(line)
        count += 1
        item['id'] = count
        item['action_seq'] = int(base) + item['action_seq'] - 1
        stream.write(json.dumps(item)+'\n')
        labels.append(item['label'])
for label in dict.fromkeys(labels):
    for suffix in ('.out', '.err'):
        source = Path(logs)/f'{label}.j{job}{suffix}'
        if source.exists():
            shutil.copyfile(source, Path(logs)/f'{label}{suffix}')
PYREPLAY
    while IFS= read -r line; do
        eval "set -- $line"
        ps="$PHASE" PHASE="$1"
        shift
        record "$@"
        PHASE="$ps"
    done < "$dir/actions.part"
    [ ! -s "$dir/deployed.part" ] || cat "$dir/deployed.part" >> "$RUN_DIR/deployed-artifacts.jsonl" || return 1
    [ -f "$dir/state.part" ] || return 0
    while IFS= read -r line; do
        key="${line%%=*}"
        eval "value=${line#*=}"
        save_state "$key" "$value" || return 1
    done < "$dir/state.part"
}

slot_take() {
    local prefix="$1" count="$2" base="$3" deadline="$4" fd got=""
    SLOT_FD=""
    [[ "$count" =~ ^[1-9][0-9]?$ && "$base" =~ ^[1-9][0-9]{0,2}$ && "$deadline" =~ ^[0-9]{1,5}$ ]] \
        && [ $((base + count)) -le 255 ] || return 1
    for ((fd = base; fd < base + count; fd++)); do
        { : >&"$fd"; } 2>/dev/null && return 1
    done
    mkdir -p "$(dirname "$prefix")" || return 1
    for ((fd = base; fd < base + count; fd++)); do
        eval "exec $fd>>\"\$prefix.$((fd - base + 1))\"" || break
    done
    [ "$fd" -lt $((base + count)) ] || got=$(python3 -c '
import fcntl, sys, time
end = time.monotonic() + int(sys.argv[1])
fds = [int(fd) for fd in sys.argv[2:]]
while True:
    for fd in fds:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            continue
        print(fd)
        sys.exit(0)
    if time.monotonic() >= end:
        sys.exit(1)
    time.sleep(0.5)
' "$deadline" $(seq "$base" $((base + count - 1)))) || got=""
    for ((fd = base; fd < base + count; fd++)); do
        [ "$fd" = "$got" ] || eval "exec $fd>&-"
    done
    [ -n "$got" ] || return 1
    SLOT_FD="$got"
}

rpc_hold() {
    slot_take "$INTEG_DIR/runs/.slots/rpc" "${E2E_RPC_SLOTS:-12}" 150 600
}

throttle_sleep() {
    [[ "$1" =~ ^[1-9][0-9]*$ ]] && [ "$1" -le "${THROTTLE_RETRIES:-6}" ] || return 1
    local s=$((2 << ($1 < 5 ? $1 - 1 : 4)))
    s=$((s + RANDOM % (s / 2 + 1)))
    log "rate-limited read; backoff $1 of ${THROTTLE_RETRIES:-6}: ${s}s"
    sleep "$s"
}

is_contract_id() { [[ "$1" =~ ^C[A-Z2-7]{55}$ ]]; }

is_wasm_hash() { [[ "$1" =~ ^[0-9a-f]{64}$ ]]; }

check_tools() {
    local missing=0 t
    for t in $REQUIRED_TOOLS; do
        if ! command -v "$t" >/dev/null 2>&1; then
            echo "MISSING REQUIRED TOOL: $t" >&2
            missing=1
        fi
    done
    return $missing
}

check_stellar_version() {
    local output ver min major minor min_major min_minor
    output=$(stellar --version 2>/dev/null) || { echo "cannot determine stellar version" >&2; return 1; }
    output=${output%%$'\n'*}
    # Bound numeric fields before shell arithmetic; reject malformed output
    # instead of letting failed integer comparisons fall through to success.
    if [[ ! "$output" =~ ^stellar[[:space:]]+([0-9]{1,9})\.([0-9]{1,9})\.([0-9]{1,9})([-+][0-9A-Za-z.-]+)?([[:space:]].*)?$ ]]; then
        echo "cannot determine stellar version: $output" >&2
        return 1
    fi
    major=$((10#${BASH_REMATCH[1]})); minor=$((10#${BASH_REMATCH[2]}))
    ver=${output#* }; ver=${ver%% *}
    min="${STELLAR_CLI_MIN_VERSION:-22.0}"
    if [[ ! "$min" =~ ^([0-9]{1,9})\.([0-9]{1,9})(\.[0-9]{1,9})?$ ]]; then
        echo "invalid minimum stellar version: $min" >&2
        return 1
    fi
    min_major=$((10#${BASH_REMATCH[1]})); min_minor=$((10#${BASH_REMATCH[2]}))
    if [ "$major" -lt "$min_major" ] || { [ "$major" -eq "$min_major" ] && [ "$minor" -lt "$min_minor" ]; }; then
        echo "stellar CLI $ver < required min $min" >&2
        return 1
    fi
    return 0
}

extract_signing_hash() {
    local f="$1"
    [ -f "$f" ] || return 1
    grep -oE 'Signing transaction: [0-9a-f]{64}' "$f" | tail -1 | awk '{print $3}'
}

sanitize_output() {
    local f="$1"
    [ -f "$f" ] || { echo ""; return 1; }
    tr -d '"\n[:space:]' < "$f"
}

require_var() {
    local name="$1" label="${2:-$1}"
    local val
    eval "val=\"\${$name:-}\""
    [ -n "$val" ] || die "require_$name" "$label is empty (missing from state.env or prior phase)"
}

tail_err_note() {
    local f="$1" n="${2:-300}"
    [ -f "$f" ] || { echo ""; return 0; }
    tail -c "$n" "$f" | tr '\n\t' '  '
}

run_captured() {
    local label="$1" out_f="$2" err_f="$3"; shift 3
    [ "$1" = "--" ] && shift
    "$@" >"$out_f" 2>"$err_f"
}

# The gate requires a terminal record for each checked-in case. Failures in
# command substitutions still append to actions.tsv and remain sticky.
run_case() {
    local id="$1" first last rc=0 sel=0; shift
    [ -z "${E2E_JOB:-}" ] || { job_refuse run_case "$id"; return 1; }
    jq -e --arg id "$id" '.selected_cases | if type == "array" then index($id) != null else error("selected_cases is not an array") end' \
        "$RUN_DIR/metadata.json" >/dev/null 2>&1 || sel=$?
    if [ "$sel" -eq 1 ]; then
        log "case $id not selected for lane ${E2E_LANE:-}"
        return 0
    elif [ "$sel" -ne 0 ]; then
        record "${id}_selection" FAIL case "" "" "" "" "" "case selection unreadable (jq exit $sel)"
        return 1
    fi
    if [ "${E2E_RESUME:-0}" = 1 ] && awk -F'\t' -v id="$id" '$1==id && $2=="pass" {found=1} END {exit !found}' "$RUN_DIR/cases.tsv"; then
        log "resuming completed case $id"
        return 0
    fi
    [ ! -e "$RUN_DIR/active-case" ] || { echo "unfinished case cannot be replayed" >&2; return 1; }
    first=$(wc -l < "$ACTIONS_TSV")
    printf '%s\t%s\n' "$id" "$first" > "$RUN_DIR/active-case"
    "$@" || rc=$?
    if [ -n "${GROUP_DIR:-}" ]; then
        log "case $id returned with group ${GROUP_DIR##*/} still open"
        group_abort
        rc=1
    fi
    last=$(( $(wc -l < "$ACTIONS_TSV") - 1 ))
    if [ "$rc" -eq 0 ] && ! awk -F'\t' -v first="$first" -v last="$last" \
        'NR>1 && $1>=first && $1<=last && ($4=="FAIL" || $4=="UNEXPECTED-OK") {exit 1}' "$ACTIONS_TSV"; then
        log "case $id recorded a failed action despite returning success"
        rc=1
    fi
    printf '%s\t%s\t%s\t%s\n' "$id" "$([ "$rc" -eq 0 ] && echo pass || echo fail)" "$first" "$last" >> "$RUN_DIR/cases.tsv"
    rm "$RUN_DIR/active-case"
    [ "$rc" -eq 0 ] || record "$id" FAIL phase "" "" "" "" "" "case returned $rc"
    return "$rc"
}
