RPC_TRANSIENT_RE='rejected .?50[0-9]|status_code: 50[0-9]|No status yet|Transport\(Rejected|error sending request|SendRequest|client error|timed out|timeout|connection (reset|refused|closed)|tcp connect error|temporarily unavailable|TxBadSeq|tx_bad_seq|TxInsufficientFee|tx_insufficient_fee|not present in the snapshot'

DEPLOY_PROPAGATION_RE='Contract not found|non-existing value for contract instance'

INV_MAX_ATTEMPTS="${INV_MAX_ATTEMPTS:-8}"
DEPLOY_MAX_ATTEMPTS="${DEPLOY_MAX_ATTEMPTS:-8}"
XFAIL_MAX_ATTEMPTS="${XFAIL_MAX_ATTEMPTS:-5}"

backoff_sleep() {
    local attempt="$1" step="${2:-5}" cap="${3:-20}" s
    s=$(( (attempt - 1) * step ))
    [ "$s" -gt "$cap" ] && s="$cap"
    [ "$s" -gt 0 ] && sleep "$s"
    return 0
}

run_deploy() {
    local out_f="$1" err_f="$2"; shift 2
    [ "$1" = "--" ] && shift
    # Upload and deploy use the same native simulation policy as inv(). Insert
    # the option before constructor args, and reject conflicting explicit limits.
    case "${1:-} ${2:-} ${3:-}" in
        'stellar contract upload'|'stellar contract deploy')
            local verb="$3" leeway="${INSTRUCTION_LEEWAY:-2000000}" explicit=0 value
            local deployment_args=()
            shift 3
            while [ "$#" -gt 0 ]; do
                case "$1" in
                    --) deployment_args+=("$@"); break;;
                    --instruction-leeway|--instruction-leeway=*)
                        value="${1#--instruction-leeway=}"
                        if [ "$1" = --instruction-leeway ]; then
                            [ "$#" -ge 2 ] || { record deployment_policy FAIL deploy "" "" "" "" "" 'missing instruction leeway'; return 1; }
                            value="$2"; shift
                        fi
                        [ "$explicit" -eq 0 ] && [ "$value" = "$leeway" ] || { record deployment_policy FAIL deploy "" "" "" "" "" 'conflicting or duplicate instruction leeway'; return 1; }
                        explicit=1;;
                    --instructions|--instructions=*)
                        record deployment_policy FAIL deploy "" "" "" "" "" 'absolute instruction override violates native leeway policy'; return 1;;
                    *) deployment_args+=("$1");;
                esac
                shift
            done
            set -- stellar contract "$verb" --instruction-leeway "$leeway" "${deployment_args[@]}";;
    esac
    local attempt rc hash sequence label st
    sequence=$(wc -l < "$ACTIONS_TSV")
    label=$(basename "$out_f" .out)
    DEPLOY_ATTEMPTS=0
    for attempt in $(seq 1 "$DEPLOY_MAX_ATTEMPTS"); do
        DEPLOY_ATTEMPTS=$attempt
        [ "$attempt" -gt 1 ] && backoff_sleep "$attempt" 3 15
        rc=0
        begin_attempt "$label" deploy "$sequence" "$attempt" "$out_f" "$err_f" "" || return 1
        "$@" >"$out_f" 2>"$err_f" || rc=$?
        hash=$(extract_signing_hash "$err_f")
        record_attempt "$label" deploy "$sequence" "$attempt" "$rc" "$hash" "$out_f" "$err_f" || return 1
        if [ -n "$hash" ]; then
            st=$(tx_status "$hash")
            if [ "$st" != SUCCESS ] || ! fetch_resources "$hash"; then
                : > "$out_f"
                record deployment_receipt FAIL deploy "$hash" "" "" "" "" "submitted deployment: status=$st cli=$rc; $(tail_err_note "$err_f")"
                return 1
            fi
            if [ "$rc" -ne 0 ] || [ ! -s "$out_f" ]; then
                if ! recover_output "$hash" "$out_f" command "$@"; then
                    : > "$out_f"
                    record deployment_receipt FAIL deploy "$hash" "" "" "" "" "confirmed deployment return could not be verified; cli=$rc"
                    return 1
                fi
                rc=0
            fi
        fi
        if [ "$rc" -eq 0 ] && [ -s "$out_f" ]; then
            if is_contract_id "$(sanitize_output "$out_f")" && [ -z "$hash" ]; then
                : > "$out_f"
                record deployment_receipt FAIL deploy "" "" "" "" "" "deployment lacks submission evidence"
                return 1
            fi
            if ! verify_deployed_wasm "$out_f" "$@"; then
                : > "$out_f"
                record deployment_verification FAIL deploy "" "" "" "" "" "candidate WASM verification failed"
                return 1
            fi
            return 0
        fi
        [ -z "$(extract_signing_hash "$err_f")" ] || break
        grep -qE "$DEPLOY_PROPAGATION_RE|Wasm does not exist|TxInsufficientFee" "$err_f" || break
    done
    return 1
}

