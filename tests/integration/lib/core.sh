# Run-directory management, structured action recording, and state persistence.

init_run() {
    mkdir -p "$RUN_DIR" "$LOG_DIR"
    if [ ! -f "$ACTIONS_TSV" ]; then
        printf 'seq\tphase\tlabel\tstatus\tfn\thash\tinstructions\tread_bytes\twrite_bytes\tresource_fee\tnote\n' > "$ACTIONS_TSV"
    fi
    # Load persisted state first, then let explicit env overrides win.
    # Export overrides after init_run.
    [ -f "$STATE_ENV" ] && source "$STATE_ENV"
    PHASE="${PHASE:-init}"
}

phase() {
    PHASE="$1"
    log "===== PHASE: $PHASE ====="
}

log() {
    printf '[%s] %s\n' "$(date +%H:%M:%S)" "$*" >&2
}

# Persist a key=value into state.env (idempotent per key).
save_state() {
    local key="$1" value="$2"
    touch "$STATE_ENV"
    grep -v "^${key}=" "$STATE_ENV" > "$STATE_ENV.tmp" 2>/dev/null || true
    printf '%s=%q\n' "$key" "$value" >> "$STATE_ENV.tmp"
    mv "$STATE_ENV.tmp" "$STATE_ENV"
    eval "$key=\$value"
}

# Append one action row. Args: label status fn hash instructions read write fee note
record() {
    local seq
    seq=$(($(wc -l < "$ACTIONS_TSV")))
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$seq" "$PHASE" "$1" "$2" "$3" "${4:-}" "${5:-}" "${6:-}" "${7:-}" "${8:-}" "${9:-}" >> "$ACTIONS_TSV"
}

run_summary() {
    awk -F'\t' 'NR>1 {c[$4]++} END {for (k in c) printf "  %s: %d\n", k, c[k]}' "$ACTIONS_TSV" >&2
}

# Abort the run loudly when a must-succeed setup step (a deploy / id capture)
# exhausts its retries. Records a gate-visible FAIL row and exits so the EXIT
# trap still renders report.md — far clearer than letting the empty variable
# trip a downstream `set -u` reference with a cryptic "unbound variable" abort.
#   die <label> <message>
die() {
    local label="$1" msg="$2"
    log "FATAL [$label]: $msg"
    record "$label" FAIL fatal "" "" "" "" "" "$msg"
    exit 1
}

# True when the argument is a well-formed Stellar contract/strkey id (C… 56
# chars). A captured deploy id that fails this is empty or garbled — never a
# usable contract address — so callers die loudly instead of pressing on.
#   is_contract_id <value>
is_contract_id() { [[ "$1" =~ ^C[A-Z2-7]{55}$ ]]; }

# True when the argument is a 64-hex wasm/tx hash (what `stellar contract
# upload` lands on stdout). A captured upload hash that fails this is empty or
# truncated, so callers die loudly rather than wiring a bogus hash downstream.
#   is_wasm_hash <value>
is_wasm_hash() { [[ "$1" =~ ^[0-9a-f]{64}$ ]]; }
