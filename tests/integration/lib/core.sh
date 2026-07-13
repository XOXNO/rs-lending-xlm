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

# --- Quality & guard helpers (added to address CLI fragility, missing guards,
# portability, and duplication identified in audit) ---

# Check that required external tools exist. Prints missing ones to stderr.
#   check_tools
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

# Very lightweight stellar CLI version check (parses `stellar --version`).
# Requires STELLAR_CLI_MIN_VERSION (e.g. "22.0") from env.
#   check_stellar_version
check_stellar_version() {
    local ver min major minor
    ver=$(stellar --version 2>/dev/null | awk '{print $2}' | head -1)
    [ -z "$ver" ] && { echo "cannot determine stellar version" >&2; return 1; }
    min="${STELLAR_CLI_MIN_VERSION:-22.0}"
    # crude major.minor compare
    major=${ver%%.*}; minor=${ver#*.}; minor=${minor%%.*}
    local min_major min_minor
    min_major=${min%%.*}; min_minor=${min#*.}; min_minor=${min_minor%%.*}
    if [ "$major" -lt "$min_major" ] || { [ "$major" -eq "$min_major" ] && [ "$minor" -lt "$min_minor" ]; }; then
        echo "stellar CLI $ver < required min $min" >&2
        return 1
    fi
    return 0
}

# Extract the tx hash from a stellar CLI stderr log that contains
# "Signing transaction: <64hex>". Returns the hash or empty string.
# Centralizes the most duplicated fragile pattern across the harness.
#   extract_signing_hash <errfile>
extract_signing_hash() {
    local f="$1"
    [ -f "$f" ] || return 1
    grep -oE 'Signing transaction: [0-9a-f]{64}' "$f" | tail -1 | awk '{print $3}'
}

# Sanitize a captured stellar contract output file (strip quotes, newlines,
# spaces). Common post-processing for IDs, hashes, view results.
#   sanitize_output <file>
sanitize_output() {
    local f="$1"
    [ -f "$f" ] || { echo ""; return 1; }
    tr -d '"\n[:space:]' < "$f"
}

# Require that a variable is non-empty. Calls die on failure (so report is written).
#   require_var <varname> [label]
require_var() {
    local name="$1" label="${2:-$1}"
    local val
    eval "val=\"\${$name:-}\""
    [ -n "$val" ] || die "require_$name" "$label is empty (missing from state.env or prior phase)"
}

# Produce a compact error note from the tail of an err file (used in records/die).
#   tail_err_note <errfile> [bytes=300]
tail_err_note() {
    local f="$1" n="${2:-300}"
    [ -f "$f" ] || { echo ""; return 0; }
    tail -c "$n" "$f" | tr '\n\t' '  '
}

# Convenience wrapper for a direct stellar (or other) command when we want
# capture + optional hash extraction + record skeleton. Callers still do their
# own run_deploy / inv for full retry semantics.
#   run_captured <label> <out_f> <err_f> -- <cmd...>
run_captured() {
    local label="$1" out_f="$2" err_f="$3"; shift 3
    [ "$1" = "--" ] && shift
    "$@" >"$out_f" 2>"$err_f"
}

# --- End quality helpers ---
