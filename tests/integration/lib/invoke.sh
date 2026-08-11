RPC_TRANSIENT_RE='rejected .?50[0-9]|status_code: 50[0-9]|No status yet|Transport\(Rejected|error sending request|timed out|timeout|connection (reset|refused|closed)|tcp connect error|temporarily unavailable|TxBadSeq|tx_bad_seq|not present in the snapshot'

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
    local attempt
    for attempt in $(seq 1 "$DEPLOY_MAX_ATTEMPTS"); do
        [ "$attempt" -gt 1 ] && backoff_sleep "$attempt" 3 15
        if "$@" >"$out_f" 2>"$err_f" && [ -s "$out_f" ]; then
            return 0
        fi
        grep -qE "$RPC_TRANSIENT_RE|Wasm does not exist|Storage, MissingValue|ResourceLimitExceeded" "$err_f" || break
    done
    return 1
}

tx_status() {
    local hash="$1" resp st _
    for _ in 1 2 3 4 5; do
        resp=$(curl -s -m 30 -X POST "$RPC_URL" -H 'Content-Type: application/json' \
            -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getTransaction\",\"params\":{\"hash\":\"$hash\"}}") \
            || { sleep 3; continue; }
        st=$(jq -r '.result.status // empty' <<<"$resp")
        case "$st" in
            SUCCESS|FAILED) echo "$st"; return 0 ;;
            *) sleep 3 ;;
        esac
    done
    echo NOT_FOUND
}

fetch_resources() {
    local hash="$1"
    RES_INSTR="" RES_READ="" RES_WRITE="" RES_FEE=""
    local resp env_json
    resp=$(curl -s -m 30 -X POST "$RPC_URL" -H 'Content-Type: application/json' \
        -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"getTransaction\",\"params\":{\"hash\":\"$hash\"}}") || return 0
    local env_xdr
    env_xdr=$(jq -r '.result.envelopeXdr // empty' <<<"$resp")
    [ -z "$env_xdr" ] && return 0
    env_json=$(echo "$env_xdr" | stellar xdr decode --type TransactionEnvelope --output json 2>/dev/null) || return 0
    local sdata
    sdata=$(jq -c '[.. | objects | select(has("resources"))] | first // empty' <<<"$env_json")
    [ -z "$sdata" ] && return 0
    RES_INSTR=$(jq -r '.resources.instructions // empty' <<<"$sdata")
    RES_READ=$(jq -r '.resources.disk_read_bytes // .resources.read_bytes // empty' <<<"$sdata")
    RES_WRITE=$(jq -r '.resources.write_bytes // empty' <<<"$sdata")
    RES_FEE=$(jq -r '.resource_fee // empty' <<<"$sdata")
}

inv() {
    local label="$1" signer="$2" contract="$3"; shift 3
    [ "$1" = "--" ] && shift
    local fn="$1"
    local out_f="$LOG_DIR/$label.out" err_f="$LOG_DIR/$label.err"
    local attempt
    for attempt in $(seq 1 "$INV_MAX_ATTEMPTS"); do
        [ "$attempt" -gt 1 ] && backoff_sleep "$attempt"
        log "inv [$label] $fn"
        if stellar contract invoke --id "$contract" --source "$signer" "${NET_ARGS[@]}" -- "$@" \
            >"$out_f" 2>"$err_f"; then
            local hash
            hash=$(extract_signing_hash "$err_f")
            if [ -n "$hash" ]; then
                fetch_resources "$hash"
                record "$label" ok "$fn" "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" ""
            else

                record "$label" read "$fn" "" "" "" "" "" ""
            fi
            cat "$out_f"
            return 0
        fi

        if [ "$attempt" -lt "$INV_MAX_ATTEMPTS" ] \
            && [ -n "${INV_TRANSIENT_CONTRACT_RE:-}" ] \
            && grep -qE "$INV_TRANSIENT_CONTRACT_RE" "$err_f"; then
            record "$label" retry "$fn" "" "" "" "" "" "transient contract state; resimulating"
            continue
        fi

        if [ "$attempt" -lt "$INV_MAX_ATTEMPTS" ] \
            && grep -qE "$DEPLOY_PROPAGATION_RE" "$err_f" \
            && ! grep -q "Error(Contract" "$err_f"; then
            record "$label" retry "$fn" "" "" "" "" "" "freshly-deployed contract not yet visible; resimulating"
            continue
        fi

        if [ "$attempt" -lt "$INV_MAX_ATTEMPTS" ] \
            && grep -qE "$RPC_TRANSIENT_RE" "$err_f" \
            && ! grep -q "Error(Contract" "$err_f"; then
            local thash
            thash=$(extract_signing_hash "$err_f")
            if [ -z "$thash" ]; then

                record "$label" retry "$fn" "" "" "" "" "" "transient rpc failure pre-send; retrying"
                continue
            fi
            case "$(tx_status "$thash")" in
                SUCCESS)

                    fetch_resources "$thash"
                    record "$label" ok "$fn" "$thash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" "recovered: tx landed despite transient response"
                    cat "$out_f"
                    return 0
                    ;;
                NOT_FOUND)

                    record "$label" retry "$fn" "" "" "" "" "" "transient after send; tx not on ledger, resubmitting"
                    continue
                    ;;
                *)

                    break
                    ;;
            esac
        fi

        if [ "$attempt" -lt "$INV_MAX_ATTEMPTS" ] \
            && grep -q "Signing transaction" "$err_f" \
            && grep -qE "Trapped|ResourceLimitExceeded" "$err_f" \
            && ! grep -q "Error(Contract" "$err_f"; then
            record "$label" retry "$fn" "" "" "" "" "" "transient apply failure; resimulating"
            continue
        fi
        break
    done

    record "$label" "${INV_FAIL_STATUS:-FAIL}" "$fn" "" "" "" "" "" "$(tail -c 300 "$err_f" | tr '\n\t' '  ')"
    log "${INV_FAIL_STATUS:-FAIL} [$label]: $(tail -3 "$err_f")"
    return 1
}