# record_attempt already preserved the original CLI rc/stdout/stderr. Keep the
# verified recovery alongside them, then expose it through the usual output.
recover_output() {
    local hash="$1" out="$2"; shift 2
    local recovered="${out%.out}.recovered.json"
    python3 "$INTEG_DIR/receipts.py" "$LOG_DIR/$hash.receipt.json" "$hash" "$NETWORK_PASSPHRASE" "$@" \
        > "$recovered" 2> "${out%.out}.recovery.err" || return 1
    cp "$recovered" "$out"
}

tx_status() {
    local hash="$1" resp st attempt
    [[ "$hash" =~ ^[0-9a-f]{64}$ ]] || return 1
    for attempt in 1 2 3 4 5; do
        resp="$LOG_DIR/$hash.receipt.$attempt.json"
        if curl --fail-with-body -sS -m 30 -X POST "$RPC_URL" -H 'Content-Type: application/json' \
            -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getTransaction\",\"params\":{\"hash\":\"$hash\"}}" >"$resp" \
            && jq -e '.jsonrpc == "2.0" and .id == 1 and (has("error") | not) and (.result | type == "object")' "$resp" >/dev/null; then
            st=$(jq -r '.result.status' "$resp")
            case "$st" in
                SUCCESS|FAILED)
                    if jq -e --arg h "$hash" '.result | .txHash == $h and (.resultMetaXdr | type == "string" and length > 0) and (.ledger | type == "number") and (.envelopeXdr | type == "string" and length > 0) and (.resultXdr | type == "string" and length > 0)' "$resp" >/dev/null; then
                        cp "$resp" "$LOG_DIR/$hash.receipt.json"
                        echo "$st"; return 0
                    fi ;;
            esac
        fi
        [ "$attempt" -eq 5 ] || sleep 3
    done
    echo UNKNOWN
    return 1
}

fetch_resources() {
    local hash="$1"
    RES_INSTR="" RES_READ="" RES_WRITE="" RES_FEE=""
    local resp env_json
    resp=$(cat "$LOG_DIR/$hash.receipt.json") || return 1
    local env_xdr
    env_xdr=$(jq -r '.result.envelopeXdr // empty' <<<"$resp")
    [ -z "$env_xdr" ] && return 1
    env_json=$(echo "$env_xdr" | stellar xdr decode --type TransactionEnvelope --output json 2>/dev/null) || return 1
    local sdata
    sdata=$(jq -c '[.. | objects | select(has("resources"))] | first // empty' <<<"$env_json")
    [ -z "$sdata" ] && return 1
    RES_INSTR=$(jq -r '.resources.instructions // empty' <<<"$sdata")
    RES_READ=$(jq -r '.resources.disk_read_bytes // .resources.read_bytes // empty' <<<"$sdata")
    RES_WRITE=$(jq -r '.resources.write_bytes // empty' <<<"$sdata")
    RES_FEE=$(jq -r '.resource_fee // empty' <<<"$sdata")
    printf '%s\n' "$sdata" > "$LOG_DIR/$hash.resources.json"
    python3 "$INTEG_DIR/resources.py" "$RUN_DIR/network-limits.json" "$LOG_DIR/$hash.resources.json" "$LOG_DIR/$hash.receipt.json" \
        > "$LOG_DIR/$hash.budget.json" || return 1
    [[ "$RES_INSTR" =~ ^[0-9]+$ && "$RES_READ" =~ ^[0-9]+$ && "$RES_WRITE" =~ ^[0-9]+$ && "$RES_FEE" =~ ^[0-9]+$ ]]
}

