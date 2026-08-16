#!/bin/bash

set -e

NETWORK=${NETWORK:-testnet}
SIGNER=${SIGNER:-deployer}
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

NETWORKS_FILE="$SCRIPT_DIR/networks.json"
HUBS_FILE="$SCRIPT_DIR/${NETWORK}/hubs.json"
SPOKES_FILE="$SCRIPT_DIR/${NETWORK}/spokes.json"
MARKET_CONFIG_FILE="$SCRIPT_DIR/${NETWORK}/markets.json"
BLEND_POOLS_FILE="$SCRIPT_DIR/${NETWORK}/blend.json"

require_tool() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "ERROR: Missing required tool: $1" >&2
        exit 1
    fi
}

die() {
    echo "ERROR: $*" >&2
    exit 1
}

require_tool stellar
require_tool jq

SIGNER_ADDRESS=$(stellar keys public-key "$SIGNER" 2>/dev/null || stellar keys address "$SIGNER" 2>/dev/null || echo "$SIGNER")
if [ "$SIGNER" = "ledger" ]; then
    SOURCE_FLAG="--source-account $SIGNER_ADDRESS --sign-with-ledger"
else
    SOURCE_FLAG="--source $SIGNER"
fi

_cfg_rpc=$(jq -r ".\"$NETWORK\".rpc_url // empty" "$NETWORKS_FILE" 2>/dev/null)
_cfg_pass=$(jq -r ".\"$NETWORK\".network_passphrase // empty" "$NETWORKS_FILE" 2>/dev/null)
if [ -n "$_cfg_rpc" ]; then export STELLAR_RPC_URL="$_cfg_rpc"; fi
if [ -n "$_cfg_pass" ]; then export STELLAR_NETWORK_PASSPHRASE="$_cfg_pass"; fi

get_network_value() {
    jq -r ".\"$NETWORK\".\"$1\"" "$NETWORKS_FILE"
}

require_static_config() {
    if [ ! -f "$NETWORKS_FILE" ]; then
        echo "ERROR: Config file not found: $NETWORKS_FILE" >&2
        exit 1
    fi
    if ! jq -e --arg network "$NETWORK" '.[$network] != null' "$NETWORKS_FILE" >/dev/null; then
        echo "ERROR: Network '$NETWORK' not found in $NETWORKS_FILE" >&2
        exit 1
    fi
    if [ ! -f "$MARKET_CONFIG_FILE" ]; then
        echo "ERROR: Config file not found: $MARKET_CONFIG_FILE" >&2
        exit 1
    fi
    if ! jq -e '.markets | type == "array" and length > 0' "$MARKET_CONFIG_FILE" >/dev/null; then
        echo "ERROR: No configured markets in $MARKET_CONFIG_FILE" >&2
        exit 1
    fi
    if ! jq -e 'all(.markets[]; (.name // "") != "" and (.asset_address // "") != "")' "$MARKET_CONFIG_FILE" >/dev/null; then
        echo "ERROR: Every configured market must have name and asset_address in $MARKET_CONFIG_FILE" >&2
        exit 1
    fi
    if ! jq -e 'any(.markets[]; .enabled != false)' "$MARKET_CONFIG_FILE" >/dev/null; then
        echo "ERROR: All markets in $MARKET_CONFIG_FILE have enabled=false; nothing to deploy" >&2
        exit 1
    fi
    if [ ! -f "$SPOKES_FILE" ]; then
        echo "ERROR: Config file not found: $SPOKES_FILE" >&2
        exit 1
    fi
    if ! jq -e 'type == "object"' "$SPOKES_FILE" >/dev/null; then
        echo "ERROR: Spoke config in $SPOKES_FILE is not a JSON object" >&2
        exit 1
    fi
}

get_market_value() {
    local market=$1
    local field=$2
    jq -r ".markets[] | select(.name == \"$market\") | .$field" "$MARKET_CONFIG_FILE"
}

get_spoke_value() {
    local category_id=$1
    local path=$2
    jq -r ".\"$category_id\"$path" "$SPOKES_FILE"
}

# Deploy filter: explicit "enabled": false excludes an entry from bulk setupAll* /
# claim*/view-all helpers. Missing field or true means included (backward compatible).
# Direct verbs (createMarket, addSpoke, addAssetToSpoke, ...) still accept disabled
# entries so a later opt-in listing does not require deleting the flag first.

is_market_enabled() {
    local market=$1
    jq -e --arg m "$market" '
        (first(.markets[] | select(.name == $m)) // null) as $mkt |
        $mkt != null and ($mkt.enabled != false)
    ' "$MARKET_CONFIG_FILE" >/dev/null
}

enabled_market_names() {
    jq -r '.markets[] | select(.enabled != false) | .name' "$MARKET_CONFIG_FILE"
}

disabled_market_names() {
    jq -r '[.markets[] | select(.enabled == false) | .name] | join(", ")' "$MARKET_CONFIG_FILE"
}

is_spoke_enabled() {
    local category_id=$1
    jq -e --arg c "$category_id" '
        (.[$c] // null) as $s |
        $s != null and ($s.enabled != false)
    ' "$SPOKES_FILE" >/dev/null
}

enabled_spoke_ids() {
    jq -r 'to_entries[] | select(.value.enabled != false) | .key' "$SPOKES_FILE"
}

disabled_spoke_ids() {
    jq -r '[to_entries[] | select(.value.enabled == false) | .key] | join(", ")' "$SPOKES_FILE"
}

is_spoke_asset_enabled() {
    local category_id=$1
    local asset_name=$2
    jq -e --arg c "$category_id" --arg a "$asset_name" '
        (.[$c] // null) as $s |
        $s != null and ($s.enabled != false) and
        (($s.assets[$a] // null) as $asset |
            $asset != null and ($asset.enabled != false))
    ' "$SPOKES_FILE" >/dev/null
}

enabled_spoke_asset_names() {
    local category_id=$1
    jq -r --arg c "$category_id" '
        (.[$c] // empty) | select(.enabled != false) |
        (.assets // {}) | to_entries[] | select(.value.enabled != false) | .key
    ' "$SPOKES_FILE"
}

require_spoke_cap() {
    local category_id=$1
    local asset_name=$2
    local field=$3
    local value
    value=$(get_spoke_value "$category_id" ".assets.\"$asset_name\".$field")
    if [ -z "$value" ] || [ "$value" = "null" ]; then
        die "spoke asset ${asset_name} (category ${category_id}) missing ${field} in ${SPOKES_FILE}; every cap is an enforced ceiling and there is no unlimited sentinel, so state one explicitly (\"0\" accepts nothing on that side)"
    fi
    case "$value" in
        ''|*[!0-9]*)
            die "spoke asset ${asset_name} (category ${category_id}) has invalid ${field} '${value}' in ${SPOKES_FILE}; expected a decimal integer of base units quoted as a JSON string" ;;
    esac
    printf '%s\n' "$value"
}

require_spoke_caps_configured() {
    local bad
    bad=$(jq -r '
        to_entries[] | .key as $cat | (.value.assets // {}) | to_entries[] |
        .key as $asset | .value as $cfg |
        ("supply_cap", "borrow_cap") as $field |
        select(($cfg[$field] == null) or (($cfg[$field] | tostring | test("^[0-9]+$")) | not)) |
        "       category \($cat), asset \($asset): \($field) is \($cfg[$field] | tojson)"
    ' "$SPOKES_FILE")
    if [ -n "$bad" ]; then
        echo "ERROR: ${SPOKES_FILE} has spoke assets without a usable cap:" >&2
        printf '%s\n' "$bad" >&2
        die "refusing to submit spoke transactions; every cap is an enforced ceiling and there is no unlimited sentinel (\"0\" accepts nothing on that side)"
    fi
}

get_controller() {
    stellar contract alias show controller --network "$NETWORK" 2>/dev/null || get_network_value "controller"
}

get_governance() {
    stellar contract alias show governance --network "$NETWORK" 2>/dev/null || get_network_value "governance"
}

get_price_aggregator() {
    local gov addr
    gov=$(get_governance 2>/dev/null) || gov=""
    if [ -n "$gov" ] && [ "$gov" != "null" ]; then
        addr=$(stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
            --send=no -- price_aggregator 2>/dev/null | tr -d '"') || addr=""
        if [ -n "$addr" ] && [ "$addr" != "null" ]; then
            echo "$addr"
            return 0
        fi
    fi
    stellar contract alias show price-aggregator --network "$NETWORK" 2>/dev/null \
        || stellar contract alias show price_aggregator --network "$NETWORK" 2>/dev/null \
        || get_network_value "price_aggregator" \
        || get_network_value "price-aggregator"
}

get_pool() {
    get_network_value "pool"
}

get_aggregator_address() {
    local addr
    addr=$(jq -r ".\"$NETWORK\".aggregator // empty" "$NETWORKS_FILE")
    if [ -n "${AGGREGATOR_CONTRACT:-}" ]; then
        addr="$AGGREGATOR_CONTRACT"
    fi
    if [ -z "$addr" ] || [ "$addr" = "null" ]; then
        echo ""
        return 1
    fi
    echo "$addr"
}

get_accumulator_address() {
    local addr
    addr=$(jq -r ".\"$NETWORK\".accumulator // empty" "$NETWORKS_FILE")
    if [ -n "${ACCUMULATOR_CONTRACT:-}" ]; then
        addr="$ACCUMULATOR_CONTRACT"
    fi
    if [ -z "$addr" ] || [ "$addr" = "null" ]; then
        echo ""
        return 1
    fi
    echo "$addr"
}

get_cex_oracle() { get_network_value "reflector_cex_oracle"; }
get_dex_oracle() { get_network_value "reflector_dex_oracle"; }
get_fx_oracle()  { get_network_value "reflector_fx_oracle"; }

get_oracle() { get_cex_oracle; }

get_redstone_adapter() {
    get_network_value "redstone_adapter_contract"
}

get_xoxno_oracle_adapter() {
    get_network_value "xoxno_oracle_adapter"
}

get_signer_address() {
    echo "$SIGNER_ADDRESS"
}

invoke_view() {

    local output
    output=$(stellar contract invoke --id "$1" $SOURCE_FLAG --network "$NETWORK" --send=no -- "${@:2}")
    if command -v jq >/dev/null 2>&1 && printf '%s' "$output" | jq . >/dev/null 2>&1; then
        printf '%s' "$output" | jq .
    else
        printf '%s\n' "$output"
    fi
}

get_contract_decimals() {
    invoke_view "$1" decimals | tail -n1
}

ZERO_PREDECESSOR_HEX="0000000000000000000000000000000000000000000000000000000000000000"

OPS_DIR="$ROOT_DIR/configs/ops/$NETWORK"

AWAIT_POLL_SECONDS=${AWAIT_POLL_SECONDS:-5}

AWAIT_MAX_WAIT_SECONDS=${AWAIT_MAX_WAIT_SECONDS:-0}

ops_dir() {
    mkdir -p "$OPS_DIR"
    echo "$OPS_DIR"
}

op_record_path() {
    echo "$(ops_dir)/$1.json"
}

gen_salt() {
    local function=$1
    local args_json=$2
    local key
    key=$(printf '%s|%s|%s' "$NETWORK" "$function" "$args_json")
    if [ -n "${SALT_NONCE:-}" ]; then
        key="${key}|nonce:${SALT_NONCE}"
    fi
    local hash
    if command -v sha256sum >/dev/null 2>&1; then
        hash=$(printf '%s' "$key" | sha256sum | cut -c1-64)
    else
        hash=$(printf '%s' "$key" | shasum -a 256 | cut -c1-64)
    fi
    echo "$hash"
}

scval_address() { jq -nc --arg v "$1" '{address:$v}'; }
scval_symbol()  { jq -nc --arg v "$1" '{symbol:$v}'; }
scval_bytes()   { jq -nc --arg v "$1" '{bytes:$v}'; }
scval_u32()     { jq -nc --argjson v "$1" '{u32:$v}'; }
scval_u64()     { jq -nc --arg v "$1" '{u64:$v}'; }
scval_bool()    { jq -nc --argjson v "$1" '{bool:$v}'; }
scval_i128()    { jq -nc --arg v "$1" '{i128:$v}'; }
scval_vec_u32() {

    jq -nc --argjson a "$1" '{vec: ($a | map({u32: .}))}'
}

scval_position_limits() {

    local j=$1
    jq -nc \
        --argjson mb "$(printf '%s' "$j" | jq '.max_borrow_positions')" \
        --argjson ms "$(printf '%s' "$j" | jq '.max_supply_positions')" \
        '{map:[
            {key:{symbol:"max_borrow_positions"},val:{u32:$mb}},
            {key:{symbol:"max_supply_positions"},val:{u32:$ms}}
        ]}'
}

scval_interest_rate_model() {
    local j=$1
    jq -nc --argjson p "$j" '
        def i(k): {key:{symbol:k}, val:{i128:($p[k] | tostring)}};
        {map: [
            i("base_borrow_rate"),
            {key:{symbol:"flashloan_fee"}, val:{u32:($p.flashloan_fee)}},
            {key:{symbol:"is_flashloanable"}, val:{bool:($p.is_flashloanable)}},
            i("max_borrow_rate"),
            i("max_utilization"),
            i("mid_utilization"),
            i("optimal_utilization"),
            {key:{symbol:"reserve_factor"}, val:{u32:($p.reserve_factor)}},
            i("slope1"),
            i("slope2"),
            i("slope3")
        ]}'
}

scval_market_params() {
    local j=$1
    jq -nc --argjson p "$j" '
        def i(k): {key:{symbol:k}, val:{i128:($p[k] | tostring)}};
        {map: [
            {key:{symbol:"asset_decimals"}, val:{u32:($p.asset_decimals)}},
            {key:{symbol:"asset_id"}, val:{address:($p.asset_id)}},
            i("base_borrow_rate"),
            {key:{symbol:"flashloan_fee"}, val:{u32:($p.flashloan_fee)}},
            {key:{symbol:"is_flashloanable"}, val:{bool:($p.is_flashloanable)}},
            i("max_borrow_rate"),
            i("max_utilization"),
            i("mid_utilization"),
            i("optimal_utilization"),
            {key:{symbol:"reserve_factor"}, val:{u32:($p.reserve_factor)}},
            i("slope1"),
            i("slope2"),
            i("slope3")
        ]}'
}

scval_hub_asset() {
    local asset=$1 hub_id=$2
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "scval_hub_asset: missing hub_id for asset ${asset}"
    fi
    jq -nc --arg a "$asset" --argjson h "$hub_id" \
        '{map:[
            {key:{symbol:"asset"}, val:{address:$a}},
            {key:{symbol:"hub_id"}, val:{u32:$h}}
        ]}'
}

scval_spoke_args() {
    local hub=$1 asset=$2 spoke=$3 cc=$4 cb=$5 ltv=$6 thr=$7 bonus=$8 sc=$9 bc=${10} lf=${11}
    local paused=${12:-false} frozen=${13:-false}
    case "$sc" in
        ''|*[!0-9]*) die "scval_spoke_args: supply_cap '${sc}' for asset ${asset} is not a decimal integer; caps are always enforced and have no unlimited sentinel" ;;
    esac
    case "$bc" in
        ''|*[!0-9]*) die "scval_spoke_args: borrow_cap '${bc}' for asset ${asset} is not a decimal integer; caps are always enforced and have no unlimited sentinel" ;;
    esac
    jq -nc \
        --argjson hub "$hub" \
        --arg asset "$asset" --argjson spoke "$spoke" --argjson cc "$cc" --argjson cb "$cb" \
        --argjson ltv "$ltv" --argjson thr "$thr" --argjson bonus "$bonus" \
        --arg sc "$sc" --arg bc "$bc" --argjson lf "$lf" \
        --argjson paused "$paused" --argjson frozen "$frozen" \
        '{map:[
            {key:{symbol:"asset"},val:{address:$asset}},
            {key:{symbol:"bonus"},val:{u32:$bonus}},
            {key:{symbol:"borrow_cap"},val:{i128:$bc}},
            {key:{symbol:"can_borrow"},val:{bool:$cb}},
            {key:{symbol:"can_collateral"},val:{bool:$cc}},
            {key:{symbol:"frozen"},val:{bool:$frozen}},
            {key:{symbol:"hub_id"},val:{u32:$hub}},
            {key:{symbol:"liquidation_fees"},val:{u32:$lf}},
            {key:{symbol:"ltv"},val:{u32:$ltv}},
            {key:{symbol:"paused"},val:{bool:$paused}},
            {key:{symbol:"spoke_id"},val:{u32:$spoke}},
            {key:{symbol:"supply_cap"},val:{i128:$sc}},
            {key:{symbol:"threshold"},val:{u32:$thr}}
        ]}'
}

friendly_spoke_args() {
    local hub=$1 asset=$2 spoke=$3 cc=$4 cb=$5 ltv=$6 thr=$7 bonus=$8 sc=$9 bc=${10} lf=${11}
    local paused=${12:-false} frozen=${13:-false}
    jq -nc \
        --argjson hub "$hub" \
        --arg asset "$asset" --argjson spoke "$spoke" --argjson cc "$cc" --argjson cb "$cb" \
        --argjson ltv "$ltv" --argjson thr "$thr" --argjson bonus "$bonus" \
        --arg sc "$sc" --arg bc "$bc" --argjson lf "$lf" \
        --argjson paused "$paused" --argjson frozen "$frozen" \
        '{hub_id:$hub, asset:$asset, spoke_id:$spoke, can_collateral:$cc, can_borrow:$cb,
          paused:$paused, frozen:$frozen,
          ltv:$ltv, threshold:$thr, bonus:$bonus, liquidation_fees:$lf,
          supply_cap:$sc, borrow_cap:$bc}'
}

admin_op() {
    local variant=$1
    shift
    if [ "$#" -eq 0 ]; then
        jq -nc --arg v "$variant" '$v'
    elif [ "$#" -eq 1 ]; then
        jq -nc --arg v "$variant" --argjson p "$1" '{($v): $p}'
    else
        jq -nc --arg v "$variant" \
            --argjson fields "$(jq -nc '$ARGS.positional' --jsonargs "$@")" \
            '{($v): $fields}'
    fi
}

write_op_record() {
    local op_id=$1
    local controller_fn=$2
    local args_json=$3
    local salt_hex=$4
    local cli_executable=$5
    local ctrl
    ctrl=$(get_controller)
    local path
    path=$(op_record_path "$op_id")
    local executed=false
    if [ -f "$path" ]; then
        executed=$(jq -r '.executed // false' "$path")
    fi
    jq -nc \
        --arg op_id "$op_id" \
        --arg network "$NETWORK" \
        --arg target "$ctrl" \
        --arg function "$controller_fn" \
        --argjson args "$args_json" \
        --arg predecessor "$ZERO_PREDECESSOR_HEX" \
        --arg salt "$salt_hex" \
        --argjson cli_executable "$cli_executable" \
        --argjson executed "$executed" \
        '{kind:"controller", op_id:$op_id, network:$network, target:$target, function:$function,
          args:$args, predecessor:$predecessor, salt:$salt,
          cli_executable:$cli_executable, executed:$executed}' > "$path"
    echo "  Recorded op $op_id -> $path" >&2
}

write_gov_self_op_record() {
    local op_id=$1
    local execute_label=$2
    local admin_op_json=$3
    local salt_hex=$4
    local cli_executable=$5
    local path
    path=$(op_record_path "$op_id")
    local executed=false
    if [ -f "$path" ]; then
        executed=$(jq -r '.executed // false' "$path")
    fi
    jq -nc \
        --arg op_id "$op_id" \
        --arg network "$NETWORK" \
        --arg execute_label "$execute_label" \
        --arg salt "$salt_hex" \
        --argjson op "$admin_op_json" \
        --argjson cli_executable "$cli_executable" \
        --argjson executed "$executed" \
        '{kind:"governance_self", op_id:$op_id, network:$network, execute_label:$execute_label,
          salt:$salt, op:$op, cli_executable:$cli_executable, executed:$executed}' > "$path"
    echo "  Recorded governance-self op $op_id -> $path" >&2
}

write_oracle_op_record() {
    local op_id=$1
    local aggregator_fn=$2
    local view_fn=$3
    local resolve_args_json=$4
    local salt_hex=$5
    local agg
    agg=$(get_price_aggregator)
    local path
    path=$(op_record_path "$op_id")
    local executed=false
    if [ -f "$path" ]; then
        executed=$(jq -r '.executed // false' "$path")
    fi
    jq -nc \
        --arg op_id "$op_id" \
        --arg network "$NETWORK" \
        --arg target "$agg" \
        --arg function "$aggregator_fn" \
        --arg predecessor "$ZERO_PREDECESSOR_HEX" \
        --arg salt "$salt_hex" \
        --arg view_fn "$view_fn" \
        --argjson resolve_args "$resolve_args_json" \
        --argjson executed "$executed" \
        '{kind:"price_aggregator", op_id:$op_id, network:$network, target:$target, function:$function,
          predecessor:$predecessor, salt:$salt, cli_executable:true, executed:$executed,
          resolve:{view_fn:$view_fn, args:$resolve_args}}' > "$path"
    echo "  Recorded oracle op $op_id -> $path" >&2
}

price_key_token() {
    jq -nc --arg a "$1" '{Token:$a}'
}

price_key_ref() {
    jq -nc --arg s "$1" '{Ref:$s}'
}

oracle_cfg_cli_union() {
    jq -c '
        def cli_union:
            if type == "object" and has("tag") and has("values") then
                if .values == null or .values == [] then
                    .tag
                elif (.values | type) == "array" and (.values | length) == 1 then
                    {(.tag): (.values[0] | cli_union)}
                else
                    {(.tag): (.values | map(cli_union))}
                end
            elif type == "object" then
                with_entries(.value |= cli_union)
            elif type == "array" then
                map(cli_union)
            else
                .
            end;
        cli_union
    '
}

mark_op_executed() {
    local op_id=$1
    local path
    path=$(op_record_path "$op_id")
    if [ ! -f "$path" ]; then
        return 0
    fi
    local tmp
    tmp=$(mktemp)
    jq '.executed = true' "$path" > "$tmp" && mv "$tmp" "$path"
}

resolve_oracle_args_for() {
    local view_fn=$1 target=$2 function=$3 key_json=$4 payload=$5
    local gov resolved tx_xdr key_file
    gov=$(get_governance)
    key_file=$(mktemp)
    printf '%s' "$key_json" > "$key_file"
    case "$view_fn" in
        resolve_asset_oracle)
            local oracle_file oracle_file2
            oracle_file=$(mktemp)
            printf '%s' "$payload" > "$oracle_file"
            resolved=$(stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
                --send=no -- resolve_asset_oracle \
                --key-file-path "$key_file" --oracle-file-path "$oracle_file")
            rm -f "$oracle_file"
            oracle_file2=$(mktemp)
            printf '%s' "$resolved" > "$oracle_file2"

            tx_xdr=$(stellar contract invoke --id "$target" $SOURCE_FLAG --network "$NETWORK" \
                --build-only --send=no -- "$function" \
                --key-file-path "$key_file" --oracle-file-path "$oracle_file2")
            rm -f "$oracle_file2"
            printf '%s' "$tx_xdr" | stellar tx decode \
                | jq -c 'first(.. | objects | select(has("invoke_contract")) | .invoke_contract.args)'
            ;;
        resolve_oracle_tolerance)
            resolved=$(stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
                --send=no -- resolve_oracle_tolerance --tolerance "$payload")
            local tol_file
            tol_file=$(mktemp)
            printf '%s' "$resolved" > "$tol_file"
            tx_xdr=$(stellar contract invoke --id "$target" $SOURCE_FLAG --network "$NETWORK" \
                --build-only --send=no -- "$function" \
                --key-file-path "$key_file" --tolerance-file-path "$tol_file")
            rm -f "$tol_file"
            printf '%s' "$tx_xdr" | stellar tx decode \
                | jq -c 'first(.. | objects | select(has("invoke_contract")) | .invoke_contract.args)'
            ;;
        *)
            rm -f "$key_file"
            echo "ERROR: unknown oracle resolve view '${view_fn}'." >&2
            exit 1
            ;;
    esac
    rm -f "$key_file"
}

resolve_oracle_op_args() {
    local path=$1
    local target function view_fn key_json
    target=$(jq -r '.target' "$path")
    function=$(jq -r '.function' "$path")
    view_fn=$(jq -r '.resolve.view_fn' "$path")

    key_json=$(jq -c '.resolve.args.key // (if .resolve.args.asset then {Token:.resolve.args.asset} else empty end)' "$path")
    if [ -z "$key_json" ] || [ "$key_json" = "null" ]; then
        echo "ERROR: oracle op record ${path} missing resolve.args.key (and asset)." >&2
        exit 1
    fi
    case "$view_fn" in
        resolve_asset_oracle)
            resolve_oracle_args_for "$view_fn" "$target" "$function" "$key_json" \
                "$(jq -c '.resolve.args.oracle // .resolve.args.cfg' "$path")"
            ;;
        resolve_oracle_tolerance)
            resolve_oracle_args_for "$view_fn" "$target" "$function" "$key_json" \
                "$(jq -r '.resolve.args.tolerance' "$path")"
            ;;
        *)
            echo "ERROR: unknown oracle resolve view '${view_fn}' in ${path}." >&2
            exit 1
            ;;
    esac
}

parse_op_id() {
    printf '%s' "$1" | tail -n1 | tr -d '"' | tr -d '[:space:]'
}

parse_returned_u32() {
    printf '%s\n' "$1" | tail -n1 | tr -d '"' | grep -oE '[0-9]+' | tail -n1
}

RPC_RETRYABLE_RE='TxBadSeq|error sending request|tcp connect error|client error \(Connect\)|Connection refused|connection closed before message completed|dns error'
STELLAR_TX_MAX_RETRIES=${STELLAR_TX_MAX_RETRIES:-4}
STELLAR_TX_RETRY_DELAY=${STELLAR_TX_RETRY_DELAY:-4}

