# Contract invocation wrappers: send, expect-revert, view, and budget probes.
#
# Testnet facts these encode:
# - `stellar contract invoke` prints the return value on stdout and the tx
#   hash on stderr as `Signing transaction: <64hex>`; failed simulations
#   never reach signing, so a captured hash is a reliable success signal.
# - Declared resources (instructions / disk-read / write bytes, resource fee)
#   live in the signed envelope's SorobanTransactionData; fetched post-send
#   via RPC getTransaction and decoded with `stellar xdr`.

# Infra-level transients (gateway 5xx, request timeouts, connection resets,
# sequence-number races) carry no on-ledger effect and are always safe to
# resubmit — distinct from a contract revert, which is deterministic. A
# TxBadSeq is rejected before apply (the source account's sequence read lagged
# a just-landed tx), so re-submitting re-fetches the sequence and lands clean.
# Shared by the inv / xfail / trustline retry loops.
RPC_TRANSIENT_RE='rejected .?50[0-9]|error sending request|timed out|timeout|connection (reset|refused|closed)|tcp connect error|temporarily unavailable|TxBadSeq|tx_bad_seq'

# A just-deployed contract can lag the RPC read replica the next invoke
# simulates against: the instance entry reads as missing ("Contract not found"
# / "non-existing value for contract instance"). The deploy already committed,
# so re-simulating with backoff lands once the replica catches up — a genuinely
# absent contract recurs and falls through to FAIL on the final attempt.
DEPLOY_PROPAGATION_RE='Contract not found|non-existing value for contract instance'

# Retry budgets, env-overridable. Sized to outlast a sustained testnet
# congestion window (each transient resubmit waits a capped linear backoff):
# state-changing/read invokes get up to INV_MAX_ATTEMPTS, raw deploy/upload up
# to DEPLOY_MAX_ATTEMPTS, expect-revert probes XFAIL_MAX_ATTEMPTS. The happy
# path settles on attempt 1, so raising the ceiling only adds resilience.
INV_MAX_ATTEMPTS="${INV_MAX_ATTEMPTS:-8}"
DEPLOY_MAX_ATTEMPTS="${DEPLOY_MAX_ATTEMPTS:-8}"
XFAIL_MAX_ATTEMPTS="${XFAIL_MAX_ATTEMPTS:-5}"

# Capped linear backoff in seconds before retry attempt N (N>=1): (N-1)*step,
# clamped to cap so a long retry chain stays bounded.
#   backoff_sleep <attempt> [step=5] [cap=20]
backoff_sleep() {
    local attempt="$1" step="${2:-5}" cap="${3:-20}" s
    s=$(( (attempt - 1) * step ))
    [ "$s" -gt "$cap" ] && s="$cap"
    [ "$s" -gt 0 ] && sleep "$s"
    return 0
}

# Runs a raw `stellar contract deploy/upload` command up to 5x, retrying the
# transients testnet throws around contract installation: a deploy racing its
# own wasm upload (Storage,MissingValue / "Wasm does not exist"), TxBadSeq, and
# RPC 5xx. The contract id / wasm hash lands on stdout (captured to out_f);
# success requires a non-empty stdout. Re-running is safe — an already-uploaded
# wasm re-uploads idempotently and a fresh deploy simply gets a new id.
#   run_deploy <out_f> <err_f> -- <stellar ...>
run_deploy() {
    local out_f="$1" err_f="$2"; shift 2
    [ "$1" = "--" ] && shift
    local attempt
    for attempt in $(seq 1 "$DEPLOY_MAX_ATTEMPTS"); do
        [ "$attempt" -gt 1 ] && backoff_sleep "$attempt" 3 15
        if "$@" >"$out_f" 2>"$err_f" && [ -s "$out_f" ]; then
            return 0
        fi
        grep -qE "$RPC_TRANSIENT_RE|Wasm does not exist|Storage, MissingValue" "$err_f" || break
    done
    return 1
}

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
    local attempt
    for attempt in $(seq 1 "$INV_MAX_ATTEMPTS"); do
        [ "$attempt" -gt 1 ] && backoff_sleep "$attempt"
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
        # Opt-in: a contract error the CALLER marks as a state-propagation
        # transient via INV_TRANSIENT_CONTRACT_RE (e.g. a just-established
        # classic trustline not yet visible to the mint's simulate → SAC #13).
        # The prerequisite tx already committed, so re-simulating with backoff
        # lands once the read replica catches up. Unset by default → no effect.
        if [ "$attempt" -lt "$INV_MAX_ATTEMPTS" ] \
            && [ -n "${INV_TRANSIENT_CONTRACT_RE:-}" ] \
            && grep -qE "$INV_TRANSIENT_CONTRACT_RE" "$err_f"; then
            record "$label" retry "$fn" "" "" "" "" "" "transient contract state; resimulating"
            continue
        fi
        # A freshly-deployed contract not yet visible to this invoke's simulate.
        # The deploy committed; re-simulate with backoff until the replica syncs.
        if [ "$attempt" -lt "$INV_MAX_ATTEMPTS" ] \
            && grep -qE "$DEPLOY_PROPAGATION_RE" "$err_f" \
            && ! grep -q "Error(Contract" "$err_f"; then
            record "$label" retry "$fn" "" "" "" "" "" "freshly-deployed contract not yet visible; resimulating"
            continue
        fi
        # Transient RPC/gateway failure (5xx, timeout, connection reset, bad
        # sequence) at simulate or send: no on-ledger effect, resubmit.
        if [ "$attempt" -lt "$INV_MAX_ATTEMPTS" ] \
            && grep -qE "$RPC_TRANSIENT_RE" "$err_f" \
            && ! grep -q "Error(Contract" "$err_f"; then
            record "$label" retry "$fn" "" "" "" "" "" "transient rpc failure; retrying"
            continue
        fi
        # Transient sim-vs-apply divergence: the tx simulated clean (it was
        # signed) but the apply read keys outside the simulated footprint —
        # live Reflector round rotation, DEX state movement, accrual drift.
        # Shows as Trapped or ResourceLimitExceeded with no contract error.
        # Re-simulate and resend; a deterministic failure recurs and falls
        # through to FAIL on the final attempt.
        if [ "$attempt" -lt "$INV_MAX_ATTEMPTS" ] \
            && grep -q "Signing transaction" "$err_f" \
            && grep -qE "Trapped|ResourceLimitExceeded" "$err_f" \
            && ! grep -q "Error(Contract" "$err_f"; then
            record "$label" retry "$fn" "" "" "" "" "" "transient apply failure; resimulating"
            continue
        fi
        break
    done
    # INV_FAIL_STATUS=retry marks an attempt inside a retry loop so transient
    # DEX-state races (sim-ok, apply-trapped) don't read as suite failures.
    record "$label" "${INV_FAIL_STATUS:-FAIL}" "$fn" "" "" "" "" "" "$(tail -c 300 "$err_f" | tr '\n\t' '  ')"
    log "${INV_FAIL_STATUS:-FAIL} [$label]: $(tail -3 "$err_f")"
    return 1
}

