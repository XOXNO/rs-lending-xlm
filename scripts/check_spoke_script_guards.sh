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
{"5": {"assets": {"SILENT": {"ltv": 1}, "TIGHTEN": {"paused": true}, "RELAX": {"paused": false}, "XLM": {"hub_id": 1}}}}
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
$(extract merge_relaxed_flags)
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
expect_die "RELAX_SPOKE_FLAGS=1 resolve_spoke_flag paused 5 RELAX '$LIVE_ON'" # no override
refusal=$(bash -c "source '$tmp/lib.sh'; resolve_spoke_flag paused 5 RELAX '$LIVE_ON'" 2>&1 || true)
grep -q "relaxAssetFlags 5 RELAX paused" <<<"$refusal" || fail "a refused relaxation must name relaxAssetFlags"
expect_die "resolve_spoke_flag paused 5 SILENT ''"              # unreadable listing: fail closed
expect_die "resolve_spoke_flag paused 5 SILENT '{\"paused\":1}'" # not a boolean

expect "merge_tightened_flags '$LIVE_ON' frozen" '{"paused":true,"frozen":true,"no_seize":false}'
expect "merge_tightened_flags '$LIVE_OFF' paused,no_seize" '{"paused":true,"frozen":false,"no_seize":true}'
expect_die "merge_tightened_flags '$LIVE_ON' unpause"   # unknown flag name
expect_die "merge_tightened_flags '$LIVE_ON' ''"        # nothing requested
expect_die "merge_tightened_flags '{\"paused\":1}' paused" # listing without boolean flags
expect_die "merge_tightened_flags '' paused"             # unreadable listing: fail closed

LIVE_ALL='{"paused":true,"frozen":true,"no_seize":true}'
expect "merge_relaxed_flags '$LIVE_ALL' frozen" '{"paused":true,"frozen":false,"no_seize":true}'
expect "merge_relaxed_flags '$LIVE_ALL' paused,no_seize" '{"paused":false,"frozen":true,"no_seize":false}'
expect "merge_relaxed_flags '$LIVE_ON' paused" '{"paused":false,"frozen":false,"no_seize":false}'
expect_die "merge_relaxed_flags '$LIVE_ON' frozen"         # not set on chain: nothing to clear
expect_die "merge_relaxed_flags '$LIVE_ALL' unfreeze"      # unknown flag name
expect_die "merge_relaxed_flags '$LIVE_ALL' ''"            # nothing requested
expect_die "merge_relaxed_flags '{\"paused\":1}' paused"   # listing without boolean flags
expect_die "merge_relaxed_flags '' paused"                 # unreadable listing: fail closed

