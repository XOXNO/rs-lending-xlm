#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SCRIPT="$ROOT/configs/script.sh"
fail() { echo "FAIL: $*" >&2; exit 1; }

extract() { awk -v fn="$1" '$0 ~ "^"fn"\\(\\) \\{" {p=1} p {print} p && /^}/ {exit}' "$SCRIPT"; }
dispatch() { awk '/^    "setupAll"\)/ {p=1} p {print} p && /^        ;;/ {exit}' "$SCRIPT"; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/dispatch.sh" <<EOF
set -e
validate_configs() { echo validate; }
setup_all_markets() { echo markets; }
setup_all_spokes() { echo spokes; }
setup_all_wave() { echo wave; }
case "\$1" in
$(dispatch)
esac
EOF
[ "$(bash "$tmp/dispatch.sh" setupAll | tr '\n' ' ')" = "validate markets spokes === Full setup complete === " ] || fail "default setupAll is not the serial path"
[ "$(SETUP_JOBS=1 bash "$tmp/dispatch.sh" setupAll | tr '\n' ' ')" = "validate markets spokes === Full setup complete === " ] || fail "SETUP_JOBS=1 is not the serial path"
[ "$(SETUP_JOBS=4 bash "$tmp/dispatch.sh" setupAll | tr '\n' ' ')" = "validate wave === Full setup complete === " ] || fail "SETUP_JOBS=4 does not select the wave path"

cat > "$tmp/lib.sh" <<EOF
set -e
die() { echo "ERROR: \$*" >&2; exit 1; }
$(extract wave_each)
$(extract wave_stream_wait)
$(extract setup_all_wave)
EOF

cat > "$tmp/stubs.sh" <<'EOF'
log() { printf '%s %s src=%s\n' "$(date +%s%N 2>/dev/null || date +%s)" "$*" "${SOURCE_FLAG#--source }" >> "$LOG"; }
source_of() { printf '%s' "${SOURCE_FLAG#--source }"; }
ensure_hubs() { log hubs; }
require_spoke_caps_configured() { :; }
enabled_market_names() { echo "M1 M2 M3 M4 M5 M6"; }
enabled_spoke_ids() { echo "1 2"; }
enabled_spoke_asset_names() { echo "A B C"; }
ensure_spoke() {
    log spoke "$1"
    jq --arg c "$1" --argjson id "$(( $1 + 10 ))" '.testnet.spoke_ids[$c] = $id' "$NETWORKS_FILE" > "$NETWORKS_FILE.tmp" && mv "$NETWORKS_FILE.tmp" "$NETWORKS_FILE"
    echo "$(( $1 + 10 ))"
}
get_mapped_spoke_id() { jq -r --arg c "$1" '.testnet.spoke_ids[$c] // empty' "$NETWORKS_FILE"; }
setup_all_reference_oracles() { log refs; }
create_market() {
    [ "${FAIL_MARKET:-}" != "$1" ] || return 1
    log market "$1" "cfg=$NETWORKS_FILE"; sleep 1
}
configure_market_oracle() {
    [ "${AUTO_EXECUTE:-1}" = 0 ] || return 1
    log propose "$1"
    echo "Scheduled op 0${1#M}0 (AUTO_EXECUTE=0; run 'executeOp 0${1#M}0' after the delay)." >&2
}
await_op_ready() { log await "$1"; }
execute_op() { log exec-start "$1"; sleep 1; echo "Signing transaction: $1" >&2; log exec-end "$1"; }
configure_spoke_curves() { log curves; }
ensure_asset_in_spoke() { log asset "$1" "$2" "$3"; }
EOF

run_wave() {
    : > "$tmp/log"
    echo '{"testnet":{"spoke_ids":{}}}' > "$tmp/networks.json"
    env LOG="$tmp/log" NETWORKS_FILE="$tmp/networks.json" NETWORK="${NET:-testnet}" SIGNER="${SIGNER_ID:-admin}" \
        SETUP_JOBS="${JOBS:-5}" SETUP_SOURCES="${SOURCES:-c0 c1 c2 c3 c4}" MARKET_CONFIG_FILE=markets.json FAIL_MARKET="${FAIL_MARKET:-}" \
        bash -c "source '$tmp/stubs.sh'; source '$tmp/lib.sh'; setup_all_wave" > "$tmp/out" 2> "$tmp/err"
}

run_wave || fail "wave setup failed: $(tail -3 "$tmp/err")"
grep -q '^[0-9]* hubs ' "$tmp/log" || fail "hubs not created"
[ "$(grep -c ' market ' "$tmp/log")" = 6 ] || fail "not every market created"
grep ' market ' "$tmp/log" | grep -qE 'src=c[01]$' && fail "a market worker reused the spoke or reference stream source"
grep ' market ' "$tmp/log" | grep -q "cfg=$tmp/networks.json src" && fail "market workers read the live config, not the snapshot"
grep ' spoke ' "$tmp/log" | grep -qv 'src=c0$' && fail "spokes left the spoke stream source"
grep ' refs ' "$tmp/log" | grep -q 'src=c1$' || fail "reference oracles left their stream source"
[ "$(grep -c ' propose ' "$tmp/log")" = 6 ] || fail "not every market oracle proposed with AUTO_EXECUTE=0"
[ "$(grep -c ' exec-end ' "$tmp/log")" = 6 ] || fail "not every scheduled oracle op executed"
grep -E ' exec-(start|end) ' "$tmp/log" | grep -qv 'src=c0$' && fail "oracle executes left the executor source"
awk '/ exec-start /{if (open) bad=1; open=1} / exec-end /{open=0} END {exit bad}' "$tmp/log" || fail "oracle executes overlapped"
[ "$(grep ' exec-start ' "$tmp/log" | awk '{print $3}' | tr '\n' ' ')" = "010 020 030 040 050 060 " ] || fail "oracle executes left market config order"
last_propose=$(grep -n ' propose ' "$tmp/log" | tail -1 | cut -d: -f1)
first_exec=$(grep -n ' exec-start ' "$tmp/log" | head -1 | cut -d: -f1)
[ "$first_exec" -gt "$last_propose" ] || fail "an oracle executed before every proposal was scheduled"
[ "$(grep -c ' asset ' "$tmp/log")" = 6 ] || fail "not every spoke asset listed"
grep ' asset ' "$tmp/log" | grep -qE 'src=c[01]$' && fail "an asset worker shared the executor or curve source"
grep ' asset 11 A 1 ' "$tmp/log" >/dev/null && grep ' asset 12 C 2 ' "$tmp/log" >/dev/null || fail "asset items lost their on-chain spoke ids"
grep ' curves ' "$tmp/log" | grep -q 'src=c1$' || fail "curves left their stream source"
[ "$(grep -c 'Signing transaction:' "$tmp/err")" = 6 ] || fail "worker submissions missing from stderr"

NET=mainnet run_wave && fail "wave mode ran on mainnet"
grep -q 'testnet-only' "$tmp/err" || fail "mainnet refusal message missing"
SIGNER_ID=ledger run_wave && fail "wave mode ran with a Ledger signer"
SOURCES="c0 c1" run_wave && fail "wave mode ran with fewer than 3 sources"
FAIL_MARKET=M3 run_wave && fail "a failed market worker did not fail the wave"
echo "setupAll wave mode: default stays serial, testnet-only, isolated sources, serial oracle executes"
