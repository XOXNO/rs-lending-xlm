#!/usr/bin/env bash
# Pins three operator-safety rules of configs/script.sh.
# 1. A CONFIG spoke id is never sent on chain unresolved. Mainnet maps config
#    5 -> on-chain 4, and a verb that used one number for both roles listed
#    USDT0 on the wrong spoke.
# 2. A listing edit carries the LIVE paused/frozen/no_seize flags when the
#    config is silent. Defaulting them to false cleared GUARDIAN flags.
# 3. The GUARDIAN verb tightenAssetFlags can only raise a flag.
#
# Runs offline: extracts the resolver functions (script.sh runs its dispatch
# when sourced) and exercises them against a temporary networks file.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SCRIPT="$ROOT/configs/script.sh"
fail() { echo "FAIL: $*" >&2; exit 1; }

extract() { awk -v fn="$1" '$0 ~ "^"fn"\\(\\) \\{" {p=1} p {print} p && /^}/ {exit}' "$SCRIPT"; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cat > "$tmp/networks.json" <<'JSON'
{"mainnet": {"spoke_ids": {"1": 1, "5": 4, "6": 5}}}
JSON
cat > "$tmp/spokes.json" <<'JSON'
{"5": {"assets": {"SILENT": {"ltv": 1}, "TIGHTEN": {"paused": true}, "RELAX": {"paused": false}}}}
JSON

cat > "$tmp/lib.sh" <<EOF
NETWORK=mainnet
NETWORKS_FILE="$tmp/networks.json"
SPOKES_FILE="$tmp/spokes.json"
die() { echo "ERROR: \$*" >&2; exit 1; }
$(extract get_mapped_spoke_id)
$(extract resolve_config_spoke_id)
$(extract resolve_spoke_arg)
$(extract get_spoke_value)
$(extract resolve_spoke_flag)
$(extract merge_tightened_flags)
EOF

run() { bash -c "source '$tmp/lib.sh'; $1" 2>/dev/null; }
expect() {
    local got
    got=$(run "$1") || fail "'$1' exited non-zero, expected '$2'"
    [ "$got" = "$2" ] || fail "'$1' returned '$got', expected '$2'"
}
expect_die() { if run "$1" >/dev/null; then fail "'$1' must exit non-zero"; fi; }

expect 'resolve_config_spoke_id 5' 4
expect 'resolve_config_spoke_id 6' 5
expect 'resolve_spoke_arg 5' 4
expect 'resolve_spoke_arg onchain:5' 5
expect_die 'resolve_config_spoke_id 3'         # unmapped: never fall back to the same number
expect_die 'resolve_config_spoke_id onchain:5' # add/edit read config; a raw id has no params
expect_die 'resolve_spoke_arg onchain:'
expect_die 'resolve_spoke_arg 5x'
expect_die 'resolve_spoke_arg ""'

# Flags: config silent -> live; explicit tighten ok; explicit relax needs consent.
LIVE_ON='{"paused":true,"frozen":false,"no_seize":false}'
LIVE_OFF='{"paused":false,"frozen":false,"no_seize":false}'
expect "resolve_spoke_flag paused 5 SILENT '$LIVE_ON'" true
expect "resolve_spoke_flag paused 5 SILENT '$LIVE_OFF'" false
expect "resolve_spoke_flag paused 5 TIGHTEN '$LIVE_OFF'" true
expect "resolve_spoke_flag paused 5 RELAX '$LIVE_OFF'" false
expect_die "resolve_spoke_flag paused 5 RELAX '$LIVE_ON'"
expect "RELAX_SPOKE_FLAGS=1 resolve_spoke_flag paused 5 RELAX '$LIVE_ON'" false
expect_die "resolve_spoke_flag paused 5 SILENT ''"              # unreadable listing: fail closed
expect_die "resolve_spoke_flag paused 5 SILENT '{\"paused\":1}'" # not a boolean

# GUARDIAN verb: requested flags are OR-ed onto the live ones, never cleared.
expect "merge_tightened_flags '$LIVE_ON' frozen" '{"paused":true,"frozen":true,"no_seize":false}'
expect "merge_tightened_flags '$LIVE_OFF' paused,no_seize" '{"paused":true,"frozen":false,"no_seize":true}'
expect_die "merge_tightened_flags '$LIVE_ON' unpause"   # unknown flag name
expect_die "merge_tightened_flags '$LIVE_ON' ''"        # nothing requested
expect_die "merge_tightened_flags '{\"paused\":1}' paused" # listing without boolean flags
expect_die "merge_tightened_flags '' paused"             # unreadable listing: fail closed

# Static pins on the call sites.
if grep -n 'config_category_id=\${3:-' "$SCRIPT"; then
    fail "add/edit_asset_in_spoke must not default the config id to the on-chain id"
fi
for verb in addAssetToSpoke editAssetInSpoke; do
    awk -v v="\"$verb\")" '$0 ~ v {p=1} p {print} p && /;;/ {exit}' "$SCRIPT" \
        | grep -q 'resolve_config_spoke_id "\$2"' || fail "$verb must resolve its config spoke id"
done
for verb in removeSpoke removeAssetFromSpoke setSpokeLiquidationCurve getSpoke getSpokeAsset; do
    awk -v v="\"$verb\")" '$0 ~ v {p=1} p {print} p && /;;/ {exit}' "$SCRIPT" \
        | grep -q 'resolve_spoke_arg "\$2"' || fail "$verb must resolve its spoke argument"
done
if extract ensure_spoke | grep -q 'fetch_spoke_json "\$config_category_id"'; then
    fail "ensure_spoke must not reuse the on-chain spoke that shares the config number"
fi

awk '$0 ~ /"tightenAssetFlags"\)/ {p=1} p {print} p && /;;/ {exit}' "$SCRIPT" \
    | grep -q 'resolve_config_spoke_id "\$2"' || fail "tightenAssetFlags must resolve its config spoke id"
if extract edit_asset_in_spoke | grep -qE '(paused|frozen|no_seize)=false'; then
    fail "edit_asset_in_spoke must not default an emergency flag to false"
fi
for flag in paused frozen no_seize; do
    extract edit_asset_in_spoke | grep -q "resolve_spoke_flag $flag " || fail "edit_asset_in_spoke must resolve $flag from the live listing"
done

echo "spoke script guards: OK"
