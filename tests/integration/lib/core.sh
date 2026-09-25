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
    sdk_version='1.0.220', stellar_sdk_version='16.0.1', network='testnet', rpc_url=rpc,
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
    touch "$STATE_ENV"
    grep -v "^${key}=" "$STATE_ENV" > "$STATE_ENV.tmp" 2>/dev/null || true
    printf '%s=%q\n' "$key" "$value" >> "$STATE_ENV.tmp"
    mv "$STATE_ENV.tmp" "$STATE_ENV"
    eval "$key=\$value"
}

record() {
    local seq
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
    python3 - "$RUN_DIR" "$PHASE" "$@" <<'PYBEGIN'
import json, sys
from datetime import datetime, timezone
from pathlib import Path
run, phase, label, method, sequence, attempt, out, err, contract = sys.argv[1:]
root = Path(run)
Path(out).write_text('')
Path(err).write_text('')
item = dict(action_seq=int(sequence), phase=phase, label=label, method=method,
    attempt=int(attempt), contract=contract or None, started_at=datetime.now(timezone.utc).isoformat(),
    stdout=str(Path(out).relative_to(root)), stderr=str(Path(err).relative_to(root)))
path = root/'active-attempt.json'
temporary = path.with_suffix('.json.tmp')
temporary.write_text(json.dumps(item)+'\n')
temporary.replace(path)
PYBEGIN
}

record_attempt() {
    local label="$1" fn="$2" sequence="$3" attempt="$4" rc="$5" hash="$6" out="$7" err="$8" contract="${9:-}"
    local ordinal=1
    [ ! -f "$RUN_DIR/attempts.jsonl" ] || ordinal=$(( $(wc -l < "$RUN_DIR/attempts.jsonl") + 1 ))
    local prefix="$LOG_DIR/$label.s$sequence.a$attempt.t$ordinal"
    cp "$out" "$prefix.out" && cp "$err" "$prefix.err" || return 1
    python3 - "$RUN_DIR" "$PHASE" "$label" "$fn" "$sequence" "$attempt" "$rc" "$hash" "$prefix" "$contract" "$ordinal" <<'PYATTEMPT'
import json, sys
from datetime import datetime, timezone
from pathlib import Path
run, phase, label, method, sequence, attempt, rc, hash_, prefix, contract, ordinal = sys.argv[1:]
root = Path(run)
item = dict(id=int(ordinal), action_seq=int(sequence), phase=phase, label=label, method=method,
    attempt=int(attempt), cli_exit=int(rc), hash=hash_ or None, contract=contract or None,
    observed_at=datetime.now(timezone.utc).isoformat(),
    stdout=str(Path(prefix+'.out').relative_to(root)), stderr=str(Path(prefix+'.err').relative_to(root)),
    receipt='logs/'+hash_+'.receipt.json' if hash_ else None)
active = root/'active-attempt.json'
if active.exists():
    item['started_at'] = json.loads(active.read_text())['started_at']
with (root/'attempts.jsonl').open('a') as stream:
    stream.write(json.dumps(item)+'\n')
active.unlink(missing_ok=True)
PYATTEMPT
}

run_summary() {
    awk -F'\t' 'NR>1 {c[$4]++} END {for (k in c) printf "  %s: %d\n", k, c[k]}' "$ACTIONS_TSV" >&2
}

finish_run() {
    local rc="$1"
    if [ "$rc" -ne 0 ]; then
        python3 "$INTEG_DIR/gate.py" mark-incomplete "$RUN_DIR" "scenario exit $rc" || true
    fi
    python3 "$INTEG_DIR/gate.py" summary "$RUN_DIR" completed "$rc" || true
    write_report
    run_summary
}

die() {
    local label="$1" msg="$2"
    log "FATAL [$label]: $msg"
    record "$label" FAIL fatal "" "" "" "" "" "$msg"
    exit 1
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
    local id="$1" first last rc=0; shift
    if [ "${E2E_RESUME:-0}" = 1 ] && awk -F'\t' -v id="$id" '$1==id && $2=="pass" {found=1} END {exit !found}' "$RUN_DIR/cases.tsv"; then
        log "resuming completed case $id"
        return 0
    fi
    [ ! -e "$RUN_DIR/active-case" ] || { echo "unfinished case cannot be replayed" >&2; return 1; }
    first=$(wc -l < "$ACTIONS_TSV")
    printf '%s\t%s\n' "$id" "$first" > "$RUN_DIR/active-case"
    "$@" || rc=$?
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
