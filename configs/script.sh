#!/bin/bash
# ===========================================================================
# Stellar Lending Protocol — Deployment & Configuration Script
#
# Shared deployment helper layout:
#   - All values pre-configured in JSON files
#   - CLI references by name/ID, not raw values
#   - Ledger signing via SIGNER=ledger
#
# Usage:
#   NETWORK=testnet ./configs/script.sh <command> [args...]
#
# Config files:
#   configs/networks.json          — RPC URLs, contract addresses
#   configs/emodes.json            — E-Mode categories per network
#   configs/testnet_markets.json   — Market configs (testnet)
#   configs/mainnet_markets.json   — Market configs (mainnet)
# ===========================================================================

set -e

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

NETWORK=${NETWORK:-testnet}
SIGNER=${SIGNER:-deployer}
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

NETWORKS_FILE="$SCRIPT_DIR/networks.json"
EMODES_FILE="$SCRIPT_DIR/emodes.json"
MARKET_CONFIG_FILE="$SCRIPT_DIR/${NETWORK}_markets.json"
BLEND_POOLS_FILE="$SCRIPT_DIR/blend_pools.json"

require_tool() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "ERROR: Missing required tool: $1" >&2
        exit 1
    fi
}

# Fail fast with a message on stderr. Used for mandatory-field guards (e.g. a
# market or spoke asset missing its hub_id) so a misconfig aborts the deploy
# instead of silently defaulting.
die() {
    echo "ERROR: $*" >&2
    exit 1
}

require_tool stellar
require_tool jq

# Source account flag
SIGNER_ADDRESS=$(stellar keys public-key "$SIGNER" 2>/dev/null || stellar keys address "$SIGNER" 2>/dev/null || echo "$SIGNER")
if [ "$SIGNER" = "ledger" ]; then
    SOURCE_FLAG="--source-account $SIGNER_ADDRESS --sign-with-ledger"
else
    SOURCE_FLAG="--source $SIGNER"
fi

# Pin every stellar call to the RPC + passphrase from networks.json. These env
# vars take precedence over the RPC the CLI would resolve from `--network`, so
# the network name is still used for contract-alias resolution while the actual
# endpoint comes from config. Point rpc_url at a reliable provider to avoid the
# public RPC's transient TxBadSeq / read-after-write lag on long deploys. Falls
# back to the CLI's built-in endpoint when rpc_url is absent.
_cfg_rpc=$(jq -r ".\"$NETWORK\".rpc_url // empty" "$NETWORKS_FILE" 2>/dev/null)
_cfg_pass=$(jq -r ".\"$NETWORK\".network_passphrase // empty" "$NETWORKS_FILE" 2>/dev/null)
if [ -n "$_cfg_rpc" ]; then export STELLAR_RPC_URL="$_cfg_rpc"; fi
if [ -n "$_cfg_pass" ]; then export STELLAR_NETWORK_PASSPHRASE="$_cfg_pass"; fi

# ---------------------------------------------------------------------------
# Config readers (using jq)
# ---------------------------------------------------------------------------

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
    if [ ! -f "$EMODES_FILE" ]; then
        echo "ERROR: Config file not found: $EMODES_FILE" >&2
        exit 1
    fi
    if ! jq -e --arg network "$NETWORK" '.[$network] | type == "object"' "$EMODES_FILE" >/dev/null; then
        echo "ERROR: E-mode config for '$NETWORK' not found in $EMODES_FILE" >&2
        exit 1
    fi
}

get_market_value() {
    local market=$1
    local field=$2
    jq -r ".markets[] | select(.name == \"$market\") | .$field" "$MARKET_CONFIG_FILE"
}

get_emode_value() {
    local category_id=$1
    local path=$2
    jq -r ".\"$NETWORK\".\"$category_id\"$path" "$EMODES_FILE"
}

get_controller() {
    stellar contract alias show controller --network "$NETWORK" 2>/dev/null || get_network_value "controller"
}

# Governance owns the controller: all admin writes (markets, oracles, e-modes,
# pause, roles) route through it. Views and operational role-gated calls
# (update_indexes, claim_revenue) stay controller-direct.
get_governance() {
    stellar contract alias show governance --network "$NETWORK" 2>/dev/null || get_network_value "governance"
}

# Aggregator router (WASM contract): networks.json first, then AGGREGATOR_CONTRACT.
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

# Revenue treasury (G-account wallet or contract). Required for claimRevenue (#211
# NoAccumulator if unset). Never falls back to the swap aggregator.
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

# Reflector oracle addresses sourced from networks.json per network.
# Three classes per Reflector's V3 deployment:
#   - CEX: External CEX/FX aggregator, keyed by Other(symbol) e.g. "USDC"
#   - DEX: Stellar Pubnet DEX, keyed by Stellar(SAC) e.g. XLM native SAC
#   - FX:  Fiat exchange rates (forex pairs)
get_cex_oracle() { get_network_value "reflector_cex_oracle"; }
get_dex_oracle() { get_network_value "reflector_dex_oracle"; }
get_fx_oracle()  { get_network_value "reflector_fx_oracle"; }

# Backward-compat alias for existing call sites — defaults to CEX oracle.
get_oracle() { get_cex_oracle; }

get_redstone_adapter() {
    get_network_value "redstone_adapter_contract"
}

get_signer_address() {
    echo "$SIGNER_ADDRESS"
}

invoke_view() {
    # Capture raw stellar output, then pretty-print JSON via jq when available.
    # `local` is declared separately so set -e still propagates stellar failures.
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

# ---------------------------------------------------------------------------
# Timelock (OpenZeppelin governance) schedule / execute / cancel tooling
#
# Governance timelocks every controller-targeted admin op. Each op is queued via
# the generic `propose(proposer, op: AdminOperation, salt)` (validates inputs,
# schedules at min_delay) which returns an operation id. The `AdminOperation` is
# encoded as an explicit ScVal vec `{vec:[{symbol:Variant}, ...payload]}` (see
# admin_op) and passed via `--op-file-path`. After the delay the op is replayed
# through the generic `execute(executor, target=controller, function, args,
# predecessor=0, salt)`; governance-self ops replay through `execute_self`.
#
# To execute later we must replay the EXACT scheduled args (a `Vec<Val>`). We
# persist each scheduled op's (target, function, ScVal args, salt) to a record
# file keyed by op-id under tmp/ops/<network>/, so `executeOp <op-id>` can
# reconstruct the Operation without re-deriving anything.
#
# Oracle ops (configureMarketOracle / editOracleTolerance) schedule the
# governance-RESOLVED struct (MarketOracleConfig / OraclePriceFluctuation), not
# the raw input. The CLI renders a struct view as friendly JSON, which is not the
# ScVal `Vec<Val>` form `execute` needs, so we cannot capture the resolved args
# directly from the view. Instead each oracle op record stores a `resolve` block
# (the governance resolve_* view + its friendly inputs); at execute time
# `resolve_oracle_op_args` runs the view, feeds the friendly result back through
# the controller's typed setter with `--build-only`, and decodes the
# CLI-encoded ScVal args. Those match the proposer's scheduled args byte-for-byte
# because both encode the same `#[contracttype]` struct (canonical sorted map).
# Every other op (primitives and the plain field-map structs: PositionLimits /
# AssetConfigRaw / MarketParamsRaw / InterestRateModel) stores its ScVal args
# directly. All ops are CLI-executable.
# ---------------------------------------------------------------------------

# 32-byte zero predecessor (no dependency), hex form for ScVal/record use.
ZERO_PREDECESSOR_HEX="0000000000000000000000000000000000000000000000000000000000000000"

OPS_DIR="$ROOT_DIR/tmp/ops/$NETWORK"

# Ledger-aware await: poll until the chain sequence reaches the op's ready
# ledger (from get_operation_ledger), then confirm Ready/Done. AWAIT_MAX_WAIT_
# SECONDS caps total wall time (default scales with governance min_delay).
AWAIT_POLL_SECONDS=${AWAIT_POLL_SECONDS:-5}
# ~6s/ledger close + 2h headroom when unset; override for soak runs.
AWAIT_MAX_WAIT_SECONDS=${AWAIT_MAX_WAIT_SECONDS:-0}

ops_dir() {
    mkdir -p "$OPS_DIR"
    echo "$OPS_DIR"
}

op_record_path() {
    echo "$(ops_dir)/$1.json"
}

# Deterministic, unique salt: sha256 over network|function|args-json, truncated
# to 32 bytes (64 hex). Same op + same args ⇒ same salt ⇒ same op-id (idempotent
# re-schedule); different args ⇒ different salt.
gen_salt() {
    local function=$1
    local args_json=$2
    local hash
    if command -v sha256sum >/dev/null 2>&1; then
        hash=$(printf '%s|%s|%s' "$NETWORK" "$function" "$args_json" | sha256sum | cut -c1-64)
    else
        hash=$(printf '%s|%s|%s' "$NETWORK" "$function" "$args_json" | shasum -a 256 | cut -c1-64)
    fi
    echo "$hash"
}

# ScVal JSON element builders (validated against `stellar xdr encode --type
# ScVal`). i128 uses the decimal-string form so large RAY/WAD values stay exact.
scval_address() { jq -nc --arg v "$1" '{address:$v}'; }
scval_symbol()  { jq -nc --arg v "$1" '{symbol:$v}'; }
scval_bytes()   { jq -nc --arg v "$1" '{bytes:$v}'; }
scval_u32()     { jq -nc --argjson v "$1" '{u32:$v}'; }
scval_u64()     { jq -nc --arg v "$1" '{u64:$v}'; }
scval_bool()    { jq -nc --argjson v "$1" '{bool:$v}'; }
scval_i128()    { jq -nc --arg v "$1" '{i128:$v}'; }
scval_vec_u32() {
    # $1 = friendly JSON array of integers (e.g. "[]" or "[1,2]")
    jq -nc --argjson a "$1" '{vec: ($a | map({u32: .}))}'
}

# Struct → ScVal map. ScMap keys MUST be sorted; `--sort-keys`/explicit ordering
# below keeps the symbol keys in canonical order so the host decodes the UDT.
scval_position_limits() {
    # $1 = {"max_supply_positions":N,"max_borrow_positions":M}
    local j=$1
    jq -nc \
        --argjson mb "$(printf '%s' "$j" | jq '.max_borrow_positions')" \
        --argjson ms "$(printf '%s' "$j" | jq '.max_supply_positions')" \
        '{map:[
            {key:{symbol:"max_borrow_positions"},val:{u32:$mb}},
            {key:{symbol:"max_supply_positions"},val:{u32:$ms}}
        ]}'
}