inv() {
    local label="$1" signer="$2" contract="$3"; shift 3
    [ "$1" = "--" ] && shift
    local fn="$1" attempt hash rc st sequence
    sequence=$(wc -l < "$ACTIONS_TSV")
    local out_f="$LOG_DIR/$label.out" err_f="$LOG_DIR/$label.err"
    for attempt in $(seq 1 "$INV_MAX_ATTEMPTS"); do
        [ "$attempt" -gt 1 ] && backoff_sleep "$attempt"
        rc=0
        begin_attempt "$label" "$fn" "$sequence" "$attempt" "$out_f" "$err_f" "$contract" || return 1
        stellar contract invoke --id "$contract" --source "$signer" "${NET_ARGS[@]}" \
            --instruction-leeway "${INSTRUCTION_LEEWAY:-2000000}" --send=yes -- "$@" >"$out_f" 2>"$err_f" || rc=$?
        hash=$(extract_signing_hash "$err_f")
        record_attempt "$label" "$fn" "$sequence" "$attempt" "$rc" "$hash" "$out_f" "$err_f" "$contract" || return 1
        if [ -n "$hash" ]; then
            st=$(tx_status "$hash")
            if [ "$st" = SUCCESS ] && fetch_resources "$hash"; then
                if { [ "$rc" -eq 0 ] && [ -s "$out_f" ]; } || recover_output "$hash" "$out_f" invoke "$contract" "$fn"; then
                    record "$label" ok "$fn" "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" "confirmed attempt $attempt; cli=$rc" transaction "$contract"
                    cat "$out_f"
                    return 0
                fi
            fi
            # A signed envelope is potentially submitted. Never rebuild it,
            # including when the RPC cannot find it or the CLI lost its result.
            record "$label" FAIL "$fn" "$hash" "" "" "" "" "submitted attempt $attempt: status=$st cli=$rc; $(tail_err_note "$err_f")"
            return 1
        fi
        if [ "$rc" -ne 0 ] && [ "$attempt" -lt "$INV_MAX_ATTEMPTS" ] \
            && grep -qE "$DEPLOY_PROPAGATION_RE|Wasm does not exist" "$err_f" \
            && ! grep -q 'Error(Contract' "$err_f"; then
            record "$label" retry "$fn" "" "" "" "" "" "explicit simulation prerequisite unavailable, attempt $attempt"
            continue
        fi
        record "$label" FAIL "$fn" "" "" "" "" "" "missing confirmed transaction: $(tail_err_note "$err_f")"
        return 1
    done
    return 1
}

inv_create() {
    local label="$1" contract="$3" acct
    acct=$(inv "$@") || return 1
    acct=$(tr -d '\"[:space:]' <<<"$acct")
    [[ "$acct" =~ ^[1-9][0-9]*$ ]] || { _assert_fail "$label" "invalid committed account id"; return 1; }
    assert_view_eq_at "$contract" "${label}_persisted" true account_exists --account_id "$acct" || return 1
    printf '%s\n' "$acct"
}

xfail() {
    local label="$1" pattern="$2" signer="$3" contract="$4"; shift 4
    [ "$1" = "--" ] && shift
    local fn="$1" config="$RUN_DIR/private/negative-$(wc -l < "$ACTIONS_TSV")"
    # CLI auto-signs every available identity. Negative authorization probes
    # must expose only the selected signer's key, never the victim's key.
    mkdir -p "$config/identity" || return 1
    cp "$XDG_CONFIG_HOME/stellar/identity/$signer.toml" "$config/identity/" \
        || { _assert_fail "$label" "cannot isolate negative-test signer"; return 1; }
    local out_f="$LOG_DIR/$label.out" err_f="$LOG_DIR/$label.err"
    local attempt rc signed_hash sequence
    sequence=$(wc -l < "$ACTIONS_TSV")
    for attempt in $(seq 1 "$XFAIL_MAX_ATTEMPTS"); do
        [ "$attempt" -gt 1 ] && backoff_sleep "$attempt"
        log "xfail [$label] $fn (expect: $pattern)"
        rc=0
        begin_attempt "$label" "$fn" "$sequence" "$attempt" "$out_f" "$err_f" "$contract" || return 1
        stellar contract invoke --config-dir "$config" --id "$contract" --source "$signer" "${NET_ARGS[@]}" --instruction-leeway "${INSTRUCTION_LEEWAY:-2000000}" --send="${XFAIL_SEND_MODE:-yes}" -- "$@" \
            >"$out_f" 2>"$err_f" || rc=$?
        signed_hash=$(extract_signing_hash "$err_f")
        record_attempt "$label" "$fn" "$sequence" "$attempt" "$rc" "$signed_hash" "$out_f" "$err_f" "$contract" || return 1
        if [ "$rc" -eq 0 ]; then
            record "$label" UNEXPECTED-OK "$fn" "" "" "" "" "" "expected revert '$pattern'"
            log "UNEXPECTED-OK [$label]"
            return 1
        fi
        if [ -n "$signed_hash" ] && [ "$(tx_status "$signed_hash")" != FAILED ]; then
            record "$label" FAIL "$fn" "$signed_hash" "" "" "" "" "expected failure has ambiguous/committed submission"
            return 1
        fi
        if grep -qE "$pattern" "$err_f"; then
            record "$label" xfail "$fn" "$signed_hash" "" "" "" "" "reverted as expected: $pattern" "$([ -n "$signed_hash" ] && echo rejected_transaction || echo simulation)" "$contract"
            return 0
        fi
        if [ "$attempt" -lt "$XFAIL_MAX_ATTEMPTS" ] \
            && [ -z "$signed_hash" ] \
            && grep -qE "$DEPLOY_PROPAGATION_RE" "$err_f" \
            && ! grep -q "Error(Contract" "$err_f"; then
            record "$label" retry "$fn" "" "" "" "" "" "transient infra failure; resimulating"
            continue
        fi
        break
    done
    record "$label" "${INV_FAIL_STATUS:-FAIL}" "$fn" "" "" "" "" "" "wrong revert; wanted '$pattern' got: $(tail -c 200 "$err_f" | tr '\n\t' '  ')"
    log "WRONG-REVERT [$label]: $(tail -2 "$err_f")"
    return 1
}