# The verb end to end, with the chain reads and the proposer stubbed.
cat > "$tmp/relax.sh" <<EOF
source '$tmp/lib.sh'
NETWORK=mainnet SOURCE_FLAG=
get_controller() { echo CCTRL; }
get_market_value() { echo CASSET; }
stellar() {
    case "\$*" in
        *"--send=no -- get_spoke_asset --spoke_id 4 "*) echo listing >> '$tmp/calls'; echo '$LIVE_ALL' ;;
        *"--send=no -- get_spoke_asset_flags_epoch --spoke_id 4 "*)
            echo epoch >> '$tmp/calls'; sed -n 1p '$tmp/epochs'; sed -i.bak 1d '$tmp/epochs' ;;
        *) echo "unexpected stellar call: \$*" >&2; return 1 ;;
    esac
}
schedule_via_proposer() { printf '%s\n' "\$@" > '$tmp/proposed'; echo OPID; }
schedule_and_maybe_execute() { [ "\$1" = OPID ]; }
$(extract scval_hub_asset)
$(extract admin_op)
$(extract gen_salt)
$(extract read_spoke_flags_epoch)
$(extract relax_asset_flags)
EOF
run_relax() {
    printf '%s\n' $1 > "$tmp/epochs"
    : > "$tmp/calls"
    rm -f "$tmp/proposed"
    bash -c "source '$tmp/relax.sh'; relax_asset_flags 4 XLM 5 $2"
}
expect_relax() {
    local cleared=$1 p=$2 f=$3 n=$4 got
    run_relax "7 7" "$cleared" 2>/dev/null \
        || fail "relax_asset_flags $cleared failed against a stubbed live listing"
    [ "$(tr '\n' ' ' < "$tmp/calls")" = "epoch listing epoch " ] \
        || fail "relax must read the epoch before and after the listing, read '$(tr '\n' ' ' < "$tmp/calls")'"
    [ "$(sed -n 1p "$tmp/proposed")" = relax_spoke_asset_flags ] || fail "relax must schedule relax_spoke_asset_flags"
    got=$(sed -n 2p "$tmp/proposed")
    [ "$got" = "{\"RelaxSpokeAssetFlags\":{\"spoke_id\":4,\"hub_asset\":{\"hub_id\":1,\"asset\":\"CASSET\"},\"expected_epoch\":7,\"paused\":$p,\"frozen\":$f,\"no_seize\":$n}}" ] \
        || fail "relax $cleared proposed '$got'"
    got=$(sed -n 3p "$tmp/proposed")
    [ "$got" = "[{\"u32\":4},{\"map\":[{\"key\":{\"symbol\":\"asset\"},\"val\":{\"address\":\"CASSET\"}},{\"key\":{\"symbol\":\"hub_id\"},\"val\":{\"u32\":1}}]},{\"u64\":\"7\"},{\"bool\":$p},{\"bool\":$f},{\"bool\":$n}]" ] \
        || fail "relax $cleared call args '$got'"
}
expect_relax frozen true false true
expect_relax paused false true true
expect_relax frozen,no_seize true false false
if run_relax "7 7" bogus >/dev/null 2>&1; then
    fail "relax_asset_flags must refuse an unknown flag"
fi
if race=$(run_relax "7 8" frozen 2>&1); then
    fail "relax_asset_flags must stop when the flags epoch moves during its reads"
fi
[ ! -e "$tmp/proposed" ] || fail "relax_asset_flags scheduled an operation although the flags epoch moved"
grep -q "flags changed while reading" <<<"$race" || fail "a moved flags epoch must say the flags changed: '$race'"

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

for verb in tightenAssetFlags relaxAssetFlags; do
    awk -v v="\"$verb\")" '$0 ~ v {p=1} p {print} p && /;;/ {exit}' "$SCRIPT" \
        | grep -q 'resolve_config_spoke_id "\$2"' || fail "$verb must resolve its config spoke id"
done
if grep -q RELAX_SPOKE_FLAGS "$SCRIPT"; then
    fail "RELAX_SPOKE_FLAGS must not exist: an edit cannot clear a flag on chain"
fi
if extract edit_asset_in_spoke | grep -qE '(paused|frozen|no_seize)=false'; then
    fail "edit_asset_in_spoke must not default an emergency flag to false"
fi
for flag in paused frozen no_seize; do
    extract edit_asset_in_spoke | grep -q "resolve_spoke_flag $flag " || fail "edit_asset_in_spoke must resolve $flag from the live listing"
done

echo "spoke script guards: OK"

# A failed submission inside command substitution must never mark an op done.
# Both retry entry points share this guarantee; Bash errexit is disabled by ||.
cat > "$tmp/execute.sh" <<EOF
set -e
SOURCE_FLAG= NETWORK=testnet
op_record_path() { echo '$tmp/op.json'; }
get_governance() { echo CTEST; }
retry_tx() { echo injected-submission-failure >&2; return 7; }
mark_op_executed() { echo marked >> '$tmp/incorrectly-marked'; }
$(extract execute_gov_self_op)
$(extract execute_op)
EOF
printf '%s\n' '{"cli_executable":true,"kind":"controller","target":"C","function":"pause","predecessor":"00","salt":"00","args":[]}' > "$tmp/op.json"
if bash -c "source '$tmp/execute.sh'; result=\$(execute_op test) || exit \$?"; then fail 'failed controller execution reported success'; fi
printf '%s\n' '{"cli_executable":true,"kind":"governance_self","execute_label":"UpdateGovDelay","salt":"00","op":{}}' > "$tmp/op.json"
if bash -c "source '$tmp/execute.sh'; result=\$(execute_op test) || exit \$?"; then fail 'failed self execution reported success'; fi
[ ! -f "$tmp/incorrectly-marked" ] || fail 'failed operation was marked executed'
echo 'operator failed-submission regression: OK'