# Build an InterestRateModel ScVal map from a friendly params object carrying the
# 9 rate fields (the RAY fields are i128 decimal strings, reserve_factor is u32).
scval_interest_rate_model() {
    local j=$1
    jq -nc --argjson p "$j" '
        def i(k): {key:{symbol:k}, val:{i128:($p[k] | tostring)}};
        {map: [
            i("base_borrow_rate"),
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

# MarketParamsRaw = InterestRateModel fields + hub caps + flash-loan eligibility
# (is_flashloanable / flashloan_fee) + asset_id (address) + asset_decimals
# (u32). Friendly object must already carry asset_id, asset_decimals, and the
# flash-loan fields. Keys sorted (canonical ScMap order).
scval_market_params() {
    local j=$1
    jq -nc --argjson p "$j" '
        def i(k): {key:{symbol:k}, val:{i128:($p[k] | tostring)}};
        {map: [
            {key:{symbol:"asset_decimals"}, val:{u32:($p.asset_decimals)}},
            {key:{symbol:"asset_id"}, val:{address:($p.asset_id)}},
            i("base_borrow_rate"),
            i("borrow_cap"),
            {key:{symbol:"flashloan_fee"}, val:{u32:($p.flashloan_fee)}},
            {key:{symbol:"is_flashloanable"}, val:{bool:($p.is_flashloanable)}},
            i("max_borrow_rate"),
            i("max_utilization"),
            i("mid_utilization"),
            i("optimal_utilization"),
            {key:{symbol:"reserve_factor"}, val:{u32:($p.reserve_factor)}},
            i("slope1"),
            i("slope2"),
            i("slope3"),
            i("supply_cap")
        ]}'
}

# HubAssetKey ScVal map (sorted keys: asset, hub_id). hub_id is mandatory —
# there is no implicit hub 0.
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

# SpokeAssetConfig ScVal map (11 fields, sorted keys). Flash-loan eligibility
# moved to MarketParamsRaw; spoke caps + paused/frozen/oracle_override live here.
# oracle_override is the MarketOracleConfigOption unit variant `None`.
scval_asset_config() {
    local j=$1
    jq -nc --argjson c "$j" '
        def u(k): {key:{symbol:k}, val:{u32:($c[k])}};
        def b(k): {key:{symbol:k}, val:{bool:($c[k])}};
        {map: [
            {key:{symbol:"borrow_cap"}, val:{i128:(($c.borrow_cap) // 0 | tostring)}},
            b("frozen"),
            b("is_borrowable"),
            b("is_collateralizable"),
            u("liquidation_bonus"),
            u("liquidation_fees"),
            u("liquidation_threshold"),
            u("loan_to_value"),
            {key:{symbol:"oracle_override"}, val:{vec:[{symbol:"None"}]}},
            b("paused"),
            {key:{symbol:"supply_cap"}, val:{i128:(($c.supply_cap) // 0 | tostring)}}
        ]}'
}

# Map a markets-file asset_config (old AssetConfigRaw shape) to a friendly
# SpokeAssetConfig object. Flash-loan eligibility moved to MarketParamsRaw;
# paused/frozen default false; spoke caps default 0 (hub caps live on params);
# oracle_override is the None unit variant. Used for both the propose `--op`
# payload and the scval_asset_config replay map.
#   spoke_config_friendly <asset_config_json> [is_collateralizable] [is_borrowable]
spoke_config_friendly() {
    local cfg=$1 coll=${2:-true} borr=${3:-true}
    jq -nc --argjson c "$cfg" --argjson coll "$coll" --argjson borr "$borr" '{
        is_collateralizable: $coll,
        is_borrowable: $borr,
        paused: ($c.paused // false),
        frozen: ($c.frozen // false),
        loan_to_value: $c.loan_to_value,
        liquidation_threshold: $c.liquidation_threshold,
        liquidation_bonus: $c.liquidation_bonus,
        liquidation_fees: $c.liquidation_fees,
        supply_cap: (($c.supply_cap // 0) | tostring),
        borrow_cap: (($c.borrow_cap // 0) | tostring),
        oracle_override: "None"
    }'
}

# SpokeAssetArgs ScVal map (sorted keys), used for the REPLAY args_json only.
# resolve_op schedules a single SpokeAssetArgs struct, so the stored replay args
# are `[<this map>]`. supply_cap / borrow_cap are i128 decimal strings. (The
# propose `--op` payload uses the friendly form below.)
scval_spoke_args() {
    local hub=$1 asset=$2 spoke=$3 cc=$4 cb=$5 ltv=$6 thr=$7 bonus=$8 sc=$9 bc=${10}
    jq -nc \
        --argjson hub "$hub" \
        --arg asset "$asset" --argjson spoke "$spoke" --argjson cc "$cc" --argjson cb "$cb" \
        --argjson ltv "$ltv" --argjson thr "$thr" --argjson bonus "$bonus" \
        --arg sc "$sc" --arg bc "$bc" \
        '{map:[
            {key:{symbol:"asset"},val:{address:$asset}},
            {key:{symbol:"bonus"},val:{u32:$bonus}},
            {key:{symbol:"borrow_cap"},val:{i128:$bc}},
            {key:{symbol:"can_borrow"},val:{bool:$cb}},
            {key:{symbol:"can_collateral"},val:{bool:$cc}},
            {key:{symbol:"hub_id"},val:{u32:$hub}},
            {key:{symbol:"ltv"},val:{u32:$ltv}},
            {key:{symbol:"spoke_id"},val:{u32:$spoke}},
            {key:{symbol:"supply_cap"},val:{i128:$sc}},
            {key:{symbol:"threshold"},val:{u32:$thr}}
        ]}'
}

# Friendly SpokeAssetArgs object for the propose `--op` payload (plain JSON, Rust
# field names). Address is a bare strkey; i128 caps are decimal strings.
friendly_spoke_args() {
    local hub=$1 asset=$2 spoke=$3 cc=$4 cb=$5 ltv=$6 thr=$7 bonus=$8 sc=$9 bc=${10}
    jq -nc \
        --argjson hub "$hub" \
        --arg asset "$asset" --argjson spoke "$spoke" --argjson cc "$cc" --argjson cb "$cb" \
        --argjson ltv "$ltv" --argjson thr "$thr" --argjson bonus "$bonus" \
        --arg sc "$sc" --arg bc "$bc" \
        '{hub_id:$hub, asset:$asset, spoke_id:$spoke, can_collateral:$cc, can_borrow:$cb,
          ltv:$ltv, threshold:$thr, bonus:$bonus, supply_cap:$sc, borrow_cap:$bc}'
}

# Build an AdminOperation enum value in stellar-cli FRIENDLY-JSON form. The
# `propose`/`execute_self` `op` argument is a TYPED `AdminOperation` enum, so the
# CLI expects friendly JSON (like the typed `--asset "{\"Stellar\":\"...\"}"`
# enum arg in tests/integration/lib/oracle.sh), NOT the explicit `{vec:[...]}`
# ScVal form (that form is only for untyped `Vec<Val>` args such as execute's
# --args).
#   - unit variant (0 fields)   -> the bare string "Variant"
#   - single-field variant      -> {"Variant": <friendly-payload>}
#   - multi-field TUPLE variant   -> {"Variant": [<field0>, <field1>, ...]}
# Payloads are PLAIN friendly JSON (objects/strings/numbers), NOT scval forms:
# Address fields are bare strkey strings, Symbols are bare strings, struct fields
# carry the Rust struct field names, i128 values are decimal strings.
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

# Persist an op record so executeOp/cancelOp can replay it. args_json is the full
# ScVal `Vec<Val>` array (JSON); cli_executable gates executeOp.
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
    jq -nc \
        --arg op_id "$op_id" \
        --arg network "$NETWORK" \
        --arg target "$ctrl" \
        --arg function "$controller_fn" \
        --argjson args "$args_json" \
        --arg predecessor "$ZERO_PREDECESSOR_HEX" \
        --arg salt "$salt_hex" \
        --argjson cli_executable "$cli_executable" \
        '{kind:"controller", op_id:$op_id, network:$network, target:$target, function:$function,
          args:$args, predecessor:$predecessor, salt:$salt,
          cli_executable:$cli_executable}' > "$path"
    echo "  Recorded op $op_id -> $path" >&2
}

# Governance-self ops replay through the generic `execute_self(executor, op,
# salt)`, re-passing the same AdminOperation. The record stores the admin_op
# ScVal JSON; executeOp writes it to a temp file and invokes execute_self with
# `--op-file-path`.
write_gov_self_op_record() {
    local op_id=$1
    local execute_label=$2
    local admin_op_json=$3
    local salt_hex=$4
    local cli_executable=$5
    local path
    path=$(op_record_path "$op_id")
    jq -nc \
        --arg op_id "$op_id" \
        --arg network "$NETWORK" \
        --arg execute_label "$execute_label" \
        --arg salt "$salt_hex" \
        --argjson op "$admin_op_json" \
        --argjson cli_executable "$cli_executable" \
        '{kind:"governance_self", op_id:$op_id, network:$network, execute_label:$execute_label,
          salt:$salt, op:$op, cli_executable:$cli_executable}' > "$path"
    echo "  Recorded governance-self op $op_id -> $path" >&2
}

# Persist an oracle op record whose scheduled args are a governance-RESOLVED
# struct (MarketOracleConfig / OraclePriceFluctuation). The CLI cannot capture
# that struct as ScVal JSON from the friendly view output, so instead of storing
# `args` we store a `resolve` block (the governance view + its friendly inputs).
# At execute time `resolve_oracle_op_args` replays the view through the
# controller's typed setter (`--build-only`) and decodes the ScVal args the CLI
# itself encoded — byte-identical to the proposer's scheduled args because both
# come from the same `#[contracttype]` spec.
write_oracle_op_record() {
    local op_id=$1
    local controller_fn=$2
    local view_fn=$3
    local resolve_args_json=$4
    local salt_hex=$5
    local ctrl
    ctrl=$(get_controller)
    local path
    path=$(op_record_path "$op_id")
    jq -nc \
        --arg op_id "$op_id" \
        --arg network "$NETWORK" \
        --arg target "$ctrl" \
        --arg function "$controller_fn" \
        --arg predecessor "$ZERO_PREDECESSOR_HEX" \
        --arg salt "$salt_hex" \
        --arg view_fn "$view_fn" \
        --argjson resolve_args "$resolve_args_json" \
        '{kind:"controller", op_id:$op_id, network:$network, target:$target, function:$function,
          predecessor:$predecessor, salt:$salt, cli_executable:true,
          resolve:{view_fn:$view_fn, args:$resolve_args}}' > "$path"
    echo "  Recorded oracle op $op_id -> $path" >&2
}

# Resolve an oracle op's scheduled ScVal `Vec<Val>` args at execute time.
#
# Reads the record's `resolve` block, invokes the matching governance view under
# simulation to get the resolved struct (friendly JSON), feeds it back through
# the controller's typed setter with `--build-only` so the CLI re-encodes it to
# ScVal exactly as the proposer scheduled, then decodes that transaction and
# extracts the InvokeContract args. Prints the ScVal `Vec<Val>` JSON array.
resolve_oracle_op_args() {
    local path=$1
    local gov ctrl view_fn function asset
    gov=$(get_governance)
    ctrl=$(jq -r '.target' "$path")
    function=$(jq -r '.function' "$path")
    view_fn=$(jq -r '.resolve.view_fn' "$path")
    asset=$(jq -r '.resolve.args.asset' "$path")

    local resolved
    case "$view_fn" in
        resolve_market_oracle_config)
            local cfg_file
            cfg_file=$(mktemp)
            jq -c '.resolve.args.cfg' "$path" > "$cfg_file"
            resolved=$(stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
                --send=no -- "$view_fn" --asset "$asset" --cfg-file-path "$cfg_file")
            rm -f "$cfg_file"
            local cfg_file2
            cfg_file2=$(mktemp)
            printf '%s' "$resolved" > "$cfg_file2"
            local tx_xdr
            tx_xdr=$(stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
                --build-only --send=no -- "$function" \
                --asset "$asset" --config-file-path "$cfg_file2")
            rm -f "$cfg_file2"
            printf '%s' "$tx_xdr" | stellar tx decode \
                | jq -c 'first(.. | objects | select(has("invoke_contract")) | .invoke_contract.args)'
            ;;
        resolve_oracle_tolerance)
            local tolerance
            tolerance=$(jq -r '.resolve.args.tolerance' "$path")
            resolved=$(stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
                --send=no -- "$view_fn" --tolerance "$tolerance")
            local tol_file
            tol_file=$(mktemp)
            printf '%s' "$resolved" > "$tol_file"
            local tx_xdr
            tx_xdr=$(stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
                --build-only --send=no -- "$function" \
                --asset "$asset" --tolerance-file-path "$tol_file")
            rm -f "$tol_file"
            printf '%s' "$tx_xdr" | stellar tx decode \
                | jq -c 'first(.. | objects | select(has("invoke_contract")) | .invoke_contract.args)'
            ;;
        *)
            echo "ERROR: unknown oracle resolve view '${view_fn}' in ${path}." >&2
            exit 1
            ;;
    esac
}

# Parse the operation id (quoted BytesN hex on the proposer's last output line).
parse_op_id() {
    printf '%s' "$1" | tail -n1 | tr -d '"' | tr -d '[:space:]'
}

# Errors that guarantee the transaction never reached or was rejected by the
# network BEFORE any state change — safe to retry without risking a double
# submit. TxBadSeq is a stale-sequence rejection (the CLI refetches on retry);
# the rest are pre-send connection failures. Ambiguous post-submission timeouts
# are deliberately NOT listed so a tx that may have landed is never re-sent.
RPC_RETRYABLE_RE='TxBadSeq|error sending request|tcp connect error|client error \(Connect\)|Connection refused|connection closed before message completed|dns error'
STELLAR_TX_MAX_RETRIES=${STELLAR_TX_MAX_RETRIES:-4}
STELLAR_TX_RETRY_DELAY=${STELLAR_TX_RETRY_DELAY:-4}

# Run a stellar tx command, retrying only on safe-to-retry transient errors so a
# flaky endpoint or a stale-sequence read does not abort a long deploy. The
# underlying command's stdout is preserved verbatim (callers parse op ids and
# returned addresses from it); diagnostics and errors are forwarded to stderr.
retry_tx() {
    local attempt=1 out rc errfile
    errfile=$(mktemp)
    while :; do
        # `&& rc=0 || rc=$?` captures the command's real exit status while
        # staying exempt from `set -e` (a bare assignment of a failing command
        # substitution would abort the script before the error is inspected).
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

# Core scheduler: invoke the generic `propose(proposer, op, salt)` on governance
# and record the controller op for replay through the generic `execute`.
#   $1 controller_fn       controller thin-setter the op targets (for the record)
#   $2 admin_op_json       AdminOperation friendly JSON ("Variant" | {"Variant":payload})
#   $3 args_json           ScVal Vec<Val> array (JSON) for replay (resolve_op args)
#   $4 cli_executable      true|false (false ⇒ executeOp refuses; oracle ops)
#   $5 salt_hex            deterministic salt (64 hex)
schedule_via_proposer() {
    local controller_fn=$1; shift
    local admin_op_json=$1; shift
    local args_json=$1; shift
    local cli_executable=$1; shift
    local salt_hex=$1; shift
    local gov
    gov=$(get_governance)
    local proposer
    proposer=$(get_signer_address)

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

# Schedule a governance-self admin op (target = governance contract). Replay
# uses the same AdminOperation through `execute_self`, so the op record stores
# the admin_op JSON itself.
#   $1 execute_label       human label for the record/log (e.g. update_delay)
#   $2 admin_op_json       AdminOperation friendly JSON ("Variant" | {"Variant":payload})
#   $3 salt_hex            deterministic salt (64 hex)
schedule_via_gov_self_proposer() {
    local execute_label=$1; shift
    local admin_op_json=$1; shift
    local salt_hex=$1; shift
    local gov
    gov=$(get_governance)
    local proposer
    proposer=$(get_signer_address)

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
    # ~6s/ledger + 2h buffer for mainnet-scale delays.
    echo $(( delay * 6 + 7200 ))
}

op_ready_ledger() {
    local op_id=$1
    local gov
    gov=$(get_governance)
    stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" --send=no \
        -- get_operation_ledger --operation_id "$op_id" | tr -d '"' | tr -d '[:space:]'
}

# Read an operation's lifecycle state as a bare string (Unset|Waiting|Ready|Done).
op_state() {
    local op_id=$1
    local gov
    gov=$(get_governance)
    stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" --send=no \
        -- get_operation_state --operation_id "$op_id" | tr -d '"' | tr -d '[:space:]'
}

# Poll until the op is Ready (Done short-circuits as already executed). Uses
# ledger sequence + get_operation_ledger so mainnet-scale delays are supported.
await_op_ready() {
    local op_id=$1
    local started_at ready_ledger current state max_wait waited unset_seen
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
                echo "  Op ${op_id} Waiting (ledger ${current:-?}/${ready_ledger:-?}, waited ${waited}s/${max_wait}s); sleeping ${AWAIT_POLL_SECONDS}s..." >&2
                sleep "$AWAIT_POLL_SECONDS"
                ;;
            Unset)
                # A just-confirmed schedule can briefly read back Unset on a
                # lagging RPC (read-after-write). Tolerate a few polls before
                # treating it as a genuine never-scheduled / cancelled op.
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

# Execute a governance-self op via the generic execute_self(executor, op, salt),
# re-passing the stored AdminOperation. Self-target ops cannot use the generic
# execute (the timelock rejects target == governance to avoid self-reentry).
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
    # Open execution: a ready op already waited the full delay. Option<Address>
    # executor is passed as JSON null (None).
    retry_tx stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
        -- execute_self \
        --executor null \
        --op-file-path "$op_file" \
        --salt "$salt"
    rm -f "$op_file"
    echo "Executed governance-self op ${op_id}." >&2
}

# Execute a recorded op. Controller ops replay through generic execute;
# governance-self ops use typed execute_* entrypoints.
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
    # Oracle ops carry a `resolve` block instead of stored args: the scheduled
    # struct is re-derived through the governance view at execute time so it
    # matches byte-for-byte. Every other op stores its ScVal args directly.
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
    # Open execution: a ready op was already proposed, validated, and waited the
    # full delay, so triggering it is unprivileged. `Option<Address>` is passed
    # as JSON `null` (None); a bare address is not valid JSON for this arg.
    retry_tx stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
        -- execute \
        --executor null \
        --target "$target" \
        --function "$function" \
        --args-file-path "$args_file" \
        --predecessor "$predecessor" \
        --salt "$salt"
    rm -f "$args_file"
    echo "Executed op ${op_id}." >&2
}

# Cancel a pending op (CANCELLER role). Drops the local record on success.
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

# Schedule, await the delay, then execute — the one-shot setup path. Honors
# AUTO_EXECUTE=0 to schedule-only (record op-id for a later executeOp).
schedule_and_maybe_execute() {
    local op_id=$1
    if [ "${AUTO_EXECUTE:-1}" != "1" ]; then
        echo "Scheduled op ${op_id} (AUTO_EXECUTE=0; run 'executeOp ${op_id}' after the delay)." >&2
        return 0
    fi
    await_op_ready "$op_id"
    execute_op "$op_id"
}

require_static_config

# ---------------------------------------------------------------------------
# List functions
# ---------------------------------------------------------------------------

list_markets() {
    echo "Available markets (${NETWORK}):"
    if [ -f "$MARKET_CONFIG_FILE" ]; then
        jq -r '.markets[] | "  \(.name) — \(.asset_address // "no address")"' "$MARKET_CONFIG_FILE"
    else
        echo "  No config file found: $MARKET_CONFIG_FILE"
    fi
}

list_emode_categories() {
    echo "E-Mode categories (${NETWORK}):"
    if [ -f "$EMODES_FILE" ]; then
        jq -r --arg network "$NETWORK" --slurpfile networks "$NETWORKS_FILE" '
            .[$network] as $cats |
            ($networks[0][$network].emode_category_ids // {}) as $ids |
            $cats | to_entries[] |
            "  \(.key) -> on-chain \($ids[.key] // "unmapped"): \(.value.name) — assets: \(.value.assets | keys | join(", "))"
        ' "$EMODES_FILE"
    else
        echo "  No emodes config found: $EMODES_FILE"
    fi
}

# Pair each market's configured hub_id with its asset_address, producing the
# Vec<HubAssetKey> JSON (`[{"hub_id":<n>,"asset":"<addr>"}, ...]`) the controller
# expects. There is no implicit hub 0: a market missing its hub_id aborts.
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

# ---------------------------------------------------------------------------
# E-Mode functions
# ---------------------------------------------------------------------------

add_emode_category() {
    local category_id=$1

    local name
    name=$(get_emode_value "$category_id" ".name")

    echo "Adding E-Mode category ${category_id}: ${name}" >&2

    # add_spoke() — no on-chain args; risk params are per-asset (e-mode categories
    # are now spokes). The salt is seeded with the config category id so that
    # creating several spokes in one setup run derives distinct timelock op ids
    # (the call args stay []; a shared salt would collide on the second spoke).
    local args_json='[]'
    local salt
    salt=$(gen_salt "add_spoke:${category_id}" "$args_json")

    local op_id
    op_id=$(schedule_via_proposer \
        add_spoke "$(admin_op AddSpoke)" "$args_json" true "$salt")

    if [ "${AUTO_EXECUTE:-1}" != "1" ]; then
        echo "Scheduled e-mode category ${category_id} as op ${op_id} (AUTO_EXECUTE=0)." >&2
        echo "$op_id"
        return 0
    fi

    await_op_ready "$op_id"
    # The controller's add_e_mode_category returns the new on-chain id; the
    # generic execute prints that returned Val on its last line.
    local result
    result=$(execute_op "$op_id" 2>/dev/null)
    local onchain_id
    onchain_id=$(echo "$result" | sed -nE 's/.*([0-9]+).*/\1/p' | tail -n1)
    if [ -z "$onchain_id" ]; then
        echo "ERROR: Could not parse on-chain e-mode category id from execute result: $result" >&2
        exit 1
    fi

    echo "E-Mode category ${category_id} created with on-chain id ${onchain_id}." >&2
    echo "$onchain_id"
}

get_mapped_emode_category_id() {
    local config_category_id=$1
    jq -r --arg network "$NETWORK" --arg config_id "$config_category_id" \
        '.[$network].emode_category_ids[$config_id] // empty' "$NETWORKS_FILE"
}

persist_emode_category_id() {
    local config_category_id=$1
    local onchain_id=$2
    local tmp
    tmp=$(mktemp)
    jq --arg network "$NETWORK" --arg config_id "$config_category_id" --argjson onchain_id "$onchain_id" \
        '.[$network].emode_category_ids = (.[$network].emode_category_ids // {}) |
         .[$network].emode_category_ids[$config_id] = $onchain_id' \
        "$NETWORKS_FILE" > "$tmp" && mv "$tmp" "$NETWORKS_FILE"
}

fetch_emode_category_json() {
    # E-mode reads stay on the controller; only writes route through governance.
    local onchain_id=$1
    local ctrl
    ctrl=$(get_controller)
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        --send=no -- get_spoke --spoke_id "$onchain_id"
}

emode_is_deprecated() {
    local category_json=$1
    printf '%s' "$category_json" | jq -e '.is_deprecated == true' >/dev/null
}

# Content guard for category reuse. Returns 0 when every asset already
# configured on-chain in `category_json` also appears in config category
# `config_category_id`; returns 1 when the on-chain category holds any asset
# this config does not list. An on-chain category with no assets is compatible
# (setup will populate it). On-chain categories carry no name, so a foreign
# category whose assets are a strict subset of this config's assets cannot be
# distinguished here — closing that residual needs an on-chain identity field.
emode_category_assets_match_config() {
    local config_category_id=$1
    local category_json=$2

    # A degraded response missing a readable `.assets` map (null/absent, not an
    # empty `{}`) cannot be verified; refuse reuse instead of masking it as an
    # empty (compatible) category.
    if ! printf '%s' "$category_json" | jq -e '.assets | type == "object"' >/dev/null 2>&1; then
        echo "ERROR: on-chain E-Mode category for config ${config_category_id} has no readable .assets map; refusing to reuse." >&2
        return 1
    fi

    local onchain_assets
    onchain_assets=$(printf '%s' "$category_json" | jq -r '.assets | keys[]')
    # An empty on-chain category is compatible — setup will populate it.
    [ -z "$onchain_assets" ] && return 0

    local expected_addrs=" "
    local asset_name asset_addr
    for asset_name in $(jq -r ".\"$NETWORK\".\"$config_category_id\".assets | keys[]" "$EMODES_FILE"); do
        asset_addr=$(get_market_value "$asset_name" "asset_address")
        # An unresolved asset means the config references something the markets
        # file lacks; fail with that specific reason rather than silently
        # dropping it (which would later mislabel an on-chain asset as foreign).
        if [ -z "$asset_addr" ] || [ "$asset_addr" = "null" ]; then
            echo "ERROR: e-mode config ${config_category_id} lists asset '${asset_name}' missing from the markets file; cannot verify category reuse." >&2
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

# A category only groups assets and tracks deprecation; risk params live on the
# per-asset configs (ensured by `ensure_asset_in_emode`). Reuse therefore
# requires two checks: the category must not be deprecated, and every asset it
# already holds on-chain must belong to this config category — otherwise we
# would silently rewrite a different category's (possibly live) risk params.
ensure_emode_category() {
    local config_category_id=$1
    local mapped_id
    local category_json

    mapped_id=$(get_mapped_emode_category_id "$config_category_id")
    if [ -n "$mapped_id" ] && [ "$mapped_id" != "null" ]; then
        if category_json=$(fetch_emode_category_json "$mapped_id" 2>/dev/null); then
            if emode_is_deprecated "$category_json"; then
                echo "Mapped E-Mode id ${mapped_id} for config ${config_category_id} is deprecated; creating a replacement."
            elif ! emode_category_assets_match_config "$config_category_id" "$category_json"; then
                echo "ERROR: mapped E-Mode id ${mapped_id} for config ${config_category_id} holds assets this config does not list." >&2
                echo "       Refusing to apply config ${config_category_id} to an unverified on-chain category; it may be a different category or have live users." >&2
                echo "       Fix the mapping in ${NETWORKS_FILE}, or deprecate the on-chain category, then re-run." >&2
                return 1
            else
                echo "E-Mode config ${config_category_id} already mapped to on-chain id ${mapped_id}."
                echo "$mapped_id"
                return 0
            fi
        else
            echo "Mapped E-Mode id ${mapped_id} for config ${config_category_id} is not readable; creating a replacement."
        fi
    fi

    if category_json=$(fetch_emode_category_json "$config_category_id" 2>/dev/null); then
        if emode_is_deprecated "$category_json"; then
            echo "On-chain E-Mode id ${config_category_id} is deprecated; creating a new category."
        elif ! emode_category_assets_match_config "$config_category_id" "$category_json"; then
            echo "ERROR: on-chain E-Mode id ${config_category_id} holds assets config category ${config_category_id} does not list." >&2
            echo "       Refusing to reuse it by numeric id; it may be a different category or have live users." >&2
            echo "       Map config ${config_category_id} to the correct on-chain id in ${NETWORKS_FILE}, or deprecate the on-chain category, then re-run." >&2
            return 1
        else
            persist_emode_category_id "$config_category_id" "$config_category_id"
            echo "E-Mode config ${config_category_id} reuses existing on-chain id ${config_category_id}."
            echo "$config_category_id"
            return 0
        fi
    fi

    local onchain_id
    onchain_id=$(add_emode_category "$config_category_id")
    persist_emode_category_id "$config_category_id" "$onchain_id"
    echo "$onchain_id"
}

add_asset_to_emode() {
    local category_id=$1
    local asset_name=$2
    local config_category_id=${3:-$category_id}

    echo "Adding asset ${asset_name} to E-Mode category ${category_id}..."

    local asset_address
    asset_address=$(get_market_value "$asset_name" "asset_address")
    local can_collateral
    can_collateral=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".can_be_collateral")
    local can_borrow
    can_borrow=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".can_be_borrowed")
    local ltv
    ltv=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".ltv")
    local threshold
    threshold=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".liquidation_threshold")
    local bonus
    bonus=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".liquidation_bonus")
    local supply_cap
    supply_cap=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".supply_cap")
    local borrow_cap
    borrow_cap=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".borrow_cap")
    if [ -z "$supply_cap" ] || [ "$supply_cap" = "null" ]; then supply_cap=0; fi
    if [ -z "$borrow_cap" ] || [ "$borrow_cap" = "null" ]; then borrow_cap=0; fi

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
    hub_id=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".hub_id")
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "e-mode asset ${asset_name} (category ${config_category_id}) missing hub_id in ${EMODES_FILE}"
    fi

    # add_asset_to_spoke(SpokeAssetArgs). resolve_op schedules a single
    # SpokeAssetArgs struct, so the replay args_json is one struct element.
    local args_json
    args_json=$(jq -nc \
        --argjson arg "$(scval_spoke_args "$hub_id" "$asset_address" "$category_id" "$can_collateral" \
            "$can_borrow" "$ltv" "$threshold" "$bonus" "$supply_cap" "$borrow_cap")" \
        '[$arg]')
    local salt
    salt=$(gen_salt "add_asset_to_spoke" "$args_json")

    # The propose `--op` payload is the single SpokeAssetArgs in friendly form.
    local admin_op_json
    admin_op_json=$(admin_op AddAssetToSpoke \
        "$(friendly_spoke_args "$hub_id" "$asset_address" "$category_id" "$can_collateral" "$can_borrow" \
            "$ltv" "$threshold" "$bonus" "$supply_cap" "$borrow_cap")")

    local op_id
    op_id=$(schedule_via_proposer \
        add_asset_to_spoke "$admin_op_json" "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"

    echo "Asset ${asset_name} scheduled into E-Mode category ${category_id}."
}