retry_tx() {
    local attempt=1 out rc errfile
    errfile=$(mktemp)
    while :; do

        out=$("$@" 2>"$errfile") && rc=0 || rc=$?
        if [ "$rc" -eq 0 ]; then
            cat "$errfile" >&2
            rm -f "$errfile"
            printf '%s' "$out"
            return 0
        fi
        if [ "$attempt" -lt "$STELLAR_TX_MAX_RETRIES" ] && grep -qiE "$RPC_RETRYABLE_RE" "$errfile"; then
            echo "  transient RPC error (attempt ${attempt}/${STELLAR_TX_MAX_RETRIES}); retrying in ${STELLAR_TX_RETRY_DELAY}s..." >&2
            sed 's/^/    | /' "$errfile" >&2
            attempt=$(( attempt + 1 ))
            sleep "$STELLAR_TX_RETRY_DELAY"
            continue
        fi
        cat "$errfile" >&2
        rm -f "$errfile"
        return "$rc"
    done
}

precomputed_op_id() {
    local target=$1
    local function=$2
    local args_json=$3
    local salt_hex=$4
    local gov args_file op_id
    gov=$(get_governance)
    args_file=$(mktemp)
    printf '%s' "$args_json" > "$args_file"
    op_id=$(stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" --send=no \
        -- hash_operation \
        --target "$target" \
        --function "$function" \
        --args-file-path "$args_file" \
        --predecessor "$ZERO_PREDECESSOR_HEX" \
        --salt "$salt_hex" 2>/dev/null | tail -n1 | tr -d '"' | tr -d '[:space:]')
    rm -f "$args_file"
    echo "$op_id"
}

salt_generation() {
    local base=$1
    local n=$2
    if [ "$n" -eq 0 ]; then
        echo "$base"
        return 0
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        printf '%s|gen:%s' "$base" "$n" | sha256sum | cut -c1-64
    else
        printf '%s|gen:%s' "$base" "$n" | shasum -a 256 | cut -c1-64
    fi
}

MAX_SALT_GENERATIONS=${MAX_SALT_GENERATIONS:-16}

probe_salt_generations() {
    local target=$1
    local fn=$2
    local args=$3
    local base=$4
    local n=0 salt id state
    while [ "$n" -le "$MAX_SALT_GENERATIONS" ]; do
        salt=$(salt_generation "$base" "$n")
        id=$(precomputed_op_id "$target" "$fn" "$args" "$salt")
        if [ -z "$id" ]; then
            printf '%s %s %s %s\n' "$base" "-" "Unknown" 0
            return 0
        fi
        state=$(op_state "$id" 2>/dev/null) || state="Unknown"
        if [ "$state" != "Done" ]; then
            printf '%s %s %s %s\n' "$salt" "$id" "$state" "$n"
            return 0
        fi
        n=$(( n + 1 ))
    done
    printf '%s %s %s %s\n' "$base" "-" "Exhausted" "$n"
}

schedule_via_proposer() {
    local controller_fn=$1; shift
    local admin_op_json=$1; shift
    local args_json=$1; shift
    local cli_executable=$1; shift
    local salt_hex=$1; shift
    local reapply=${1:-auto}; shift || true
    local gov
    gov=$(get_governance)
    local proposer
    proposer=$(get_signer_address)

    local ctrl salt_use known_id state gen
    ctrl=$(get_controller)
    read -r salt_use known_id state gen < <(probe_salt_generations "$ctrl" "$controller_fn" "$args_json" "$salt_hex")
    case "$state" in
        Ready|Waiting)
            echo "Op ${known_id} (${controller_fn}) already ${state}; reusing it instead of re-proposing." >&2

            write_op_record "$known_id" "$controller_fn" "$args_json" "$salt_use" "$cli_executable"
            echo "$known_id"
            return 0
            ;;
        Exhausted)
            die "${controller_fn}: all ${MAX_SALT_GENERATIONS} salt generations already executed for these args; re-run with a fresh SALT_NONCE=<n>"
            ;;
        Unset)
            if [ "$gen" -gt 0 ]; then
                if [ "$reapply" = "never" ] || [ "${REAPPLY_ON_DONE:-1}" != "1" ]; then
                    local done_id
                    done_id=$(precomputed_op_id "$ctrl" "$controller_fn" "$args_json" "$salt_hex")
                    echo "Op ${done_id} (${controller_fn}) already executed with these exact args; skipping propose (converge mode)." >&2
                    write_op_record "$done_id" "$controller_fn" "$args_json" "$salt_hex" "$cli_executable"
                    mark_op_executed "$done_id"
                    echo "$done_id"
                    return 0
                fi
                echo "Op (${controller_fn}) with these exact args already executed; RE-APPLYING as generation ${gen}." >&2
                salt_hex=$salt_use
            fi
            ;;
        *)

            ;;
    esac

    echo "Scheduling ${controller_fn} via propose (salt ${salt_hex})..." >&2
    local op_file
    op_file=$(mktemp)
    printf '%s' "$admin_op_json" > "$op_file"
    local out
    out=$(retry_tx stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
        -- propose \
        --proposer "$proposer" \
        --op-file-path "$op_file" \
        --salt "$salt_hex")
    rm -f "$op_file"

    local op_id
    op_id=$(parse_op_id "$out")
    if [ -z "$op_id" ]; then
        echo "ERROR: propose ${controller_fn} returned no operation id (output: $out)" >&2
        exit 1
    fi
    write_op_record "$op_id" "$controller_fn" "$args_json" "$salt_hex" "$cli_executable"
    echo "Scheduled op ${op_id} (function ${controller_fn})." >&2
    echo "$op_id"
}

schedule_via_gov_self_proposer() {
    local execute_label=$1; shift
    local admin_op_json=$1; shift
    local salt_hex=$1; shift
    local gov_fn=${1:-}; shift || true
    local gov_args=${1:-}; shift || true
    local gov
    gov=$(get_governance)
    local proposer
    proposer=$(get_signer_address)

    if [ -n "$gov_fn" ] && [ -n "$gov_args" ]; then
        local salt_use known_id state gen
        read -r salt_use known_id state gen < <(probe_salt_generations "$gov" "$gov_fn" "$gov_args" "$salt_hex")
        case "$state" in
            Ready|Waiting)
                echo "Governance-self op ${known_id} (${execute_label}) already ${state}; reusing it instead of re-proposing." >&2
                write_gov_self_op_record "$known_id" "$execute_label" "$admin_op_json" "$salt_use" true
                echo "$known_id"
                return 0
                ;;
            Exhausted)
                die "${execute_label}: all ${MAX_SALT_GENERATIONS} salt generations already executed for these args; re-run with a fresh SALT_NONCE=<n>"
                ;;
            Unset)
                if [ "$gen" -gt 0 ]; then
                    if [ "${REAPPLY_ON_DONE:-1}" != "1" ]; then
                        local done_id
                        done_id=$(precomputed_op_id "$gov" "$gov_fn" "$gov_args" "$salt_hex")
                        echo "Governance-self op ${done_id} (${execute_label}) already executed with these exact args; skipping propose (converge mode)." >&2
                        write_gov_self_op_record "$done_id" "$execute_label" "$admin_op_json" "$salt_hex" true
                        mark_op_executed "$done_id"
                        echo "$done_id"
                        return 0
                    fi
                    echo "Governance-self op (${execute_label}) with these exact args already executed; RE-APPLYING as generation ${gen}." >&2
                    salt_hex=$salt_use
                fi
                ;;
            *) ;;
        esac
    fi

    echo "Scheduling governance-self ${execute_label} via propose (salt ${salt_hex})..." >&2
    local op_file
    op_file=$(mktemp)
    printf '%s' "$admin_op_json" > "$op_file"
    local out
    out=$(retry_tx stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
        -- propose \
        --proposer "$proposer" \
        --op-file-path "$op_file" \
        --salt "$salt_hex")
    rm -f "$op_file"

    local op_id
    op_id=$(parse_op_id "$out")
    if [ -z "$op_id" ]; then
        echo "ERROR: propose ${execute_label} returned no operation id (output: $out)" >&2
        exit 1
    fi
    write_gov_self_op_record "$op_id" "$execute_label" "$admin_op_json" "$salt_hex" true
    echo "Scheduled governance-self op ${op_id} (${execute_label})." >&2
    echo "$op_id"
}

current_ledger_sequence() {
    stellar ledger latest --network "$NETWORK" 2>/dev/null \
        | awk -F': ' '/^Sequence:/ {print $2; exit}'
}

min_delay_ledgers() {
    local gov min_delay
    gov=$(get_governance)
    min_delay=$(stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" --send=no \
        -- get_min_delay | tr -d '"' | tr -d '[:space:]')
    if [ -z "$min_delay" ] || [ "$min_delay" = "null" ]; then
        echo "0"
        return
    fi
    echo "$min_delay"
}

await_max_wait_seconds() {
    if [ "${AWAIT_MAX_WAIT_SECONDS:-0}" -gt 0 ]; then
        echo "$AWAIT_MAX_WAIT_SECONDS"
        return
    fi
    local delay
    delay=$(min_delay_ledgers)

    echo $(( delay * 6 + 7200 ))
}

op_ready_ledger() {
    local op_id=$1
    local gov
    gov=$(get_governance)
    stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" --send=no \
        -- get_operation_ledger --operation_id "$op_id" | tr -d '"' | tr -d '[:space:]'
}

op_state() {
    local op_id=$1
    local gov state
    gov=$(get_governance)
    state=$(stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" --send=no \
        -- get_operation_state --operation_id "$op_id" | tr -d '"' | tr -d '[:space:]')
    if [ "$state" = "Unset" ]; then
        local path
        path=$(op_record_path "$op_id")
        if [ -f "$path" ] && [ "$(jq -r '.executed // false' "$path")" = "true" ]; then
            echo "Done"
            return 0
        fi
    fi
    echo "$state"
}

await_op_ready() {
    local op_id=$1
    local started_at ready_ledger current state max_wait waited unset_seen sleep_s
    started_at=$(date +%s)
    max_wait=$(await_max_wait_seconds)
    unset_seen=0

    while true; do
        state=$(op_state "$op_id")
        case "$state" in
            Ready) echo "Op ${op_id} is Ready." >&2; return 0 ;;
            Done)  echo "Op ${op_id} already Done." >&2; return 0 ;;
            Waiting)
                ready_ledger=$(op_ready_ledger "$op_id")
                current=$(current_ledger_sequence)
                if [ -n "$ready_ledger" ] && [ "$ready_ledger" != "0" ] && [ "$ready_ledger" != "1" ] \
                    && [ -n "$current" ] && [ "$current" -ge "$ready_ledger" ]; then
                    state=$(op_state "$op_id")
                    if [ "$state" = "Ready" ] || [ "$state" = "Done" ]; then
                        echo "Op ${op_id} is ${state} (ledger ${current} >= ${ready_ledger})." >&2
                        return 0
                    fi
                fi
                waited=$(( $(date +%s) - started_at ))
                if [ "$waited" -ge "$max_wait" ]; then
                    echo "ERROR: op ${op_id} did not reach Ready within ${max_wait}s (ready_ledger=${ready_ledger}, current=${current})." >&2
                    echo "       Re-run: NETWORK=$NETWORK $0 awaitOp ${op_id} && $0 executeOp ${op_id}" >&2
                    exit 1
                fi

                sleep_s=$AWAIT_POLL_SECONDS
                if [ -n "$ready_ledger" ] && [ -n "$current" ] && [ "$ready_ledger" -gt "$current" ] 2>/dev/null; then
                    sleep_s=$(( (ready_ledger - current) * 6 / 2 ))
                    if [ "$sleep_s" -lt "$AWAIT_POLL_SECONDS" ]; then sleep_s=$AWAIT_POLL_SECONDS; fi
                    if [ "$sleep_s" -gt 600 ]; then sleep_s=600; fi
                fi
                echo "  Op ${op_id} Waiting (ledger ${current:-?}/${ready_ledger:-?}, waited ${waited}s/${max_wait}s); sleeping ${sleep_s}s..." >&2
                sleep "$sleep_s"
                ;;
            Unset)

                unset_seen=$(( unset_seen + 1 ))
                if [ "$unset_seen" -ge "${UNSET_MAX_POLLS:-6}" ]; then
                    echo "ERROR: op ${op_id} is Unset (never scheduled or cancelled) after ${unset_seen} polls." >&2
                    exit 1
                fi
                echo "  Op ${op_id} read Unset (RPC lag?); retry ${unset_seen}/${UNSET_MAX_POLLS:-6}, sleeping ${AWAIT_POLL_SECONDS}s..." >&2
                sleep "$AWAIT_POLL_SECONDS"
                ;;
            *) echo "ERROR: unexpected op state '${state}' for ${op_id}." >&2; exit 1 ;;
        esac
    done
}

execute_gov_self_op() {
    local op_id=$1
    local path
    path=$(op_record_path "$op_id")
    local gov execute_label salt
    gov=$(get_governance)
    execute_label=$(jq -r '.execute_label' "$path")
    salt=$(jq -r '.salt' "$path")
    echo "Executing governance-self op ${op_id} -> ${execute_label}..." >&2
    local op_file
    op_file=$(mktemp)
    jq -c '.op' "$path" > "$op_file"

    retry_tx stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
        -- execute_self \
        --executor null \
        --op-file-path "$op_file" \
        --salt "$salt"
    rm -f "$op_file"
    mark_op_executed "$op_id"
    echo "Executed governance-self op ${op_id}." >&2
}

execute_op() {
    local op_id=$1
    local path
    path=$(op_record_path "$op_id")
    if [ ! -f "$path" ]; then
        echo "ERROR: no op record for ${op_id} at ${path}." >&2
        echo "       executeOp replays a locally-scheduled op; schedule it on this host first." >&2
        exit 1
    fi
    local cli_executable
    cli_executable=$(jq -r '.cli_executable' "$path")
    if [ "$cli_executable" != "true" ]; then
        echo "ERROR: op ${op_id} is not CLI-executable." >&2
        echo "       Execute it via the typed SDK/keeper path." >&2
        exit 1
    fi

    local kind
    kind=$(jq -r '.kind // "controller"' "$path")
    if [ "$kind" = "governance_self" ]; then
        execute_gov_self_op "$op_id"
        return 0
    fi

    local gov target function predecessor salt args_json
    gov=$(get_governance)
    target=$(jq -r '.target' "$path")
    function=$(jq -r '.function' "$path")
    predecessor=$(jq -r '.predecessor' "$path")
    salt=$(jq -r '.salt' "$path")

    if [ "$(jq -r 'has("resolve")' "$path")" = "true" ]; then
        args_json=$(resolve_oracle_op_args "$path")
        if [ -z "$args_json" ] || [ "$args_json" = "null" ]; then
            echo "ERROR: failed to resolve oracle op ${op_id} args via the governance view." >&2
            exit 1
        fi
    else
        args_json=$(jq -c '.args' "$path")
    fi
    echo "Executing op ${op_id} -> ${function} on ${target}..." >&2
    local args_file
    args_file=$(mktemp)
    printf '%s' "$args_json" > "$args_file"

    retry_tx stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
        -- execute \
        --executor null \
        --target "$target" \
        --function "$function" \
        --args-file-path "$args_file" \
        --predecessor "$predecessor" \
        --salt "$salt"
    rm -f "$args_file"
    mark_op_executed "$op_id"
    echo "Executed op ${op_id}." >&2
}

cancel_op() {
    local op_id=$1
    local gov
    gov=$(get_governance)
    local canceller
    canceller=$(get_signer_address)
    stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
        -- cancel \
        --canceller "$canceller" \
        --operation_id "$op_id"
    rm -f "$(op_record_path "$op_id")"
    echo "Cancelled op ${op_id}." >&2
}