# Disposable roots route every config and operation write away from the checkout.
cat > "$tmp/isolation.sh" <<EOF
CONFIG_ROOT='$tmp/disposable-config' OPS_ROOT='$tmp/disposable-ops' NETWORK=testnet
$(sed -n '5,15p' "$SCRIPT")
$(sed -n '/^OPS_DIR=/p; /^ORACLE_FEEDS_FILE=/p' "$SCRIPT")
$(extract ops_dir)
$(extract op_record_path)
$(extract get_mapped_hub_id)
$(extract persist_hub_id)
$(extract persist_spoke_id)
$(extract ensure_hub)
$(extract parse_returned_u32)
die() { echo "ERROR: \$*" >&2; exit 1; }
gen_salt() { echo salt; }
admin_op() { echo '{}'; }
schedule_via_proposer() { echo scheduled >> '$tmp/schedules'; echo op; }
op_state() { echo Ready; }
await_op_ready() { :; }
execute_op() { echo 1; }
EOF
mkdir -p "$tmp/disposable-config/testnet"
printf '%s\n' '{"testnet":{"hub_ids":{},"spoke_ids":{}}}' > "$tmp/disposable-config/networks.json"
bash -c 'source "$1"; for f in "$NETWORKS_FILE" "$HUBS_FILE" "$SPOKES_FILE" "$MARKET_CONFIG_FILE" "$BLEND_POOLS_FILE" "$ORACLE_FEEDS_FILE"; do
    [[ "$f" == "$CONFIG_ROOT/"* ]] || exit 1
done
[[ "$(op_record_path sample)" == "$OPS_ROOT/testnet/sample.json" ]] || exit 1
ensure_hub 1 || exit 1
persist_spoke_id 1 1 || exit 1
cp "$NETWORKS_FILE" "$NETWORKS_FILE.before"
ensure_hub 1 || exit 1
cmp "$NETWORKS_FILE.before" "$NETWORKS_FILE"' _ "$tmp/isolation.sh" || fail 'operator configuration isolation or replay failed'
[ "$(wc -l < "$tmp/schedules" | tr -d ' ')" = 1 ] || fail 'setup replay scheduled duplicate hub'
echo 'operator root isolation and idempotent hub replay: OK'

# Probe the selected controller, independent of spoke listings or stale local pool mappings.
cat > "$tmp/market.json" <<'JSON'
{"markets":[{"name":"XLM","asset_address":"CASSET","hub_id":1,"market_params":{"is_flashloanable":false,"flashloan_fee":0}}]}
JSON
cat > "$tmp/market.sh" <<EOF
NETWORK=testnet SOURCE_FLAG= MARKET_CONFIG_FILE='$tmp/market.json'
get_pool() { echo CSTALE; }
get_controller() { echo CCTRL; }
get_market_value() { jq -r --arg k "\$2" '.markets[0][\$k]' "\$MARKET_CONFIG_FILE"; }
get_contract_decimals() { echo 7; }
build_hub_assets_json() { echo '[{"hub_id":1,"asset":"CASSET"}]'; }
stellar() {
    [[ "\$*" == *'--id CCTRL '* && "\$*" == *'--send=no -- get_market_index --hub_asset '* ]] || exit 99
    [ "\$MARKET_EXISTS" = yes ]
}
scval_market_params() { echo '{}'; }
gen_salt() { echo salt; }
admin_op() { echo '{}'; }
schedule_via_proposer() { echo scheduled >> '$tmp/market-schedules'; echo op; }
schedule_and_maybe_execute() { [ "\$1" = op ]; }
$(extract create_market)
EOF
bash -c 'source "$1"; MARKET_EXISTS=yes create_market XLM' _ "$tmp/market.sh" >/dev/null || fail 'existing pool market must skip creation without a spoke'
[ ! -e "$tmp/market-schedules" ] || fail 'existing market was scheduled again'
bash -c 'source "$1"; MARKET_EXISTS=no create_market XLM; MARKET_EXISTS=yes create_market XLM' _ "$tmp/market.sh" >/dev/null || fail 'missing market creation or replay failed'
[ "$(wc -l < "$tmp/market-schedules" | tr -d ' ')" = 1 ] || fail 'market creation must schedule exactly once'