edit_asset_in_emode() {
    local category_id=$1
    local asset_name=$2
    local config_category_id=${3:-$category_id}

    local asset_address
    asset_address=$(get_market_value "$asset_name" "asset_address")
    local can_collateral
    can_collateral=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".can_be_collateral")
    local can_borrow
    can_borrow=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".can_be_borrowed")
    local ltv
    ltv=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".ltv")
    local threshold
    threshold=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".liquidation_threshold")
    local bonus
    bonus=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".liquidation_bonus")
    local supply_cap
    supply_cap=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".supply_cap")
    local borrow_cap
    borrow_cap=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".borrow_cap")
    if [ -z "$supply_cap" ] || [ "$supply_cap" = "null" ]; then supply_cap=0; fi
    if [ -z "$borrow_cap" ] || [ "$borrow_cap" = "null" ]; then borrow_cap=0; fi

    echo "Editing asset ${asset_name} in E-Mode category ${category_id}..." >&2

    local hub_id
    hub_id=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".hub_id")
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "e-mode asset ${asset_name} (category ${config_category_id}) missing hub_id in ${EMODES_FILE}"
    fi

    # edit_asset_in_spoke(SpokeAssetArgs). resolve_op schedules a single
    # SpokeAssetArgs struct, so the replay args_json is one struct element.
    local args_json
    args_json=$(jq -nc \
        --argjson arg "$(scval_spoke_args "$hub_id" "$asset_address" "$category_id" "$can_collateral" \
            "$can_borrow" "$ltv" "$threshold" "$bonus" "$supply_cap" "$borrow_cap")" \
        '[$arg]')
    local salt
    salt=$(gen_salt "edit_asset_in_spoke" "$args_json")

    # The propose `--op` payload is the single SpokeAssetArgs in friendly form.
    local admin_op_json
    admin_op_json=$(admin_op EditAssetInSpoke \
        "$(friendly_spoke_args "$hub_id" "$asset_address" "$category_id" "$can_collateral" "$can_borrow" \
            "$ltv" "$threshold" "$bonus" "$supply_cap" "$borrow_cap")")

    local op_id
    op_id=$(schedule_via_proposer \
        edit_asset_in_spoke "$admin_op_json" "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
}

