#!/usr/bin/env bash
# Offline checks of the spoke-id and listing-flag helpers in configs/script.sh.
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
$(extract persist_spoke_id)
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

expect "merge_tightened_flags '$LIVE_ON' frozen" '{"paused":true,"frozen":true,"no_seize":false}'
expect "merge_tightened_flags '$LIVE_OFF' paused,no_seize" '{"paused":true,"frozen":false,"no_seize":true}'
expect_die "merge_tightened_flags '$LIVE_ON' unpause"   # unknown flag name
expect_die "merge_tightened_flags '$LIVE_ON' ''"        # nothing requested
expect_die "merge_tightened_flags '{\"paused\":1}' paused" # listing without boolean flags
expect_die "merge_tightened_flags '' paused"             # unreadable listing: fail closed

expect "persist_spoke_id 9 8 && jq -r '.mainnet.spoke_ids[\"9\"]' '$tmp/networks.json'" 8
# An op id (AUTO_EXECUTE=0) must stop the caller, also inside $(...), where errexit is off.
expect "x=\$(persist_spoke_id 7 478cd356fa3de0e609f43b6df283fd3062e95b4b6e2f20232c525b8025980a01; echo reached) || true; echo \"[\$x]\"" '[]'

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