# Explicit hub listings need one qualified read, never the discarded spoke read.
cat > "$tmp/listing-spokes.json" <<'JSON'
{"5":{"assets":{"XLM":{"hub_id":1,"can_be_collateral":true,"can_be_borrowed":true,"ltv":1,"liquidation_threshold":2,"liquidation_bonus":3,"liquidation_fees":4}}}}
JSON
cat > "$tmp/listing.json" <<'JSON'
{"is_collateralizable":true,"is_borrowable":true,"loan_to_value":1,"liquidation_threshold":2,"liquidation_bonus":3,"liquidation_fees":4,"supply_cap":"1000","borrow_cap":"1000"}
JSON
cat > "$tmp/listing.sh" <<EOF
NETWORK=testnet SOURCE_FLAG= SPOKES_FILE='$tmp/listing-spokes.json'
get_controller() { echo CCTRL; }
get_market_value() { echo CASSET; }
require_spoke_cap() { echo 1000; }
fetch_spoke_json() { echo unnecessary >> '$tmp/spoke-fetch'; return 1; }
stellar() {
    [[ "\$*" == *'--id CCTRL '* && "\$*" == *'--send=no -- get_spoke_asset --spoke_id 4 --hub_asset '* ]] || exit 99
    echo read >> '$tmp/listing-reads'
    [ "\$LISTING_EXISTS" = yes ] || return 1
    cat '$tmp/listing.json'
}
add_asset_to_spoke() { echo add >> '$tmp/listing-mutations'; }
edit_asset_in_spoke() { echo edit >> '$tmp/listing-mutations'; }
$(extract get_spoke_value)
$(extract ensure_asset_in_spoke)
EOF
bash -c 'source "$1"; LISTING_EXISTS=yes ensure_asset_in_spoke 4 XLM 5' _ "$tmp/listing.sh" >/dev/null || fail 'matching hub listing failed'
[ ! -e "$tmp/listing-mutations" ] || fail 'matching listing was mutated'
bash -c 'source "$1"; LISTING_EXISTS=no ensure_asset_in_spoke 4 XLM 5' _ "$tmp/listing.sh" >/dev/null || fail 'missing listing was not added'
jq '.loan_to_value=0' "$tmp/listing.json" > "$tmp/listing.changed"
mv "$tmp/listing.changed" "$tmp/listing.json"
bash -c 'source "$1"; LISTING_EXISTS=yes ensure_asset_in_spoke 4 XLM 5' _ "$tmp/listing.sh" >/dev/null || fail 'changed listing was not edited'
[ "$(tr '\n' ' ' < "$tmp/listing-mutations")" = 'add edit ' ] || fail 'listing reconciliation changed'
[ "$(wc -l < "$tmp/listing-reads" | tr -d ' ')" = 3 ] || fail 'each listing needs exactly one read'
[ ! -e "$tmp/spoke-fetch" ] || fail 'explicit hub listing fetched discarded spoke data'
echo 'operator market replay and qualified listing reads: OK'