list_ops() {
    local dir="$OPS_DIR"
    if [ ! -d "$dir" ] || ! ls "$dir"/*.json >/dev/null 2>&1; then
        echo "No recorded ops for ${NETWORK} under ${dir}." >&2
        return 0
    fi
    echo "=== Governance ops (${NETWORK}) — records in ${dir} ===" >&2
    local path op_id kind label state ready
    local n_ready=0 n_waiting=0 n_done=0 n_other=0
    for path in "$dir"/*.json; do
        op_id=$(jq -r '.op_id' "$path")
        kind=$(jq -r '.kind // "controller"' "$path")
        label=$(jq -r '.function // .execute_label // "?"' "$path")
        state=$(op_state "$op_id" 2>/dev/null) || state="unknown"
        ready="-"
        case "$state" in
            Ready)   n_ready=$(( n_ready + 1 )) ;;
            Waiting) n_waiting=$(( n_waiting + 1 )); ready=$(op_ready_ledger "$op_id" 2>/dev/null || echo "?") ;;
            Done)    n_done=$(( n_done + 1 )) ;;
            *)       n_other=$(( n_other + 1 )) ;;
        esac
        printf '%-8s %-16s %-32s ready_ledger=%-10s %s\n' "$state" "$kind" "$label" "$ready" "$op_id"
    done
    echo "--- ${n_ready} Ready, ${n_waiting} Waiting, ${n_done} Done, ${n_other} other ---" >&2
    if [ "$n_ready" -gt 0 ]; then
        echo "Run 'executeReady' to execute all Ready ops." >&2
    fi
}

execute_ready_ops() {
    local dir="$OPS_DIR"
    if [ ! -d "$dir" ] || ! ls "$dir"/*.json >/dev/null 2>&1; then
        echo "No recorded ops for ${NETWORK} under ${dir}." >&2
        return 0
    fi
    local path op_id state any=0
    for path in "$dir"/*.json; do
        op_id=$(jq -r '.op_id' "$path")
        state=$(op_state "$op_id" 2>/dev/null) || continue
        if [ "$state" = "Ready" ]; then
            any=1
            execute_op "$op_id"
        fi
    done
    if [ "$any" -eq 0 ]; then
        echo "No Ready ops for ${NETWORK}." >&2
    fi
}

schedule_and_maybe_execute() {
    local op_id=$1
    if [ "${AUTO_EXECUTE:-1}" != "1" ]; then
        echo "Scheduled op ${op_id} (AUTO_EXECUTE=0; run 'executeOp ${op_id}' after the delay)." >&2
        return 0
    fi
    if [ "$(op_state "$op_id")" = "Done" ]; then
        echo "Op ${op_id} already executed; skipping." >&2
        return 0
    fi
    await_op_ready "$op_id"
    execute_op "$op_id"
}

require_static_config

validate_configs() {
    local errors=0 warnings=0
    vc_err() { echo "ERROR: $*" >&2; errors=$(( errors + 1 )); }
    vc_warn() { echo "WARN:  $*" >&2; warnings=$(( warnings + 1 )); }

    echo "=== Validating ${NETWORK} configs ===" >&2

    local f v
    for f in rpc_url network_passphrase timelock_min_delay_ledgers; do
        v=$(get_network_value "$f")
        if [ -z "$v" ] || [ "$v" = "null" ]; then
            vc_err "networks.json ${NETWORK}.${f} missing"
        fi
    done

    local cex dex fx redstone xoxno_adapter
    cex=$(get_cex_oracle)
    dex=$(get_dex_oracle)
    fx=$(get_fx_oracle)
    redstone=$(get_redstone_adapter)
    xoxno_adapter=$(get_xoxno_oracle_adapter)

    local dup
    dup=$(jq -r '[.markets[].name] | group_by(.) | map(select(length > 1) | .[0]) | join(", ")' "$MARKET_CONFIG_FILE")
    if [ -n "$dup" ]; then
        vc_err "duplicate market names: ${dup}"
    fi
    dup=$(jq -r '[.markets[] | "\(.hub_id)/\(.asset_address)"] | group_by(.) | map(select(length > 1) | .[0]) | join(", ")' "$MARKET_CONFIG_FILE")
    if [ -n "$dup" ]; then
        vc_err "duplicate (hub_id, asset_address) pairs: ${dup}"
    fi

    local oracle_bot_heartbeat_seconds=3600
    local max_leg_age_spread_seconds=3600
    local m mj hub addr missing o strat anchor_tag minw maxw ptag pcontract atag acontract pstale astale
    for m in $(jq -r '.markets[].name' "$MARKET_CONFIG_FILE"); do
        mj=$(jq -c --arg m "$m" 'first(.markets[] | select(.name == $m))' "$MARKET_CONFIG_FILE")

        hub=$(printf '%s' "$mj" | jq -r '.hub_id // "missing"')
        case "$hub" in
            missing|null) vc_err "market ${m}: missing hub_id" ;;
            0) vc_err "market ${m}: hub_id 0 (there is no hub 0)" ;;
        esac

        addr=$(printf '%s' "$mj" | jq -r '.asset_address // ""')
        if ! printf '%s' "$addr" | grep -qE '^C[A-Z2-7]{55}$'; then
            vc_err "market ${m}: asset_address '${addr}' is not a contract strkey"
        fi

        missing=$(printf '%s' "$mj" | jq -r '[(.market_params // {}) |
            {max_borrow_rate, base_borrow_rate, slope1, slope2, slope3, mid_utilization,
             optimal_utilization, max_utilization, reserve_factor}
            | to_entries[] | select(.value == null) | .key] | join(", ")')
        if [ -n "$missing" ]; then
            vc_err "market ${m}: market_params missing ${missing}"
        fi
        if ! printf '%s' "$mj" | jq -e '
            (.market_params // {}) as $p |
            (($p.mid_utilization // "0") | tonumber) < (($p.optimal_utilization // "0") | tonumber) and
            (($p.optimal_utilization // "0") | tonumber) < (($p.max_utilization // "0") | tonumber) and
            (($p.max_utilization // "0") | tonumber) <= 1e27' >/dev/null; then
            vc_err "market ${m}: utilization must satisfy mid < optimal < max <= RAY (1e27)"
        fi
        if ! printf '%s' "$mj" | jq -e '(.market_params.reserve_factor // 99999) <= 10000' >/dev/null; then
            vc_err "market ${m}: reserve_factor out of [0, 10000] bps"
        fi

        if ! printf '%s' "$mj" | jq -e '(.market_params.flashloan_fee // 0) <= 500' >/dev/null; then
            vc_err "market ${m}: flashloan_fee > 500 bps (MAX_FLASHLOAN_FEE_BPS)"
        fi
        if ! printf '%s' "$mj" | jq -e '
            (.market_params.is_flashloanable | type) == "boolean"' >/dev/null; then
            vc_err "market ${m}: market_params.is_flashloanable must be a boolean"
        fi

        o=$(printf '%s' "$mj" | jq -c '.oracle // {}')
        if ! printf '%s' "$o" | jq -e '(.sources | type) == "array" and ((.sources | length) == 1 or (.sources | length) == 2)' >/dev/null; then
            vc_err "market ${m}: oracle.sources must be an array of length 1 or 2"
        fi
        if printf '%s' "$o" | jq -e 'has("primary") or has("anchor") or has("strategy") or has("tolerance_bps")' >/dev/null; then
            vc_err "market ${m}: oracle still uses legacy primary/anchor/strategy/tolerance_bps fields"
        fi
        if ! printf '%s' "$o" | jq -e '
            any(.sources[]?; has("AquariusLp") or has("AquariusStableLp")) or (
                (.tolerance.upper_ratio_bps // 0) > 10000 and
                (.tolerance.lower_ratio_bps // 99999) < 10000 and
                (.tolerance.lower_ratio_bps // 0) >= 1
            )' >/dev/null; then
            vc_err "market ${m}: oracle.tolerance must be a reciprocal band around 10000 bps"
        fi
        if ! printf '%s' "$o" | jq -e '
            .independence == "RequireDisjoint" or
            (.independence | type) == "object"' >/dev/null; then
            vc_err "market ${m}: oracle.independence missing or invalid"
        fi
        # engine.rs blends two legs at their midpoint and marks the result stale
        # once two market legs sit further apart in time than the spread bound.
        # A market leg may therefore never declare a staleness budget wider than
        # that bound when its partner is also market-nature.
        if printf '%s' "$o" | jq -e --argjson cap "$max_leg_age_spread_seconds" '
            (.sources | length) == 2 and
            ([.sources[] | (.Feed // .Scaled.factor) | select(. != null) |
                if (.provider | has("Reflector")) then "Market"
                else (.provider.RedStone // .provider.Xoxno).nature end
             ] | length == 2 and all(. == "Market")) and
            ([.sources[] | (.Feed // .Scaled.factor).max_stale_seconds] | any(. > $cap))
        ' >/dev/null; then
            vc_err "market ${m}: both legs are market-nature, so neither may declare max_stale_seconds above the ${max_leg_age_spread_seconds}s leg-spread bound"
        fi
        ceiling=$(printf '%s' "$o" | jq -r '.max_price_stale_seconds // "missing"')
        if [ "$ceiling" = "missing" ]; then
            vc_err "market ${m}: oracle.max_price_stale_seconds missing"
        fi
        minw=$(printf '%s' "$o" | jq -r '.min_sanity_price_wad // "missing"')
        maxw=$(printf '%s' "$o" | jq -r '.max_sanity_price_wad // "missing"')
        if [ "$minw" = "missing" ] || [ "$maxw" = "missing" ]; then
            vc_err "market ${m}: oracle missing min/max_sanity_price_wad"
        elif [ "$minw" = "0" ] && [ "$maxw" = "0" ]; then
            if [ "$NETWORK" = "mainnet" ]; then
                vc_err "market ${m}: (0,0) sanity-bound sentinel not allowed on mainnet"
            else
                vc_warn "market ${m}: (0,0) sanity-bound sentinel (test-only)"
            fi
        elif ! jq -ne --arg a "$minw" --arg b "$maxw" '($a | tonumber) < ($b | tonumber)' >/dev/null; then
            vc_err "market ${m}: min_sanity_price_wad >= max_sanity_price_wad"
        fi

        local nsrc i sjson pkind pcontract fstale
        nsrc=$(printf '%s' "$o" | jq -r '.sources | length')
        i=0
        while [ "$i" -lt "$nsrc" ]; do
            sjson=$(printf '%s' "$o" | jq -c --argjson i "$i" '.sources[$i]')
            if printf '%s' "$sjson" | jq -e 'has("Feed")' >/dev/null; then
                fstale=$(printf '%s' "$sjson" | jq -r '.Feed.max_stale_seconds // "missing"')
                pkind=$(printf '%s' "$sjson" | jq -r '
                    if .Feed.provider | has("Reflector") then "Reflector"
                    elif .Feed.provider | has("RedStone") then "RedStone"
                    elif .Feed.provider | has("Xoxno") then "Xoxno"
                    else "unknown" end')
                pcontract=$(printf '%s' "$sjson" | jq -r '
                    if .Feed.provider | has("Reflector") then .Feed.provider.Reflector.contract
                    elif .Feed.provider | has("RedStone") then .Feed.provider.RedStone.contract
                    elif .Feed.provider | has("Xoxno") then .Feed.provider.Xoxno.contract
                    else "" end')
            elif printf '%s' "$sjson" | jq -e 'has("Scaled")' >/dev/null; then
                fstale=$(printf '%s' "$sjson" | jq -r '.Scaled.factor.max_stale_seconds // "missing"')
                pkind=$(printf '%s' "$sjson" | jq -r '
                    if .Scaled.factor.provider | has("Reflector") then "Reflector"
                    elif .Scaled.factor.provider | has("RedStone") then "RedStone"
                    elif .Scaled.factor.provider | has("Xoxno") then "Xoxno"
                    else "unknown" end')
                pcontract=$(printf '%s' "$sjson" | jq -r '
                    if .Scaled.factor.provider | has("Reflector") then .Scaled.factor.provider.Reflector.contract
                    elif .Scaled.factor.provider | has("RedStone") then .Scaled.factor.provider.RedStone.contract
                    elif .Scaled.factor.provider | has("Xoxno") then .Scaled.factor.provider.Xoxno.contract
                    else "" end')
                local quote_ref quote_token
                quote_ref=$(printf '%s' "$sjson" | jq -r '.Scaled.quote.Ref // empty')
                quote_token=$(printf '%s' "$sjson" | jq -r '.Scaled.quote.Token // empty')
                if [ -n "$quote_ref" ]; then
                    if ! jq -e --arg n "$quote_ref" '
                        any(.references[]?; (.name == $n) or (.key.Ref == $n))
                    ' "$MARKET_CONFIG_FILE" >/dev/null; then
                        vc_err "market ${m}: sources[$i] Scaled quote Ref ${quote_ref} not in markets.json references"
                    fi
                elif [ -n "$quote_token" ]; then
                    if ! jq -e --arg a "$quote_token" '
                        any(.markets[]?; .asset_address == $a)
                    ' "$MARKET_CONFIG_FILE" >/dev/null; then
                        vc_err "market ${m}: sources[$i] Scaled quote Token ${quote_token} not in markets.json"
                    fi
                else
                    vc_err "market ${m}: sources[$i] Scaled missing quote Ref or Token"
                fi
                if ! printf '%s' "$sjson" | jq -e '
                    (.Scaled.min_factor_wad | tonumber) > 0 and
                    (.Scaled.max_factor_wad | tonumber) > (.Scaled.min_factor_wad | tonumber)
                ' >/dev/null; then
                    vc_err "market ${m}: sources[$i] Scaled min/max_factor_wad invalid"
                fi
            elif printf '%s' "$sjson" | jq -e 'has("AquariusLp") or has("AquariusStableLp")' >/dev/null; then
                if ! printf '%s' "$sjson" | jq -e '
                    (.AquariusLp // .AquariusStableLp) as $lp |
                    $lp.pool and
                    ($lp.token_a | type == "string") and
                    ($lp.token_b | type == "string") and
                    ($lp.token_a != $lp.token_b) and
                    ($lp.key_a != $lp.key_b) and
                    (($lp.key_a.Ref != null) or
                     ($lp.key_a.Token == $lp.token_a)) and
                    (($lp.key_b.Ref != null) or
                     ($lp.key_b.Token == $lp.token_b)) and
                    ($lp.reserve_a_decimals | type == "number") and
                    ($lp.reserve_b_decimals | type == "number") and
                    ($lp.min_pool_value_wad | tonumber) > 0
                ' >/dev/null; then
                    vc_err "market ${m}: sources[$i] Aquarius LP has invalid token bindings or liquidity floor"
                fi
                i=$((i + 1))
                continue
            else
                vc_err "market ${m}: sources[$i] must be Feed, Scaled, AquariusLp, or AquariusStableLp"
                i=$((i + 1))
                continue
            fi
            if [ "$fstale" = "missing" ]; then
                vc_err "market ${m}: sources[$i] missing max_stale_seconds"
            elif [ "$fstale" -gt "$ceiling" ]; then
                vc_err "market ${m}: sources[$i] max_stale_seconds ${fstale} > asset ceiling ${ceiling}"
            fi
            case "$pkind" in
                Reflector)
                    case "$pcontract" in
                        "$cex"|"$dex"|"$fx"|"") ;;
                        *) vc_warn "market ${m}: sources[$i] Reflector ${pcontract} is none of networks.json cex/dex/fx oracles" ;;
                    esac
                    if [ -n "$dex" ] && [ "$pcontract" = "$dex" ] &&
                       printf '%s' "$sjson" | jq -e 'has("Feed")' >/dev/null; then
                        vc_err "market ${m}: sources[$i] reads the Reflector DEX oracle as a bare Feed; its base is USDC, not USD, so attest rejects it — wrap it in Scaled with quote Token(USDC)"
                    fi
                    ;;
                RedStone)
                    if [ -n "$pcontract" ] && [ "$pcontract" != "$redstone" ]; then
                        vc_warn "market ${m}: sources[$i] RedStone contract differs from networks.json redstone_adapter_contract"
                    fi
                    local fnature
                    fnature=$(printf '%s' "$sjson" | jq -r '
                        ((.Feed // .Scaled.factor).provider.RedStone.nature) // "unknown"')
                    # Only a fundamental leg needs room for missed heartbeats. A
                    # market leg is bounded from above by the leg-spread rule, and
                    # the two floors are not simultaneously satisfiable.
                    if [ "$fnature" != "Market" ] && [ "$fstale" != "missing" ] &&
                       [ "$fstale" -lt $(( oracle_bot_heartbeat_seconds * 4 )) ]; then
                        vc_err "market ${m}: sources[$i] RedStone max_stale_seconds ${fstale} < 4x oracle bot heartbeat (${oracle_bot_heartbeat_seconds}s)"
                    fi
                    ;;
                Xoxno)
                    if [ -n "$pcontract" ] && [ "$pcontract" != "$xoxno_adapter" ]; then
                        vc_warn "market ${m}: sources[$i] Xoxno contract differs from networks.json xoxno_oracle_adapter"
                    fi
                    if [ "$fstale" != "missing" ] && [ "$fstale" -lt $(( oracle_bot_heartbeat_seconds * 4 )) ]; then
                        vc_err "market ${m}: sources[$i] Xoxno max_stale_seconds ${fstale} < 4x oracle bot heartbeat (${oracle_bot_heartbeat_seconds}s)"
                    fi
                    ;;
            esac
            i=$((i + 1))
        done
    done

    local rname ro needed
    for rname in $(jq -r '(.references // [])[].name // empty' "$MARKET_CONFIG_FILE"); do
        ro=$(jq -c --arg n "$rname" '.references[] | select(.name == $n) | .oracle' "$MARKET_CONFIG_FILE")
        if ! printf '%s' "$ro" | jq -e '(.sources | type) == "array" and ((.sources | length) == 1 or (.sources | length) == 2)' >/dev/null; then
            vc_err "reference ${rname}: oracle.sources must be length 1 or 2"
        fi
        if ! printf '%s' "$ro" | jq -e '(.asset_decimals // -1) == 0' >/dev/null; then
            vc_err "reference ${rname}: asset_decimals must be 0 for PriceKey::Ref"
        fi
        if ! printf '%s' "$ro" | jq -e 'has("min_sanity_price_wad") and has("max_sanity_price_wad")' >/dev/null; then
            vc_err "reference ${rname}: missing min/max_sanity_price_wad"
        fi
        if printf '%s' "$ro" | jq -e --argjson cap "$max_leg_age_spread_seconds" '
            (.sources | length) == 2 and
            ([.sources[] | (.Feed // .Scaled.factor) | select(. != null) |
                if (.provider | has("Reflector")) then "Market"
                else (.provider.RedStone // .provider.Xoxno).nature end
             ] | length == 2 and all(. == "Market")) and
            ([.sources[] | (.Feed // .Scaled.factor).max_stale_seconds] | any(. > $cap))
        ' >/dev/null; then
            vc_err "reference ${rname}: both legs are market-nature, so neither may declare max_stale_seconds above the ${max_leg_age_spread_seconds}s leg-spread bound"
        fi
    done
    for needed in $(all_oracle_ref_dependencies); do
        if ! jq -e --arg n "$needed" '
            any(.references[]?; (.name == $n) or (.key.Ref == $n))
        ' "$MARKET_CONFIG_FILE" >/dev/null; then
            vc_err "oracle dependency Ref ${needed} is not listed under .references[]"
        fi
    done
    for rname in $(jq -r '(.references // [])[].name // empty' "$MARKET_CONFIG_FILE"); do
        if ! jq -e --arg n "$rname" '
            any(.markets[]?; any(.oracle.sources[]?;
                (.Scaled.quote.Ref // "") == $n or
                ((.AquariusLp // .AquariusStableLp).key_a.Ref // "") == $n or
                ((.AquariusLp // .AquariusStableLp).key_b.Ref // "") == $n))
        ' "$MARKET_CONFIG_FILE" >/dev/null; then
            vc_warn "reference ${rname} is listed but no market Scaled source quotes it"
        fi
    done

    local first_dex
    first_dex=$(jq -r --arg dex "$dex" '
        first(.markets | to_entries[] |
            select(any(.value.oracle.sources[]?;
                (.Feed.provider.Reflector.contract // "") == $dex)) | .key) // empty
        ' "$MARKET_CONFIG_FILE")
    if [ "$first_dex" = "0" ]; then
        vc_err "first market in ${MARKET_CONFIG_FILE} uses DEX Reflector; its USD quote market must come before it (file order = setup order)"
    elif [ -n "$first_dex" ]; then
        vc_warn "DEX-oracle markets present: each one's USD quote market must appear EARLIER in ${MARKET_CONFIG_FILE} (file order = setup order)"
    fi

    local cat a sj maddr mhub spoke_en asset_en market_en
    for cat in $(jq -r 'keys[]' "$SPOKES_FILE"); do
        spoke_en=$(jq -r --arg c "$cat" '.[$c].enabled != false' "$SPOKES_FILE")
        for a in $(jq -r --arg c "$cat" '.[$c].assets | keys[]' "$SPOKES_FILE"); do
            sj=$(jq -c --arg c "$cat" --arg a "$a" '.[$c].assets[$a]' "$SPOKES_FILE")
            asset_en=$(printf '%s' "$sj" | jq -r '.enabled != false')
            maddr=$(get_market_value "$a" "asset_address")
            if [ -z "$maddr" ] || [ "$maddr" = "null" ]; then
                vc_err "spoke ${cat}: asset '${a}' has no market in ${MARKET_CONFIG_FILE}"
                continue
            fi
            market_en=false
            if is_market_enabled "$a"; then
                market_en=true
            fi
            if [ "$spoke_en" = "true" ] && [ "$asset_en" = "true" ] && [ "$market_en" != "true" ]; then
                vc_err "spoke ${cat}/${a}: enabled for deploy but market ${a} has enabled=false (disable the spoke asset, or enable the market)"
            fi
            mhub=$(get_market_value "$a" "hub_id")
            if ! printf '%s' "$sj" | jq -e --argjson mh "${mhub:-null}" '(.hub_id // null) == $mh' >/dev/null; then
                vc_err "spoke ${cat}/${a}: hub_id $(printf '%s' "$sj" | jq -r '.hub_id // "missing"') != market hub_id ${mhub}"
            fi
            local field
            for field in supply_cap borrow_cap; do
                if ! printf '%s' "$sj" | jq -e --arg f "$field" '
                    (.[$f] != null) and (.[$f] | tostring | test("^[0-9]+$"))' >/dev/null; then
                    vc_err "spoke ${cat}/${a}: ${field} is $(printf '%s' "$sj" | jq -c --arg f "$field" '.[$f]') (need a decimal integer of base units; caps are always enforced and \"0\" accepts nothing on that side)"
                fi
            done
            if ! printf '%s' "$sj" | jq -e '
                (.ltv // 99999) < (.liquidation_threshold // 0) and
                (.liquidation_threshold // 99999) <= 10000 and
                (.liquidation_bonus // 99999) <= 10000 and
                ((.liquidation_threshold // 0) * (10000 + (.liquidation_bonus // 0))) <= 100000000 and
                ((.liquidation_fees // 0) <= 10000)' >/dev/null; then
                vc_err "spoke ${cat}/${a}: risk bounds invalid (need ltv < threshold <= 10000, bonus/fees <= 10000, threshold*(1+bonus) <= 100%)"
            fi
        done
    done

    for m in $(enabled_market_names); do
        if ! jq -e --arg m "$m" '
            [to_entries[] | select(.value.enabled != false) |
             (.value.assets // {}) | to_entries[] |
             select(.value.enabled != false) | .key] | index($m) != null
        ' "$SPOKES_FILE" >/dev/null; then
            vc_warn "market ${m} is not referenced by any enabled spoke asset (deploys pending; unusable until listed)"
        fi
    done

    echo "=== Validation: ${errors} error(s), ${warnings} warning(s) ===" >&2
    if [ "$errors" -gt 0 ]; then
        exit 1
    fi
    return 0
}

list_markets() {
    echo "Available markets (${NETWORK}):"
    if [ -f "$MARKET_CONFIG_FILE" ]; then
        jq -r '
            .markets[] |
            "  \(.name) — \(.asset_address // "no address")\(if .enabled == false then " [enabled=false, skipped by setupAll*]" else "" end)"
        ' "$MARKET_CONFIG_FILE"
    else
        echo "  No config file found: $MARKET_CONFIG_FILE"
    fi
}

list_spokes() {
    echo "Spoke categories (${NETWORK}):"
    if [ -f "$SPOKES_FILE" ]; then
        jq -r --arg network "$NETWORK" --slurpfile networks "$NETWORKS_FILE" '
            . as $cats |
            ($networks[0][$network].spoke_ids // {}) as $ids |
            $cats | to_entries[] |
            (
                (.value.assets // {}) | to_entries |
                map(
                    .key + (if .value.enabled == false then "!" else "" end)
                ) | join(", ")
            ) as $assets |
            "  \(.key) -> on-chain \($ids[.key] // "unmapped"): \(.value.name)\(if .value.enabled == false then " [enabled=false, skipped by setupAll*]" else "" end) — assets: \($assets)"
        ' "$SPOKES_FILE"
        echo "  (asset names ending in ! have enabled=false and are skipped by setupAll*)"
    else
        echo "  No spokes config found: $SPOKES_FILE"
    fi
}

build_hub_assets_json() {
    local assets_json="["
    local first=1

    for market_name in "$@"; do
        local asset_address
        asset_address=$(get_market_value "$market_name" "asset_address")
        if [ -z "$asset_address" ] || [ "$asset_address" = "null" ]; then
            echo "ERROR: Unknown market '${market_name}'" >&2
            list_markets >&2
            exit 1
        fi

        local hub_id
        hub_id=$(get_market_value "$market_name" "hub_id")
        if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
            die "market '${market_name}' missing hub_id"
        fi

        if [ $first -eq 0 ]; then
            assets_json+=","
        fi
        assets_json+="{\"hub_id\":$hub_id,\"asset\":\"$asset_address\"}"
        first=0
    done

    assets_json+="]"
    echo "$assets_json"
}

add_spoke() {
    local category_id=$1

    local name
    name=$(get_spoke_value "$category_id" ".name")

    echo "Adding Spoke category ${category_id}: ${name}" >&2

    local args_json='[]'
    local salt
    salt=$(gen_salt "add_spoke:${category_id}" "$args_json")

    local op_id
    op_id=$(schedule_via_proposer \
        add_spoke "$(admin_op AddSpoke)" "$args_json" true "$salt" never)

    if [ "${AUTO_EXECUTE:-1}" != "1" ]; then
        echo "Scheduled spoke category ${category_id} as op ${op_id} (AUTO_EXECUTE=0)." >&2
        echo "$op_id"
        return 0
    fi

    if [ "$(op_state "$op_id")" = "Done" ]; then
        die "spoke-create op ${op_id} already executed; its returned id cannot be re-read. Record the on-chain id in ${NETWORKS_FILE} spoke_ids manually."
    fi
    await_op_ready "$op_id"

    local result errf
    errf=$(mktemp)
    result=$(execute_op "$op_id" 2>"$errf") || {
        cat "$errf" >&2
        rm -f "$errf"
        die "execute of spoke-create op ${op_id} failed"
    }
    rm -f "$errf"
    local onchain_id
    onchain_id=$(parse_returned_u32 "$result")
    if [ -z "$onchain_id" ]; then
        echo "ERROR: Could not parse on-chain spoke category id from execute result: $result" >&2
        exit 1
    fi

    echo "Spoke category ${category_id} created with on-chain id ${onchain_id}." >&2
    echo "$onchain_id"
}

get_mapped_spoke_id() {
    local config_category_id=$1
    jq -r --arg network "$NETWORK" --arg config_id "$config_category_id" \
        '.[$network].spoke_ids[$config_id] // empty' "$NETWORKS_FILE"
}

persist_spoke_id() {
    local config_category_id=$1
    local onchain_id=$2
    local tmp
    tmp=$(mktemp)
    jq --arg network "$NETWORK" --arg config_id "$config_category_id" --argjson onchain_id "$onchain_id" \
        '.[$network].spoke_ids = (.[$network].spoke_ids // {}) |
         .[$network].spoke_ids[$config_id] = $onchain_id' \
        "$NETWORKS_FILE" > "$tmp" && mv "$tmp" "$NETWORKS_FILE"
}

fetch_spoke_json() {

    local onchain_id=$1
    local ctrl
    ctrl=$(get_controller)
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        --send=no -- get_spoke --spoke_id "$onchain_id"
}

spoke_is_deprecated() {
    local category_json=$1
    printf '%s' "$category_json" | jq -e '.is_deprecated == true' >/dev/null
}

spoke_assets_match_config() {
    local config_category_id=$1
    local category_json=$2

    if ! printf '%s' "$category_json" | jq -e '.assets | type == "object"' >/dev/null 2>&1; then
        echo "WARN: on-chain Spoke category for config ${config_category_id} has no readable .assets map (current contract); cannot fully verify. Proceeding." >&2
        return 0
    fi

    local onchain_assets
    onchain_assets=$(printf '%s' "$category_json" | jq -r '.assets | keys[]')

    [ -z "$onchain_assets" ] && return 0

    local expected_addrs=" "
    local asset_name asset_addr
    for asset_name in $(jq -r ".\"$config_category_id\".assets | keys[]" "$SPOKES_FILE"); do
        asset_addr=$(get_market_value "$asset_name" "asset_address")

        if [ -z "$asset_addr" ] || [ "$asset_addr" = "null" ]; then
            echo "ERROR: spoke config ${config_category_id} lists asset '${asset_name}' missing from the markets file; cannot verify category reuse." >&2
            return 1
        fi
        expected_addrs="${expected_addrs}${asset_addr} "
    done

    local onchain_addr
    for onchain_addr in $onchain_assets; do
        case "$expected_addrs" in
            *" $onchain_addr "*) ;;
            *) return 1 ;;
        esac
    done
    return 0
}

ensure_spoke() {
    local config_category_id=$1
    local mapped_id
    local category_json

    mapped_id=$(get_mapped_spoke_id "$config_category_id")
    if [ -n "$mapped_id" ] && [ "$mapped_id" != "null" ]; then
        if category_json=$(fetch_spoke_json "$mapped_id" 2>/dev/null); then
            if spoke_is_deprecated "$category_json"; then
                echo "Mapped Spoke id ${mapped_id} for config ${config_category_id} is deprecated; creating a replacement."
            elif ! spoke_assets_match_config "$config_category_id" "$category_json"; then
                echo "ERROR: mapped Spoke id ${mapped_id} for config ${config_category_id} holds assets this config does not list." >&2
                echo "       Refusing to apply config ${config_category_id} to an unverified on-chain category; it may be a different category or have live users." >&2
                echo "       Fix the mapping in ${NETWORKS_FILE}, or deprecate the on-chain category, then re-run." >&2
                return 1
            else
                echo "Spoke config ${config_category_id} already mapped to on-chain id ${mapped_id}."
                echo "$mapped_id"
                return 0
            fi
        else
            echo "Mapped Spoke id ${mapped_id} for config ${config_category_id} is not readable; creating a replacement."
        fi
    fi

    if category_json=$(fetch_spoke_json "$config_category_id" 2>/dev/null); then
        if spoke_is_deprecated "$category_json"; then
            echo "On-chain Spoke id ${config_category_id} is deprecated; creating a new category."
        elif ! spoke_assets_match_config "$config_category_id" "$category_json"; then
            echo "ERROR: on-chain Spoke id ${config_category_id} holds assets config category ${config_category_id} does not list." >&2
            echo "       Refusing to reuse it by numeric id; it may be a different category or have live users." >&2
            echo "       Map config ${config_category_id} to the correct on-chain id in ${NETWORKS_FILE}, or deprecate the on-chain category, then re-run." >&2
            return 1
        else
            persist_spoke_id "$config_category_id" "$config_category_id"
            echo "Spoke config ${config_category_id} reuses existing on-chain id ${config_category_id}."
            echo "$config_category_id"
            return 0
        fi
    fi

    local onchain_id
    onchain_id=$(add_spoke "$config_category_id")
    persist_spoke_id "$config_category_id" "$onchain_id"
    echo "$onchain_id"
}

add_asset_to_spoke() {
    local category_id=$1
    local asset_name=$2
    local config_category_id=${3:-$category_id}

    echo "Adding asset ${asset_name} to Spoke category ${category_id}..."

    local asset_address
    asset_address=$(get_market_value "$asset_name" "asset_address")
    local can_collateral
    can_collateral=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".can_be_collateral")
    local can_borrow
    can_borrow=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".can_be_borrowed")
    local ltv
    ltv=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".ltv")
    local threshold
    threshold=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".liquidation_threshold")
    local bonus
    bonus=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".liquidation_bonus")
    local supply_cap
    supply_cap=$(require_spoke_cap "$config_category_id" "$asset_name" supply_cap)
    local borrow_cap
    borrow_cap=$(require_spoke_cap "$config_category_id" "$asset_name" borrow_cap)

    local liquidation_fees
    liquidation_fees=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".liquidation_fees")
    if [ -z "$liquidation_fees" ] || [ "$liquidation_fees" = "null" ]; then
        liquidation_fees=$(get_market_value "$asset_name" "liquidation_fees")
    fi
    if [ -z "$liquidation_fees" ] || [ "$liquidation_fees" = "null" ]; then liquidation_fees=0; fi

    local paused frozen
    paused=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".paused")
    frozen=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".frozen")
    if [ -z "$paused" ] || [ "$paused" = "null" ]; then paused=false; fi
    if [ -z "$frozen" ] || [ "$frozen" = "null" ]; then frozen=false; fi

    echo "  Asset Address: ${asset_address}"
    echo "  Config Category: ${config_category_id}"
    echo "  Can Be Collateral: ${can_collateral}"
    echo "  Can Be Borrowed: ${can_borrow}"
    echo "  LTV: ${ltv}  Threshold: ${threshold}  Bonus: ${bonus}"
    echo "  Spoke supply cap: ${supply_cap}  Spoke borrow cap: ${borrow_cap}"

    if [ -z "$asset_address" ] || [ "$asset_address" = "null" ] || [ "$asset_address" = "" ]; then
        echo "ERROR: No asset address found for ${asset_name} in ${MARKET_CONFIG_FILE}"
        exit 1
    fi

    local hub_id
    hub_id=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".hub_id")
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "spoke asset ${asset_name} (category ${config_category_id}) missing hub_id in ${SPOKES_FILE}"
    fi

    local args_json
    args_json=$(jq -nc \
        --argjson arg "$(scval_spoke_args "$hub_id" "$asset_address" "$category_id" "$can_collateral" \
            "$can_borrow" "$ltv" "$threshold" "$bonus" "$supply_cap" "$borrow_cap" "$liquidation_fees" "$paused" "$frozen")" \
        '[$arg]')
    local salt
    salt=$(gen_salt "add_asset_to_spoke" "$args_json")

    local admin_op_json
    admin_op_json=$(admin_op AddAssetToSpoke \
        "$(friendly_spoke_args "$hub_id" "$asset_address" "$category_id" "$can_collateral" "$can_borrow" \
            "$ltv" "$threshold" "$bonus" "$supply_cap" "$borrow_cap" "$liquidation_fees" "$paused" "$frozen")")

    local op_id
    op_id=$(schedule_via_proposer \
        add_asset_to_spoke "$admin_op_json" "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"

    echo "Asset ${asset_name} scheduled into Spoke category ${category_id}."
}

edit_asset_in_spoke() {
    local category_id=$1
    local asset_name=$2
    local config_category_id=${3:-$category_id}

    local asset_address
    asset_address=$(get_market_value "$asset_name" "asset_address")
    local can_collateral
    can_collateral=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".can_be_collateral")
    local can_borrow
    can_borrow=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".can_be_borrowed")
    local ltv
    ltv=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".ltv")
    local threshold
    threshold=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".liquidation_threshold")
    local bonus
    bonus=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".liquidation_bonus")
    local supply_cap
    supply_cap=$(require_spoke_cap "$config_category_id" "$asset_name" supply_cap)
    local borrow_cap
    borrow_cap=$(require_spoke_cap "$config_category_id" "$asset_name" borrow_cap)

    local liquidation_fees
    liquidation_fees=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".liquidation_fees")
    if [ -z "$liquidation_fees" ] || [ "$liquidation_fees" = "null" ]; then
        liquidation_fees=$(get_market_value "$asset_name" "liquidation_fees")
    fi
    if [ -z "$liquidation_fees" ] || [ "$liquidation_fees" = "null" ]; then liquidation_fees=0; fi

    local paused frozen
    paused=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".paused")
    frozen=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".frozen")
    if [ -z "$paused" ] || [ "$paused" = "null" ]; then paused=false; fi
    if [ -z "$frozen" ] || [ "$frozen" = "null" ]; then frozen=false; fi

    echo "Editing asset ${asset_name} in Spoke category ${category_id}..." >&2

    local hub_id
    hub_id=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".hub_id")
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "spoke asset ${asset_name} (category ${config_category_id}) missing hub_id in ${SPOKES_FILE}"
    fi

    local args_json
    args_json=$(jq -nc \
        --argjson arg "$(scval_spoke_args "$hub_id" "$asset_address" "$category_id" "$can_collateral" \
            "$can_borrow" "$ltv" "$threshold" "$bonus" "$supply_cap" "$borrow_cap" "$liquidation_fees" "$paused" "$frozen")" \
        '[$arg]')
    local salt
    salt=$(gen_salt "edit_asset_in_spoke" "$args_json")

    local admin_op_json
    admin_op_json=$(admin_op EditAssetInSpoke \
        "$(friendly_spoke_args "$hub_id" "$asset_address" "$category_id" "$can_collateral" "$can_borrow" \
            "$ltv" "$threshold" "$bonus" "$supply_cap" "$borrow_cap" "$liquidation_fees" "$paused" "$frozen")")

    local op_id
    op_id=$(schedule_via_proposer \
        edit_asset_in_spoke "$admin_op_json" "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
}

ensure_asset_in_spoke() {
    local category_id=$1
    local asset_name=$2
    local config_category_id=${3:-$category_id}

    local asset_address
    asset_address=$(get_market_value "$asset_name" "asset_address")
    local can_collateral
    can_collateral=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".can_be_collateral")
    local can_borrow
    can_borrow=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".can_be_borrowed")
    local ltv
    ltv=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".ltv")
    local threshold
    threshold=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".liquidation_threshold")
    local bonus
    bonus=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".liquidation_bonus")
    local supply_cap
    supply_cap=$(require_spoke_cap "$config_category_id" "$asset_name" supply_cap)
    local borrow_cap
    borrow_cap=$(require_spoke_cap "$config_category_id" "$asset_name" borrow_cap)

    local liquidation_fees
    liquidation_fees=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".liquidation_fees")
    if [ -z "$liquidation_fees" ] || [ "$liquidation_fees" = "null" ]; then
        liquidation_fees=$(get_market_value "$asset_name" "liquidation_fees")
    fi
    if [ -z "$liquidation_fees" ] || [ "$liquidation_fees" = "null" ]; then liquidation_fees=0; fi
    local category_json

    if [ -z "$asset_address" ] || [ "$asset_address" = "null" ] || [ "$asset_address" = "" ]; then
        echo "ERROR: No asset address found for ${asset_name} in ${MARKET_CONFIG_FILE}"
        exit 1
    fi

    category_json=$(fetch_spoke_json "$category_id")

    local _hub _ha _probe
    _hub=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".hub_id")
    if [ -n "$_hub" ] && [ "$_hub" != "null" ]; then
        _ha=$(jq -nc --argjson h "$_hub" --arg a "$asset_address" '{hub_id:$h, asset:$a}')
        if _probe=$(stellar contract invoke --id "$(get_controller)" $SOURCE_FLAG --network "$NETWORK" --send=no -- get_spoke_asset --spoke_id "$category_id" --hub_asset "$_ha" 2>/dev/null); then
            category_json=$(jq -nc --arg addr "$asset_address" --argjson cfg "$_probe" '{assets: {($addr): $cfg}}')
        else
            category_json='{"assets":{}}'
        fi
    fi
    if printf '%s' "$category_json" | jq -e --arg asset "$asset_address" '.assets[$asset] != null' >/dev/null; then
        if printf '%s' "$category_json" | jq -e \
            --arg asset "$asset_address" \
            --argjson can_collateral "$can_collateral" \
            --argjson can_borrow "$can_borrow" \
            --argjson ltv "$ltv" \
            --argjson threshold "$threshold" \
            --argjson bonus "$bonus" \
            --argjson liquidation_fees "$liquidation_fees" \
            --arg supply_cap "$supply_cap" \
            --arg borrow_cap "$borrow_cap" \
            '.assets[$asset].is_collateralizable == $can_collateral and
             .assets[$asset].is_borrowable == $can_borrow and
             .assets[$asset].loan_to_value == $ltv and
             .assets[$asset].liquidation_threshold == $threshold and
             .assets[$asset].liquidation_bonus == $bonus and
             .assets[$asset].liquidation_fees == $liquidation_fees and
             (.assets[$asset].supply_cap | tostring) == $supply_cap and
             (.assets[$asset].borrow_cap | tostring) == $borrow_cap' >/dev/null; then
            echo "Asset ${asset_name} already configured in Spoke category ${category_id}."
        else

            REAPPLY_ON_DONE=1 edit_asset_in_spoke "$category_id" "$asset_name" "$config_category_id"
        fi
    else

        REAPPLY_ON_DONE=1 add_asset_to_spoke "$category_id" "$asset_name" "$config_category_id"
    fi
}

setup_all_spokes() {
    echo "=== Setting up all Spoke categories for ${NETWORK} ==="

    require_spoke_caps_configured

    local disabled_cats
    disabled_cats=$(disabled_spoke_ids)
    if [ -n "$disabled_cats" ]; then
        echo "Skipping disabled spokes (enabled=false): ${disabled_cats}"
    fi

    local categories
    categories=$(enabled_spoke_ids)

    for cat_id in $categories; do
        local onchain_id

        onchain_id=$(ensure_spoke "$cat_id")
        onchain_id=$(printf '%s\n' "$onchain_id" | tail -n1)

        local assets asset_name
        assets=$(enabled_spoke_asset_names "$cat_id")
        local skipped
        skipped=$(jq -r --arg c "$cat_id" '
            [.[$c].assets | to_entries[] | select(.value.enabled == false) | .key] | join(", ")
        ' "$SPOKES_FILE")
        if [ -n "$skipped" ]; then
            echo "Spoke ${cat_id}: skipping disabled assets (enabled=false): ${skipped}"
        fi
        for asset_name in $assets; do
            ensure_asset_in_spoke "$onchain_id" "$asset_name" "$cat_id"
        done
    done
    configure_spoke_curves
    echo "=== All Spoke categories configured ==="
}

get_mapped_hub_id() {
    local config_hub_id=$1
    jq -r --arg network "$NETWORK" --arg id "$config_hub_id" \
        '(.[$network].hub_ids // {})[$id] // empty' "$NETWORKS_FILE"
}

persist_hub_id() {
    local config_hub_id=$1
    local onchain_id=$2
    local tmp
    tmp=$(mktemp)
    jq --arg network "$NETWORK" --arg id "$config_hub_id" --argjson onchain_id "$onchain_id" \
        '.[$network].hub_ids = (.[$network].hub_ids // {}) |
         .[$network].hub_ids[$id] = $onchain_id' \
        "$NETWORKS_FILE" > "$tmp" && mv "$tmp" "$NETWORKS_FILE"
}

ensure_hub() {
    local expected=$1
    case "$expected" in
        ''|*[!0-9]*) die "invalid hub_id '${expected}' in ${MARKET_CONFIG_FILE}" ;;
    esac
    if [ "$expected" -lt 1 ]; then
        die "hub_id must be >= 1 (got ${expected}); there is no hub 0"
    fi

    local mapped
    mapped=$(get_mapped_hub_id "$expected")
    if [ -n "$mapped" ] && [ "$mapped" != "null" ]; then
        echo "Hub ${expected} already created (on-chain id ${mapped})." >&2
        return 0
    fi

    local args_json='[]'
    local salt
    salt=$(gen_salt "create_hub:${expected}" "$args_json")

    local op_id
    op_id=$(schedule_via_proposer \
        create_hub "$(admin_op CreateHub)" "$args_json" true "$salt" never)

    if [ "${AUTO_EXECUTE:-1}" != "1" ]; then
        echo "Scheduled hub ${expected} as op ${op_id} (AUTO_EXECUTE=0; execute before listing markets)." >&2
        return 0
    fi

    if [ "$(op_state "$op_id")" = "Done" ]; then
        die "hub-create op ${op_id} already executed; its returned id cannot be re-read. Record the on-chain id in ${NETWORKS_FILE} hub_ids manually."
    fi
    await_op_ready "$op_id"

    local result onchain_id errf
    errf=$(mktemp)
    result=$(execute_op "$op_id" 2>"$errf") || {
        cat "$errf" >&2
        rm -f "$errf"
        die "execute of hub-create op ${op_id} failed"
    }
    rm -f "$errf"
    onchain_id=$(parse_returned_u32 "$result")
    if [ -z "$onchain_id" ]; then
        die "could not parse on-chain hub id from execute result: ${result}"
    fi
    if [ "$onchain_id" != "$expected" ]; then
        die "create_hub returned id ${onchain_id} but the config expects hub ${expected}; create hubs in ascending order with no gaps (there is no hub 0), or fix ${MARKET_CONFIG_FILE}"
    fi
    persist_hub_id "$expected" "$onchain_id"
    echo "Hub ${expected} created with on-chain id ${onchain_id}." >&2
}

ensure_hubs() {
    echo "=== Ensuring hubs for ${NETWORK} ===" >&2
    local hub_ids
    # Only hubs referenced by enabled markets are created during bulk setup.
    hub_ids=$(jq -r '[.markets[] | select(.enabled != false) | .hub_id] | map(select(. != null)) | unique | .[]' "$MARKET_CONFIG_FILE")
    if [ -z "$hub_ids" ]; then
        die "no hub_id found on any enabled market in ${MARKET_CONFIG_FILE}"
    fi
    local h
    for h in $hub_ids; do
        ensure_hub "$h"
    done
    echo "=== Hubs ready ===" >&2
}

create_market() {
    local market_name=$1

    echo "Creating market for ${market_name}..."

    local asset_address
    asset_address=$(get_market_value "$market_name" "asset_address")
    local decimals
    decimals=$(get_contract_decimals "$asset_address")

    echo "  Asset Address: ${asset_address}"
    echo "  On-chain Decimals: ${decimals}"

    if [ -z "$asset_address" ] || [ "$asset_address" = "null" ] || [ "$asset_address" = "" ]; then
        echo "ERROR: No asset address for ${market_name}. Set it in ${MARKET_CONFIG_FILE}"
        exit 1
    fi
    if [ -z "$decimals" ] || [ "$decimals" = "null" ] || [ "$decimals" = "" ]; then
        echo "ERROR: Could not read on-chain decimals for ${market_name} (${asset_address})"
        exit 1
    fi

    local hub_id
    hub_id=$(get_market_value "$market_name" "hub_id")
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "market ${market_name} missing hub_id in ${MARKET_CONFIG_FILE}"
    fi

    local ctrl
    ctrl=$(get_controller)

    local hub_asset
    hub_asset=$(build_hub_assets_json "$market_name" | jq -c '.[0]')
    if stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" --send=no -- get_spoke_asset --spoke_id 0 --hub_asset "$hub_asset" &>/dev/null; then
        echo "Market for ${market_name} already exists, skipping creation."
        return 0
    fi

    local params
    params=$(jq -c --arg decimals "$decimals" \
        ".markets[] | select(.name == \"$market_name\") | .market_params + {
            asset_id: .asset_address,
            asset_decimals: (\$decimals | tonumber),
            is_flashloanable: (.market_params.is_flashloanable // false),
            flashloan_fee: (.market_params.flashloan_fee // 0)
        }" \
        "$MARKET_CONFIG_FILE")

    local params_scval
    params_scval=$(scval_market_params "$params")
    local args_json
    args_json=$(jq -nc \
        --argjson hub_id "$hub_id" \
        --arg asset "$asset_address" \
        --argjson params "$params_scval" \
        '[{u32:$hub_id}, {address:$asset}, $params]')
    local salt
    salt=$(gen_salt "create_liquidity_pool" "$args_json")

    local admin_op_json
    admin_op_json=$(admin_op CreateLiquidityPool \
        "$(jq -nc --argjson hub_id "$hub_id" --arg asset "$asset_address" --argjson params "$params" \
            '{hub_id:$hub_id, asset:$asset, params:$params}')")

    local op_id
    op_id=$(schedule_via_proposer \
        create_liquidity_pool "$admin_op_json" "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"

    echo "Market ${market_name} scheduled/created."
}

update_market_params() {
    local market_name=$1

    echo "Updating market params for ${market_name}..."

    local asset_address
    asset_address=$(get_market_value "$market_name" "asset_address")

    local params
    params=$(jq -c \
        ".markets[] | select(.name == \"$market_name\") | .market_params" \
        "$MARKET_CONFIG_FILE")
    if ! printf '%s' "$params" | jq -e '
        (.is_flashloanable | type) == "boolean" and
        (.flashloan_fee | type) == "number"' >/dev/null; then
        die "market ${market_name}: market_params must include is_flashloanable (bool) and flashloan_fee (u32)"
    fi

    local hub_id
    hub_id=$(get_market_value "$market_name" "hub_id")
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "market ${market_name} missing hub_id in ${MARKET_CONFIG_FILE}"
    fi

    local args_json
    args_json=$(jq -nc \
        --argjson hub_asset "$(scval_hub_asset "$asset_address" "$hub_id")" \
        --argjson params "$(scval_interest_rate_model "$params")" \
        '[$hub_asset, $params]')
    local salt
    salt=$(gen_salt "upgrade_liquidity_pool_params" "$args_json")

    local irm_friendly
    irm_friendly=$(jq -nc --argjson p "$params" '{
        base_borrow_rate: ($p.base_borrow_rate|tostring),
        flashloan_fee: $p.flashloan_fee,
        is_flashloanable: $p.is_flashloanable,
        max_borrow_rate: ($p.max_borrow_rate|tostring),
        max_utilization: ($p.max_utilization|tostring),
        mid_utilization: ($p.mid_utilization|tostring),
        optimal_utilization: ($p.optimal_utilization|tostring),
        reserve_factor: $p.reserve_factor,
        slope1: ($p.slope1|tostring),
        slope2: ($p.slope2|tostring),
        slope3: ($p.slope3|tostring)
    }')
    local admin_op_json
    admin_op_json=$(admin_op UpgradeLiquidityPoolParams \
        "$(jq -nc --argjson hub_id "$hub_id" --arg asset "$asset_address" --argjson params "$irm_friendly" \
            '{hub_asset:{hub_id:$hub_id, asset:$asset}, params:$params}')")

    local op_id
    op_id=$(schedule_via_proposer \
        upgrade_liquidity_pool_params "$admin_op_json" "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"

    echo "Market params scheduled for ${market_name}."
}

update_indexes() {
    if [ $# -eq 0 ]; then
        echo "Usage: $0 updateIndexes <market_name> [market_name...]" >&2
        list_markets >&2
        exit 1
    fi

    echo "Updating indexes for markets: $*"

    local ctrl
    ctrl=$(get_controller)
    local caller
    caller=$(get_signer_address)
    local assets_json
    assets_json=$(build_hub_assets_json "$@")

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- update_indexes \
        --caller "$caller" \
        --assets "$assets_json"

    echo "Indexes updated."
}

claim_revenue() {

    if [ $# -eq 0 ]; then
        echo "Usage: $0 claimRevenue <market_name> [market_name...]" >&2
        list_markets >&2
        exit 1
    fi

    echo "Claiming revenue for markets: $*"

    local ctrl
    ctrl=$(get_controller)
    local caller
    caller=$(get_signer_address)
    local assets_json
    assets_json=$(build_hub_assets_json "$@")

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- claim_revenue \
        --caller "$caller" \
        --assets "$assets_json"

    echo "Revenue claimed."
}

claim_revenue_all() {
    local hub_assets_json
    hub_assets_json=$(all_configured_hub_assets)

    if [ -z "$hub_assets_json" ] || [ "$hub_assets_json" = "[]" ]; then
        echo "No markets with asset_address configured in ${MARKET_CONFIG_FILE}" >&2
        exit 1
    fi

    echo "Claiming revenue for all configured markets..."

    local ctrl
    ctrl=$(get_controller)
    local caller
    caller=$(get_signer_address)

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- claim_revenue \
        --caller "$caller" \
        --assets "$hub_assets_json"

    echo "Revenue claimed for all markets."
}

is_blend_pool_whitelisted() {
    local pool=$1
    local ctrl
    ctrl=$(get_controller)
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" --send=no \
        -- is_blend_pool_approved --pool "$pool" 2>/dev/null | tr -d '"' | tr -d '[:space:]'
}

approve_blend_pool() {
    local pool=$1

    if [ "$(is_blend_pool_whitelisted "$pool")" = "true" ]; then
        echo "Blend pool ${pool} already whitelisted; skipping." >&2
        return 0
    fi

    echo "Whitelisting Blend pool ${pool} (timelocked approve_blend_pool)..." >&2

    local args_json
    args_json=$(jq -nc --arg p "$pool" '[{address:$p}]')
    local salt
    salt=$(gen_salt "approve_blend_pool" "$args_json")

    local op_id
    op_id=$(schedule_via_proposer \
        approve_blend_pool "$(admin_op ApproveBlendPool "$(jq -nc --arg a "$pool" '$a')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"

    echo "Blend pool ${pool} whitelisted." >&2
}

whitelist_blend_pools() {
    if [ ! -f "$BLEND_POOLS_FILE" ]; then
        echo "ERROR: Blend pools config not found: $BLEND_POOLS_FILE" >&2
        exit 1
    fi

    local pools
    pools=$(jq -r '(.pools // [])[] | .address' "$BLEND_POOLS_FILE")
    if [ -z "$pools" ]; then
        echo "No Blend pools configured for ${NETWORK} in ${BLEND_POOLS_FILE}." >&2
        return 0
    fi

    echo "=== Whitelisting Blend pools for ${NETWORK} ===" >&2
    for pool in $pools; do
        approve_blend_pool "$pool"
    done
    echo "=== Blend pool whitelist complete (${NETWORK}) ===" >&2
}

configure_spoke_curves() {
    if [ ! -f "$SPOKES_FILE" ]; then
        echo "ERROR: Spokes config not found: $SPOKES_FILE" >&2
        exit 1
    fi

    local config_ids
    config_ids=$(jq -r 'to_entries[] | select(.value.enabled != false and .value.liquidation_curve != null) | .key' "$SPOKES_FILE")
    if [ -z "$config_ids" ]; then
        echo "No liquidation_curve overrides configured for ${NETWORK} in ${SPOKES_FILE}." >&2
        return 0
    fi

    echo "=== Configuring spoke liquidation curves for ${NETWORK} ===" >&2
    local config_id
    for config_id in $config_ids; do
        local target knee factor
        target=$(jq -r --arg id "$config_id" '.[$id].liquidation_curve.target_hf_wad // empty' "$SPOKES_FILE")
        knee=$(jq -r --arg id "$config_id" '.[$id].liquidation_curve.hf_for_max_bonus_wad // empty' "$SPOKES_FILE")
        factor=$(jq -r --arg id "$config_id" '.[$id].liquidation_curve.liquidation_bonus_factor_bps // empty' "$SPOKES_FILE")
        if [ -z "$target" ] || [ -z "$knee" ] || [ -z "$factor" ]; then
            die "spoke ${config_id}: liquidation_curve needs target_hf_wad, hf_for_max_bonus_wad, liquidation_bonus_factor_bps"
        fi

        local onchain_id
        onchain_id=$(get_mapped_spoke_id "$config_id")
        if [ -z "$onchain_id" ]; then
            echo "WARN: spoke ${config_id} has no on-chain id in ${NETWORKS_FILE}; run setupAllSpokes first. Skipping." >&2
            continue
        fi

        local live live_target live_knee live_factor
        live=$(fetch_spoke_json "$onchain_id" 2>/dev/null | tail -n1)
        live_target=$(printf '%s' "$live" | jq -r '.liquidation_target_hf_wad // empty' 2>/dev/null)
        live_knee=$(printf '%s' "$live" | jq -r '.hf_for_max_bonus_wad // empty' 2>/dev/null)
        live_factor=$(printf '%s' "$live" | jq -r '.liquidation_bonus_factor_bps // empty' 2>/dev/null)

        if [ "$live_target" = "$target" ] && [ "$live_knee" = "$knee" ] && [ "$live_factor" = "$factor" ]; then
            echo "Spoke ${config_id} (on-chain ${onchain_id}) curve already matches config; skipping." >&2
            continue
        fi

        echo "Spoke ${config_id} (on-chain ${onchain_id}): curve ${live_target:-?}/${live_knee:-?}/${live_factor:-?} -> ${target}/${knee}/${factor}" >&2
        set_spoke_liquidation_curve_cmd "$onchain_id" "$target" "$knee" "$factor"
    done
}

set_aggregator() {
    echo "Configuring Swap Aggregator for ${NETWORK}..."
    local router
    if ! router=$(get_aggregator_address); then
        echo "ERROR: No aggregator address for ${NETWORK}. Set networks.json aggregator or AGGREGATOR_CONTRACT." >&2
        exit 1
    fi

    echo "  Swap Aggregator Address: ${router}" >&2

    local args_json
    args_json=$(jq -nc --arg a "$router" '[{address:$a}]')
    local salt
    salt=$(gen_salt "set_swap_aggregator" "$args_json")

    local op_id
    op_id=$(schedule_via_proposer \
        set_swap_aggregator "$(admin_op SetSwapAggregator "$(jq -nc --arg a "$router" '$a')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"

    echo "Swap aggregator scheduled via governance."
}

set_price_aggregator() {
    echo "Wiring Price Aggregator (oracle authority) for ${NETWORK}..."
    local agg
    if ! agg=$(get_price_aggregator); then
        echo "ERROR: No price-aggregator address for ${NETWORK}. Run the deploy step (governance deploy_price_aggregator) first." >&2
        exit 1
    fi

    echo "  Price Aggregator Address: ${agg}" >&2

    local salt
    salt=$(gen_salt "set_price_aggregator" "$(jq -nc --arg a "$agg" '{addr:$a}')")

    local op_id
    op_id=$(schedule_via_gov_self_proposer \
        set_price_aggregator "$(admin_op SetPriceAggregator "$(jq -nc --arg a "$agg" '$a')")" "$salt" \
        set_price_aggregator "$(jq -nc --arg a "$agg" '[{address:$a}]')")
    schedule_and_maybe_execute "$op_id"

    echo "Price aggregator wiring scheduled via governance."
}

set_accumulator() {
    echo "Configuring Accumulator for ${NETWORK}..."
    local accumulator
    if ! accumulator=$(get_accumulator_address); then
        echo "ERROR: No revenue accumulator for ${NETWORK}." >&2
        echo "       claimRevenue fails with NoAccumulator (#211) until this is set." >&2
        echo "       Set networks.json accumulator or ACCUMULATOR_CONTRACT (G-wallet or contract)." >&2
        exit 1
    fi

    echo "  Accumulator Address: ${accumulator}" >&2

    local args_json
    args_json=$(jq -nc --arg a "$accumulator" '[{address:$a}]')
    local salt
    salt=$(gen_salt "set_accumulator" "$args_json")

    local op_id
    op_id=$(schedule_via_proposer \
        set_accumulator "$(admin_op SetAccumulator "$(jq -nc --arg a "$accumulator" '$a')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"

    echo "Accumulator scheduled via governance."
}

supply_position() {
    local market=$1
    local amount_raw=$2
    local account_id=${3:-0}
    local spoke_id=${4:-0}

    local ctrl
    ctrl=$(get_controller)
    local caller=$SIGNER_ADDRESS
    local asset_addr
    asset_addr=$(get_market_value "$market" "asset_address")
    local hub_id
    hub_id=$(get_market_value "$market" "hub_id")
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "market '${market}' missing hub_id"
    fi

    echo "=== supply ==="
    echo "  Account:  $account_id  (0 = create new)"
    echo "  Spoke:   $spoke_id  (0 = none)"
    echo "  Asset:    $market ($asset_addr)"
    echo "  Amount:   $amount_raw"
    echo

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- supply \
        --caller "$caller" \
        --account_id "$account_id" \
        --spoke_id "$spoke_id" \
        --assets "[[{\"hub_id\":$hub_id,\"asset\":\"$asset_addr\"}, \"$amount_raw\"]]"
}

borrow_position() {
    local market=$1
    local amount_raw=$2
    local account_id=$3

    local ctrl
    ctrl=$(get_controller)
    local caller=$SIGNER_ADDRESS
    local asset_addr
    asset_addr=$(get_market_value "$market" "asset_address")
    local hub_id
    hub_id=$(get_market_value "$market" "hub_id")
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "market '${market}' missing hub_id"
    fi

    echo "=== borrow ==="
    echo "  Account: $account_id"
    echo "  Asset:   $market ($asset_addr)"
    echo "  Amount:  $amount_raw"
    echo

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- borrow \
        --caller "$caller" \
        --account_id "$account_id" \
        --borrows "[[{\"hub_id\":$hub_id,\"asset\":\"$asset_addr\"}, \"$amount_raw\"]]" \
        --to null
}

withdraw_position() {
    local market=$1
    local amount_raw=$2
    local account_id=$3

    local ctrl
    ctrl=$(get_controller)
    local caller=$SIGNER_ADDRESS
    local asset_addr
    asset_addr=$(get_market_value "$market" "asset_address")
    local hub_id
    hub_id=$(get_market_value "$market" "hub_id")
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "market '${market}' missing hub_id"
    fi

    echo "=== withdraw ==="
    echo "  Account: $account_id"
    echo "  Asset:   $market ($asset_addr)"
    echo "  Amount:  $amount_raw (0 = all)"
    echo

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- withdraw \
        --caller "$caller" \
        --account_id "$account_id" \
        --withdrawals "[[{\"hub_id\":$hub_id,\"asset\":\"$asset_addr\"}, \"$amount_raw\"]]" \
        --to null
}

market_oracle_ref_dependencies() {
    local market_name=$1
    jq -r --arg m "$market_name" '
        .markets[] | select(.name == $m) | (.oracle.sources // [])[] |
        (.Scaled.quote.Ref // (.AquariusLp // .AquariusStableLp).key_a.Ref // empty),
        ((.AquariusLp // .AquariusStableLp).key_b.Ref // empty)
    ' "$MARKET_CONFIG_FILE" | sort -u
}

all_oracle_ref_dependencies() {
    # Only Refs required by enabled markets; disabled markets do not drive bulk setup.
    jq -r '
        [.markets[] | select(.enabled != false) | (.oracle.sources // [])[] |
         (.Scaled.quote.Ref // (.AquariusLp // .AquariusStableLp).key_a.Ref // empty),
         ((.AquariusLp // .AquariusStableLp).key_b.Ref // empty)] | unique | .[]
    ' "$MARKET_CONFIG_FILE"
}

require_reference_entry() {
    local ref_name=$1
    local entry
    entry=$(jq -c --arg n "$ref_name" '
        first(.references[]? | select(.name == $n or .key.Ref == $n)) // empty
    ' "$MARKET_CONFIG_FILE")
    if [ -z "$entry" ] || [ "$entry" = "null" ]; then
        echo "ERROR: reference oracle '${ref_name}' not listed in ${MARKET_CONFIG_FILE} .references[]" >&2
        echo "       Scaled quotes need a PriceKey::Ref entry with name/key matching '${ref_name}'." >&2
        exit 1
    fi
    printf '%s' "$entry"
}

preflight_oracle_sanity() {
    local label=$1
    local oracle_json=$2
    if ! printf '%s' "$oracle_json" | jq -e 'has("min_sanity_price_wad") and has("max_sanity_price_wad")' >/dev/null; then
        echo "ERROR: ${label} oracle config missing min_sanity_price_wad / max_sanity_price_wad" >&2
        exit 1
    fi
    if [ "$NETWORK" = "mainnet" ]; then
        if printf '%s' "$oracle_json" | jq -e '
            (.min_sanity_price_wad | tostring) == "0" and
            (.max_sanity_price_wad | tostring) == "0"
        ' >/dev/null; then
            echo "ERROR: ${label} uses (0, 0) sanity-bound sentinel on mainnet" >&2
            exit 1
        fi
    fi
}

schedule_configure_asset_oracle() {
    local label=$1
    local key_json=$2
    local cfg_json=$3

    local gov proposer
    gov=$(get_governance)
    proposer=$(get_signer_address)

    local salt_input salt resolve_args
    salt_input=$(jq -nc --argjson oracle "$cfg_json" --argjson key "$key_json" \
        '{key:$key, oracle:$oracle}')
    salt=$(gen_salt "set_oracle" "$salt_input")
    resolve_args=$(jq -nc --argjson key "$key_json" --argjson oracle "$cfg_json" \
        '{key:$key, oracle:$oracle}')

    local agg resolved_args salt_use known_id state gen
    agg=$(get_price_aggregator)
    resolved_args=$(resolve_oracle_args_for resolve_asset_oracle "$agg" \
        set_oracle "$key_json" "$cfg_json" 2>/dev/null) || resolved_args=""
    if [ -n "$resolved_args" ] && [ "$resolved_args" != "null" ]; then
        read -r salt_use known_id state gen < <(probe_salt_generations "$agg" set_oracle "$resolved_args" "$salt")
        case "$state" in
            Ready|Waiting)
                echo "Oracle config op ${known_id} for ${label} already ${state}; reusing it instead of re-proposing." >&2
                write_oracle_op_record "$known_id" "set_oracle" \
                    "resolve_asset_oracle" "$resolve_args" "$salt_use"
                schedule_and_maybe_execute "$known_id"
                return 0
                ;;
            Exhausted)
                die "configure oracle ${label}: all ${MAX_SALT_GENERATIONS} salt generations already executed; re-run with a fresh SALT_NONCE=<n>"
                ;;
            Unset)
                if [ "$gen" -gt 0 ]; then
                    if [ "${REAPPLY_ON_DONE:-1}" != "1" ]; then
                        local done_id
                        done_id=$(precomputed_op_id "$agg" set_oracle "$resolved_args" "$salt")
                        echo "Oracle config for ${label} already executed with this exact config; skipping propose (converge mode)." >&2
                        write_oracle_op_record "$done_id" "set_oracle" \
                            "resolve_asset_oracle" "$resolve_args" "$salt"
                        mark_op_executed "$done_id"
                        return 0
                    fi
                    echo "Oracle config for ${label} already executed with this exact config; RE-APPLYING as generation ${gen}." >&2
                    salt=$salt_use
                fi
                ;;
            *) ;;
        esac
    fi

    local op_file
    op_file=$(mktemp)
    jq -nc --argjson key "$key_json" --argjson oracle "$cfg_json" \
        '{ConfigureAssetOracle: {key:$key, oracle:$oracle}}' > "$op_file"

    echo "Scheduling oracle config for ${label}..." >&2
    local out
    out=$(retry_tx stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
        -- propose \
        --proposer "$proposer" \
        --op-file-path "$op_file" \
        --salt "$salt")

    rm -f "$op_file"

    local op_id
    op_id=$(parse_op_id "$out")
    if [ -z "$op_id" ]; then
        echo "ERROR: propose ConfigureAssetOracle for ${label} returned no operation id (output: $out)" >&2
        exit 1
    fi
    write_oracle_op_record "$op_id" "set_oracle" \
        "resolve_asset_oracle" "$resolve_args" "$salt"

    echo "Oracle scheduled for ${label} as op ${op_id}." >&2
    schedule_and_maybe_execute "$op_id"
}

configure_reference_oracle() {
    local ref_name=$1
    if [ -z "$ref_name" ]; then
        echo "Usage: $0 configureReferenceOracle <ref_name>" >&2
        exit 1
    fi

    echo "Configuring reference oracle for ${ref_name}..."
    local entry key_json cfg_json
    entry=$(require_reference_entry "$ref_name")
    key_json=$(printf '%s' "$entry" | jq -c '.key')
    cfg_json=$(printf '%s' "$entry" | jq -c '.oracle' | oracle_cfg_cli_union)
    preflight_oracle_sanity "reference ${ref_name}" "$cfg_json"

    if ! printf '%s' "$cfg_json" | jq -e '(.asset_decimals // -1) == 0' >/dev/null; then
        echo "ERROR: reference ${ref_name}: asset_decimals must be 0 for PriceKey::Ref" >&2
        exit 1
    fi

    schedule_configure_asset_oracle "reference ${ref_name}" "$key_json" "$cfg_json"
}

ensure_reference_oracles_for_market() {
    local market_name=$1
    local ref
    for ref in $(market_oracle_ref_dependencies "$market_name"); do
        [ -z "$ref" ] && continue
        echo "Market ${market_name} requires oracle reference ${ref}; ensuring it is configured..." >&2
        configure_reference_oracle "$ref"
    done
}

setup_all_reference_oracles() {
    local refs ref
    refs=$(all_oracle_ref_dependencies)
    if [ -z "$refs" ]; then
        echo "=== No oracle Ref dependencies in markets; skipping reference oracles ==="
        return 0
    fi
    echo "=== Configuring reference oracles required by Scaled markets ==="
    for ref in $refs; do
        configure_reference_oracle "$ref"
    done
    echo "=== Reference oracles configured ==="
}

list_references() {
    echo "=== Reference oracles (${NETWORK}) from ${MARKET_CONFIG_FILE} ===" >&2
    if ! jq -e '(.references // []) | length > 0' "$MARKET_CONFIG_FILE" >/dev/null; then
        echo "(none)" >&2
        return 0
    fi
    local r i n src used_by
    for r in $(jq -r '.references[].name' "$MARKET_CONFIG_FILE"); do
        used_by=$(jq -r --arg r "$r" '
            [.markets[] | select(any(.oracle.sources[]?;
                (.Scaled.quote.Ref // "") == $r or
                ((.AquariusLp // .AquariusStableLp).key_a.Ref // "") == $r or
                ((.AquariusLp // .AquariusStableLp).key_b.Ref // "") == $r)) | .name] | join(", ")
        ' "$MARKET_CONFIG_FILE")
        [ -z "$used_by" ] && used_by="(unused by markets)"
        jq -r --arg r "$r" --arg u "$used_by" '
            first(.references[] | select(.name == $r)) |
            "\(.name) key=\(.key|tostring) used_by=\($u): sources=\((.oracle.sources // [])|length) stale=\(.oracle.max_price_stale_seconds // "?")s independence=\(.oracle.independence // "?")"
        ' "$MARKET_CONFIG_FILE" >&2
        n=$(jq -r --arg r "$r" 'first(.references[] | select(.name == $r)) | (.oracle.sources // []) | length' "$MARKET_CONFIG_FILE")
        i=0
        while [ "$i" -lt "$n" ]; do
            src=$(jq -c --arg r "$r" --argjson i "$i" \
                'first(.references[] | select(.name == $r)) | .oracle.sources[$i]' "$MARKET_CONFIG_FILE")
            describe_oracle_source "  source[$i]" "$src"
            i=$((i + 1))
        done
    done
}

configure_market_oracle() {
    local market_name=$1

    echo "Configuring market oracle for ${market_name}..."

    ensure_reference_oracles_for_market "$market_name"

    local cfg_json
    cfg_json=$(jq -c --arg market "$market_name" '
        first(.markets[] | select(.name == $market) | .oracle) // empty
    ' "$MARKET_CONFIG_FILE")
    if [ -z "$cfg_json" ] || [ "$cfg_json" = "null" ]; then
        echo "ERROR: market ${market_name} has no oracle config in ${MARKET_CONFIG_FILE}" >&2
        exit 1
    fi
    cfg_json=$(printf '%s' "$cfg_json" | oracle_cfg_cli_union)
    preflight_oracle_sanity "market ${market_name}" "$cfg_json"

    local asset_address key_json
    asset_address=$(require_market_address "$market_name")
    key_json=$(price_key_token "$asset_address")

    schedule_configure_asset_oracle "market ${market_name}" "$key_json" "$cfg_json"
}

edit_oracle_tolerance() {
    local market_name=$1
    local tolerance=$2
    if [ -z "$market_name" ] || [ -z "$tolerance" ]; then
        echo "Usage: $0 editOracleTolerance <market> <tolerance_bps>" >&2
        exit 1
    fi

    local asset_address
    asset_address=$(require_market_address "$market_name")
    local key_json
    key_json=$(price_key_token "$asset_address")
    local gov
    gov=$(get_governance)
    local proposer
    proposer=$(get_signer_address)

    local salt_input
    salt_input=$(jq -nc --argjson key "$key_json" --argjson t "$tolerance" \
        '{key:$key, tolerance:$t}')
    local salt
    salt=$(gen_salt "set_tolerance" "$salt_input")
    local resolve_args
    resolve_args=$(jq -nc --argjson key "$key_json" --argjson t "$tolerance" \
        '{key:$key, tolerance:$t}')

    local agg resolved_args salt_use known_id state gen
    agg=$(get_price_aggregator)
    resolved_args=$(resolve_oracle_args_for resolve_oracle_tolerance "$agg" \
        set_tolerance "$key_json" "$tolerance" 2>/dev/null) || resolved_args=""
    if [ -n "$resolved_args" ] && [ "$resolved_args" != "null" ]; then
        read -r salt_use known_id state gen < <(probe_salt_generations "$agg" set_tolerance "$resolved_args" "$salt")
        case "$state" in
            Ready|Waiting)
                echo "Oracle tolerance op ${known_id} for ${market_name} already ${state}; reusing it instead of re-proposing." >&2
                write_oracle_op_record "$known_id" "set_tolerance" \
                    "resolve_oracle_tolerance" "$resolve_args" "$salt_use"
                schedule_and_maybe_execute "$known_id"
                return 0
                ;;
            Exhausted)
                die "editOracleTolerance ${market_name}: all ${MAX_SALT_GENERATIONS} salt generations already executed; re-run with a fresh SALT_NONCE=<n>"
                ;;
            Unset)
                if [ "$gen" -gt 0 ]; then
                    if [ "${REAPPLY_ON_DONE:-1}" != "1" ]; then
                        local done_id
                        done_id=$(precomputed_op_id "$agg" set_tolerance "$resolved_args" "$salt")
                        echo "Oracle tolerance for ${market_name} already executed with this exact value; skipping propose (converge mode)." >&2
                        write_oracle_op_record "$done_id" "set_tolerance" \
                            "resolve_oracle_tolerance" "$resolve_args" "$salt"
                        mark_op_executed "$done_id"
                        return 0
                    fi
                    echo "Oracle tolerance for ${market_name} already executed with this exact value; RE-APPLYING as generation ${gen}." >&2
                    salt=$salt_use
                fi
                ;;
            *) ;;
        esac
    fi

    local admin_op_json
    admin_op_json=$(admin_op EditOracleTolerance \
        "$(jq -nc --argjson key "$key_json" --argjson t "$tolerance" \
            '{key:$key, tolerance:$t}')")
    local op_file
    op_file=$(mktemp)
    printf '%s' "$admin_op_json" > "$op_file"

    echo "Scheduling oracle tolerance edit for ${market_name} (tolerance=${tolerance})..." >&2
    local out
    out=$(retry_tx stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
        -- propose \
        --proposer "$proposer" \
        --op-file-path "$op_file" \
        --salt "$salt")

    rm -f "$op_file"

    local op_id
    op_id=$(parse_op_id "$out")
    if [ -z "$op_id" ]; then
        echo "ERROR: propose EditOracleTolerance returned no operation id (output: $out)" >&2
        exit 1
    fi
    write_oracle_op_record "$op_id" "set_tolerance" \
        "resolve_oracle_tolerance" "$resolve_args" "$salt"

    echo "Oracle tolerance edit scheduled for ${market_name} as op ${op_id}." >&2
    schedule_and_maybe_execute "$op_id"
}

setup_all_markets() {
    echo "=== Setting up all markets for ${NETWORK} ==="

    local disabled
    disabled=$(disabled_market_names)
    if [ -n "$disabled" ]; then
        echo "Skipping disabled markets (enabled=false): ${disabled}"
    fi

    ensure_hubs

    setup_all_reference_oracles
    local markets
    markets=$(enabled_market_names)
    if [ -z "$markets" ]; then
        die "no enabled markets in ${MARKET_CONFIG_FILE}"
    fi

    for market_name in $markets; do
        create_market "$market_name"
        configure_market_oracle "$market_name"
    done
    echo "=== All markets configured ==="
}

require_market_address() {
    local market_name=$1
    local asset_address
    asset_address=$(get_market_value "$market_name" "asset_address")
    if [ -z "$asset_address" ] || [ "$asset_address" = "null" ] || [ "$asset_address" = "" ]; then
        echo "ERROR: Unknown market '${market_name}' in ${MARKET_CONFIG_FILE}" >&2
        list_markets >&2
        exit 1
    fi
    echo "$asset_address"
}

all_configured_asset_addresses() {
    jq -c '[.markets[] | select(.enabled != false) | select(.asset_address != null and .asset_address != "") | .asset_address]' "$MARKET_CONFIG_FILE"
}

all_configured_hub_assets() {
    jq -c '[.markets[] | select(.enabled != false) | select(.asset_address != null and .asset_address != "") | {hub_id, asset: .asset_address}]' "$MARKET_CONFIG_FILE"
}

schedule_upgrade_controller() {
    local hash=$1
    if [ -z "$hash" ]; then
        echo "Usage: $0 upgradeControllerHash <wasm_hash_hex>" >&2
        exit 1
    fi

    local args_json
    args_json=$(jq -nc --arg h "$hash" '[{bytes:$h}]')
    local salt
    salt=$(gen_salt "upgrade" "$args_json")
    local op_id
    op_id=$(schedule_via_proposer \
        upgrade "$(admin_op UpgradeController "$(jq -nc --arg h "$hash" '$h')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "Controller upgrade scheduled (hash ${hash})."
}

schedule_upgrade_governance() {
    local hash=$1
    if [ -z "$hash" ]; then
        echo "Usage: $0 upgradeGovernanceHash <wasm_hash_hex>" >&2
        exit 1
    fi
    local salt
    salt=$(gen_salt "governance_upgrade" "$(jq -nc --arg h "$hash" '{hash:$h}')")
    local op_id
    op_id=$(schedule_via_gov_self_proposer \
        upgrade_gov "$(admin_op UpgradeGov "$(jq -nc --arg h "$hash" '$h')")" "$salt" \
        upgrade "$(jq -nc --arg h "$hash" '[{bytes:$h}]')")
    schedule_and_maybe_execute "$op_id"
    echo "Governance upgrade scheduled (hash ${hash})."
}

schedule_update_delay() {
    local new_delay=$1
    if [ -z "$new_delay" ]; then
        echo "Usage: $0 updateDelay <new_delay_ledgers>" >&2
        exit 1
    fi
    local salt
    salt=$(gen_salt "update_delay" "$(jq -nc --argjson d "$new_delay" '{delay:$d}')")
    local op_id
    op_id=$(schedule_via_gov_self_proposer \
        update_gov_delay "$(admin_op UpdateGovDelay "$(jq -nc --argjson d "$new_delay" '$d')")" "$salt" \
        update_delay "$(jq -nc --argjson d "$new_delay" '[{u32:$d}]')")
    schedule_and_maybe_execute "$op_id"
    echo "Governance min-delay update scheduled (${new_delay} ledgers)."
}

schedule_transfer_gov_ownership() {
    local new_owner=$1
    local live_until=$2
    if [ -z "$new_owner" ] || [ -z "$live_until" ]; then
        echo "Usage: $0 transferGovOwnership <new_owner> <live_until_ledger>" >&2
        exit 1
    fi
    local salt
    salt=$(gen_salt "transfer_gov_ownership" "$(jq -nc --arg o "$new_owner" --argjson l "$live_until" '{owner:$o,live:$l}')")
    local op_id
    op_id=$(schedule_via_gov_self_proposer \
        transfer_gov_ownership \
        "$(admin_op TransferGovOwnership "$(jq -nc --arg o "$new_owner" --argjson l "$live_until" \
            '{new_owner:$o, live_until_ledger:$l}')")" \
        "$salt" \
        transfer_ownership "$(jq -nc --arg o "$new_owner" --argjson l "$live_until" '[{address:$o},{u32:$l}]')")
    schedule_and_maybe_execute "$op_id"
    echo "Governance ownership transfer scheduled to ${new_owner}."
}

# One-shot governance deploy of the position NFT — the account-ownership
# authority. Mirrors schedule_deploy_pool: standard-tier timelock op, `never`
# reapply (the controller asserts PositionNftAlreadyDeployed), and the returned
# address must be captured on first execution.
schedule_deploy_position_nft() {
    local hash=$1
    if [ -z "$hash" ]; then
        echo "Usage: $0 deployPositionNft <wasm_hash_hex>" >&2
        exit 1
    fi
    local uri=${POSITION_NFT_URI:-"https://xoxno.com/nft/lending/"}
    local name=${POSITION_NFT_NAME:-"XOXNO Lending Position"}
    local symbol=${POSITION_NFT_SYMBOL:-"XLP"}
    local args_json
    args_json=$(jq -nc --arg h "$hash" --arg u "$uri" --arg n "$name" --arg s "$symbol" \
        '[{bytes:$h},{string:$u},{string:$n},{string:$s}]')
    local salt
    salt=$(gen_salt "deploy_position_nft" "$args_json")

    local op_id
    op_id=$(schedule_via_proposer \
        deploy_position_nft "$(admin_op DeployPositionNft \
            "$(jq -nc --arg h "$hash" --arg u "$uri" --arg n "$name" --arg s "$symbol" \
                '{wasm_hash:$h, uri:$u, name:$n, symbol:$s}')")" \
        "$args_json" true "$salt" never)
    if [ "${AUTO_EXECUTE:-1}" != "1" ]; then
        echo "Scheduled deploy_position_nft as op ${op_id} (AUTO_EXECUTE=0)." >&2
        echo "$op_id"
        return 0
    fi
    if [ "$(op_state "$op_id")" = "Done" ]; then
        die "deploy_position_nft op ${op_id} already executed; its returned address cannot be re-read. Record the position_nft address in ${NETWORKS_FILE} manually."
    fi
    await_op_ready "$op_id"
    local result errf
    errf=$(mktemp)
    result=$(execute_op "$op_id" 2>"$errf") || { cat "$errf" >&2; rm -f "$errf"; die "execute of deploy_position_nft op ${op_id} failed"; }
    rm -f "$errf"
    local nft
    nft=$(printf '%s' "$result" | tail -n1 | tr -d '"' | tr -d '[:space:]')
    if [ -z "$nft" ]; then
        echo "ERROR: deploy_position_nft execute returned no address (result: $result)" >&2
        exit 1
    fi
    echo "$nft"
}

schedule_deploy_pool() {
    local hash=$1
    if [ -z "$hash" ]; then
        echo "Usage: $0 deployPool <wasm_hash_hex>" >&2
        exit 1
    fi
    local args_json
    args_json=$(jq -nc --arg h "$hash" '[{bytes:$h}]')
    local salt
    salt=$(gen_salt "deploy_pool" "$args_json")

    local op_id
    op_id=$(schedule_via_proposer \
        deploy_pool "$(admin_op DeployPool "$(jq -nc --arg h "$hash" '$h')")" \
        "$args_json" true "$salt" never)
    if [ "${AUTO_EXECUTE:-1}" != "1" ]; then
        echo "Scheduled deploy_pool as op ${op_id} (AUTO_EXECUTE=0)." >&2
        echo "$op_id"
        return 0
    fi
    if [ "$(op_state "$op_id")" = "Done" ]; then
        die "deploy_pool op ${op_id} already executed; its returned address cannot be re-read. Record the pool address in ${NETWORKS_FILE} manually."
    fi
    await_op_ready "$op_id"
    local result errf
    errf=$(mktemp)
    result=$(execute_op "$op_id" 2>"$errf") || {
        cat "$errf" >&2
        rm -f "$errf"
        die "execute of deploy_pool op ${op_id} failed"
    }
    rm -f "$errf"
    local pool
    pool=$(printf '%s' "$result" | tail -n1 | tr -d '"' | tr -d '[:space:]')
    if [ -z "$pool" ]; then
        echo "ERROR: deploy_pool execute returned no address (result: $result)" >&2
        exit 1
    fi
    echo "$pool"
}

schedule_upgrade_pool() {
    local hash=$1
    if [ -z "$hash" ]; then
        echo "Usage: $0 upgradePoolHash <wasm_hash_hex>" >&2
        exit 1
    fi

    local args_json
    args_json=$(jq -nc --arg h "$hash" '[{bytes:$h}]')
    local salt
    salt=$(gen_salt "upgrade_pool" "$args_json")
    local op_id
    op_id=$(schedule_via_proposer \
        upgrade_pool "$(admin_op UpgradePool "$(jq -nc --arg h "$hash" '$h')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "Pool upgrade scheduled (hash ${hash})."
}

pause_protocol() {
    local gov caller
    gov=$(get_governance)
    caller=$(get_signer_address)

    stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" -- \
        pause --caller "$caller"
    echo "Protocol paused on ${NETWORK} (GUARDIAN immediate)."
}

unpause_protocol() {

    if [ "$NETWORK" = "mainnet" ]; then
        local floor current
        floor=$(jq -r '.["mainnet"].timelock_min_delay_ledgers // empty' "$NETWORKS_FILE")
        if [ -z "$floor" ] || [ "$floor" = "null" ]; then
            echo "Refusing to unpause mainnet: timelock_min_delay_ledgers is not configured in networks.json." >&2
            return 1
        fi
        current=$(min_delay_ledgers)
        if [ "$current" -lt "$floor" ]; then
            echo "Refusing to unpause mainnet: on-chain timelock delay ${current} < production floor ${floor} ledgers." >&2
            echo "Raise it first, then unpause:  make mainnet updateDelay ${floor}" >&2
            return 1
        fi
        echo "Mainnet timelock delay ${current} >= floor ${floor}: unpause permitted."
    fi

    local args_json='[]'
    local salt op_id
    salt=$(gen_salt "unpause" "$args_json")
    op_id=$(schedule_via_proposer \
        unpause "$(admin_op Unpause)" "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "Protocol unpause scheduled/executed on ${NETWORK} (timelocked AdminOperation::Unpause, op ${op_id})."
}

schedule_address_op() {
    local variant=$1
    local controller_fn=$2
    local addr=$3
    local args_json
    args_json=$(jq -nc --arg a "$addr" '[{address:$a}]')
    local salt
    salt=$(gen_salt "$controller_fn" "$args_json")
    local op_id
    op_id=$(schedule_via_proposer \
        "$controller_fn" "$(admin_op "$variant" "$(jq -nc --arg a "$addr" '$a')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "${controller_fn} scheduled for ${addr}."
}

revoke_blend_pool_cmd() {
    schedule_address_op RevokeBlendPool revoke_blend_pool "$1"
}

remove_spoke_cmd() {
    local spoke_id=$1
    local args_json
    args_json=$(jq -nc --argjson id "$spoke_id" '[{u32:$id}]')
    local salt
    salt=$(gen_salt "remove_spoke" "$args_json")
    local op_id
    op_id=$(schedule_via_proposer \
        remove_spoke "$(admin_op RemoveSpoke "$(jq -nc --argjson id "$spoke_id" '$id')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "remove_spoke scheduled for spoke ${spoke_id}."
}

remove_asset_from_spoke_cmd() {
    local spoke_id=$1
    local market_name=$2
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local hub_id
    hub_id=$(get_market_value "$market_name" "hub_id")
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "market ${market_name} missing hub_id in ${MARKET_CONFIG_FILE}"
    fi

    local args_json
    args_json=$(jq -nc \
        --argjson hub_asset "$(scval_hub_asset "$asset_address" "$hub_id")" \
        --argjson spoke "$spoke_id" \
        '[$hub_asset, {u32:$spoke}]')
    local salt
    salt=$(gen_salt "remove_asset_from_spoke" "$args_json")
    local admin_op_json
    admin_op_json=$(admin_op RemoveAssetFromSpoke \
        "$(jq -nc --argjson hub_id "$hub_id" --arg asset "$asset_address" --argjson spoke "$spoke_id" \
            '{hub_asset:{hub_id:$hub_id, asset:$asset}, spoke_id:$spoke}')")
    local op_id
    op_id=$(schedule_via_proposer \
        remove_asset_from_spoke "$admin_op_json" "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "remove_asset_from_spoke scheduled for ${market_name} in spoke ${spoke_id}."
}

set_spoke_liquidation_curve_cmd() {
    local spoke_id=$1
    local target_hf_wad=$2
    local hf_for_max_bonus_wad=$3
    local bonus_factor_bps=$4

    local args_json
    args_json=$(jq -nc \
        --argjson spoke "$spoke_id" \
        --arg target "$target_hf_wad" \
        --arg maxb "$hf_for_max_bonus_wad" \
        --argjson factor "$bonus_factor_bps" \
        '[{u32:$spoke}, {i128:$target}, {i128:$maxb}, {u32:$factor}]')
    local salt
    salt=$(gen_salt "set_spoke_liquidation_curve" "$args_json")
    local admin_op_json
    admin_op_json=$(admin_op SetSpokeLiquidationCurve \
        "$(jq -nc --argjson spoke "$spoke_id" --arg target "$target_hf_wad" --arg maxb "$hf_for_max_bonus_wad" --argjson factor "$bonus_factor_bps" \
            '{spoke_id:$spoke, target_hf_wad:$target, hf_for_max_bonus_wad:$maxb, liquidation_bonus_factor_bps:$factor}')")
    local op_id
    op_id=$(schedule_via_proposer \
        set_spoke_liquidation_curve "$admin_op_json" "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "set_spoke_liquidation_curve scheduled for spoke ${spoke_id} (target_hf=${target_hf_wad}, hf_for_max_bonus=${hf_for_max_bonus_wad}, bonus_factor_bps=${bonus_factor_bps})."
}

set_position_limits_cmd() {
    local max_supply=$1
    local max_borrow=$2
    local friendly
    friendly=$(jq -nc --argjson s "$max_supply" --argjson b "$max_borrow" \
        '{max_supply_positions:$s, max_borrow_positions:$b}')
    local args_json
    args_json=$(jq -nc --argjson l "$(scval_position_limits "$friendly")" '[$l]')
    local salt
    salt=$(gen_salt "set_position_limits" "$args_json")
    local op_id
    op_id=$(schedule_via_proposer \
        set_position_limits "$(admin_op SetPositionLimits "$friendly")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "set_position_limits scheduled (supply ${max_supply}, borrow ${max_borrow})."
}

set_min_borrow_collateral_cmd() {
    local floor_wad=$1
    local args_json
    args_json=$(jq -nc --arg v "$floor_wad" '[{i128:$v}]')
    local salt
    salt=$(gen_salt "set_min_borrow_collateral_usd" "$args_json")
    local op_id
    op_id=$(schedule_via_proposer \
        set_min_borrow_collateral_usd \
        "$(admin_op SetMinBorrowCollateralUsd "$(jq -nc --arg v "$floor_wad" '$v')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "set_min_borrow_collateral_usd scheduled (${floor_wad} WAD)."
}

set_position_manager_cmd() {
    local manager=$1
    local is_active=$2
    case "$is_active" in
        true|false) ;;
        *) die "setPositionManager: second arg must be true or false (got '${is_active}')" ;;
    esac
    local args_json
    args_json=$(jq -nc --arg a "$manager" --argjson b "$is_active" '[{address:$a},{bool:$b}]')
    local salt
    salt=$(gen_salt "set_position_manager" "$args_json")

    local op_id
    op_id=$(schedule_via_proposer \
        set_position_manager \
        "$(admin_op SetPositionManager "$(jq -nc --arg a "$manager" '$a')" "$(jq -nc --argjson b "$is_active" '$b')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "set_position_manager scheduled (${manager} -> ${is_active})."
}

transfer_ctrl_ownership_cmd() {
    local new_owner=$1
    local live_until=$2
    local args_json
    args_json=$(jq -nc --arg o "$new_owner" --argjson l "$live_until" '[{address:$o},{u32:$l}]')
    local salt
    salt=$(gen_salt "transfer_ctrl_ownership" "$args_json")
    local op_id
    op_id=$(schedule_via_proposer \
        transfer_ownership \
        "$(admin_op TransferCtrlOwnership "$(jq -nc --arg o "$new_owner" --argjson l "$live_until" \
            '{new_owner:$o, live_until_ledger:$l}')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "Controller ownership transfer scheduled to ${new_owner}."
}

migrate_controller_cmd() {
    local version=$1
    local args_json
    args_json=$(jq -nc --argjson v "$version" '[{u32:$v}]')
    local salt
    salt=$(gen_salt "migrate" "$args_json")
    local op_id
    op_id=$(schedule_via_proposer \
        migrate "$(admin_op MigrateController "$(jq -nc --argjson v "$version" '$v')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "Controller migrate scheduled (version ${version})."
}

validate_governance_role() {
    case "$1" in
        ORACLE|PROPOSER|EXECUTOR|CANCELLER) return 0 ;;
        *)
            echo "ERROR: Invalid governance role '$1'. Use ORACLE, PROPOSER, EXECUTOR, or CANCELLER." >&2
            exit 1
            ;;
    esac
}

grant_gov_role_cmd() {
    local account=$1
    local role=$2
    validate_governance_role "$role"
    local salt
    salt=$(gen_salt "grant_governance_role" "$(jq -nc --arg a "$account" --arg r "$role" '{account:$a,role:$r}')")
    local op_id
    op_id=$(schedule_via_gov_self_proposer \
        grant_gov_role \
        "$(admin_op GrantGovRole "$(jq -nc --arg a "$account" --arg r "$role" '{account:$a, role:$r}')")" \
        "$salt" \
        grant_role "$(jq -nc --arg a "$account" --arg r "$role" '[{address:$a},{symbol:$r}]')")
    schedule_and_maybe_execute "$op_id"
    echo "Governance role ${role} grant scheduled for ${account}."
}

revoke_gov_role_cmd() {
    local account=$1
    local role=$2
    validate_governance_role "$role"
    local salt
    salt=$(gen_salt "revoke_governance_role" "$(jq -nc --arg a "$account" --arg r "$role" '{account:$a,role:$r}')")
    local op_id
    op_id=$(schedule_via_gov_self_proposer \
        revoke_gov_role \
        "$(admin_op RevokeGovRole "$(jq -nc --arg a "$account" --arg r "$role" '{account:$a, role:$r}')")" \
        "$salt" \
        revoke_role "$(jq -nc --arg a "$account" --arg r "$role" '[{address:$a},{symbol:$r}]')")
    schedule_and_maybe_execute "$op_id"
    echo "Governance role ${role} revoke scheduled for ${account}."
}

has_role_cmd() {
    local account=$1
    local role=$2
    local gov
    gov=$(get_governance)
    invoke_view "$gov" has_role --account "$account" --role "$role"
}

show_info() {
    echo "=== Deployment info (${NETWORK}) ==="
    local gov_alias
    gov_alias=$(stellar contract alias show governance --network "$NETWORK" 2>/dev/null || echo "not deployed")
    local ctrl_alias
    ctrl_alias=$(stellar contract alias show controller --network "$NETWORK" 2>/dev/null || echo "not deployed")
    local agg_alias
    agg_alias=$(stellar contract alias show aggregator --network "$NETWORK" 2>/dev/null || echo "not set")
    echo "Signer:     $(get_signer_address)"
    echo "Governance: ${gov_alias} (controller owner; all admin ops route through it)"
    echo "Controller: ${ctrl_alias}"
    echo "Pool:       $(get_pool)"

    echo "Aggregator (local alias, NOT chain-verified): ${agg_alias}"
    echo "Aggregator (networks.json, NOT chain-verified): $(get_aggregator_address 2>/dev/null || echo 'not set (set networks.json or AGGREGATOR_CONTRACT)')"
    echo "Accumulator (networks.json, NOT chain-verified): $(get_accumulator_address 2>/dev/null || echo 'not set (required for claimRevenue)')"
    echo "  NOTE: controller has no get_aggregator/get_accumulator/get_position_limits/get_hub"
    echo "  view, and neither controller nor governance exposes is_paused. The lines above"
    echo "  and 'listHubs'/'checkDelay' read local config, not chain truth, for those fields."
    echo "Pool WASM Hash: $(get_network_value "pool_wasm_hash")"
    echo "Spoke ID Map: $(jq -c --arg network "$NETWORK" '.[$network].spoke_ids // {}' "$NETWORKS_FILE")"
    echo "Reflector CEX: $(get_cex_oracle)"
    echo "Reflector DEX: $(get_dex_oracle)"
    echo "Reflector FX:  $(get_fx_oracle)"
    echo "RedStone adapter: $(get_redstone_adapter)"
    echo "XOXNO oracle adapter (networks.json, NOT chain-verified): $(get_oracle_adapter_address 2>/dev/null || echo 'not set (make <network> deployOracleAdapter)')"

    local agg_addr adapter_addr
    if agg_addr=$(get_aggregator_address 2>/dev/null); then
        echo "Aggregator owner (chain-verified): $(invoke_view "$agg_addr" admin 2>/dev/null | tail -n1 || echo 'read failed')"
    fi
    if adapter_addr=$(get_oracle_adapter_address 2>/dev/null); then
        echo "Oracle adapter owner (chain-verified): $(invoke_view "$adapter_addr" get_owner 2>/dev/null | tail -n1 || echo 'read failed')"
    fi

    echo "RedStone markets: $(jq -r '[.markets[] | select(any(.oracle.sources[]?; .Feed.provider.RedStone? != null)) | .name] | if length == 0 then "none" else join(", ") end' "$MARKET_CONFIG_FILE" 2>/dev/null || echo "n/a")"
}

check_delay() {
    local live cfg
    live=$(min_delay_ledgers)
    cfg=$(get_network_value "timelock_min_delay_ledgers")
    echo "Timelock min delay: live=${live} ledgers, configured target=${cfg} ledgers" >&2
    if [ -n "$cfg" ] && [ "$cfg" != "null" ] && [ "$live" -lt "$cfg" ] 2>/dev/null; then
        cat >&2 <<EOF

EOF
    fi
    return 0
}

list_hubs() {
    echo "Hubs (${NETWORK}) referenced by ${MARKET_CONFIG_FILE}:"
    echo "  NOTE: the controller has no get_hub view; this reads the LOCAL id map in"
    echo "  networks.json, not the on-chain HubConfig.is_active flag." >&2
    local h mapped name only_disabled
    for h in $(jq -r '[.markets[].hub_id] | map(select(. != null)) | unique | .[]' "$MARKET_CONFIG_FILE"); do
        name=""
        if [ -f "$HUBS_FILE" ]; then
            name=$(jq -r --arg h "$h" '.[$h].name // empty' "$HUBS_FILE")
        fi
        only_disabled=$(jq -r --argjson h "$h" '
            ([.markets[] | select(.hub_id == $h and .enabled != false)] | length) == 0 and
            ([.markets[] | select(.hub_id == $h)] | length) > 0
        ' "$MARKET_CONFIG_FILE")
        mapped=$(get_mapped_hub_id "$h")
        if [ -n "$mapped" ] && [ "$mapped" != "null" ]; then
            echo "  hub ${h}${name:+ (${name})} -> on-chain ${mapped}"
        elif [ "$only_disabled" = "true" ]; then
            echo "  hub ${h}${name:+ (${name})} -> deferred (only referenced by enabled=false markets)"
        else
            echo "  hub ${h}${name:+ (${name})} -> not created (created on first createMarket/setupAllMarkets)"
        fi
    done
}

list_oracles() {
    list_references
    echo "=== Configured market oracles (${NETWORK}) ===" >&2
    local m i n src refs status
    for m in $(jq -r '.markets[].name' "$MARKET_CONFIG_FILE"); do
        refs=$(market_oracle_ref_dependencies "$m" | tr '\n' ',' | sed 's/,$//')
        [ -z "$refs" ] && refs="-"
        status=$(jq -r --arg m "$m" '
            first(.markets[] | select(.name == $m)) |
            if .enabled == false then " [enabled=false]" else "" end
        ' "$MARKET_CONFIG_FILE")
        jq -r --arg m "$m" --arg refs "$refs" --arg status "$status" 'first(.markets[] | select(.name == $m)) |
            "\(.name)\($status) (hub \(.hub_id // "?")): sources=\((.oracle.sources // [])|length) scaled_refs=\($refs) stale=\(.oracle.max_price_stale_seconds // "?")s tolerance=[\(.oracle.tolerance.upper_ratio_bps // "?")/\(.oracle.tolerance.lower_ratio_bps // "?")] sanity=[\(.oracle.min_sanity_price_wad // "?") .. \(.oracle.max_sanity_price_wad // "?")] independence=\(.oracle.independence // "?")"' \
            "$MARKET_CONFIG_FILE" >&2
        n=$(jq -r --arg m "$m" 'first(.markets[] | select(.name == $m)) | (.oracle.sources // []) | length' "$MARKET_CONFIG_FILE")
        i=0
        while [ "$i" -lt "$n" ]; do
            src=$(jq -c --arg m "$m" --argjson i "$i" \
                'first(.markets[] | select(.name == $m)) | .oracle.sources[$i]' "$MARKET_CONFIG_FILE")
            describe_oracle_source "  source[$i]" "$src"
            i=$((i + 1))
        done
    done
}

ORACLE_FEEDS_FILE="$SCRIPT_DIR/${NETWORK}/oracle_feeds.json"

get_oracle_adapter_address() {
    local addr
    addr=$(jq -r ".\"$NETWORK\".xoxno_oracle_adapter // empty" "$NETWORKS_FILE")
    if [ -z "$addr" ] || [ "$addr" = "null" ]; then
        echo ""
        return 1
    fi
    echo "$addr"
}

_oracle_asset_json() {
    local tag=$1 value=$2
    case "$tag" in
        Stellar) printf '{"Stellar":"%s"}' "$value" ;;
        Other)   printf '{"Other":"%s"}' "$value" ;;
        *) die "Unknown asset.tag '${tag}' (expected Stellar or Other)" ;;
    esac
}

configure_oracle_feeds() {
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}. Run: make ${NETWORK} deployOracleAdapter"
    [ -f "$ORACLE_FEEDS_FILE" ] || die "Feeds config file not found: $ORACLE_FEEDS_FILE"

    echo "=== Configuring oracle feeds on ${NETWORK} (adapter ${adapter}) ===" >&2
    local count i feed_id tag value asset_json errfile out rc feed_enabled skipped
    count=$(jq '.feeds | length' "$ORACLE_FEEDS_FILE")
    skipped=$(jq -r '[.feeds[] | select(.enabled == false) | .feed_id] | join(", ")' "$ORACLE_FEEDS_FILE")
    if [ -n "$skipped" ]; then
        echo "  Skipping disabled feeds (enabled=false): ${skipped}" >&2
    fi
    for ((i = 0; i < count; i++)); do
        feed_enabled=$(jq -r ".feeds[$i].enabled != false" "$ORACLE_FEEDS_FILE")
        if [ "$feed_enabled" != "true" ]; then
            continue
        fi
        feed_id=$(jq -r ".feeds[$i].feed_id" "$ORACLE_FEEDS_FILE")
        tag=$(jq -r ".feeds[$i].asset.tag" "$ORACLE_FEEDS_FILE")
        value=$(jq -r ".feeds[$i].asset.value" "$ORACLE_FEEDS_FILE")
        asset_json=$(_oracle_asset_json "$tag" "$value")

        echo "  add_feed ${feed_id} -> ${asset_json}" >&2
        errfile=$(mktemp)
        out=$(stellar contract invoke --id "$adapter" $SOURCE_FLAG --network "$NETWORK" \
            -- add_feed --feed_id "$feed_id" --asset "$asset_json" 2>"$errfile") && rc=0 || rc=$?
        if [ "$rc" -ne 0 ]; then

            if grep -qiE 'FeedAlreadyMapped|Error\(Contract, #12\)' "$errfile"; then
                echo "    already mapped, skipping" >&2
            else
                cat "$errfile" >&2
                rm -f "$errfile"
                die "add_feed failed for ${feed_id}"
            fi
        fi
        rm -f "$errfile"
    done
    echo "=== Oracle feeds configured (${NETWORK}) ===" >&2
}

reconfigure_oracle_feeds() {
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}. Run: make ${NETWORK} deployOracleAdapter"
    [ -f "$ORACLE_FEEDS_FILE" ] || die "Feeds config file not found: $ORACLE_FEEDS_FILE"

    echo "=== Reconfiguring oracle feeds on ${NETWORK} (adapter ${adapter}) ===" >&2
    echo "  Each feed: remove_feed (wipe) then add_feed (mapping + allowlist + FeedOwner)" >&2
    local count i feed_id tag value asset_json errfile rc feed_enabled skipped
    count=$(jq '.feeds | length' "$ORACLE_FEEDS_FILE")
    skipped=$(jq -r '[.feeds[] | select(.enabled == false) | .feed_id] | join(", ")' "$ORACLE_FEEDS_FILE")
    if [ -n "$skipped" ]; then
        echo "  Skipping disabled feeds (enabled=false): ${skipped}" >&2
    fi
    for ((i = 0; i < count; i++)); do
        feed_enabled=$(jq -r ".feeds[$i].enabled != false" "$ORACLE_FEEDS_FILE")
        if [ "$feed_enabled" != "true" ]; then
            continue
        fi
        feed_id=$(jq -r ".feeds[$i].feed_id" "$ORACLE_FEEDS_FILE")
        tag=$(jq -r ".feeds[$i].asset.tag" "$ORACLE_FEEDS_FILE")
        value=$(jq -r ".feeds[$i].asset.value" "$ORACLE_FEEDS_FILE")
        asset_json=$(_oracle_asset_json "$tag" "$value")

        echo "  remove_feed ${asset_json}" >&2
        errfile=$(mktemp)
        stellar contract invoke --id "$adapter" $SOURCE_FLAG --network "$NETWORK" \
            -- remove_feed --asset "$asset_json" 2>"$errfile" && rc=0 || rc=$?
        if [ "$rc" -ne 0 ]; then

            if grep -qiE 'FeedNotMapped|Error\(Contract, #13\)' "$errfile"; then
                echo "    not mapped, skip remove" >&2
            else
                cat "$errfile" >&2
                rm -f "$errfile"
                die "remove_feed failed for ${feed_id}"
            fi
        fi
        rm -f "$errfile"

        echo "  add_feed ${feed_id} -> ${asset_json}" >&2
        errfile=$(mktemp)
        stellar contract invoke --id "$adapter" $SOURCE_FLAG --network "$NETWORK" \
            -- add_feed --feed_id "$feed_id" --asset "$asset_json" 2>"$errfile" && rc=0 || rc=$?
        if [ "$rc" -ne 0 ]; then
            if grep -qiE 'FeedAlreadyMapped|Error\(Contract, #12\)' "$errfile"; then
                echo "    already mapped after remove (unexpected), skipping" >&2
            else
                cat "$errfile" >&2
                rm -f "$errfile"
                die "add_feed failed for ${feed_id}"
            fi
        fi
        rm -f "$errfile"
    done
    echo "=== Oracle feeds reconfigured (${NETWORK}); wait for bot quorum ===" >&2
}

list_oracle_feeds() {
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}."
    echo "=== Oracle adapter feeds (${NETWORK}, ${adapter}) ===" >&2
    invoke_view "$adapter" assets
}

add_oracle_signer() {
    local signer=$1
    [ -n "$signer" ] || die "Usage: $0 addOracleSigner <signer_address>"
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}. Run: make ${NETWORK} deployOracleAdapter"

    echo "=== Adding oracle signer ${signer} on ${NETWORK} (adapter ${adapter}) ===" >&2
    local errfile rc
    errfile=$(mktemp)
    stellar contract invoke --id "$adapter" $SOURCE_FLAG --network "$NETWORK" \
        -- add_signer --signer "$signer" 2>"$errfile" && rc=0 || rc=$?
    if [ "$rc" -ne 0 ]; then

        if grep -qiE 'SignerAlreadyRegistered|Error\(Contract, #4\)' "$errfile"; then
            echo "  already registered, skipping" >&2
        else
            cat "$errfile" >&2
            rm -f "$errfile"
            die "add_signer failed for ${signer}"
        fi
    fi
    rm -f "$errfile"
    echo "=== Signer added (${NETWORK}) ===" >&2
}

_invoke_set_window() {
    local fn=$1 seconds=$2 adapter=$3
    stellar contract invoke --id "$adapter" $SOURCE_FLAG --network "$NETWORK" \
        -- "$fn" --seconds "$seconds" >/dev/null 2>&1
}

configure_oracle_windows() {
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}. Run: make ${NETWORK} deployOracleAdapter"
    [ -f "$ORACLE_FEEDS_FILE" ] || die "Feeds config file not found: $ORACLE_FEEDS_FILE"

    local max_stale age skew
    max_stale=$(jq -r '.max_stale_seconds // empty' "$ORACLE_FEEDS_FILE")
    age=$(jq -r '.max_submission_age_seconds // empty' "$ORACLE_FEEDS_FILE")
    skew=$(jq -r '.max_relative_skew_seconds // empty' "$ORACLE_FEEDS_FILE")
    [ -n "${max_stale}${age}${skew}" ] || die "Set max_stale_seconds, max_submission_age_seconds, and/or max_relative_skew_seconds in ${ORACLE_FEEDS_FILE}"

    echo "=== Configuring oracle windows on ${NETWORK} (adapter ${adapter}) ===" >&2
    echo "  max_stale_seconds=${max_stale:-<unchanged>} max_submission_age_seconds=${age:-<unchanged>} max_relative_skew_seconds=${skew:-<unchanged>}" >&2

    if [ -n "$max_stale" ] && [ -n "$age" ]; then
        { _invoke_set_window set_max_stale_seconds "$max_stale" "$adapter" \
            && _invoke_set_window set_max_submission_age_seconds "$age" "$adapter"; } \
        || { _invoke_set_window set_max_submission_age_seconds "$age" "$adapter" \
            && _invoke_set_window set_max_stale_seconds "$max_stale" "$adapter"; } \
        || die "Failed to apply windows; ensure 60 <= max_submission_age_seconds (${age}) <= max_stale_seconds (${max_stale})"
    elif [ -n "$max_stale" ]; then
        _invoke_set_window set_max_stale_seconds "$max_stale" "$adapter" || die "set_max_stale_seconds failed"
    elif [ -n "$age" ]; then
        _invoke_set_window set_max_submission_age_seconds "$age" "$adapter" || die "set_max_submission_age_seconds failed"
    fi

    if [ -n "$skew" ]; then

        stellar contract invoke --id "$adapter" $SOURCE_FLAG --network "$NETWORK" \
            -- set_max_relative_skew_seconds --seconds "$skew" \
            || die "set_max_relative_skew_seconds failed (must be <= MaxSubmissionAgeSeconds)"
    fi
    echo "=== Oracle windows configured (${NETWORK}) ===" >&2
}

set_oracle_submission_age() {
    local seconds=$1
    [ -n "$seconds" ] || die "Usage: $0 setOracleSubmissionAge <seconds>"
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}."
    echo "=== set_max_submission_age_seconds ${seconds} on ${NETWORK} (adapter ${adapter}) ===" >&2
    stellar contract invoke --id "$adapter" $SOURCE_FLAG --network "$NETWORK" \
        -- set_max_submission_age_seconds --seconds "$seconds"
}

set_oracle_max_stale() {
    local seconds=$1
    [ -n "$seconds" ] || die "Usage: $0 setOracleMaxStale <seconds>"
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}."
    echo "=== set_max_stale_seconds ${seconds} on ${NETWORK} (adapter ${adapter}) ===" >&2
    stellar contract invoke --id "$adapter" $SOURCE_FLAG --network "$NETWORK" \
        -- set_max_stale_seconds --seconds "$seconds"
}

set_oracle_relative_skew() {
    local seconds=$1
    [ -n "$seconds" ] || die "Usage: $0 setOracleRelativeSkew <seconds>"
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}."
    echo "=== set_max_relative_skew_seconds ${seconds} on ${NETWORK} (adapter ${adapter}) ===" >&2
    stellar contract invoke --id "$adapter" $SOURCE_FLAG --network "$NETWORK" \
        -- set_max_relative_skew_seconds --seconds "$seconds"
}

verify_oracle_adapter_windows() {
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}."
    echo "=== Oracle adapter windows (${NETWORK}, ${adapter}) ===" >&2
    echo -n "  max_submission_age_seconds: " >&2
    invoke_view "$adapter" max_submission_age_seconds
    echo -n "  max_stale_seconds: " >&2
    invoke_view "$adapter" max_stale_seconds
    echo -n "  max_relative_skew_seconds: " >&2
    invoke_view "$adapter" max_relative_skew_seconds
}

finalize_oracle_adapter_upgrade() {
    echo "=== Finalizing oracle adapter upgrade on ${NETWORK} (signer=${SIGNER}) ===" >&2
    configure_oracle_windows
    reconfigure_oracle_feeds
    verify_oracle_adapter_windows
    list_oracle_feeds
    echo "" >&2
    echo "Next: wait for stellar bots to re-submit until threshold is met on each feed," >&2
    echo "then probe: make ${NETWORK} queryRedStone <feed_id>" >&2
    echo "=== Oracle adapter upgrade finalize complete (${NETWORK}) ===" >&2
}

_json_addr_vec() {
    jq -nc '$ARGS.positional' --args "$@"
}

set_aggregator_fee() {
    local bps=$1
    [ -n "$bps" ] || die "Usage: $0 setAggregatorFee <bps>"
    local router
    router=$(get_aggregator_address) || die "No aggregator deployed for ${NETWORK}."
    echo "=== set_static_fee ${bps} on aggregator ${router} (${NETWORK}) ===" >&2
    stellar contract invoke --id "$router" $SOURCE_FLAG --network "$NETWORK" \
        -- set_static_fee --fee_bps "$bps"
}

add_aggregator_whitelist() {
    local token=$1
    [ -n "$token" ] || die "Usage: $0 addAggregatorWhitelist <token_address>"
    local router
    router=$(get_aggregator_address) || die "No aggregator deployed for ${NETWORK}."
    echo "=== add_to_whitelist ${token} on aggregator ${router} (${NETWORK}) ===" >&2
    stellar contract invoke --id "$router" $SOURCE_FLAG --network "$NETWORK" \
        -- add_to_whitelist --token "$token"
}

remove_aggregator_whitelist() {
    local token=$1
    [ -n "$token" ] || die "Usage: $0 removeAggregatorWhitelist <token_address>"
    local router
    router=$(get_aggregator_address) || die "No aggregator deployed for ${NETWORK}."
    echo "=== remove_from_whitelist ${token} on aggregator ${router} (${NETWORK}) ===" >&2
    stellar contract invoke --id "$router" $SOURCE_FLAG --network "$NETWORK" \
        -- remove_from_whitelist --token "$token"
}

add_aggregator_referral() {
    local owner=$1 fee_bps=$2
    [ -n "$owner" ] && [ -n "$fee_bps" ] || die "Usage: $0 addAggregatorReferral <owner_address> <fee_bps>"
    local router
    router=$(get_aggregator_address) || die "No aggregator deployed for ${NETWORK}."
    echo "=== add_referral ${owner} ${fee_bps}bps on aggregator ${router} (${NETWORK}) ===" >&2
    stellar contract invoke --id "$router" $SOURCE_FLAG --network "$NETWORK" \
        -- add_referral --owner "$owner" --fee_bps "$fee_bps"
}

set_aggregator_referral_fee() {
    local id=$1 fee_bps=$2
    [ -n "$id" ] && [ -n "$fee_bps" ] || die "Usage: $0 setAggregatorReferralFee <id> <fee_bps>"
    local router
    router=$(get_aggregator_address) || die "No aggregator deployed for ${NETWORK}."
    echo "=== set_referral_fee ${id} ${fee_bps}bps on aggregator ${router} (${NETWORK}) ===" >&2
    stellar contract invoke --id "$router" $SOURCE_FLAG --network "$NETWORK" \
        -- set_referral_fee --id "$id" --fee_bps "$fee_bps"
}

set_aggregator_referral_active() {
    local id=$1 active=$2
    [ -n "$id" ] && [ -n "$active" ] || die "Usage: $0 setAggregatorReferralActive <id> <true|false>"
    local router
    router=$(get_aggregator_address) || die "No aggregator deployed for ${NETWORK}."
    echo "=== set_referral_active ${id} ${active} on aggregator ${router} (${NETWORK}) ===" >&2
    stellar contract invoke --id "$router" $SOURCE_FLAG --network "$NETWORK" \
        -- set_referral_active --id "$id" --active "$active"
}

set_aggregator_referral_owner() {
    local id=$1 new_owner=$2
    [ -n "$id" ] && [ -n "$new_owner" ] || die "Usage: $0 setAggregatorReferralOwner <id> <new_owner>"
    local router
    router=$(get_aggregator_address) || die "No aggregator deployed for ${NETWORK}."
    echo "=== set_referral_owner ${id} -> ${new_owner} on aggregator ${router} (${NETWORK}) ===" >&2
    stellar contract invoke --id "$router" $SOURCE_FLAG --network "$NETWORK" \
        -- set_referral_owner --id "$id" --new_owner "$new_owner"
}

claim_aggregator_admin_fees() {
    local recipient=$1
    shift || true
    [ -n "$recipient" ] && [ $# -ge 1 ] || die "Usage: $0 claimAggregatorAdminFees <recipient> <token> [token...]"
    local router tokens_json
    router=$(get_aggregator_address) || die "No aggregator deployed for ${NETWORK}."
    tokens_json=$(_json_addr_vec "$@")
    echo "=== claim_admin_fees -> ${recipient} on aggregator ${router} (${NETWORK}) ===" >&2
    stellar contract invoke --id "$router" $SOURCE_FLAG --network "$NETWORK" \
        -- claim_admin_fees --recipient "$recipient" --tokens "$tokens_json"
}

sweep_aggregator_balance() {
    local recipient=$1
    shift || true
    [ -n "$recipient" ] && [ $# -ge 1 ] || die "Usage: $0 sweepAggregatorBalance <recipient> <token> [token...]"
    local router tokens_json
    router=$(get_aggregator_address) || die "No aggregator deployed for ${NETWORK}."
    tokens_json=$(_json_addr_vec "$@")
    echo "=== sweep_balance -> ${recipient} on aggregator ${router} (${NETWORK}) ===" >&2
    stellar contract invoke --id "$router" $SOURCE_FLAG --network "$NETWORK" \
        -- sweep_balance --recipient "$recipient" --tokens "$tokens_json"
}

upgrade_aggregator_hash() {
    local hash=$1
    [ -n "$hash" ] || die "Usage: $0 upgradeAggregatorHash <wasm_hash>"
    local router
    router=$(get_aggregator_address) || die "No aggregator deployed for ${NETWORK}."
    echo "=== upgrade aggregator ${router} -> ${hash} (${NETWORK}) ===" >&2
    stellar contract invoke --id "$router" $SOURCE_FLAG --network "$NETWORK" \
        -- upgrade --new_wasm_hash "$hash"
}

upgrade_oracle_adapter_hash() {
    local hash=$1
    [ -n "$hash" ] || die "Usage: $0 upgradeOracleAdapterHash <wasm_hash>"
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}."
    echo "=== upgrade oracle adapter ${adapter} -> ${hash} (${NETWORK}) ===" >&2
    stellar contract invoke --id "$adapter" $SOURCE_FLAG --network "$NETWORK" \
        -- upgrade --new_wasm_hash "$hash"
}

transfer_aggregator_ownership() {
    local new_owner=$1 live_until=$2
    [ -n "$new_owner" ] && [ -n "$live_until" ] || die "Usage: $0 transferAggregatorOwnership <new_owner> <live_until_ledger>"
    local router
    router=$(get_aggregator_address) || die "No aggregator deployed for ${NETWORK}."
    echo "=== transfer_ownership(${new_owner}, ${live_until}) on aggregator ${router} (${NETWORK}) ===" >&2
    stellar contract invoke --id "$router" $SOURCE_FLAG --network "$NETWORK" \
        -- transfer_ownership --new_owner "$new_owner" --live_until_ledger "$live_until"
}

accept_aggregator_ownership() {
    local router
    router=$(get_aggregator_address) || die "No aggregator deployed for ${NETWORK}."
    echo "=== accept_ownership on aggregator ${router} (${NETWORK}); signer must be the pending owner ===" >&2
    stellar contract invoke --id "$router" $SOURCE_FLAG --network "$NETWORK" \
        -- accept_ownership
}

transfer_oracle_adapter_ownership() {
    local new_owner=$1 live_until=$2
    [ -n "$new_owner" ] && [ -n "$live_until" ] || die "Usage: $0 transferOracleAdapterOwnership <new_owner> <live_until_ledger>"
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}."
    echo "=== transfer_ownership(${new_owner}, ${live_until}) on oracle adapter ${adapter} (${NETWORK}) ===" >&2
    stellar contract invoke --id "$adapter" $SOURCE_FLAG --network "$NETWORK" \
        -- transfer_ownership --new_owner "$new_owner" --live_until_ledger "$live_until"
}

accept_oracle_adapter_ownership() {
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}."
    echo "=== accept_ownership on oracle adapter ${adapter} (${NETWORK}); signer must be the pending owner ===" >&2
    stellar contract invoke --id "$adapter" $SOURCE_FLAG --network "$NETWORK" \
        -- accept_ownership
}

get_price() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local hub_assets
    hub_assets=$(build_hub_assets_json "$market_name")
    local ctrl
    ctrl=$(get_controller)
    echo "=== Price for ${market_name} (${asset_address}) ===" >&2
    invoke_view "$ctrl" get_market_indexes_detailed --hub_assets "$hub_assets"
}

get_market_config_view_cmd() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local hub_asset
    hub_asset=$(build_hub_assets_json "$market_name" | jq -c '.[0]')
    local ctrl
    ctrl=$(get_controller)

    echo "=== Market config (base spoke 0) for ${market_name} (${asset_address}) ===" >&2
    invoke_view "$ctrl" get_spoke_asset --spoke_id 0 --hub_asset "$hub_asset"
}

get_index_cmd() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local hub_assets
    hub_assets=$(build_hub_assets_json "$market_name")
    local ctrl
    ctrl=$(get_controller)
    echo "=== Index for ${market_name} (${asset_address}) ===" >&2
    invoke_view "$ctrl" get_market_indexes_detailed --hub_assets "$hub_assets"
}

get_spoke_cmd() {
    local cat_id=$1
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" get_spoke --spoke_id "$cat_id"
}

get_all_markets_cmd() {

    get_all_indexes_cmd
}

get_all_indexes_cmd() {
    local assets_json
    assets_json=$(all_configured_hub_assets)
    local ctrl
    ctrl=$(get_controller)
    echo "=== All market indexes + oracle status (${NETWORK}) ===" >&2
    invoke_view "$ctrl" get_market_indexes_detailed --hub_assets "$assets_json"
}

get_spoke_asset_cmd() {
    local spoke_id=$1
    local market_name=$2
    require_market_address "$market_name" >/dev/null
    local hub_asset
    hub_asset=$(build_hub_assets_json "$market_name" | jq -c '.[0]')
    local ctrl
    ctrl=$(get_controller)
    echo "=== Spoke ${spoke_id} config for ${market_name} ===" >&2
    invoke_view "$ctrl" get_spoke_asset --spoke_id "$spoke_id" --hub_asset "$hub_asset"
}

get_min_borrow_collateral_cmd() {
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" get_min_borrow_collateral_usd
}

account_exists_cmd() {
    local account_id=$1
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" account_exists --account_id "$account_id"
}

is_blend_pool_approved_cmd() {
    local pool=$1
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" is_blend_pool_approved --pool "$pool"
}

max_withdraw_cmd() {
    local account_id=$1 market_name=$2
    local hub_asset
    hub_asset=$(build_hub_assets_json "$market_name" | jq -c '.[0]')
    invoke_view "$(get_controller)" max_withdraw --account_id "$account_id" --hub_asset "$hub_asset"
}

max_supply_cmd() {
    local account_id=$1 market_name=$2
    local hub_asset
    hub_asset=$(build_hub_assets_json "$market_name" | jq -c '.[0]')
    invoke_view "$(get_controller)" max_supply --account_id "$account_id" --hub_asset "$hub_asset"
}

max_borrow_cmd() {
    local account_id=$1 market_name=$2
    local hub_asset
    hub_asset=$(build_hub_assets_json "$market_name" | jq -c '.[0]')
    invoke_view "$(get_controller)" max_borrow --account_id "$account_id" --hub_asset "$hub_asset"
}

get_liquidation_estimate_cmd() {
    local account_id=$1; shift
    local payments_json="[]"
    if [ "$#" -gt 0 ]; then
        local first=1
        payments_json="["
        while [ "$#" -ge 2 ]; do
            local market=$1 amount=$2; shift 2
            local hub_id asset_addr
            hub_id=$(get_market_value "$market" "hub_id")
            asset_addr=$(get_market_value "$market" "asset_address")
            if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
                die "market '${market}' missing hub_id"
            fi
            [ "$first" -eq 0 ] && payments_json+=","
            payments_json+="[{\"hub_id\":$hub_id,\"asset\":\"$asset_addr\"}, \"$amount\"]"
            first=0
        done
        payments_json+="]"
    fi
    invoke_view "$(get_controller)" get_liquidation_estimate \
        --account_id "$account_id" --debt_payments "$payments_json"
}

pool_view_for_market() {
    local fn=$1 market_name=$2
    local hub_asset
    hub_asset=$(build_hub_assets_json "$market_name" | jq -c '.[0]')
    invoke_view "$(get_pool)" "$fn" --hub_asset "$hub_asset"
}

get_utilisation_cmd()  { pool_view_for_market get_utilisation "$1"; }
get_reserves_cmd()     { pool_view_for_market get_reserves "$1"; }
get_supplied_cmd()     { pool_view_for_market get_supplied_amount "$1"; }
get_borrowed_cmd()     { pool_view_for_market get_borrowed_amount "$1"; }
get_deposit_rate_cmd() { pool_view_for_market get_deposit_rate "$1"; }
get_borrow_rate_cmd()  { pool_view_for_market get_borrow_rate "$1"; }
get_revenue_cmd()      { pool_view_for_market get_revenue "$1"; }
get_sync_data_cmd()    { pool_view_for_market get_sync_data "$1"; }

get_bulk_indexes_cmd() {
    local assets_json
    assets_json=$(all_configured_hub_assets)
    echo "=== Pool bulk indexes (${NETWORK}) ===" >&2
    invoke_view "$(get_pool)" get_bulk_indexes --hub_assets "$assets_json"
}

get_health_cmd() {
    local account_id=$1
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" get_health_factor --account_id "$account_id"
}

get_account_cmd() {
    local account_id=$1
    local ctrl
    ctrl=$(get_controller)
    echo "=== Positions for account ${account_id} ===" >&2
    invoke_view "$ctrl" get_account_positions --account_id "$account_id"
    echo "=== Attributes for account ${account_id} ===" >&2
    invoke_view "$ctrl" get_account_attributes --account_id "$account_id"
}

get_collateral_usd_cmd() {
    local account_id=$1
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" get_total_collateral_usd --account_id "$account_id"
}

get_borrow_usd_cmd() {
    local account_id=$1
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" get_total_borrow_usd --account_id "$account_id"
}

get_ltv_usd_cmd() {
    local account_id=$1
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" get_ltv_collateral_usd --account_id "$account_id"
}

get_liq_available_cmd() {
    local account_id=$1
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" get_liquidation_collateral --account_id "$account_id"
}

can_liquidate_cmd() {
    local account_id=$1
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" is_liquidatable --account_id "$account_id"
}

get_collateral_cmd() {
    local account_id=$1
    local market_name=$2
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local hub_asset
    hub_asset=$(build_hub_assets_json "$market_name" | jq -c '.[0]')
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" get_collateral_amount --account_id "$account_id" --hub_asset "$hub_asset"
}

get_borrow_cmd() {
    local account_id=$1
    local market_name=$2
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local hub_asset
    hub_asset=$(build_hub_assets_json "$market_name" | jq -c '.[0]')
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" get_borrow_amount --account_id "$account_id" --hub_asset "$hub_asset"
}

build_reflector_asset_json() {
    local kind=$1
    local value=$2
    case "$kind" in
        stellar|Stellar|0)

            printf '{"Stellar":"%s"}' "$value"
            ;;
        other|Other|1)
            printf '{"Other":"%s"}' "$value"
            ;;
        *)
            echo "ERROR: kind must be 'stellar' or 'other' (got '$kind')" >&2
            exit 1
            ;;
    esac
}

query_reflector_cmd() {
    local oracle=$1
    if [ -z "$oracle" ]; then
        echo "Usage: $0 queryReflector <oracle_address>" >&2
        exit 1
    fi
    echo "=== Reflector metadata (${oracle}) ===" >&2
    echo "decimals:" >&2
    invoke_view "$oracle" decimals
    echo "resolution (seconds per bucket):" >&2
    invoke_view "$oracle" resolution
}

query_reflector_price_cmd() {
    local oracle=$1
    local kind=$2
    local value=$3
    if [ -z "$oracle" ] || [ -z "$kind" ] || [ -z "$value" ]; then
        echo "Usage: $0 queryReflectorPrice <oracle> stellar|other <symbol_or_sac>" >&2
        exit 1
    fi
    local asset_json
    asset_json=$(build_reflector_asset_json "$kind" "$value")
    echo "=== lastprice on ${oracle} for ${kind}(${value}) ===" >&2
    invoke_view "$oracle" lastprice --asset "$asset_json"
}

query_reflector_twap_cmd() {
    local oracle=$1
    local kind=$2
    local value=$3
    local records=${4:-3}
    if [ -z "$oracle" ] || [ -z "$kind" ] || [ -z "$value" ]; then
        echo "Usage: $0 queryReflectorTwap <oracle> stellar|other <symbol_or_sac> [records=3]" >&2
        exit 1
    fi
    local asset_json
    asset_json=$(build_reflector_asset_json "$kind" "$value")
    echo "=== prices on ${oracle} for ${kind}(${value}), ${records} records ===" >&2
    invoke_view "$oracle" prices --asset "$asset_json" --records "$records"
}

query_redstone_cmd() {
    local feed_id=$1
    local adapter=${2:-$(get_redstone_adapter)}
    if [ -z "$feed_id" ] || [ -z "$adapter" ] || [ "$adapter" = "null" ]; then
        echo "Usage: $0 queryRedStone <feed_id> [adapter_contract]" >&2
        exit 1
    fi
    local feed_ids_json
    feed_ids_json=$(jq -nc --arg feed "$feed_id" '[$feed]')
    echo "=== RedStone adapter (${adapter}) feed_id=${feed_id} ===" >&2
    echo "read_price_data_for_feed:" >&2
    invoke_view "$adapter" read_price_data_for_feed --feed_id "$feed_id"
    echo "read_timestamp:" >&2
    invoke_view "$adapter" read_timestamp --feed_id "$feed_id"
    echo "read_prices:" >&2
    invoke_view "$adapter" read_prices --feed_ids "$feed_ids_json"
}

oracle_union_tag() {
    jq -r 'if type == "object" and has("tag") then .tag else keys_unsorted[0] end'
}

oracle_union_value() {
    jq -c 'if type == "object" and has("values") then (.values[0] // null) else .[keys_unsorted[0]] end'
}

describe_reflector_asset() {
    jq -r '
        def tag: if type == "object" and has("tag") then .tag else keys_unsorted[0] end;
        def value: if type == "object" and has("values") then (.values[0] // "") else .[keys_unsorted[0]] end;
        "\(tag):\(value)"
    '
}

describe_read_mode() {
    jq -r '
        def tag: if type == "object" and has("tag") then .tag else keys_unsorted[0] end;
        def value: if type == "object" and has("values") then (.values[0] // 0) else (.[keys_unsorted[0]] // 0) end;
        if tag == "Twap" then "Twap(" + (value | tostring) + ")" else tag end
    '
}

describe_oracle_source() {
    local label=$1
    local source_json=$2
    if [ -z "$source_json" ] || [ "$source_json" = "null" ]; then
        echo "[${label}] not configured" >&2
        return
    fi

    local shape provider_tag body feed_decimals feed_stale
    shape=$(printf '%s' "$source_json" | oracle_union_tag)
    case "$shape" in
        Feed)
            body=$(printf '%s' "$source_json" | oracle_union_value)
            feed_decimals=$(printf '%s' "$body" | jq -r '.decimals // "input"')
            feed_stale=$(printf '%s' "$body" | jq -r '.max_stale_seconds // "input"')
            provider_tag=$(printf '%s' "$body" | jq -c '.provider' | oracle_union_tag)
            body=$(printf '%s' "$body" | jq -c '.provider' | oracle_union_value)
            ;;
        Scaled)
            local quote factor_json
            quote=$(printf '%s' "$source_json" | jq -c '.Scaled.quote // empty')
            factor_json=$(printf '%s' "$source_json" | jq -c '.Scaled.factor // empty')
            feed_decimals=$(printf '%s' "$factor_json" | jq -r '.decimals // "input"')
            feed_stale=$(printf '%s' "$factor_json" | jq -r '.max_stale_seconds // "input"')
            provider_tag=$(printf '%s' "$factor_json" | jq -c '.provider' | oracle_union_tag)
            body=$(printf '%s' "$factor_json" | jq -c '.provider' | oracle_union_value)
            echo "[${label}] Scaled quote=${quote} factor_provider=${provider_tag}" >&2
            ;;
        Reflector|RedStone|Xoxno)

            provider_tag="$shape"
            body=$(printf '%s' "$source_json" | oracle_union_value)
            feed_decimals="input"
            feed_stale="input"
            ;;
        *)
            echo "[${label}] ${shape}: ${source_json}" >&2
            return
            ;;
    esac

    case "$provider_tag" in
        Reflector)
            local contract asset read_mode
            contract=$(printf '%s' "$body" | jq -r '.contract // empty')
            asset=$(printf '%s' "$body" | jq -c '.asset' | describe_reflector_asset)
            read_mode=$(printf '%s' "$body" | jq -c '.read_mode' | describe_read_mode)
            echo "[${label}] Reflector contract=${contract} asset=${asset} read_mode=${read_mode} decimals=${feed_decimals} max_stale=${feed_stale}" >&2
            ;;
        RedStone|Xoxno)

            local contract feed_id nature
            contract=$(printf '%s' "$body" | jq -r '.contract // empty')
            feed_id=$(printf '%s' "$body" | jq -r '.feed_id // empty')
            nature=$(printf '%s' "$body" | jq -r '.nature // empty')
            echo "[${label}] ${provider_tag} nature=${nature} contract=${contract} feed_id=${feed_id} decimals=${feed_decimals} max_stale=${feed_stale}" >&2
            ;;
        *)
            echo "[${label}] unknown provider ${provider_tag}: ${source_json}" >&2
            ;;
    esac
}

get_oracle_cmd() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local hub_assets
    hub_assets=$(build_hub_assets_json "$market_name")
    local ctrl
    ctrl=$(get_controller)

    echo "=== Oracle price components for ${market_name} (${asset_address}) ===" >&2
    echo "Note: the raw stored oracle config is no longer a readable view; showing live price components." >&2
    invoke_view "$ctrl" get_market_indexes_detailed --hub_assets "$hub_assets"
}

get_reflector_cmd() {
    echo "getReflector is deprecated; showing generic Oracle V2 wiring." >&2
    get_oracle_cmd "$1"
}

case "$1" in
    "listMarkets")
        list_markets
        ;;
    "listSpokes")
        list_spokes
        ;;
    "addSpoke")
        if [ -z "$2" ]; then
            echo "Usage: $0 addSpoke <category_id>"
            list_spokes
            exit 1
        fi
        add_spoke "$2"
        ;;
    "addAssetToSpoke")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 addAssetToSpoke <category_id> <asset_name>"
            list_spokes
            exit 1
        fi
        add_asset_to_spoke "$2" "$3"
        ;;
    "editAssetInSpoke")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 editAssetInSpoke <category_id> <asset_name>"
            list_spokes
            exit 1
        fi
        edit_asset_in_spoke "$2" "$3"
        ;;
    "setupAllSpokes")

        export REAPPLY_ON_DONE=${REAPPLY_ON_DONE:-0}
        validate_configs
        setup_all_spokes
        ;;
    "validateConfigs")
        validate_configs
        ;;
    "listOps")
        list_ops
        ;;
    "executeReady")
        execute_ready_ops
        ;;
    "checkDelay")
        check_delay
        ;;
    "listHubs")
        list_hubs
        ;;
    "listOracles")
        list_oracles
        ;;
    "configureOracleFeeds")
        configure_oracle_feeds
        ;;
    "reconfigureOracleFeeds")
        reconfigure_oracle_feeds
        ;;
    "listOracleFeeds")
        list_oracle_feeds
        ;;
    "addOracleSigner")
        if [ -z "$2" ]; then
            echo "Usage: $0 addOracleSigner <signer_address>" >&2
            exit 1
        fi
        add_oracle_signer "$2"
        ;;
    "configureOracleWindows")
        configure_oracle_windows
        ;;
    "setOracleSubmissionAge")
        set_oracle_submission_age "$2"
        ;;
    "setOracleMaxStale")
        set_oracle_max_stale "$2"
        ;;
    "setOracleRelativeSkew")
        set_oracle_relative_skew "$2"
        ;;
    "verifyOracleAdapterWindows")
        verify_oracle_adapter_windows
        ;;
    "finalizeOracleAdapterUpgrade")
        finalize_oracle_adapter_upgrade
        ;;
    "setAggregatorFee")
        set_aggregator_fee "$2"
        ;;
    "addAggregatorWhitelist")
        add_aggregator_whitelist "$2"
        ;;
    "removeAggregatorWhitelist")
        remove_aggregator_whitelist "$2"
        ;;
    "addAggregatorReferral")
        add_aggregator_referral "$2" "$3"
        ;;
    "setAggregatorReferralFee")
        set_aggregator_referral_fee "$2" "$3"
        ;;
    "setAggregatorReferralActive")
        set_aggregator_referral_active "$2" "$3"
        ;;
    "setAggregatorReferralOwner")
        set_aggregator_referral_owner "$2" "$3"
        ;;
    "claimAggregatorAdminFees")
        shift
        claim_aggregator_admin_fees "$@"
        ;;
    "sweepAggregatorBalance")
        shift
        sweep_aggregator_balance "$@"
        ;;
    "upgradeAggregatorHash")
        upgrade_aggregator_hash "$2"
        ;;
    "upgradeOracleAdapterHash")
        upgrade_oracle_adapter_hash "$2"
        ;;
    "transferAggregatorOwnership")
        transfer_aggregator_ownership "$2" "$3"
        ;;
    "acceptAggregatorOwnership")
        accept_aggregator_ownership
        ;;
    "transferOracleAdapterOwnership")
        transfer_oracle_adapter_ownership "$2" "$3"
        ;;
    "acceptOracleAdapterOwnership")
        accept_oracle_adapter_ownership
        ;;
    "createHub")
        if [ -z "$2" ]; then
            echo "Usage: $0 createHub <hub_id>" >&2
            exit 1
        fi
        ensure_hub "$2"
        ;;
    "removeSpoke")
        if [ -z "$2" ]; then
            echo "Usage: $0 removeSpoke <spoke_id>" >&2
            exit 1
        fi
        remove_spoke_cmd "$2"
        ;;
    "removeAssetFromSpoke")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 removeAssetFromSpoke <spoke_id> <market>" >&2
            exit 1
        fi
        remove_asset_from_spoke_cmd "$2" "$3"
        ;;
    "setSpokeLiquidationCurve")
        if [ -z "$2" ] || [ -z "$3" ] || [ -z "$4" ] || [ -z "$5" ]; then
            echo "Usage: $0 setSpokeLiquidationCurve <spoke_id> <target_hf_wad> <hf_for_max_bonus_wad> <bonus_factor_bps>" >&2
            exit 1
        fi
        set_spoke_liquidation_curve_cmd "$2" "$3" "$4" "$5"
        ;;
    "revokeBlendPool")
        if [ -z "$2" ]; then
            echo "Usage: $0 revokeBlendPool <pool_contract_id>" >&2
            exit 1
        fi
        revoke_blend_pool_cmd "$2"
        ;;
    "setPositionLimits")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 setPositionLimits <max_supply_positions> <max_borrow_positions>" >&2
            exit 1
        fi
        set_position_limits_cmd "$2" "$3"
        ;;
    "setMinBorrowCollateralUsd")
        if [ -z "$2" ]; then
            echo "Usage: $0 setMinBorrowCollateralUsd <floor_wad>" >&2
            exit 1
        fi
        set_min_borrow_collateral_cmd "$2"
        ;;
    "setPositionManager")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 setPositionManager <manager_address> <true|false>" >&2
            exit 1
        fi
        set_position_manager_cmd "$2" "$3"
        ;;
    "transferCtrlOwnership")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 transferCtrlOwnership <new_owner> <live_until_ledger>" >&2
            exit 1
        fi
        transfer_ctrl_ownership_cmd "$2" "$3"
        ;;
    "migrateController")
        if [ -z "$2" ]; then
            echo "Usage: $0 migrateController <version>" >&2
            exit 1
        fi
        migrate_controller_cmd "$2"
        ;;
    "createMarket")
    if [ -z "$2" ]; then
        echo "Usage: $0 createMarket <market_name>"
        list_markets
        exit 1
    fi
    ensure_hub "$(get_market_value "$2" "hub_id")"
    create_market "$2"
    ;;
    "updateMarketParams")
        if [ -z "$2" ]; then
            echo "Usage: $0 updateMarketParams <market_name>"
            list_markets
            exit 1
        fi
        update_market_params "$2"
        ;;
    "configureMarketOracle")
        if [ -z "$2" ]; then
            echo "Usage: $0 configureMarketOracle <market_name>"
            list_markets
            exit 1
        fi
        configure_market_oracle "$2"
        ;;
    "configureReferenceOracle")
        if [ -z "$2" ]; then
            echo "Usage: $0 configureReferenceOracle <ref_name>"
            list_references
            exit 1
        fi
        configure_reference_oracle "$2"
        ;;
    "listReferences")
        list_references
        ;;
    "setupAllReferenceOracles")
        export REAPPLY_ON_DONE=${REAPPLY_ON_DONE:-0}
        validate_configs
        setup_all_reference_oracles
        ;;
    "editOracleTolerance")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 editOracleTolerance <market> <tolerance_bps>"
            list_markets
            exit 1
        fi
        edit_oracle_tolerance "$2" "$3"
        ;;
    "updateIndexes")
        if [ -z "$2" ]; then
            echo "Usage: $0 updateIndexes <market_name> [market_name...]"
            list_markets
            exit 1
        fi
        shift
        update_indexes "$@"
        ;;
    "claimRevenue")
        if [ -z "$2" ]; then
            echo "Usage: $0 claimRevenue <market_name> [market_name...]"
            list_markets
            exit 1
        fi
        shift
        claim_revenue "$@"
        ;;
    "claimRevenueAll")
        claim_revenue_all
        ;;
    "setupAllMarkets")
        export REAPPLY_ON_DONE=${REAPPLY_ON_DONE:-0}
        validate_configs

        setup_all_markets
        ;;
    "setupAll")
        export REAPPLY_ON_DONE=${REAPPLY_ON_DONE:-0}
        validate_configs

        setup_all_markets
        setup_all_spokes
        echo "=== Full setup complete ==="
        ;;
    "whitelistBlendPools")
        whitelist_blend_pools
        ;;
    "configureSpokeCurves")
        configure_spoke_curves
        ;;
    "approveBlendPools")
        whitelist_blend_pools
        ;;
    "setAggregator")
        set_aggregator
        ;;
    "setPriceAggregator")
        set_price_aggregator
        ;;
    "setAccumulator")
        set_accumulator
        ;;
    "supply")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 supply <market> <amount_raw> [<account_id:0>] [<spoke_id:0>]" >&2
            list_markets >&2
            exit 1
        fi
        supply_position "$2" "$3" "$4" "$5"
        ;;
    "borrow")
        if [ -z "$2" ] || [ -z "$3" ] || [ -z "$4" ]; then
            echo "Usage: $0 borrow <market> <amount_raw> <account_id>" >&2
            exit 1
        fi
        borrow_position "$2" "$3" "$4"
        ;;
    "withdraw")
        if [ -z "$2" ] || [ -z "$3" ] || [ -z "$4" ]; then
            echo "Usage: $0 withdraw <market> <amount_raw> <account_id>" >&2
            exit 1
        fi
        withdraw_position "$2" "$3" "$4"
        ;;
    "pause")
        pause_protocol
        ;;
    "unpause")
        unpause_protocol
        ;;
    "executeOp")
        if [ -z "$2" ]; then
            echo "Usage: $0 executeOp <op-id>" >&2
            echo "Replays a locally-scheduled op through governance execute after the delay." >&2
            exit 1
        fi
        execute_op "$2"
        ;;
    "cancelOp")
        if [ -z "$2" ]; then
            echo "Usage: $0 cancelOp <op-id>" >&2
            exit 1
        fi
        cancel_op "$2"
        ;;
    "opState")
        if [ -z "$2" ]; then
            echo "Usage: $0 opState <op-id>" >&2
            exit 1
        fi
        op_state "$2"
        ;;
    "awaitOp")
        if [ -z "$2" ]; then
            echo "Usage: $0 awaitOp <op-id>" >&2
            exit 1
        fi
        await_op_ready "$2"
        ;;
    "upgradeControllerHash")
        schedule_upgrade_controller "$2"
        ;;
    "upgradeGovernanceHash")
        schedule_upgrade_governance "$2"
        ;;
    "updateDelay")
        schedule_update_delay "$2"
        ;;
    "transferGovOwnership")
        schedule_transfer_gov_ownership "$2" "$3"
        ;;
    "upgradePoolHash")
        schedule_upgrade_pool "$2"
        ;;
    "deployPool")
        schedule_deploy_pool "$2"
        ;;
    "deployPositionNft")
        schedule_deploy_position_nft "$2"
        ;;
"grantGovRole")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 grantGovRole <account> <role>" >&2
            echo "Governance roles: ORACLE | PROPOSER | EXECUTOR | CANCELLER (timelocked)" >&2
            exit 1
        fi
        grant_gov_role_cmd "$2" "$3"
        ;;
    "revokeGovRole")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 revokeGovRole <account> <role>" >&2
            exit 1
        fi
        revoke_gov_role_cmd "$2" "$3"
        ;;
    "hasRole")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 hasRole <account> <role>" >&2
            exit 1
        fi
        has_role_cmd "$2" "$3"
        ;;
    "info")
        show_info
        ;;
    "getPrice")
        if [ -z "$2" ]; then echo "Usage: $0 getPrice <market>" >&2; list_markets >&2; exit 1; fi
        get_price "$2"
        ;;
    "getMarket")
        if [ -z "$2" ]; then echo "Usage: $0 getMarket <market>" >&2; list_markets >&2; exit 1; fi
        get_market_config_view_cmd "$2"
        ;;
    "getIndex")
        if [ -z "$2" ]; then echo "Usage: $0 getIndex <market>" >&2; list_markets >&2; exit 1; fi
        get_index_cmd "$2"
        ;;
    "getAllMarkets")
        get_all_markets_cmd
        ;;
    "getAllIndexes")
        get_all_indexes_cmd
        ;;
    "getSpoke")
        if [ -z "$2" ]; then echo "Usage: $0 getSpoke <category_id>" >&2; list_spokes >&2; exit 1; fi
        get_spoke_cmd "$2"
        ;;
    "getSpokeAsset")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 getSpokeAsset <spoke_id> <market>" >&2; list_markets >&2; exit 1
        fi
        get_spoke_asset_cmd "$2" "$3"
        ;;
    "getMinBorrowCollateralUsd")
        get_min_borrow_collateral_cmd
        ;;
    "accountExists")
        if [ -z "$2" ]; then echo "Usage: $0 accountExists <account_id>" >&2; exit 1; fi
        account_exists_cmd "$2"
        ;;
    "isBlendPoolApproved")
        if [ -z "$2" ]; then echo "Usage: $0 isBlendPoolApproved <pool_contract_id>" >&2; exit 1; fi
        is_blend_pool_approved_cmd "$2"
        ;;
    "maxWithdraw")
        if [ -z "$2" ] || [ -z "$3" ]; then echo "Usage: $0 maxWithdraw <account_id> <market>" >&2; exit 1; fi
        max_withdraw_cmd "$2" "$3"
        ;;
    "maxSupply")
        if [ -z "$2" ] || [ -z "$3" ]; then echo "Usage: $0 maxSupply <account_id> <market>" >&2; exit 1; fi
        max_supply_cmd "$2" "$3"
        ;;
    "maxBorrow")
        if [ -z "$2" ] || [ -z "$3" ]; then echo "Usage: $0 maxBorrow <account_id> <market>" >&2; exit 1; fi
        max_borrow_cmd "$2" "$3"
        ;;
    "getLiquidationEstimate")
        if [ -z "$2" ]; then
            echo "Usage: $0 getLiquidationEstimate <account_id> [<market> <amount>]..." >&2; exit 1
        fi
        acc=$2; shift 2
        get_liquidation_estimate_cmd "$acc" "$@"
        ;;
    "getUtilisation")
        if [ -z "$2" ]; then echo "Usage: $0 getUtilisation <market>" >&2; list_markets >&2; exit 1; fi
        get_utilisation_cmd "$2"
        ;;
    "getReserves")
        if [ -z "$2" ]; then echo "Usage: $0 getReserves <market>" >&2; list_markets >&2; exit 1; fi
        get_reserves_cmd "$2"
        ;;
    "getSupplied")
        if [ -z "$2" ]; then echo "Usage: $0 getSupplied <market>" >&2; list_markets >&2; exit 1; fi
        get_supplied_cmd "$2"
        ;;
    "getBorrowed")
        if [ -z "$2" ]; then echo "Usage: $0 getBorrowed <market>" >&2; list_markets >&2; exit 1; fi
        get_borrowed_cmd "$2"
        ;;
    "getDepositRate")
        if [ -z "$2" ]; then echo "Usage: $0 getDepositRate <market>" >&2; list_markets >&2; exit 1; fi
        get_deposit_rate_cmd "$2"
        ;;
    "getBorrowRate")
        if [ -z "$2" ]; then echo "Usage: $0 getBorrowRate <market>" >&2; list_markets >&2; exit 1; fi
        get_borrow_rate_cmd "$2"
        ;;
    "getRevenue")
        if [ -z "$2" ]; then echo "Usage: $0 getRevenue <market>" >&2; list_markets >&2; exit 1; fi
        get_revenue_cmd "$2"
        ;;
    "getSyncData")
        if [ -z "$2" ]; then echo "Usage: $0 getSyncData <market>" >&2; list_markets >&2; exit 1; fi
        get_sync_data_cmd "$2"
        ;;
    "getBulkIndexes")
        get_bulk_indexes_cmd
        ;;
    "getHealth")
        if [ -z "$2" ]; then echo "Usage: $0 getHealth <account_id>" >&2; exit 1; fi
        get_health_cmd "$2"
        ;;
    "getAccount")
        if [ -z "$2" ]; then echo "Usage: $0 getAccount <account_id>" >&2; exit 1; fi
        get_account_cmd "$2"
        ;;
    "getCollateralUsd")
        if [ -z "$2" ]; then echo "Usage: $0 getCollateralUsd <account_id>" >&2; exit 1; fi
        get_collateral_usd_cmd "$2"
        ;;
    "getBorrowUsd")
        if [ -z "$2" ]; then echo "Usage: $0 getBorrowUsd <account_id>" >&2; exit 1; fi
        get_borrow_usd_cmd "$2"
        ;;
    "getLtvUsd")
        if [ -z "$2" ]; then echo "Usage: $0 getLtvUsd <account_id>" >&2; exit 1; fi
        get_ltv_usd_cmd "$2"
        ;;
    "getLiqAvailable")
        if [ -z "$2" ]; then echo "Usage: $0 getLiqAvailable <account_id>" >&2; exit 1; fi
        get_liq_available_cmd "$2"
        ;;
    "canLiquidate")
        if [ -z "$2" ]; then echo "Usage: $0 canLiquidate <account_id>" >&2; exit 1; fi
        can_liquidate_cmd "$2"
        ;;
    "getCollateral")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 getCollateral <account_id> <market>" >&2; exit 1
        fi
        get_collateral_cmd "$2" "$3"
        ;;
    "getBorrow")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 getBorrow <account_id> <market>" >&2; exit 1
        fi
        get_borrow_cmd "$2" "$3"
        ;;
    "queryReflector")
        query_reflector_cmd "$2"
        ;;
    "queryReflectorPrice")
        query_reflector_price_cmd "$2" "$3" "$4"
        ;;
    "queryReflectorTwap")
        query_reflector_twap_cmd "$2" "$3" "$4" "$5"
        ;;
    "queryRedStone")
        query_redstone_cmd "$2"
        ;;
    "getOracle")
        if [ -z "$2" ]; then
            echo "Usage: $0 getOracle <market>" >&2
            list_markets >&2
            exit 1
        fi
        get_oracle_cmd "$2"
        ;;
    "getReflector")
        if [ -z "$2" ]; then
            echo "Usage: $0 getReflector <market>" >&2
            list_markets >&2
            exit 1
        fi
        get_reflector_cmd "$2"
        ;;
    *)
        echo "Stellar Lending Protocol — Configuration Script"
        echo ""
        echo "Usage: NETWORK=$NETWORK $0 <command> [args...]"
        echo ""
        echo "Config validation:"
        echo "  validateConfigs                 Cross-check markets/spokes/networks JSON (runs before setupAll*)"
        echo ""
        echo "Markets (writes):"
        echo "  listMarkets                     List configured markets (marks enabled=false)"
        echo "  createMarket <name>             Deploy market from config (works even if enabled=false)"
        echo "  configureMarketOracle <name>    Configure full market oracle from config"
        echo "                                  (auto-configures oracle Ref dependencies first)"
        echo "  configureReferenceOracle <name> set_oracle(PriceKey::Ref) from .references[]"
        echo "  setupAllReferenceOracles        All Refs required by Scaled markets"
        echo "  listReferences                  Reference oracles from markets.json"
        echo "  editOracleTolerance <m> <tol>   Edit a market's oracle tolerance band (bps)"
        echo "  updateIndexes <name> [...]      Sync indexes for one or more markets"
        echo "  setupAllMarkets                 Idempotently configure enabled markets only"
        echo "                                  (skips markets with enabled=false; omit field = enabled)"
        echo ""
        echo "Hubs / Spokes (writes):"
        echo "  listHubs                        Hubs referenced by config + on-chain mapping"
        echo "  createHub <id>                  Ensure hub exists (idempotent; ascending ids)"
        echo "  listSpokes                      List configured spoke categories (marks enabled=false)"
        echo "  addSpoke <id>                   Create spoke category from config"
        echo "  addAssetToSpoke <id> <asset>    Add asset to spoke from config"
        echo "  editAssetInSpoke <id> <asset>   Push updated per-spoke risk params from config"
        echo "  removeAssetFromSpoke <id> <m>   Timelocked remove_asset_from_spoke"
        echo "  removeSpoke <id>                Timelocked remove_spoke (deprecates category)"
        echo "  setupAllSpokes                  Idempotently configure enabled spokes/assets only"
        echo ""
        echo "Timelock (admin writes are scheduled then executed after the delay):"
        echo "  Admin verbs (createMarket, configureMarketOracle, spoke,"
        echo "  setAggregator, editOracleTolerance, ...) SCHEDULE a governance op and, by default"
        echo "  (AUTO_EXECUTE=1), await the min-delay then execute it. Set AUTO_EXECUTE=0"
        echo "  to schedule-only and execute later with executeOp."
        echo "  Scheduling is idempotent AND re-apply-aware: an op already Waiting/Ready"
        echo "  is reused; toggling back to a previously-executed setting automatically"
        echo "  re-applies at a fresh salt generation (direct verbs). Bulk setupAll* runs"
        echo "  in converge mode (REAPPLY_ON_DONE=0): executed ops (local record) are"
        echo "  treated as applied unless an on-chain probe proves drift. SALT_NONCE=<n>"
        echo "  = manual override. On-chain storage keeps pending ops only."
        echo "  listOps                         All recorded ops with live state"
        echo "  executeReady                    Execute every recorded op that is Ready"
        echo "  executeOp <op-id>               Execute a locally-scheduled, ready op"
        echo "  cancelOp <op-id>                Cancel a pending op (CANCELLER)"
        echo "  opState <op-id>                 Unset | Waiting | Ready | Done"
        echo "  awaitOp <op-id>                 Poll until the op is Ready"
        echo "  NOTE: oracle ops (configureMarketOracle, configureReferenceOracle, editOracleTolerance) schedule a"
        echo "  governance-resolved struct; executeOp re-derives it via the resolve_* views"
        echo "  (build-only re-encode), so they are CLI-executable like every other op."
        echo ""
        echo "Protocol control (writes, routed through governance):"
        echo "  pause                           GUARDIAN-immediate pause (caller = signer)"
        echo "  unpause                         Timelocked AdminOperation::Unpause (propose → await → execute)"
        echo "  checkDelay                      Compare live timelock delay vs configured target"
        echo "  revokeBlendPool <C...>          Timelocked Blend-pool allow-list remove"
        echo "  setPositionLimits <s> <b>       Timelocked position limits (max supply/borrow positions)"
        echo "  setMinBorrowCollateralUsd <wad> Timelocked min borrow-collateral floor"
        echo "  setPositionManager <addr> <t|f> Timelocked position-manager toggle"
        echo "  transferCtrlOwnership <a> <l>   Timelocked controller ownership handoff"
        echo "  migrateController <version>     Timelocked controller migrate"
        echo "  grantGovRole <account> <role>   Grant role (PROPOSER|EXECUTOR|CANCELLER|ORACLE|GUARDIAN; timelocked)"
        echo "  revokeGovRole <account> <role>  Revoke governance role (timelocked)"
        echo "  upgradeGovernanceHash <hash>    Timelocked governance WASM upgrade"
        echo "  updateDelay <ledgers>           Timelocked min-delay increase (cannot shorten)"
        echo "  transferGovOwnership <addr> <ledger>  Timelocked governance ownership handoff"
        echo "  setAggregator                   Set swap aggregator (networks.json or AGGREGATOR_CONTRACT)"
        echo "  setPriceAggregator              Wire controller to the governance-deployed price aggregator"
        echo "  setAccumulator                  Set revenue treasury (networks.json accumulator or ACCUMULATOR_CONTRACT)"
        echo "  Env: AGGREGATOR_CONTRACT, ACCUMULATOR_CONTRACT, AWAIT_MAX_WAIT_SECONDS"
        echo "  setupAll                        Enabled markets + spokes only; no deploy/unpause"
        echo "  claimRevenue <name> [...]       Claim revenue one or more markets"
        echo "  claimRevenueAll                 Claim revenue for every enabled market"
        echo "  whitelistBlendPools | approveBlendPools   Approve Blend V2 pools from configs/${NETWORK}/blend.json (timelocked)"
        echo "  configureSpokeCurves            Apply per-spoke liquidation_curve overrides from configs/${NETWORK}/spokes.json (timelocked)"
        echo ""
        echo "Quick views (reads):"
        echo "  info                            Deployment addresses & signer"
        echo "  listOracles                     Reference + per-market oracle wiring from config"
        echo "  configureOracleFeeds           add_feed for enabled entries in \${NETWORK}/oracle_feeds.json"
        echo "  reconfigureOracleFeeds         remove_feed then add_feed for enabled entries (wipe + rebuild FeedOwner)"
        echo "  finalizeOracleAdapterUpgrade   windows + reconfigure feeds + verify (post-Wasm)"
        echo "  verifyOracleAdapterWindows     Print live max_submission_age / max_stale / relative_skew"
        echo "  setOracleRelativeSkew <secs>   Set max_relative_skew_seconds (<= submission age)"
        echo "  configureOracleWindows         Apply max_submission_age_seconds/max_stale_seconds from \${NETWORK}/oracle_feeds.json"
        echo "  setOracleSubmissionAge <secs>  Set the tight aggregation inclusion window (>=60, <= max_stale)"
        echo "  setOracleMaxStale <secs>       Set the cache TTL (>= submission-age window)"
        echo "  listOracleFeeds                Live feed index from the deployed xoxno_oracle_adapter"
        echo ""
        echo "Aggregator + oracle adapter admin (standalone, direct invoke, no timelock):"
        echo "  setAggregatorFee <bps>"
        echo "  addAggregatorWhitelist <token> / removeAggregatorWhitelist <token>"
        echo "  addAggregatorReferral <owner> <fee_bps>"
        echo "  setAggregatorReferralFee <id> <fee_bps> / setAggregatorReferralActive <id> <true|false>"
        echo "  setAggregatorReferralOwner <id> <new_owner>"
        echo "  claimAggregatorAdminFees <recipient> <token> [token...]"
        echo "  sweepAggregatorBalance <recipient> <token> [token...]"
        echo "  upgradeAggregatorHash <wasm_hash>       (make <net> upgradeAggregator builds+uploads+invokes)"
        echo "  upgradeOracleAdapterHash <wasm_hash>    (make <net> upgradeOracleAdapter builds+uploads+invokes)"
        echo "  Ownership handoff (OZ Ownable, two-step transfer -> accept):"
        echo "    transferAggregatorOwnership <new_owner> <live_until_ledger>"
        echo "    acceptAggregatorOwnership               Run with SIGNER=<new owner>"
        echo "    transferOracleAdapterOwnership <new_owner> <live_until_ledger>"
        echo "    acceptOracleAdapterOwnership            Run with SIGNER=<new owner>"
        echo "  hasRole <account> <role>        Check role membership"
        echo "  getPrice <market>               Oracle price (spot / safe / aggregator + tolerance)"
        echo "  getMarket <market>              Market config (LTV, liq, caps, flags)"
        echo "  getIndex <market>               Supply/borrow index (RAY)"
        echo "  getAllMarkets                   All markets detailed"
        echo "  getAllIndexes                   All market indexes"
        echo "  getSpoke <id>                   Spoke category params"
        echo "  getHealth <id>                  Health factor (RAY)"
        echo "  getAccount <id>                 Positions + attributes"
        echo "  getCollateralUsd <id>           Aggregate collateral in USD"
        echo "  getBorrowUsd <id>               Aggregate borrow in USD"
        echo "  getLtvUsd <id>                  LTV-weighted collateral in USD"
        echo "  getLiqAvailable <id>            Liquidation collateral available"
        echo "  canLiquidate <id>               bool"
        echo "  getCollateral <id> <market>     Per-asset collateral amount"
        echo "  getBorrow <id> <market>         Per-asset borrow amount"
        echo "  getSpokeAsset <spoke_id> <m>    Live per-spoke-per-asset config (any spoke, not just base 0)"
        echo "  accountExists <id>              bool"
        echo "  isBlendPoolApproved <C...>      bool"
        echo "  getMinBorrowCollateralUsd       Protocol-wide borrow floor (WAD)"
        echo "  maxWithdraw <id> <market>       Largest withdraw currently executable"
        echo "  maxSupply <id> <market>         Remaining supply-cap headroom"
        echo "  maxBorrow <id> <market>         Largest borrow currently executable"
        echo "  getLiquidationEstimate <id> [<market> <amount>]...   Seize/repay/refund/bonus estimate"
        echo ""
        echo "Pool views (hub-level utilization/reserves/rates — spokes share hub liquidity):"
        echo "  getUtilisation <market>         Hub utilization"
        echo "  getReserves <market>            Hub cash reserves"
        echo "  getSupplied <market>            Total supplied (hub)"
        echo "  getBorrowed <market>            Total borrowed (hub)"
        echo "  getDepositRate <market>         Live supply APR/APY input"
        echo "  getBorrowRate <market>          Live borrow APR/APY input"
        echo "  getRevenue <market>             Accrued protocol revenue"
        echo "  getSyncData <market>            Raw pool sync snapshot"
        echo "  getBulkIndexes                  get_bulk_indexes for every configured market"
        echo ""
        echo "Oracle probes (debug Oracle V2 wiring):"
        echo "  getOracle <market>                                   Live price components (stored config is write-only; see listOracles)"
        echo "  getReflector <market>                                Deprecated alias for getOracle"
        echo "  queryReflector <oracle>                              decimals + resolution"
        echo "  queryReflectorPrice <oracle> stellar|other <sym|sac> lastprice"
        echo "  queryReflectorTwap  <oracle> stellar|other <sym|sac> [records] prices history"
        echo "  queryRedStone <feed_id> [adapter]                    RedStone multi-feed price data"
        echo ""
        echo "Examples:"
        echo "  NETWORK=testnet $0 getPrice USDC"
        echo "  NETWORK=testnet $0 getHealth 1"
        echo "  NETWORK=testnet $0 getCollateral 1 XLM"
        echo "  SIGNER=ledger NETWORK=mainnet $0 pause"
        ;;
esac