xfail_sim() {
    XFAIL_SEND_MODE=no xfail "$@"
}

view() {
    local label="$1" contract="$2"; shift 2
    [ "$1" = "--" ] && shift
    local fn="$1"
    local out_f="$LOG_DIR/$label.out" err_f="$LOG_DIR/$label.err"
    local attempt rc sequence
    sequence=$(wc -l < "$ACTIONS_TSV")
    for attempt in $(seq 1 "$INV_MAX_ATTEMPTS"); do
        [ "$attempt" -gt 1 ] && backoff_sleep "$attempt"
        rc=0
        begin_attempt "$label" "$fn" "$sequence" "$attempt" "$out_f" "$err_f" "$contract" || return 1
        stellar contract invoke --id "$contract" --source "$ADMIN" "${NET_ARGS[@]}" --instruction-leeway "${INSTRUCTION_LEEWAY:-2000000}" --send=no -- "$@" \
            >"$out_f" 2>"$err_f" || rc=$?
        record_attempt "$label" "$fn" "$sequence" "$attempt" "$rc" "" "$out_f" "$err_f" "$contract" || return 1
        if [ "$rc" -eq 0 ] && jq -e 'true' "$out_f" >/dev/null; then
            record "$label" read "$fn" "" "" "" "" "" "$(head -c 120 "$out_f" | tr '\n\t' '  ')" simulation "$contract"
            cat "$out_f"
            return 0
        fi

        [ "$attempt" -lt "$INV_MAX_ATTEMPTS" ] && continue
        break
    done
    record "$label" FAIL "$fn" "" "" "" "" "" "view failed: $(tail -c 200 "$err_f" | tr '\n\t' '  ')"
    return 1
}

# Mutation legs may already have committed before a later quote/view fails.
# Only inv() can classify a retry as pre-submission.
retry_leg() { "$@"; }