ensure_asset_in_emode() {
    local category_id=$1
    local asset_name=$2
    local config_category_id=${3:-$category_id}

    local asset_address
    asset_address=$(get_market_value "$asset_name" "asset_address")
    local can_collateral
    can_collateral=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".can_be_collateral")
    local can_borrow
    can_borrow=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".can_be_borrowed")
    local ltv
    ltv=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".ltv")
    local threshold
    threshold=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".liquidation_threshold")
    local bonus
    bonus=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".liquidation_bonus")
    local supply_cap
    supply_cap=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".supply_cap")
    local borrow_cap
    borrow_cap=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".borrow_cap")
    if [ -z "$supply_cap" ] || [ "$supply_cap" = "null" ]; then supply_cap=0; fi
    if [ -z "$borrow_cap" ] || [ "$borrow_cap" = "null" ]; then borrow_cap=0; fi
    local category_json

    if [ -z "$asset_address" ] || [ "$asset_address" = "null" ] || [ "$asset_address" = "" ]; then
        echo "ERROR: No asset address found for ${asset_name} in ${MARKET_CONFIG_FILE}"
        exit 1
    fi

    category_json=$(fetch_emode_category_json "$category_id")
    if printf '%s' "$category_json" | jq -e --arg asset "$asset_address" '.assets[$asset] != null' >/dev/null; then
        if printf '%s' "$category_json" | jq -e \
            --arg asset "$asset_address" \
            --argjson can_collateral "$can_collateral" \
            --argjson can_borrow "$can_borrow" \
            --argjson ltv "$ltv" \
            --argjson threshold "$threshold" \
            --argjson bonus "$bonus" \
            --arg supply_cap "$supply_cap" \
            --arg borrow_cap "$borrow_cap" \
            '.assets[$asset].is_collateralizable == $can_collateral and
             .assets[$asset].is_borrowable == $can_borrow and
             .assets[$asset].loan_to_value == $ltv and
             .assets[$asset].liquidation_threshold == $threshold and
             .assets[$asset].liquidation_bonus == $bonus and
             (.assets[$asset].supply_cap | tostring) == $supply_cap and
             (.assets[$asset].borrow_cap | tostring) == $borrow_cap' >/dev/null; then
            echo "Asset ${asset_name} already configured in E-Mode category ${category_id}."
        else
            edit_asset_in_emode "$category_id" "$asset_name" "$config_category_id"
        fi
    else
        add_asset_to_emode "$category_id" "$asset_name" "$config_category_id"
    fi
}

