# Run-directory management, structured action recording, and state persistence.

init_run() {
    mkdir -p "$RUN_DIR" "$LOG_DIR"
    if [ ! -f "$ACTIONS_TSV" ]; then
        printf 'seq\tphase\tlabel\tstatus\tfn\thash\tinstructions\tread_bytes\twrite_bytes\tresource_fee\tnote\n' > "$ACTIONS_TSV"
    fi
    # Resume support: restore contract addresses and wallet aliases from a
    # previous invocation of the same RUN_TS. Sourced BEFORE flows run, so
    # explicit env overrides must be exported AFTER init_run.
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
