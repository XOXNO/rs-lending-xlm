

init_run() {
    mkdir -p "$RUN_DIR" "$LOG_DIR"
    if [ ! -f "$ACTIONS_TSV" ]; then
        printf 'seq\tphase\tlabel\tstatus\tfn\thash\tinstructions\tread_bytes\twrite_bytes\tresource_fee\tnote\n' > "$ACTIONS_TSV"
    fi


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
}

run_summary() {
    awk -F'\t' 'NR>1 {c[$4]++} END {for (k in c) printf "  %s: %d\n", k, c[k]}' "$ACTIONS_TSV" >&2
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
    local ver min major minor
    ver=$(stellar --version 2>/dev/null | awk '{print $2}' | head -1)
    [ -z "$ver" ] && { echo "cannot determine stellar version" >&2; return 1; }
    min="${STELLAR_CLI_MIN_VERSION:-22.0}"

    major=${ver%%.*}; minor=${ver
    local min_major min_minor
    min_major=${min%%.*}; min_minor=${min
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