setup_all_emodes() {
    echo "=== Setting up all E-Mode categories for ${NETWORK} ==="
    local categories
    categories=$(jq -r ".\"$NETWORK\" | keys[]" "$EMODES_FILE")

    for cat_id in $categories; do
        local onchain_id
        # Bare assignment (declared separately so `local` doesn't mask the
        # status): a command substitution inside an `if` condition would suppress
        # `set -e` within ensure_emode_category and its callees, silently
        # continuing on an inner failure or the content guard's `return 1`. With
        # a plain assignment, `set -e` stays active inside the function and
        # aborts the deploy on any non-zero exit; the guard prints the specific
        # reason to stderr before returning.
        onchain_id=$(ensure_emode_category "$cat_id")
        onchain_id=$(printf '%s\n' "$onchain_id" | tail -n1)

        local assets
        assets=$(jq -r ".\"$NETWORK\".\"$cat_id\".assets | keys[]" "$EMODES_FILE")
        for asset_name in $assets; do
            ensure_asset_in_emode "$onchain_id" "$asset_name" "$cat_id"
        done
    done
    echo "=== All E-Mode categories configured ==="
}

# ---------------------------------------------------------------------------
# Hub functions
#
# Hubs are the top-level market containers. There is no implicit hub 0:
# `create_hub` is the only way to mint one and it returns ids 1, 2, … in
# creation order. Every market (`create_liquidity_pool`) and spoke-asset listing
# carries an explicit hub_id, so the hubs referenced by the market config must
# exist on-chain before any market is listed. `create_hub` is a governance-
# timelocked admin op (AdminOperation::CreateHub, no on-chain args) returning the
# new u32 id — mirrors add_spoke / add_e_mode_category.
# ---------------------------------------------------------------------------

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

# Create the hub with config id `expected` unless it is already recorded for this
# network. Hubs mint sequentially, so the on-chain id must equal `expected`
# (distinct hub_ids are processed in ascending order); a mismatch is a hard error.
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

    # create_hub() — no on-chain args; governance schedules AdminOperation::CreateHub
    # and the controller returns the new hub id on execute.
    local args_json='[]'
    local salt
    salt=$(gen_salt "create_hub:${expected}" "$args_json")

    local op_id
    op_id=$(schedule_via_proposer \
        create_hub "$(admin_op CreateHub)" "$args_json" true "$salt")

    if [ "${AUTO_EXECUTE:-1}" != "1" ]; then
        echo "Scheduled hub ${expected} as op ${op_id} (AUTO_EXECUTE=0; execute before listing markets)." >&2
        return 0
    fi

    await_op_ready "$op_id"
    # The generic execute prints the controller's returned hub id on its last line.
    local result onchain_id
    result=$(execute_op "$op_id" 2>/dev/null)
    onchain_id=$(echo "$result" | sed -nE 's/.*([0-9]+).*/\1/p' | tail -n1)
    if [ -z "$onchain_id" ]; then
        die "could not parse on-chain hub id from execute result: ${result}"
    fi
    if [ "$onchain_id" != "$expected" ]; then
        die "create_hub returned id ${onchain_id} but the config expects hub ${expected}; create hubs in ascending order with no gaps (there is no hub 0), or fix ${MARKET_CONFIG_FILE}"
    fi
    persist_hub_id "$expected" "$onchain_id"
    echo "Hub ${expected} created with on-chain id ${onchain_id}." >&2
}

# Create every distinct hub referenced by the market config (ascending order)
# before any market is listed. Idempotent: hubs already recorded in
# networks.json are skipped.
ensure_hubs() {
    echo "=== Ensuring hubs for ${NETWORK} ===" >&2
    local hub_ids
    hub_ids=$(jq -r '[.markets[].hub_id] | map(select(. != null)) | unique | .[]' "$MARKET_CONFIG_FILE")
    if [ -z "$hub_ids" ]; then
        die "no hub_id found on any market in ${MARKET_CONFIG_FILE}"
    fi
    local h
    for h in $hub_ids; do
        ensure_hub "$h"
    done
    echo "=== Hubs ready ===" >&2
}

