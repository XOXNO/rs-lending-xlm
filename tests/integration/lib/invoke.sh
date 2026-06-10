# Contract invocation wrappers: send, expect-revert, view, and budget probes.
#
# Testnet facts these encode:
# - `stellar contract invoke` prints the return value on stdout and the tx
#   hash on stderr as `Signing transaction: <64hex>`; failed simulations
#   never reach signing, so a captured hash is a reliable success signal.
# - Declared resources (instructions / disk-read / write bytes, resource fee)
#   live in the signed envelope's SorobanTransactionData; fetched post-send
#   via RPC getTransaction and decoded with `stellar xdr`.

# Fetch declared resource usage for a sent tx hash.
# Sets: RES_INSTR RES_READ RES_WRITE RES_FEE
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

# State-changing invoke. Records the action; returns the contract's return
# value on stdout. Fails the harness on unexpected revert.
#   inv <label> <signer-alias> <contract-id> -- <fn> [args...]
inv() {
    local label="$1" signer="$2" contract="$3"; shift 3
    [ "$1" = "--" ] && shift
    local fn="$1"
    local out_f="$LOG_DIR/$label.out" err_f="$LOG_DIR/$label.err"
    log "inv [$label] $fn"
    if stellar contract invoke --id "$contract" --source "$signer" --network "$NETWORK" -- "$@" \
        >"$out_f" 2>"$err_f"; then
        local hash
        hash=$(grep -oE 'Signing transaction: [0-9a-f]{64}' "$err_f" | tail -1 | awk '{print $3}')
        if [ -n "$hash" ]; then
            fetch_resources "$hash"
            record "$label" ok "$fn" "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" ""
        else
            # Read-only fn invoked via inv (no state change → no tx).
            record "$label" read "$fn" "" "" "" "" "" ""
        fi
        cat "$out_f"
        return 0
    fi
    # INV_FAIL_STATUS=retry marks an attempt inside a retry loop so transient
    # DEX-state races (sim-ok, apply-trapped) don't read as suite failures.
    record "$label" "${INV_FAIL_STATUS:-FAIL}" "$fn" "" "" "" "" "" "$(tail -c 300 "$err_f" | tr '\n\t' '  ')"
    log "${INV_FAIL_STATUS:-FAIL} [$label]: $(tail -3 "$err_f")"
    return 1
}

# Invoke that MUST revert with an error matching the given pattern.
#   xfail <label> <grep-pattern> <signer> <contract> -- <fn> [args...]
xfail() {
    local label="$1" pattern="$2" signer="$3" contract="$4"; shift 4
    [ "$1" = "--" ] && shift
    local fn="$1"
    local out_f="$LOG_DIR/$label.out" err_f="$LOG_DIR/$label.err"
    log "xfail [$label] $fn (expect: $pattern)"
    if stellar contract invoke --id "$contract" --source "$signer" --network "$NETWORK" -- "$@" \
        >"$out_f" 2>"$err_f"; then
        record "$label" UNEXPECTED-OK "$fn" "" "" "" "" "" "expected revert '$pattern'"
        log "UNEXPECTED-OK [$label]"
        return 1
    fi
    if grep -qE "$pattern" "$err_f"; then
        record "$label" xfail "$fn" "" "" "" "" "" "reverted as expected: $pattern"
        return 0
    fi
    record "$label" FAIL "$fn" "" "" "" "" "" "wrong revert; wanted '$pattern' got: $(tail -c 200 "$err_f" | tr '\n\t' '  ')"
    log "WRONG-REVERT [$label]: $(tail -2 "$err_f")"
    return 1
}

# Read-only invoke (no signing; result on stdout). Recorded as a read.
#   view <label> <contract> -- <fn> [args...]
view() {
    local label="$1" contract="$2"; shift 2
    [ "$1" = "--" ] && shift
    local fn="$1"
    local out_f="$LOG_DIR/$label.out"
    if stellar contract invoke --id "$contract" --source "$ADMIN" --network "$NETWORK" --send=no -- "$@" \
        >"$out_f" 2>"$LOG_DIR/$label.err"; then
        record "$label" read "$fn" "" "" "" "" "" "$(head -c 120 "$out_f" | tr '\n\t' '  ')"
        cat "$out_f"
        return 0
    fi
    record "$label" FAIL "$fn" "" "" "" "" "" "view failed: $(tail -c 200 "$LOG_DIR/$label.err" | tr '\n\t' '  ')"
    return 1
}

# Runs an invoke-bearing callback up to 3 times with backoff. Testnet txs can
# simulate clean and still trap at apply when chain state moves between sim
# and submit (DEX pools, interest accrual). Non-final attempts record as
# `retry`, the final one as a real FAIL.
#   retry_leg <callback-fn> [args...]
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

# Budget probe: build + simulate WITHOUT sending. Records simulated resources
# on success, or the simulation error (e.g. Budget,ExceededLimit) on failure.
# Sets PROBE_STATUS=ok|exceeded|error. Used by the stress flow to find the
# per-tx resource frontier without burning fees.
#   sim_probe <label> <signer> <contract> -- <fn> [args...]
sim_probe() {
    local label="$1" signer="$2" contract="$3"; shift 3
    [ "$1" = "--" ] && shift
    local fn="$1"
    local tx_f="$LOG_DIR/$label.txb64" sim_f="$LOG_DIR/$label.sim.json"
    PROBE_STATUS=error
    if ! stellar contract invoke --id "$contract" --source "$signer" --network "$NETWORK" --build-only -- "$@" \
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