sim_probe() {
    local label="$1" signer="$2" contract="$3"; shift 3
    [ "$1" = "--" ] && shift
    local fn="$1"
    local tx_f="$LOG_DIR/$label.txb64" sim_f="$LOG_DIR/$label.sim.json"
    PROBE_STATUS=error
    if ! stellar contract invoke --id "$contract" --source "$signer" "${NET_ARGS[@]}" --instruction-leeway "${INSTRUCTION_LEEWAY:-2000000}" --build-only -- "$@" \
        >"$tx_f" 2>"$LOG_DIR/$label.err"; then
        record "$label" FAIL "$fn" "" "" "" "" "" "build-only failed"
        return 1
    fi
    if ! curl --fail-with-body -sS -m 60 -X POST "$RPC_URL" -H 'Content-Type: application/json' \
        -d "$(jq -n --argjson leeway "${INSTRUCTION_LEEWAY:-2000000}" --rawfile tx "$tx_f" '{jsonrpc:"2.0",id:1,method:"simulateTransaction",params:{transaction:($tx|rtrimstr("\n")),resourceConfig:{instructionLeeway:$leeway}}}')" \
        >"$sim_f" || ! jq -e '.jsonrpc == "2.0" and .id == 1 and (has("error") | not) and (.result | type == "object")' "$sim_f" >/dev/null; then
        record "$label" FAIL "$fn" "" "" "" "" "" "invalid simulation transport or JSON-RPC response"
        return 1
    fi
    local err
    err=$(jq -r '.result.error // empty' <<<"$(cat "$sim_f")")
    if [ -z "$err" ]; then
        local sdata instr
        sdata=$(jq -r '.result.transactionData // empty' "$sim_f")
        RES_INSTR="" RES_READ="" RES_WRITE="" RES_FEE=""
        if [ -n "$sdata" ]; then
            local sd_json
            sd_json=$(echo "$sdata" | stellar xdr decode --type SorobanTransactionData --output json 2>/dev/null) || true
            RES_INSTR=$(jq -r '.resources.instructions // empty' <<<"$sd_json")
            RES_READ=$(jq -r '.resources.disk_read_bytes // .resources.read_bytes // empty' <<<"$sd_json")
            RES_WRITE=$(jq -r '.resources.write_bytes // empty' <<<"$sd_json")
            RES_FEE=$(jq -r '.result.minResourceFee // empty' "$sim_f")
        fi
        if ! [[ "$RES_INSTR" =~ ^[0-9]+$ && "$RES_READ" =~ ^[0-9]+$ && "$RES_WRITE" =~ ^[0-9]+$ && "$RES_FEE" =~ ^[0-9]+$ ]] \
            || ! jq -e '.result.results | type == "array" and length > 0' "$sim_f" >/dev/null; then
            record "$label" FAIL "$fn" "" "" "" "" "" "missing simulation results/resources"
            return 1
        fi
        record "$label" sim-ok "$fn" "" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" "simulation only" simulation "$contract"
        PROBE_STATUS=ok
        return 0
    fi
    if grep -q 'ExceededLimit' <<<"$err"; then
        record "$label" sim-exceeded "$fn" "" "" "" "" "" "Budget,ExceededLimit"
        PROBE_STATUS=exceeded
    else
        record "$label" sim-error "$fn" "" "" "" "" "" "$(head -c 200 <<<"$err" | tr '\n\t' '  ')"
        PROBE_STATUS=error
    fi
    return 0
}

verify_deployed_wasm() {
    local result="$1" wasm="" previous="" arg id fetched attempt fetched_ok=0
    shift
    for arg in "$@"; do
        [ "$previous" != --wasm ] || wasm="$arg"
        previous="$arg"
    done
    [ -n "$wasm" ] || return 0
    id=$(sanitize_output "$result")
    if is_wasm_hash "$id"; then
        python3 - "$wasm" "$id" <<'PYHASH'
import hashlib,sys
assert hashlib.sha256(open(sys.argv[1], 'rb').read()).hexdigest() == sys.argv[2], 'uploaded WASM hash mismatch'
PYHASH
        return $?
    fi
    is_contract_id "$id" || return 1
    for attempt in 1 2 3; do
        fetched="$LOG_DIR/$id.fetch$attempt.wasm"
        rm -f "$fetched"
        if stellar contract fetch --id "$id" "${NET_ARGS[@]}" --out-file "$fetched" \
            > "$LOG_DIR/$id.fetch$attempt.out" 2> "$LOG_DIR/$id.fetch$attempt.err" && [ -f "$fetched" ]; then
            fetched_ok=1; break
        fi
        [ "$attempt" -lt 3 ] && grep -qE "$RPC_TRANSIENT_RE|Connect" "$LOG_DIR/$id.fetch$attempt.err" || break
        backoff_sleep "$((attempt+1))" 2 4
    done
    if [ "$fetched_ok" -ne 1 ]; then
        record deployed_fetch FAIL deploy "" "" "" "" "" "unable to fetch $id candidate bytecode: $(tail_err_note "$LOG_DIR/$id.fetch$attempt.err")"
        return 1
    fi
    cmp -s "$wasm" "$fetched" || { record deployed_hash FAIL deploy "" "" "" "" "" "deployed WASM differs from $wasm"; return 1; }
    cp "$fetched" "$LOG_DIR/$id.deployed.wasm" || return 1
    python3 - "$RUN_DIR/deployed-artifacts.jsonl" "$id" "$wasm" <<'PYDEPLOY'
import hashlib,json,sys
from pathlib import Path
path,address,wasm=sys.argv[1:]
with open(path,'a') as f:
    f.write(json.dumps(dict(address=address,artifact=Path(wasm).name,sha256=hashlib.sha256(Path(wasm).read_bytes()).hexdigest()))+'\n')
PYDEPLOY
}

verify_candidate_contract() {
    local label="$1" id="$2" name="$3" proof="$LOG_DIR/$1.contract-id"
    printf '%s\n' "$id" > "$proof"
    verify_deployed_wasm "$proof" --wasm "$WASM_DIR/$name.wasm" \
        || { _assert_fail "$label" "unable to verify deployed $name against candidate"; return 1; }
    record "$label" ok assert "" "" "" "" "" "$id matches $name.wasm"
}