# ---------------------------------------------------------------------------
# Market functions
# ---------------------------------------------------------------------------

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

    # Existence probe is a controller view; creation writes go via governance.
    # The base spoke-0 listing exists once the market is created.
    if stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" --send=no -- get_spoke_asset --spoke_id 0 --asset "$asset_address" &>/dev/null; then
        echo "Market for ${market_name} already exists, skipping creation."
        return 0
    fi

    # Build MarketParamsRaw JSON: rate model + hub caps + flash-loan eligibility
    # (moved off the per-asset config) + asset_id + asset_decimals.
    local params
    params=$(jq -c --arg decimals "$decimals" \
        ".markets[] | select(.name == \"$market_name\") | .market_params + {
            asset_id: .asset_address,
            asset_decimals: (\$decimals | tonumber),
            is_flashloanable: (.asset_config.is_flashloanable // false),
            flashloan_fee: (.asset_config.flashloan_fee // 0)
        }" \
        "$MARKET_CONFIG_FILE")
    # Markets are deployed in a pending state (base spoke-0 listing not
    # collateralizable/borrowable) so they cannot be used before oracle wiring
    # and explicit activation via edit_asset_in_spoke.
    local raw_config pending_config
    raw_config=$(jq -c \
        ".markets[] | select(.name == \"$market_name\") | .asset_config" \
        "$MARKET_CONFIG_FILE")
    pending_config=$(spoke_config_friendly "$raw_config" false false)

    # Post-audit (T1-7): the controller gates `create_liquidity_pool` behind an
    # admin allow-list. Pre-approve the token first (separate timelocked op,
    # executed before the create op so the allow-list check passes).
    # `approve_token` is idempotent on chain.
    echo "Scheduling token approval for market creation..." >&2
    local approve_args
    approve_args=$(jq -nc --arg t "$asset_address" '[{address:$t}]')
    local approve_salt
    approve_salt=$(gen_salt "approve_token" "$approve_args")
    local approve_op
    approve_op=$(schedule_via_proposer \
        approve_token "$(admin_op ApproveToken "$(jq -nc --arg a "$asset_address" '$a')")" \
        "$approve_args" true "$approve_salt")
    schedule_and_maybe_execute "$approve_op"

    # create_liquidity_pool(hub_id, asset, params, config) — u32 + Address + two
    # field-map structs. The scheduled args equal these inputs (governance
    # validates but does not transform), so they are fully CLI-replayable.
    local params_scval config_scval
    params_scval=$(scval_market_params "$params")
    config_scval=$(scval_asset_config "$pending_config")
    local args_json
    args_json=$(jq -nc \
        --argjson hub_id "$hub_id" \
        --arg asset "$asset_address" \
        --argjson params "$params_scval" \
        --argjson config "$config_scval" \
        '[{u32:$hub_id}, {address:$asset}, $params, $config]')
    local salt
    salt=$(gen_salt "create_liquidity_pool" "$args_json")

    # The propose `--op` payload wraps hub_id + asset + the friendly params/config
    # objects (Rust field names) in CreatePoolArgs.
    local admin_op_json
    admin_op_json=$(admin_op CreateLiquidityPool \
        "$(jq -nc --argjson hub_id "$hub_id" --arg asset "$asset_address" --argjson params "$params" \
            --argjson config "$pending_config" \
            '{hub_id:$hub_id, asset:$asset, params:$params, config:$config}')")

    local op_id
    op_id=$(schedule_via_proposer \
        create_liquidity_pool "$admin_op_json" "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"

    echo "Market ${market_name} scheduled/created."
}

edit_asset_config() {
    local market_name=$1

    echo "Editing asset config for ${market_name}..."

    local asset_address
    asset_address=$(get_market_value "$market_name" "asset_address")
    # EditAssetConfig edits the asset's base spoke-0 listing (SpokeAssetConfig);
    # the collateral/borrow flags come from the markets file.
    local raw_config coll borr config
    raw_config=$(jq -c \
        ".markets[] | select(.name == \"$market_name\") | .asset_config" \
        "$MARKET_CONFIG_FILE")
    coll=$(jq -r '.is_collateralizable // true' <<<"$raw_config")
    borr=$(jq -r '.is_borrowable // true' <<<"$raw_config")
    config=$(spoke_config_friendly "$raw_config" "$coll" "$borr")

    # edit_asset_config maps to EditAssetConfig(Address, SpokeAssetConfig). The
    # replay args_json stays explicit ScVal (Address + SpokeAssetConfig scval map).
    local args_json
    args_json=$(jq -nc \
        --arg asset "$asset_address" \
        --argjson cfg "$(scval_asset_config "$config")" \
        '[{address:$asset}, $cfg]')
    local salt
    salt=$(gen_salt "edit_asset_config" "$args_json")

    # EditAssetConfig(Address, SpokeAssetConfig) is a TUPLE variant -> friendly form
    # {"EditAssetConfig": [<address>, <friendly config>]} (fields in declaration
    # order). TESTNET-CONFIRM: the multi-field-tuple enum friendly shape.
    local admin_op_json
    admin_op_json=$(admin_op EditAssetConfig \
        "$(jq -nc --arg a "$asset_address" '$a')" "$config")

    local op_id
    op_id=$(schedule_via_proposer \
        edit_asset_config "$admin_op_json" "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"

    echo "Asset config scheduled for ${market_name}."
}

# Push the JSON's `market_params` (rate model + max_utilization +
# reserve_factor) onto the pool via the controller's
# `upgrade_liquidity_pool_params` route. Use after changing any
# rate / utilization-ceiling field in the markets JSON.
update_market_params() {
    local market_name=$1

    echo "Updating market params for ${market_name}..."

    local asset_address
    asset_address=$(get_market_value "$market_name" "asset_address")
    # Strip `asset_id` / `asset_decimals` — those are controller-resolved
    # and the InterestRateModel struct does not carry them.
    local params
    params=$(jq -c \
        ".markets[] | select(.name == \"$market_name\") | .market_params" \
        "$MARKET_CONFIG_FILE")

    local hub_id
    hub_id=$(get_market_value "$market_name" "hub_id")
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "market ${market_name} missing hub_id in ${MARKET_CONFIG_FILE}"
    fi

    # upgrade_liquidity_pool_params(hub_asset, InterestRateModel) — HubAssetKey +
    # struct. The replay args_json stays explicit ScVal (HubAssetKey + IRM map).
    local args_json
    args_json=$(jq -nc \
        --argjson hub_asset "$(scval_hub_asset "$asset_address" "$hub_id")" \
        --argjson params "$(scval_interest_rate_model "$params")" \
        '[$hub_asset, $params]')
    local salt
    salt=$(gen_salt "upgrade_liquidity_pool_params" "$args_json")

    # The propose `--op` payload wraps hub_asset + the friendly InterestRateModel
    # (the 9 IRM fields only) in UpgradePoolParamsArgs.
    local irm_friendly
    irm_friendly=$(jq -nc --argjson p "$params" '{
        base_borrow_rate: ($p.base_borrow_rate|tostring),
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

# Push hub supply/borrow caps from `market_params` onto the central pool via
# `update_pool_caps`. Use after changing supply_cap / borrow_cap in the JSON.
update_pool_caps() {
    local market_name=$1

    echo "Updating hub pool caps for ${market_name}..."

    local asset_address
    asset_address=$(get_market_value "$market_name" "asset_address")
    local supply_cap
    supply_cap=$(get_market_value "$market_name" "market_params.supply_cap")
    local borrow_cap
    borrow_cap=$(get_market_value "$market_name" "market_params.borrow_cap")
    supply_cap=${supply_cap:-0}
    borrow_cap=${borrow_cap:-0}

    echo "  Supply cap: ${supply_cap}  Borrow cap: ${borrow_cap}"

    local hub_id
    hub_id=$(get_market_value "$market_name" "hub_id")
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "market ${market_name} missing hub_id in ${MARKET_CONFIG_FILE}"
    fi

    # update_pool_caps(hub_asset, supply_cap, borrow_cap).
    local args_json
    args_json=$(jq -nc \
        --argjson hub_asset "$(scval_hub_asset "$asset_address" "$hub_id")" \
        --arg supply_cap "$supply_cap" \
        --arg borrow_cap "$borrow_cap" \
        '[$hub_asset,{i128:$supply_cap},{i128:$borrow_cap}]')
    local salt
    salt=$(gen_salt "update_pool_caps" "$args_json")

    # The propose `--op` payload wraps the three values in friendly PoolCapsArgs
    # (hub_asset + caps as i128 decimal strings).
    local admin_op_json
    admin_op_json=$(admin_op UpdatePoolCaps \
        "$(jq -nc --argjson hub_id "$hub_id" --arg asset "$asset_address" --arg sc "$supply_cap" --arg bc "$borrow_cap" \
            '{hub_asset:{hub_id:$hub_id, asset:$asset}, supply_cap:$sc, borrow_cap:$bc}')")

    local op_id
    op_id=$(schedule_via_proposer \
        update_pool_caps "$admin_op_json" "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"

    echo "Hub pool caps scheduled for ${market_name}."
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

    # update_indexes takes Vec<HubAssetKey>; each asset is paired with its
    # configured hub_id.
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- update_indexes \
        --caller "$caller" \
        --assets "$assets_json"

    echo "Indexes updated."
}

claim_revenue() {
    # claim_revenue is REVENUE-role operational, not admin: it stays controller-direct.
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

    # claim_revenue takes Vec<HubAssetKey>; each asset is paired with its
    # configured hub_id.
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

    # claim_revenue takes Vec<HubAssetKey>; each asset is paired with its
    # configured hub_id.
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- claim_revenue \
        --caller "$caller" \
        --assets "$hub_assets_json"

    echo "Revenue claimed for all markets."
}

# ---------------------------------------------------------------------------
# Blend V2 migration source allow-list
#
# `migrate_from_blend` only accepts a governance-approved Blend pool as its
# source. `whitelistBlendPools` reads configs/blend_pools.json for the current
# network, checks each pool against the controller view `is_blend_pool_approved`,
# and schedules a timelocked `approve_blend_pool` for any that are missing.
# Already-approved pools are skipped, so re-runs cost no redundant timelock op
# (important on mainnet's multi-day delay).
# ---------------------------------------------------------------------------

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
    # approve_blend_pool(pool) — single Address arg; scheduled args equal the
    # input, so the op is fully CLI-replayable through generic execute.
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
    pools=$(jq -r --arg net "$NETWORK" '(.[$net].pools // [])[] | .address' "$BLEND_POOLS_FILE")
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

set_aggregator() {
    echo "Configuring Aggregator for ${NETWORK}..."
    local router
    if ! router=$(get_aggregator_address); then
        echo "ERROR: No aggregator address for ${NETWORK}. Set networks.json aggregator or AGGREGATOR_CONTRACT." >&2
        exit 1
    fi

    echo "  Aggregator Address: ${router}" >&2

    # set_aggregator(addr) — single Address arg.
    local args_json
    args_json=$(jq -nc --arg a "$router" '[{address:$a}]')
    local salt
    salt=$(gen_salt "set_aggregator" "$args_json")

    local op_id
    op_id=$(schedule_via_proposer \
        set_aggregator "$(admin_op SetAggregator "$(jq -nc --arg a "$router" '$a')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"

    echo "Aggregator scheduled via governance."
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

    # set_accumulator(addr) — single Address arg.
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

# ---------------------------------------------------------------------------
# Position helpers (supply / borrow)
#
# Strategy entry points (multiply, swap_debt, swap_collateral,
# repay_debt_with_collateral) are still defined on the controller but require
# an AggregatorSwap JSON sourced from the off-chain quote server backing the
# in-house swap aggregator. Invoke them via `make invoke` with a swap JSON
# produced by that quote server.
# ---------------------------------------------------------------------------

# `supply` — deposit collateral.
# Args: <market> <amount_raw> [<account_id:0>] [<e_mode_category:0>]
supply_position() {
    local market=$1
    local amount_raw=$2
    local account_id=${3:-0}
    local e_mode_category=${4:-0}

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
    echo "  E-mode:   $e_mode_category  (0 = none)"
    echo "  Asset:    $market ($asset_addr)"
    echo "  Amount:   $amount_raw"
    echo

    # i128 amounts are decimal strings so large raw values stay exact.
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- supply \
        --caller "$caller" \
        --account_id "$account_id" \
        --spoke_id "$e_mode_category" \
        --assets "[[{\"hub_id\":$hub_id,\"asset\":\"$asset_addr\"}, \"$amount_raw\"]]"
}

# `borrow` — open a borrow position against existing collateral.
# Args: <market> <amount_raw> <account_id>
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

    # i128 amounts are decimal strings so large raw values stay exact.
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- borrow \
        --caller "$caller" \
        --account_id "$account_id" \
        --borrows "[[{\"hub_id\":$hub_id,\"asset\":\"$asset_addr\"}, \"$amount_raw\"]]" \
        --to null
}

configure_market_oracle() {
    local market_name=$1

    echo "Configuring market oracle for ${market_name}..."

    # Preflight: every oracle config must carry sanity bounds. On mainnet
    # the `(0, 0)` disabled-sentinel is rejected — that combination is for
    # test setups only. (Codex adversarial-review #4.)
    local missing
    missing=$(jq -r --arg m "$market_name" '
        .markets[] | select(.name == $m) | .oracle |
        (if has("min_sanity_price_wad") and has("max_sanity_price_wad")
            then "" else "missing min_sanity_price_wad / max_sanity_price_wad" end)
    ' "$MARKET_CONFIG_FILE")
    if [ -n "$missing" ]; then
        echo "ERROR: $market_name oracle config $missing" >&2
        exit 1
    fi
    if [ "$NETWORK" = "mainnet" ]; then
        local zero
        zero=$(jq -r --arg m "$market_name" '
            .markets[] | select(.name == $m) | .oracle |
            (if (.min_sanity_price_wad == "0" and .max_sanity_price_wad == "0")
                then "yes" else "no" end)
        ' "$MARKET_CONFIG_FILE")
        if [ "$zero" = "yes" ]; then
            echo "ERROR: $market_name uses (0, 0) sanity-bound sentinel on mainnet" >&2
            exit 1
        fi
    fi

    local asset_address
    asset_address=$(get_market_value "$market_name" "asset_address")
    local cfg_file
    cfg_file=$(mktemp)
    jq -c --arg market "$market_name" '
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
        .markets[] | select(.name == $market) | .oracle | cli_union
    ' "$MARKET_CONFIG_FILE" > "$cfg_file"

    # propose(ConfigureMarketOracle{asset, cfg}) validates+probes the INPUT cfg,
    # then schedules the controller's set_market_oracle_config with the
    # governance-RESOLVED MarketOracleConfig. The CLI can't capture that struct
    # as ScVal from the friendly view output, so the op record stores a `resolve`
    # block (the resolve_market_oracle_config view + the input cfg); executeOp
    # replays the view through the controller's typed setter (`--build-only`) to
    # reconstruct byte-identical args. See resolve_oracle_op_args.
    local gov
    gov=$(get_governance)
    local proposer
    proposer=$(get_signer_address)

    local cfg_json
    cfg_json=$(jq -c . "$cfg_file")
    rm -f "$cfg_file"
    local salt
    local salt_input
    salt_input=$(jq -nc --argjson cfg "$cfg_json" --arg asset "$asset_address" \
        '{asset:$asset, cfg:$cfg}')
    salt=$(gen_salt "set_market_oracle_config" "$salt_input")

    # Generic propose takes the typed AdminOperation. ConfigureMarketOracle wraps
    # ConfigureOracleArgs { asset, cfg: MarketOracleConfigInput }. cfg is the
    # nested friendly cli_union JSON (built above); rather than hand-encode that
    # deep union tree to explicit ScVal, pass the AdminOperation in friendly-enum
    # form via --op-file-path (propose's `op` arg is typed, so the CLI's friendly
    # decoder applies — exactly as the old --cfg-file-path consumed it). The
    # execute/replay side is unaffected: it re-derives the RESOLVED config through
    # the resolve_market_oracle_config view (write_oracle_op_record below).
    local op_file
    op_file=$(mktemp)
    jq -nc --arg asset "$asset_address" --argjson cfg "$cfg_json" \
        '{ConfigureMarketOracle: {asset:$asset, cfg:$cfg}}' > "$op_file"

    echo "Scheduling market oracle config for ${market_name}..." >&2
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
        echo "ERROR: propose ConfigureMarketOracle returned no operation id (output: $out)" >&2
        exit 1
    fi
    local resolve_args
    resolve_args=$(jq -nc --arg asset "$asset_address" --argjson cfg "$cfg_json" \
        '{asset:$asset, cfg:$cfg}')
    write_oracle_op_record "$op_id" "set_market_oracle_config" \
        "resolve_market_oracle_config" "$resolve_args" "$salt"

    echo "Market oracle scheduled for ${market_name} as op ${op_id}." >&2
    schedule_and_maybe_execute "$op_id"
}

# Edit only a market's oracle tolerance bands. propose(EditOracleTolerance{...})
# schedules the controller's set_oracle_tolerance with the governance-RESOLVED
# OraclePriceFluctuation; executeOp re-derives it via resolve_oracle_tolerance.
edit_oracle_tolerance() {
    local market_name=$1
    local tolerance=$2
    if [ -z "$market_name" ] || [ -z "$tolerance" ]; then
        echo "Usage: $0 editOracleTolerance <market> <tolerance_bps>" >&2
        exit 1
    fi

    local asset_address
    asset_address=$(require_market_address "$market_name")
    local gov
    gov=$(get_governance)
    local proposer
    proposer=$(get_signer_address)

    local salt_input
    salt_input=$(jq -nc --arg asset "$asset_address" --argjson t "$tolerance" \
        '{asset:$asset, tolerance:$t}')
    local salt
    salt=$(gen_salt "set_oracle_tolerance" "$salt_input")

    # EditOracleTolerance wraps friendly EditToleranceArgs { asset,
    # tolerance(u32) }. The `--op` payload carries the INPUT tolerance; the
    # controller's RESOLVED OraclePriceFluctuation is re-derived at execute
    # time via the resolve_oracle_tolerance block below.
    local admin_op_json
    admin_op_json=$(admin_op EditOracleTolerance \
        "$(jq -nc --arg asset "$asset_address" --argjson t "$tolerance" \
            '{asset:$asset, tolerance:$t}')")
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
    local resolve_args
    resolve_args=$(jq -nc --arg asset "$asset_address" --argjson t "$tolerance" \
        '{asset:$asset, tolerance:$t}')
    write_oracle_op_record "$op_id" "set_oracle_tolerance" \
        "resolve_oracle_tolerance" "$resolve_args" "$salt"

    echo "Oracle tolerance edit scheduled for ${market_name} as op ${op_id}." >&2
    schedule_and_maybe_execute "$op_id"
}

setup_all_markets() {
    echo "=== Setting up all markets for ${NETWORK} ==="
    # Hubs must exist before any market is listed: create_liquidity_pool reverts
    # HubNotActive for an uncreated hub (there is no implicit hub 0).
    ensure_hubs
    local markets
    markets=$(jq -r '.markets[].name' "$MARKET_CONFIG_FILE")

    for market_name in $markets; do
        create_market "$market_name"
        configure_market_oracle "$market_name"
        edit_asset_config "$market_name"
    done
    echo "=== All markets configured ==="
}

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

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
    jq -c '[.markets[] | select(.asset_address != null and .asset_address != "") | .asset_address]' "$MARKET_CONFIG_FILE"
}

all_configured_hub_assets() {
    jq -c '[.markets[] | select(.asset_address != null and .asset_address != "") | {hub_id, asset: .asset_address}]' "$MARKET_CONFIG_FILE"
}

# ---------------------------------------------------------------------------
# Upgrade / template ops (WASM hash inputs) — scheduled through governance.
#
# The Makefile uploads the WASM and passes the resulting hash here; we schedule
# the matching proposer then await+execute (AUTO_EXECUTE=1) so the upgrade lands
# after the delay. Hashes are BytesN<32>; their scheduled args are fully
# CLI-replayable.
# ---------------------------------------------------------------------------

schedule_set_pool_template() {
    local hash=$1
    if [ -z "$hash" ]; then
        echo "Usage: $0 setPoolTemplate <wasm_hash_hex>" >&2
        exit 1
    fi
    # set_liquidity_pool_template(hash) — single BytesN<32> arg.
    local args_json
    args_json=$(jq -nc --arg h "$hash" '[{bytes:$h}]')
    local salt
    salt=$(gen_salt "set_liquidity_pool_template" "$args_json")
    local op_id
    op_id=$(schedule_via_proposer \
        set_liquidity_pool_template \
        "$(admin_op SetLiquidityPoolTemplate "$(jq -nc --arg h "$hash" '$h')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "Pool template set scheduled (hash ${hash})."
}

schedule_upgrade_controller() {
    local hash=$1
    if [ -z "$hash" ]; then
        echo "Usage: $0 upgradeControllerHash <wasm_hash_hex>" >&2
        exit 1
    fi
    # upgrade(new_wasm_hash) — single BytesN<32> arg.
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
        upgrade_gov "$(admin_op UpgradeGov "$(jq -nc --arg h "$hash" '$h')")" "$salt")
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
        update_gov_delay "$(admin_op UpdateGovDelay "$(jq -nc --argjson d "$new_delay" '$d')")" "$salt")
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
        "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "Governance ownership transfer scheduled to ${new_owner}."
}

# Schedule deploy_pool (no controller args), await, execute, and print the
# deployed pool Address parsed from the execute result's last line.
schedule_deploy_pool() {
    local args_json="[]"
    local salt
    salt=$(gen_salt "deploy_pool" "$args_json")
    local op_id
    op_id=$(schedule_via_proposer \
        deploy_pool "$(admin_op DeployPool)" "$args_json" true "$salt")
    if [ "${AUTO_EXECUTE:-1}" != "1" ]; then
        echo "Scheduled deploy_pool as op ${op_id} (AUTO_EXECUTE=0)." >&2
        echo "$op_id"
        return 0
    fi
    await_op_ready "$op_id"
    local result
    result=$(execute_op "$op_id" 2>/dev/null)
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
    # upgrade_pool(new_wasm_hash) — single BytesN<32> arg.
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

# ---------------------------------------------------------------------------
# Pause / unpause
# ---------------------------------------------------------------------------

pause_protocol() {
    local gov
    gov=$(get_governance)
    stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" -- pause
    echo "Protocol paused on ${NETWORK}."
}

unpause_protocol() {
    local gov
    gov=$(get_governance)
    stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" -- unpause
    echo "Protocol unpaused on ${NETWORK}."
}

# ---------------------------------------------------------------------------
# Oracle circuit-breaker (controller has no operational roles)
# ---------------------------------------------------------------------------

disable_token_oracle_cmd() {
    local asset=$1
    local args_json
    args_json=$(jq -nc --arg a "$asset" '[{address:$a}]')
    local salt
    salt=$(gen_salt "disable_token_oracle" "$args_json")
    local op_id
    op_id=$(schedule_via_proposer \
        disable_token_oracle "$(admin_op DisableTokenOracle "$(jq -nc --arg a "$asset" '$a')")" \
        "$args_json" true "$salt")
    schedule_and_maybe_execute "$op_id"
    echo "disable_token_oracle scheduled for ${asset}."
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

# Governance operational roles (ORACLE / PROPOSER / EXECUTOR / CANCELLER) are
# timelocked via propose(GrantGovRole{...}) / propose(RevokeGovRole{...}).
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
        "$salt")
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
        "$salt")
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

# ---------------------------------------------------------------------------
# Info
# ---------------------------------------------------------------------------

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
    echo "Aggregator: ${agg_alias}"
    echo "Configured Aggregator: $(get_aggregator_address 2>/dev/null || echo 'not set (set networks.json or AGGREGATOR_CONTRACT)')"
    echo "Configured Accumulator: $(get_accumulator_address 2>/dev/null || echo 'not set (required for claimRevenue)')"
    echo "Pool WASM Hash: $(get_network_value "pool_wasm_hash")"
    echo "E-Mode ID Map: $(jq -c --arg network "$NETWORK" '.[$network].emode_category_ids // {}' "$NETWORKS_FILE")"
    echo "Reflector CEX: $(get_cex_oracle)"
    echo "Reflector DEX: $(get_dex_oracle)"
    echo "Reflector FX:  $(get_fx_oracle)"
    echo "RedStone adapter: $(get_redstone_adapter)"
    # Markets that actually reference RedStone (as primary or anchor) in the
    # market config. testnet wires RedStone through the shared adapter + symbol
    # feed_ids, so this reflects real usage even when the optional per-feed
    # contract registry below is empty.
    echo "RedStone markets: $(jq -r '[.markets[] | select((.oracle.primary.tag == "RedStone") or (.oracle.anchor.tag == "Some" and (.oracle.anchor.values[0].tag // "") == "RedStone")) | .name] | if length == 0 then "none" else join(", ") end' "$MARKET_CONFIG_FILE" 2>/dev/null || echo "n/a")"
    echo "RedStone feed registry: $(jq -r --arg network "$NETWORK" '(.[$network].redstone_feeds // {}) | keys | length' "$NETWORKS_FILE") per-feed contract(s)"
}

# ---------------------------------------------------------------------------
# Market-level views
# ---------------------------------------------------------------------------

get_price() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local ctrl
    ctrl=$(get_controller)
    echo "=== Price for ${market_name} (${asset_address}) ===" >&2
    invoke_view "$ctrl" get_market_indexes_detailed --assets "[\"$asset_address\"]"
}

get_market_config_view_cmd() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local ctrl
    ctrl=$(get_controller)
    # get_market_config was removed; the asset's base spoke-0 listing
    # (SpokeAssetConfig) is the per-asset config read-back.
    echo "=== Market config (base spoke 0) for ${market_name} (${asset_address}) ===" >&2
    invoke_view "$ctrl" get_spoke_asset --spoke_id 0 --asset "$asset_address"
}

get_index_cmd() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local ctrl
    ctrl=$(get_controller)
    echo "=== Index for ${market_name} (${asset_address}) ===" >&2
    invoke_view "$ctrl" get_market_indexes_detailed --assets "[\"$asset_address\"]"
}

get_emode_cmd() {
    local cat_id=$1
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" get_spoke --spoke_id "$cat_id"
}

get_all_markets_cmd() {
    local assets_json
    assets_json=$(all_configured_asset_addresses)
    local ctrl
    ctrl=$(get_controller)
    echo "=== All markets (${NETWORK}) ===" >&2
    invoke_view "$ctrl" get_markets_detailed --assets "$assets_json"
}

get_all_indexes_cmd() {
    local assets_json
    assets_json=$(all_configured_asset_addresses)
    local ctrl
    ctrl=$(get_controller)
    echo "=== All market indexes (${NETWORK}) ===" >&2
    invoke_view "$ctrl" get_market_indexes_detailed --assets "$assets_json"
}

# ---------------------------------------------------------------------------
# Account-level views
# ---------------------------------------------------------------------------

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
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" get_collateral_amount --account_id "$account_id" --asset "$asset_address"
}

get_borrow_cmd() {
    local account_id=$1
    local market_name=$2
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local ctrl
    ctrl=$(get_controller)
    invoke_view "$ctrl" get_borrow_amount --account_id "$account_id" --asset "$asset_address"
}

# ---------------------------------------------------------------------------
# Raw Reflector oracle probes (SEP-40 ABI)
#
# Reflector exposes three independent oracle contracts per network:
#   - External CEX/FX (API-sourced)   → pass kind=other + symbol (e.g. "USDC")
#   - Stellar Pubnet DEX (on-chain)   → pass kind=stellar + SAC address
#   - Foreign Exchange                → pass kind=other + symbol (e.g. "EUR")
#
# Use these probes when hunting for the correct DEX oracle address before
# wiring it into a market's `reflector.dex_oracle`.
# ---------------------------------------------------------------------------

build_reflector_asset_json() {
    local kind=$1      # "stellar" (0) or "other" (1)
    local value=$2     # SAC address or ticker
    case "$kind" in
        stellar|Stellar|0)
            # Stellar CLI accepts enum variants via the short form {"Variant":payload}.
            # The tagged-union long form {"tag":...,"values":[...]} trips a panic
            # inside soroban-spec-tools on newer CLI releases.
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

    local tag body
    tag=$(printf '%s' "$source_json" | oracle_union_tag)
    body=$(printf '%s' "$source_json" | oracle_union_value)

    case "$tag" in
        Reflector)
            local contract asset read_mode decimals resolution
            contract=$(printf '%s' "$body" | jq -r '.contract // empty')
            asset=$(printf '%s' "$body" | jq -c '.asset' | describe_reflector_asset)
            read_mode=$(printf '%s' "$body" | jq -c '.read_mode' | describe_read_mode)
            decimals=$(printf '%s' "$body" | jq -r '.decimals // "input"')
            resolution=$(printf '%s' "$body" | jq -r '.resolution_seconds // "input"')
            echo "[${label}] Reflector contract=${contract} asset=${asset} read_mode=${read_mode} decimals=${decimals} resolution=${resolution}" >&2
            ;;
        RedStone)
            local contract feed_id decimals max_stale
            contract=$(printf '%s' "$body" | jq -r '.contract // empty')
            feed_id=$(printf '%s' "$body" | jq -r '.feed_id // empty')
            decimals=$(printf '%s' "$body" | jq -r '.decimals // "input"')
            max_stale=$(printf '%s' "$body" | jq -r '.max_stale_seconds // "input"')
            echo "[${label}] RedStone contract=${contract} feed_id=${feed_id} decimals=${decimals} max_stale=${max_stale}" >&2
            ;;
        *)
            echo "[${label}] unknown source: ${source_json}" >&2
            ;;
    esac
}