# Invoke that MUST revert with an error matching the given pattern.
# Retries transient sim/apply infra failures (Trapped, ResourceLimitExceeded
# without a contract error), same as `inv`.
#   xfail <label> <grep-pattern> <signer> <contract> -- <fn> [args...]
xfail() {
    local label="$1" pattern="$2" signer="$3" contract="$4"; shift 4
    [ "$1" = "--" ] && shift
    local fn="$1"
    local out_f="$LOG_DIR/$label.out" err_f="$LOG_DIR/$label.err"
    local attempt
    for attempt in $(seq 1 "$XFAIL_MAX_ATTEMPTS"); do
        [ "$attempt" -gt 1 ] && backoff_sleep "$attempt"
        log "xfail [$label] $fn (expect: $pattern)"
        if stellar contract invoke --id "$contract" --source "$signer" --network "$NETWORK" ${XFAIL_SEND_NO:+--send=no} -- "$@" \
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

# Simulate-only expect-revert: like xfail but never sends (--send=no), so an
# unexpectedly-successful guard cannot land a real tx and corrupt downstream
# state. Use for pure pre-condition guards (LTV / health gates) whose revert is
# already reached at simulation.
#   xfail_sim <label> <grep-pattern> <signer> <contract> -- <fn> [args...]
xfail_sim() {
    XFAIL_SEND_NO=1 xfail "$@"
}

# Read-only invoke (no signing; result on stdout). Recorded as a read. Views
# are side-effect-free, so transient infra/propagation failures (RPC 5xx, bad
# sequence, a freshly-written value the read replica hasn't synced — e.g. the
# oracle-resolve probe reading a just-set mock price) are always safe to retry.
# A deterministic failure (bad arg, real revert) recurs and falls through.
#   view <label> <contract> -- <fn> [args...]
view() {
    local label="$1" contract="$2"; shift 2
    [ "$1" = "--" ] && shift
    local fn="$1"
    local out_f="$LOG_DIR/$label.out" err_f="$LOG_DIR/$label.err"
    local attempt
    for attempt in $(seq 1 "$INV_MAX_ATTEMPTS"); do
        [ "$attempt" -gt 1 ] && backoff_sleep "$attempt"
        if stellar contract invoke --id "$contract" --source "$ADMIN" --network "$NETWORK" --send=no -- "$@" \
            >"$out_f" 2>"$err_f"; then
            record "$label" read "$fn" "" "" "" "" "" "$(head -c 120 "$out_f" | tr '\n\t' '  ')"
            cat "$out_f"
            return 0
        fi
        [ "$attempt" -lt "$INV_MAX_ATTEMPTS" ] && grep -qE "$RPC_TRANSIENT_RE|$DEPLOY_PROPAGATION_RE" "$err_f" && continue
        break
    done
    record "$label" FAIL "$fn" "" "" "" "" "" "view failed: $(tail -c 200 "$err_f" | tr '\n\t' '  ')"
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