# Create-and-verify wrapper around inv() for account-creating calls
# (supply / multiply / migrate_from_blend invoked with --account_id 0, which
# returns the new account id on stdout). inv() retries transient RPC failures,
# but an --account_id 0 create is NOT idempotent: if the first send lands on
# ledger only after tx_status' ~15s poll gives up, inv() resubmits and the id it
# finally reports can point at no persisted account — every later op on that id
# then reverts #24 AccountNotFound. Confirm the returned id actually exists on
# ledger; if it is empty or absent, recreate. Soroban txs are atomic, so a
# non-persisted create left no effect and re-running is safe.
inv_create() {
    local label="$1" contract="$3" acct attempt
    for attempt in 1 2 3; do
        acct=$(inv "$@" | tr -d '"')
        if [ -n "$acct" ] \
            && [ "$(view "${label}_persisted" "$contract" -- account_exists --account_id "$acct" 2>/dev/null | tr -d '" ')" = "true" ]; then
            printf '%s\n' "$acct"
            return 0
        fi
        log "inv_create [$label]: account id '${acct:-<none>}' not on ledger (attempt $attempt/3); recreating"
    done
    log "inv_create [$label]: no persisted account after 3 attempts"
    return 1
}

xfail() {
    local label="$1" pattern="$2" signer="$3" contract="$4"; shift 4
    [ "$1" = "--" ] && shift
    local fn="$1"
    local out_f="$LOG_DIR/$label.out" err_f="$LOG_DIR/$label.err"
    local attempt
    for attempt in $(seq 1 "$XFAIL_MAX_ATTEMPTS"); do
        [ "$attempt" -gt 1 ] && backoff_sleep "$attempt"
        log "xfail [$label] $fn (expect: $pattern)"
        if stellar contract invoke --id "$contract" --source "$signer" "${NET_ARGS[@]}" ${XFAIL_SEND_NO:+--send=no} -- "$@" \
            >"$out_f" 2>"$err_f"; then
            record "$label" UNEXPECTED-OK "$fn" "" "" "" "" "" "expected revert '$pattern'"
            log "UNEXPECTED-OK [$label]"
            return 1
        fi
        if grep -qE "$pattern" "$err_f"; then
            record "$label" xfail "$fn" "" "" "" "" "" "reverted as expected: $pattern"
            return 0
        fi
        if [ "$attempt" -lt "$XFAIL_MAX_ATTEMPTS" ] \
            && grep -qE "$RPC_TRANSIENT_RE|Trapped|ResourceLimitExceeded" "$err_f" \
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
    XFAIL_SEND_NO=1 xfail "$@"
}

view() {
    local label="$1" contract="$2"; shift 2
    [ "$1" = "--" ] && shift
    local fn="$1"
    local out_f="$LOG_DIR/$label.out" err_f="$LOG_DIR/$label.err"
    local attempt
    for attempt in $(seq 1 "$INV_MAX_ATTEMPTS"); do
        [ "$attempt" -gt 1 ] && backoff_sleep "$attempt"
        if stellar contract invoke --id "$contract" --source "$ADMIN" "${NET_ARGS[@]}" --send=no -- "$@" \
            >"$out_f" 2>"$err_f"; then
            record "$label" read "$fn" "" "" "" "" "" "$(head -c 120 "$out_f" | tr '\n\t' '  ')"
            cat "$out_f"
            return 0
        fi

        [ "$attempt" -lt "$INV_MAX_ATTEMPTS" ] && continue
        break
    done
    record "$label" FAIL "$fn" "" "" "" "" "" "view failed: $(tail -c 200 "$err_f" | tr '\n\t' '  ')"
    return 1
}

retry_leg() {
    local attempt
    for attempt in 1 2 3; do
        sleep $(( (attempt - 1) * 5 ))
        if INV_FAIL_STATUS=$([ "$attempt" -lt 3 ] && echo retry || echo FAIL) "$@"; then
            return 0
        fi
    done
    return 1
}

sim_probe() {
    local label="$1" signer="$2" contract="$3"; shift 3
    [ "$1" = "--" ] && shift
    local fn="$1"
    local tx_f="$LOG_DIR/$label.txb64" sim_f="$LOG_DIR/$label.sim.json"
    PROBE_STATUS=error
    if ! stellar contract invoke --id "$contract" --source "$signer" "${NET_ARGS[@]}" --build-only -- "$@" \
        >"$tx_f" 2>"$LOG_DIR/$label.err"; then
        record "$label" FAIL "$fn" "" "" "" "" "" "build-only failed"
        return 1
    fi
    curl -s -m 60 -X POST "$RPC_URL" -H 'Content-Type: application/json' \
        -d "$(jq -n --rawfile tx "$tx_f" '{jsonrpc:"2.0",id:1,method:"simulateTransaction",params:{transaction:($tx|rtrimstr("\n"))}}')" \
        >"$sim_f"
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
        record "$label" sim-ok "$fn" "" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" "simulation only"
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