# Live price components for a market. The raw stored Oracle V2 config is no
# longer view-exposed (set_market_oracle_config is write-only; get_market_config
# was removed), so this prints the controller's resolved/safe/aggregator prices
# via get_market_indexes_detailed instead of the provider wiring.
get_oracle_cmd() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local ctrl
    ctrl=$(get_controller)

    echo "=== Oracle price components for ${market_name} (${asset_address}) ===" >&2
    echo "Note: the raw stored oracle config is no longer a readable view; showing live price components." >&2
    invoke_view "$ctrl" get_market_indexes_detailed --assets "[\"$asset_address\"]"
}

get_reflector_cmd() {
    echo "getReflector is deprecated; showing generic Oracle V2 wiring." >&2
    get_oracle_cmd "$1"
}

# ---------------------------------------------------------------------------
# Command dispatch
# ---------------------------------------------------------------------------

case "$1" in
    "listMarkets")
        list_markets
        ;;
    "listEModeCategories")
        list_emode_categories
        ;;
    "addEModeCategory")
        if [ -z "$2" ]; then
            echo "Usage: $0 addEModeCategory <category_id>"
            list_emode_categories
            exit 1
        fi
        add_emode_category "$2"
        ;;
    "addAssetToEMode")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 addAssetToEMode <category_id> <asset_name>"
            list_emode_categories
            exit 1
        fi
        add_asset_to_emode "$2" "$3"
        ;;
    "setupAllEModes")
        setup_all_emodes
        ;;
    "createMarket")
        if [ -z "$2" ]; then
            echo "Usage: $0 createMarket <market_name>"
            list_markets
            exit 1
        fi
        create_market "$2"
        ;;
    "editAssetConfig")
        if [ -z "$2" ]; then
            echo "Usage: $0 editAssetConfig <market_name>"
            list_markets
            exit 1
        fi
        edit_asset_config "$2"
        ;;
    "updateMarketParams")
        if [ -z "$2" ]; then
            echo "Usage: $0 updateMarketParams <market_name>"
            list_markets
            exit 1
        fi
        update_market_params "$2"
        ;;
    "updatePoolCaps")
        if [ -z "$2" ]; then
            echo "Usage: $0 updatePoolCaps <market_name>"
            list_markets
            exit 1
        fi
        update_pool_caps "$2"
        ;;
    "configureMarketOracle")
        if [ -z "$2" ]; then
            echo "Usage: $0 configureMarketOracle <market_name>"
            list_markets
            exit 1
        fi
        configure_market_oracle "$2"
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
        setup_all_markets
        ;;
    "setupAll")
        setup_all_markets
        setup_all_emodes
        echo "=== Full setup complete ==="
        ;;
    "whitelistBlendPools")
        whitelist_blend_pools
        ;;
    "setAggregator")
        set_aggregator
        ;;
    "setAccumulator")
        set_accumulator
        ;;
    "supply")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 supply <market> <amount_raw> [<account_id:0>] [<e_mode_category:0>]" >&2
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
    "setPoolTemplate")
        schedule_set_pool_template "$2"
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
        schedule_deploy_pool
        ;;
    "disableTokenOracle")
        if [ -z "$2" ]; then
            echo "Usage: $0 disableTokenOracle <asset_contract_id>" >&2
            exit 1
        fi
        disable_token_oracle_cmd "$2"
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
    "getEMode")
        if [ -z "$2" ]; then echo "Usage: $0 getEMode <category_id>" >&2; list_emode_categories >&2; exit 1; fi
        get_emode_cmd "$2"
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
        echo "Markets (writes):"
        echo "  listMarkets                     List configured markets"
        echo "  createMarket <name>             Deploy market from config"
        echo "  editAssetConfig <name>          Update asset risk params from config"
        echo "  configureMarketOracle <name>    Configure full market oracle from config"
        echo "  editOracleTolerance <m> <tol>   Edit a market's oracle tolerance band (bps)"
        echo "  updateIndexes <name> [...]      Sync indexes for one or more markets"
        echo "  setupAllMarkets                 Idempotently configure markets; no deploy/unpause"
        echo ""
        echo "E-Mode (writes):"
        echo "  listEModeCategories             List configured e-mode categories"
        echo "  addEModeCategory <id>           Create e-mode category from config"
        echo "  addAssetToEMode <id> <asset>    Add asset to e-mode from config"
        echo "  setupAllEModes                  Idempotently configure e-modes; no deploy/unpause"
        echo ""
        echo "Timelock (admin writes are scheduled then executed after the delay):"
        echo "  Admin verbs (createMarket, editAssetConfig, configureMarketOracle, e-mode,"
        echo "  setAggregator, disableTokenOracle, ...) SCHEDULE a governance op and, by default"
        echo "  (AUTO_EXECUTE=1), await the min-delay then execute it. Set AUTO_EXECUTE=0"
        echo "  to schedule-only and execute later with executeOp."
        echo "  executeOp <op-id>               Execute a locally-scheduled, ready op"
        echo "  cancelOp <op-id>                Cancel a pending op (CANCELLER)"
        echo "  opState <op-id>                 Unset | Waiting | Ready | Done"
        echo "  awaitOp <op-id>                 Poll until the op is Ready"
        echo "  NOTE: oracle ops (configureMarketOracle, editOracleTolerance) schedule a"
        echo "  governance-resolved struct; executeOp re-derives it via the resolve_* views"
        echo "  (build-only re-encode), so they are CLI-executable like every other op."
        echo ""
        echo "Protocol control (writes, all routed through governance):"
        echo "  pause | unpause                 Pause/unpause protocol (immediate, owner)"
        echo "  disableTokenOracle <asset>      Timelock disable_token_oracle on controller"
        echo "  grantGovRole <account> <role>   Grant governance role (ORACLE|PROPOSER|EXECUTOR|CANCELLER; timelocked)"
        echo "  revokeGovRole <account> <role>  Revoke governance role (timelocked)"
        echo "  upgradeGovernanceHash <hash>    Timelocked governance WASM upgrade"
        echo "  updateDelay <ledgers>           Timelocked min-delay increase (cannot shorten)"
        echo "  transferGovOwnership <addr> <ledger>  Timelocked governance ownership handoff"
        echo "  setAggregator                   Set aggregator (networks.json or AGGREGATOR_CONTRACT)"
        echo "  setAccumulator                  Set revenue treasury (networks.json accumulator or ACCUMULATOR_CONTRACT)"
        echo "  Env: AGGREGATOR_CONTRACT, ACCUMULATOR_CONTRACT, AWAIT_MAX_WAIT_SECONDS"
        echo "  setupAll                        Markets + E-Modes only; no deploy/unpause"
        echo "  claimRevenue <name> [...]       Claim revenue for one or more markets (REVENUE role)"
        echo "  claimRevenueAll                 Claim revenue for every configured market"
        echo ""
        echo "Quick views (reads):"
        echo "  info                            Deployment addresses & signer"
        echo "  hasRole <account> <role>        Check role membership"
        echo "  getPrice <market>               Oracle price (spot / safe / aggregator + tolerance)"
        echo "  getMarket <market>              Market config (LTV, liq, caps, flags)"
        echo "  getIndex <market>               Supply/borrow index (RAY)"
        echo "  getAllMarkets                   All markets detailed"
        echo "  getAllIndexes                   All market indexes"
        echo "  getEMode <id>                   E-Mode category params"
        echo "  getHealth <id>                  Health factor (RAY)"
        echo "  getAccount <id>                 Positions + attributes"
        echo "  getCollateralUsd <id>           Aggregate collateral in USD"
        echo "  getBorrowUsd <id>               Aggregate borrow in USD"
        echo "  getLtvUsd <id>                  LTV-weighted collateral in USD"
        echo "  getLiqAvailable <id>            Liquidation collateral available"
        echo "  canLiquidate <id>               bool"
        echo "  getCollateral <id> <market>     Per-asset collateral amount"
        echo "  getBorrow <id> <market>         Per-asset borrow amount"
        echo ""
        echo "Oracle probes (debug Oracle V2 wiring):"
        echo "  getOracle <market>                                   Stored primary + anchor config"
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
        echo "  NETWORK=testnet $0 disableTokenOracle C...USDC"
        echo "  SIGNER=ledger NETWORK=mainnet $0 pause"
        ;;
esac
