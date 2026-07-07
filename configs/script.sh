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
#   configs/networks.json              — RPC URLs, contract addresses (shared, both networks)
#   configs/${NETWORK}/hubs.json        — Hub id -> display name registry
#   configs/${NETWORK}/spokes.json      — Spoke categories
#   configs/${NETWORK}/markets.json     — Market configs
#   configs/${NETWORK}/blend.json       — Approved Blend V2 pools
#   configs/${NETWORK}/oracle_feeds.json — xoxno-oracle-adapter feed_id -> asset mapping
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

get_controller() {
    stellar contract alias show controller --network "$NETWORK" 2>/dev/null || get_network_value "controller"
}

# Governance owns the controller: all admin writes (markets, oracles, spokes,
# pause, roles) route through it. Views and operational role-gated calls
# (update_indexes, claim_revenue) stay controller-direct.
get_governance() {
    stellar contract alias show governance --network "$NETWORK" 2>/dev/null || get_network_value "governance"
}

# Central liquidity pool: holds the hub-level utilization/reserves/rates views
# (`get_utilisation`, `get_supplied_amount`, ...). No local alias is set at
# deploy time, so this reads networks.json directly.
get_pool() {
    get_network_value "pool"
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

# Op records live under configs/ops/ (TRACKED, not tmp/): on mainnet an op can
# sit Waiting for days between schedule and execute, and losing the record
# means hand-reconstructing the replay args. Records hold no secrets.
OPS_DIR="$ROOT_DIR/configs/ops/$NETWORK"

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
#
# The timelock permanently marks an executed op id Done, so re-applying a
# PREVIOUSLY-EXECUTED setting (toggle A → B → back to A) would resolve to the
# Done id and be skipped. SALT_NONCE mints a fresh generation for that case:
#   SALT_NONCE=2 make <net> editAssetInSpoke 1 USDC
# Unset/empty SALT_NONCE keeps salts byte-identical to historical ops.
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

# MarketParamsRaw = InterestRateModel fields + flash-loan eligibility
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

# SpokeAssetArgs ScVal map (sorted keys), used for the REPLAY args_json only.
# resolve_op schedules a single SpokeAssetArgs struct, so the stored replay args
# are `[<this map>]`. supply_cap / borrow_cap are i128 decimal strings. (The
# propose `--op` payload uses the friendly form below.)
scval_spoke_args() {
    local hub=$1 asset=$2 spoke=$3 cc=$4 cb=$5 ltv=$6 thr=$7 bonus=$8 sc=$9 bc=${10} lf=${11}
    local paused=${12:-false} frozen=${13:-false}
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
            {key:{symbol:"oracle_override"},val:{vec:[{symbol:"None"}]}},
            {key:{symbol:"paused"},val:{bool:$paused}},
            {key:{symbol:"spoke_id"},val:{u32:$spoke}},
            {key:{symbol:"supply_cap"},val:{i128:$sc}},
            {key:{symbol:"threshold"},val:{u32:$thr}}
        ]}'
}

# Friendly SpokeAssetArgs object for the propose `--op` payload (plain JSON, Rust
# field names). Address is a bare strkey; i128 caps are decimal strings.
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
          supply_cap:$sc, borrow_cap:$bc, oracle_override:"None"}'
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
# Core resolver: derive the scheduled ScVal `Vec<Val>` args for an oracle op by
# running the governance resolve_* view, feeding the friendly result back
# through the controller's typed setter with `--build-only`, and decoding the
# CLI-encoded InvokeContract args. Parameterized so the execute-time record
# path and the pre-propose idempotency check share one implementation.
#   $1 view_fn   resolve_market_oracle_config | resolve_oracle_tolerance
#   $2 ctrl      controller address (op target)
#   $3 function  controller setter (set_market_oracle_config | set_oracle_tolerance)
#   $4 asset     asset address
#   $5 hub_id    hub id (market-oracle only; ignored for tolerance)
#   $6 payload   cfg JSON (market-oracle) | tolerance bps (tolerance)
resolve_oracle_args_for() {
    local view_fn=$1 ctrl=$2 function=$3 asset=$4 hub_id=$5 payload=$6
    local gov resolved tx_xdr
    gov=$(get_governance)
    case "$view_fn" in
        resolve_market_oracle_config)
            local cfg_file
            cfg_file=$(mktemp)
            printf '%s' "$payload" > "$cfg_file"
            resolved=$(stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
                --send=no -- "$view_fn" --asset "$asset" --cfg-file-path "$cfg_file")
            rm -f "$cfg_file"
            local cfg_file2
            cfg_file2=$(mktemp)
            printf '%s' "$resolved" > "$cfg_file2"
            # set_market_oracle_config takes a HubAssetKey (hub_id + asset), not a
            # bare asset; rebuild it from the resolve inputs.
            local oracle_hub_asset
            oracle_hub_asset=$(jq -nc --argjson h "$hub_id" --arg a "$asset" '{hub_id:$h, asset:$a}')
            tx_xdr=$(stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
                --build-only --send=no -- "$function" \
                --hub_asset "$oracle_hub_asset" --config-file-path "$cfg_file2")
            rm -f "$cfg_file2"
            printf '%s' "$tx_xdr" | stellar tx decode \
                | jq -c 'first(.. | objects | select(has("invoke_contract")) | .invoke_contract.args)'
            ;;
        resolve_oracle_tolerance)
            resolved=$(stellar contract invoke --id "$gov" $SOURCE_FLAG --network "$NETWORK" \
                --send=no -- "$view_fn" --tolerance "$payload")
            local tol_file
            tol_file=$(mktemp)
            printf '%s' "$resolved" > "$tol_file"
            tx_xdr=$(stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
                --build-only --send=no -- "$function" \
                --asset "$asset" --tolerance-file-path "$tol_file")
            rm -f "$tol_file"
            printf '%s' "$tx_xdr" | stellar tx decode \
                | jq -c 'first(.. | objects | select(has("invoke_contract")) | .invoke_contract.args)'
            ;;
        *)
            echo "ERROR: unknown oracle resolve view '${view_fn}'." >&2
            exit 1
            ;;
    esac
}

resolve_oracle_op_args() {
    local path=$1
    local ctrl function view_fn asset
    ctrl=$(jq -r '.target' "$path")
    function=$(jq -r '.function' "$path")
    view_fn=$(jq -r '.resolve.view_fn' "$path")
    asset=$(jq -r '.resolve.args.asset' "$path")
    case "$view_fn" in
        resolve_market_oracle_config)
            resolve_oracle_args_for "$view_fn" "$ctrl" "$function" "$asset" \
                "$(jq -r '.resolve.args.hub_id' "$path")" "$(jq -c '.resolve.args.cfg' "$path")"
            ;;
        resolve_oracle_tolerance)
            resolve_oracle_args_for "$view_fn" "$ctrl" "$function" "$asset" \
                "" "$(jq -r '.resolve.args.tolerance' "$path")"
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

# Parse a returned u32 (spoke/hub id) from the generic execute's last output
# line. Extracts the full number — an anchored sed like `.*([0-9]+)` would
# greedily eat leading digits and truncate "12" to "2".
parse_returned_u32() {
    printf '%s\n' "$1" | tail -n1 | tr -d '"' | grep -oE '[0-9]+' | tail -n1
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

# Pre-compute the deterministic operation id for (target, function, args, salt)
# via the governance `hash_operation` view. Salts are deterministic, so an op's
# id is knowable BEFORE proposing — this is what makes every schedule path
# idempotent: a re-run (e.g. `make <net> resume`) skips Done ops and reuses
# Waiting/Ready ones instead of tripping the timelock's already-scheduled error.
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

# Derive the generation-n salt from a base salt (hash chain; generation 0 = the
# base itself). Executed timelock ids are Done forever, so re-applying a
# previously-executed setting needs a fresh salt: generations provide that
# deterministically without changing historical (generation-0) ids.
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

# Probe salt generations until the first op id that is NOT Done. Prints
# "<salt> <op_id> <state> <n>". State meanings for the caller:
#   Unset     free to propose at this generation (n>0 ⇒ earlier gens executed)
#   Waiting|Ready  an identical op is already scheduled ⇒ reuse it
#   Unknown   hash view / state read unavailable ⇒ fall back to plain propose
#   Exhausted all generations Done ⇒ manual SALT_NONCE required
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

# Core scheduler: invoke the generic `propose(proposer, op, salt)` on governance
# and record the controller op for replay through the generic `execute`.
#
# Idempotent AND re-apply-aware: the op id is pre-computed per salt generation
# (probe_salt_generations). An op already Waiting/Ready is reused; when the
# current generation is Done, behavior depends on the re-apply policy:
#   - policy "never" ($6): skip — id-returning creators (add_spoke, create_hub,
#     deploy_pool) must never re-execute, that would mint a duplicate entity.
#   - REAPPLY_ON_DONE=0 (converge mode, set by setupAll*): skip — the setting
#     is treated as already applied.
#   - otherwise (direct operator verbs): auto re-apply at the next free
#     generation, so toggling back to a previous setting just works.
#   $1 controller_fn       controller thin-setter the op targets (for the record)
#   $2 admin_op_json       AdminOperation friendly JSON ("Variant" | {"Variant":payload})
#   $3 args_json           ScVal Vec<Val> array (JSON) for replay (resolve_op args)
#   $4 cli_executable      true|false (false ⇒ executeOp refuses; oracle ops)
#   $5 salt_hex            deterministic base salt (64 hex)
#   $6 reapply (optional)  "never" to forbid auto re-apply (creators)
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
            # Refresh the local record so executeOp / listOps stay usable
            # even when the original record was lost.
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
                    echo "$done_id"
                    return 0
                fi
                echo "Op (${controller_fn}) with these exact args already executed; RE-APPLYING as generation ${gen}." >&2
                salt_hex=$salt_use
            fi
            ;;
        *)
            # Unknown: hash/state views unavailable — fall through to a plain
            # propose with the base salt (pre-idempotency behavior).
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

# Schedule a governance-self admin op (target = governance contract). Replay
# uses the same AdminOperation through `execute_self`, so the op record stores
# the admin_op JSON itself. When the resolved (function, args) pair is supplied,
# the op id is pre-computed via hash_operation (target = governance) and an
# already-scheduled/executed op is reused instead of re-proposed.
#   $1 execute_label       human label for the record/log (e.g. update_delay)
#   $2 admin_op_json       AdminOperation friendly JSON ("Variant" | {"Variant":payload})
#   $3 salt_hex            deterministic salt (64 hex)
#   $4 gov_fn (optional)   governance function the op resolves to (for hash pre-check)
#   $5 gov_args (optional) ScVal Vec<Val> array the op resolves to (for hash pre-check)
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
                # Scale the sleep to half the remaining delay (~6s/ledger),
                # clamped to [AWAIT_POLL_SECONDS, 600] so a multi-day mainnet
                # timelock doesn't hammer the RPC every few seconds.
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

# List every recorded governance op with its live on-chain state. Pending ops
# (Waiting/Ready) are what still needs an executeOp; Done ops already landed.
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

# Execute every recorded op that is currently Ready. Waiting/Done ops are left
# alone; a failing execute aborts (set -e) so partial failures are visible.
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

# Schedule, await the delay, then execute — the one-shot setup path. Honors
# AUTO_EXECUTE=0 to schedule-only (record op-id for a later executeOp). An op
# that is already Done (idempotent re-run) is skipped, not re-executed.
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

# ---------------------------------------------------------------------------
# Config validation (pure jq, no chain calls)
#
# `validateConfigs` is the pre-deploy gate: it cross-checks the markets file,
# spokes file, and networks.json so a misconfig fails HERE instead of after a
# timelock delay (or worse, lands on-chain). Run automatically by setupAll /
# setupAllMarkets / setupAllSpokes and by `make <net> validateConfigs`.
# ---------------------------------------------------------------------------

validate_configs() {
    local errors=0 warnings=0
    vc_err() { echo "ERROR: $*" >&2; errors=$(( errors + 1 )); }
    vc_warn() { echo "WARN:  $*" >&2; warnings=$(( warnings + 1 )); }

    echo "=== Validating ${NETWORK} configs ===" >&2

    # networks.json basics
    local f v
    for f in rpc_url network_passphrase timelock_min_delay_ledgers; do
        v=$(get_network_value "$f")
        if [ -z "$v" ] || [ "$v" = "null" ]; then
            vc_err "networks.json ${NETWORK}.${f} missing"
        fi
    done

    # Known oracle contracts for cross-checks.
    local cex dex fx redstone
    cex=$(get_cex_oracle)
    dex=$(get_dex_oracle)
    fx=$(get_fx_oracle)
    redstone=$(get_redstone_adapter)

    # Markets: duplicates
    local dup
    dup=$(jq -r '[.markets[].name] | group_by(.) | map(select(length > 1) | .[0]) | join(", ")' "$MARKET_CONFIG_FILE")
    if [ -n "$dup" ]; then
        vc_err "duplicate market names: ${dup}"
    fi
    dup=$(jq -r '[.markets[] | "\(.hub_id)/\(.asset_address)"] | group_by(.) | map(select(length > 1) | .[0]) | join(", ")' "$MARKET_CONFIG_FILE")
    if [ -n "$dup" ]; then
        vc_err "duplicate (hub_id, asset_address) pairs: ${dup}"
    fi

    # Per-market checks
    local m mj hub addr missing o strat anchor_tag minw maxw ptag pcontract atag acontract
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

        # market_params completeness + utilization/reserve-factor relations
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

        # asset_config risk bounds (base spoke-0 listing)
        if ! printf '%s' "$mj" | jq -e '
            (.asset_config // {}) as $c |
            ($c.loan_to_value // 99999) < ($c.liquidation_threshold // 0) and
            ($c.liquidation_threshold // 99999) <= 10000 and
            ($c.liquidation_bonus // 99999) <= 10000 and
            (($c.liquidation_threshold // 0) * (10000 + ($c.liquidation_bonus // 0))) <= 100000000' >/dev/null; then
            vc_err "market ${m}: asset_config risk bounds invalid (need ltv < threshold <= 10000, bonus <= 10000, threshold*(1+bonus) <= 100%)"
        fi
        if ! printf '%s' "$mj" | jq -e '(.asset_config.flashloan_fee // 0) <= 10000' >/dev/null; then
            vc_err "market ${m}: flashloan_fee > 10000 bps"
        fi

        # Oracle config
        o=$(printf '%s' "$mj" | jq -c '.oracle // {}')
        if ! printf '%s' "$o" | jq -e '(.tolerance_bps // 0) >= 1 and (.tolerance_bps // 99999) <= 10000' >/dev/null; then
            vc_err "market ${m}: oracle tolerance_bps out of [1, 10000]"
        fi
        strat=$(printf '%s' "$o" | jq -r '.strategy // "missing"')
        anchor_tag=$(printf '%s' "$o" | jq -r '
            if (.anchor | type) == "object" then (.anchor.tag // "Some")
            elif (.anchor | type) == "string" then .anchor
            else "None" end')
        case "$strat" in
            0)
                if [ "$anchor_tag" = "Some" ]; then
                    vc_warn "market ${m}: anchor configured but strategy Single(0) ignores it"
                fi
                ;;
            1)
                if [ "$anchor_tag" != "Some" ]; then
                    vc_err "market ${m}: strategy PrimaryWithAnchor(1) requires an anchor"
                fi
                ;;
            *) vc_err "market ${m}: oracle strategy must be 0 (Single) or 1 (PrimaryWithAnchor)" ;;
        esac
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

        # Cross-check oracle contract addresses against networks.json
        ptag=$(printf '%s' "$o" | jq -r '.primary.tag // "missing"')
        pcontract=$(printf '%s' "$o" | jq -r '.primary.values[0].contract // ""')
        if [ "$ptag" = "Reflector" ] && [ -n "$pcontract" ]; then
            case "$pcontract" in
                "$cex"|"$dex"|"$fx") ;;
                *) vc_warn "market ${m}: primary Reflector contract ${pcontract} is none of networks.json cex/dex/fx oracles" ;;
            esac
        fi
        if [ "$ptag" = "RedStone" ] && [ -n "$pcontract" ] && [ "$pcontract" != "$redstone" ]; then
            vc_warn "market ${m}: primary RedStone contract differs from networks.json redstone_adapter_contract"
        fi
        atag=$(printf '%s' "$o" | jq -r '.anchor.values[0].tag // ""')
        acontract=$(printf '%s' "$o" | jq -r '.anchor.values[0].values[0].contract // ""')
        if [ "$atag" = "RedStone" ] && [ -n "$acontract" ] && [ "$acontract" != "$redstone" ]; then
            vc_warn "market ${m}: anchor RedStone contract differs from networks.json redstone_adapter_contract"
        fi
    done

    # DEX-primary markets reprice through their quote asset's oracle
    # (ReflectorBase::Quoted): the quote market must be configured FIRST, and
    # setupAllMarkets configures in file order. Fail when the very first market
    # is DEX-primary; otherwise remind that ordering is load-bearing.
    local first_dex
    first_dex=$(jq -r --arg dex "$dex" 'first(.markets | to_entries[] |
        select(.value.oracle.primary.tag == "Reflector" and .value.oracle.primary.values[0].contract == $dex) | .key) // ""' \
        "$MARKET_CONFIG_FILE")
    if [ "$first_dex" = "0" ]; then
        vc_err "first market in ${MARKET_CONFIG_FILE} is DEX-oracle-primary; its USD quote market must come before it (file order = setup order)"
    elif [ -n "$first_dex" ]; then
        vc_warn "DEX-oracle markets present: each one's USD quote market must appear EARLIER in ${MARKET_CONFIG_FILE} (file order = setup order)"
    fi

    # Spokes: every asset must resolve to a market on the SAME hub, with sane
    # risk params.
    local cat a sj maddr mhub
    for cat in $(jq -r 'keys[]' "$SPOKES_FILE"); do
        for a in $(jq -r --arg c "$cat" '.[$c].assets | keys[]' "$SPOKES_FILE"); do
            sj=$(jq -c --arg c "$cat" --arg a "$a" '.[$c].assets[$a]' "$SPOKES_FILE")
            maddr=$(get_market_value "$a" "asset_address")
            if [ -z "$maddr" ] || [ "$maddr" = "null" ]; then
                vc_err "spoke ${cat}: asset '${a}' has no market in ${MARKET_CONFIG_FILE}"
                continue
            fi
            mhub=$(get_market_value "$a" "hub_id")
            if ! printf '%s' "$sj" | jq -e --argjson mh "${mhub:-null}" '(.hub_id // null) == $mh' >/dev/null; then
                vc_err "spoke ${cat}/${a}: hub_id $(printf '%s' "$sj" | jq -r '.hub_id // "missing"') != market hub_id ${mhub}"
            fi
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

    # A market in no spoke deploys pending (base spoke-0 listing stays
    # non-collateralizable/borrowable) and is unusable until listed.
    for m in $(jq -r '.markets[].name' "$MARKET_CONFIG_FILE"); do
        if ! jq -e --arg m "$m" '[.[].assets | keys[]] | index($m) != null' "$SPOKES_FILE" >/dev/null; then
            vc_warn "market ${m} is not referenced by any spoke (deploys pending; unusable until listed)"
        fi
    done

    echo "=== Validation: ${errors} error(s), ${warnings} warning(s) ===" >&2
    if [ "$errors" -gt 0 ]; then
        exit 1
    fi
    return 0
}

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

list_spokes() {
    echo "Spoke categories (${NETWORK}):"
    if [ -f "$SPOKES_FILE" ]; then
        jq -r --arg network "$NETWORK" --slurpfile networks "$NETWORKS_FILE" '
            . as $cats |
            ($networks[0][$network].spoke_ids // {}) as $ids |
            $cats | to_entries[] |
            "  \(.key) -> on-chain \($ids[.key] // "unmapped"): \(.value.name) — assets: \(.value.assets | keys | join(", "))"
        ' "$SPOKES_FILE"
    else
        echo "  No spokes config found: $SPOKES_FILE"
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
# Spoke functions
# ---------------------------------------------------------------------------

add_spoke() {
    local category_id=$1

    local name
    name=$(get_spoke_value "$category_id" ".name")

    echo "Adding Spoke category ${category_id}: ${name}" >&2

    # add_spoke() — no on-chain args; risk params are per-asset (spoke categories
    # are now spokes). The salt is seeded with the config category id so that
    # creating several spokes in one setup run derives distinct timelock op ids
    # (the call args stay []; a shared salt would collide on the second spoke).
    local args_json='[]'
    local salt
    salt=$(gen_salt "add_spoke:${category_id}" "$args_json")

    # "never": re-executing a spoke create would mint a duplicate spoke.
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
    # The controller's add_spoke returns the new on-chain id; the
    # generic execute prints that returned Val on its last line.
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
    # Spoke reads stay on the controller; only writes route through governance.
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

# Content guard for category reuse. Returns 0 when every asset already
# configured on-chain in `category_json` also appears in config category
# `config_category_id`; returns 1 when the on-chain category holds any asset
# this config does not list. An on-chain category with no assets is compatible
# (setup will populate it). On-chain categories carry no name, so a foreign
# category whose assets are a strict subset of this config's assets cannot be
# distinguished here — closing that residual needs an on-chain identity field.
spoke_assets_match_config() {
    local config_category_id=$1
    local category_json=$2

    # Current get_spoke returns SpokeConfig (no embedded `.assets` map; assets
    # live under separate SpokeAsset keys). If the response lacks a readable
    # assets object we cannot enumerate foreign assets, so we allow reuse of
    # the mapped id. Per-asset reconciliation in ensure_asset_in_spoke uses
    # direct get_spoke_asset probes (and on-chain add/edit will enforce
    # presence).
    if ! printf '%s' "$category_json" | jq -e '.assets | type == "object"' >/dev/null 2>&1; then
        echo "WARN: on-chain Spoke category for config ${config_category_id} has no readable .assets map (current contract); cannot fully verify. Proceeding." >&2
        return 0
    fi

    local onchain_assets
    onchain_assets=$(printf '%s' "$category_json" | jq -r '.assets | keys[]')
    # An empty on-chain category is compatible — setup will populate it.
    [ -z "$onchain_assets" ] && return 0

    local expected_addrs=" "
    local asset_name asset_addr
    for asset_name in $(jq -r ".\"$config_category_id\".assets | keys[]" "$SPOKES_FILE"); do
        asset_addr=$(get_market_value "$asset_name" "asset_address")
        # An unresolved asset means the config references something the markets
        # file lacks; fail with that specific reason rather than silently
        # dropping it (which would later mislabel an on-chain asset as foreign).
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

# A category only groups assets and tracks deprecation; risk params live on the
# per-asset configs (ensured by `ensure_asset_in_spoke`). Reuse therefore
# requires two checks: the category must not be deprecated, and every asset it
# already holds on-chain must belong to this config category — otherwise we
# would silently rewrite a different category's (possibly live) risk params.
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
    supply_cap=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".supply_cap")
    local borrow_cap
    borrow_cap=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".borrow_cap")
    if [ -z "$supply_cap" ] || [ "$supply_cap" = "null" ]; then supply_cap=0; fi
    if [ -z "$borrow_cap" ] || [ "$borrow_cap" = "null" ]; then borrow_cap=0; fi
    # SpokeAssetArgs.liquidation_fees: per-spoke value from spokes.json, else fall
    # back to the market's asset_config.liquidation_fees, else 0.
    local liquidation_fees
    liquidation_fees=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".liquidation_fees")
    if [ -z "$liquidation_fees" ] || [ "$liquidation_fees" = "null" ]; then
        liquidation_fees=$(get_market_value "$asset_name" "asset_config.liquidation_fees")
    fi
    if [ -z "$liquidation_fees" ] || [ "$liquidation_fees" = "null" ]; then liquidation_fees=0; fi
    # SpokeAssetArgs.paused / .frozen: per-listing incident flags; optional in
    # spokes.json, defaulting to an active (false/false) listing.
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

    # add_asset_to_spoke(SpokeAssetArgs). resolve_op schedules a single
    # SpokeAssetArgs struct, so the replay args_json is one struct element.
    local args_json
    args_json=$(jq -nc \
        --argjson arg "$(scval_spoke_args "$hub_id" "$asset_address" "$category_id" "$can_collateral" \
            "$can_borrow" "$ltv" "$threshold" "$bonus" "$supply_cap" "$borrow_cap" "$liquidation_fees" "$paused" "$frozen")" \
        '[$arg]')
    local salt
    salt=$(gen_salt "add_asset_to_spoke" "$args_json")

    # The propose `--op` payload is the single SpokeAssetArgs in friendly form.
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
    supply_cap=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".supply_cap")
    local borrow_cap
    borrow_cap=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".borrow_cap")
    if [ -z "$supply_cap" ] || [ "$supply_cap" = "null" ]; then supply_cap=0; fi
    if [ -z "$borrow_cap" ] || [ "$borrow_cap" = "null" ]; then borrow_cap=0; fi
    # SpokeAssetArgs.liquidation_fees: per-spoke value from spokes.json, else fall
    # back to the market's asset_config.liquidation_fees, else 0.
    local liquidation_fees
    liquidation_fees=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".liquidation_fees")
    if [ -z "$liquidation_fees" ] || [ "$liquidation_fees" = "null" ]; then
        liquidation_fees=$(get_market_value "$asset_name" "asset_config.liquidation_fees")
    fi
    if [ -z "$liquidation_fees" ] || [ "$liquidation_fees" = "null" ]; then liquidation_fees=0; fi
    # SpokeAssetArgs.paused / .frozen: per-listing incident flags; optional in
    # spokes.json, defaulting to an active (false/false) listing.
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

    # edit_asset_in_spoke(SpokeAssetArgs). resolve_op schedules a single
    # SpokeAssetArgs struct, so the replay args_json is one struct element.
    local args_json
    args_json=$(jq -nc \
        --argjson arg "$(scval_spoke_args "$hub_id" "$asset_address" "$category_id" "$can_collateral" \
            "$can_borrow" "$ltv" "$threshold" "$bonus" "$supply_cap" "$borrow_cap" "$liquidation_fees" "$paused" "$frozen")" \
        '[$arg]')
    local salt
    salt=$(gen_salt "edit_asset_in_spoke" "$args_json")

    # The propose `--op` payload is the single SpokeAssetArgs in friendly form.
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
    supply_cap=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".supply_cap")
    local borrow_cap
    borrow_cap=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".borrow_cap")
    if [ -z "$supply_cap" ] || [ "$supply_cap" = "null" ]; then supply_cap=0; fi
    if [ -z "$borrow_cap" ] || [ "$borrow_cap" = "null" ]; then borrow_cap=0; fi
    # SpokeAssetArgs.liquidation_fees: per-spoke value from spokes.json, else fall
    # back to the market's asset_config.liquidation_fees, else 0.
    local liquidation_fees
    liquidation_fees=$(get_spoke_value "$config_category_id" ".assets.\"$asset_name\".liquidation_fees")
    if [ -z "$liquidation_fees" ] || [ "$liquidation_fees" = "null" ]; then
        liquidation_fees=$(get_market_value "$asset_name" "asset_config.liquidation_fees")
    fi
    if [ -z "$liquidation_fees" ] || [ "$liquidation_fees" = "null" ]; then liquidation_fees=0; fi
    local category_json

    if [ -z "$asset_address" ] || [ "$asset_address" = "null" ] || [ "$asset_address" = "" ]; then
        echo "ERROR: No asset address found for ${asset_name} in ${MARKET_CONFIG_FILE}"
        exit 1
    fi

    category_json=$(fetch_spoke_json "$category_id")
    # Bridge for current contract (get_spoke has no .assets; SpokeAsset is separate).
    # Probe the specific asset and synthesize the shape the decision/compare code expects.
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
            # Drift proven by the on-chain compare: force re-apply even in
            # converge mode, else a toggle back to an earlier setting would hit
            # its Done op and be skipped forever.
            REAPPLY_ON_DONE=1 edit_asset_in_spoke "$category_id" "$asset_name" "$config_category_id"
        fi
    else
        # Absence proven by the on-chain probe: a re-add after a removal must
        # re-apply even if an identical add executed before.
        REAPPLY_ON_DONE=1 add_asset_to_spoke "$category_id" "$asset_name" "$config_category_id"
    fi
}

setup_all_spokes() {
    echo "=== Setting up all Spoke categories for ${NETWORK} ==="
    local categories
    categories=$(jq -r "keys[]" "$SPOKES_FILE")

    for cat_id in $categories; do
        local onchain_id
        # Bare assignment (declared separately so `local` doesn't mask the
        # status): a command substitution inside an `if` condition would suppress
        # `set -e` within ensure_spoke and its callees, silently
        # continuing on an inner failure or the content guard's `return 1`. With
        # a plain assignment, `set -e` stays active inside the function and
        # aborts the deploy on any non-zero exit; the guard prints the specific
        # reason to stderr before returning.
        onchain_id=$(ensure_spoke "$cat_id")
        onchain_id=$(printf '%s\n' "$onchain_id" | tail -n1)

        local assets
        assets=$(jq -r ".\"$cat_id\".assets | keys[]" "$SPOKES_FILE")
        for asset_name in $assets; do
            ensure_asset_in_spoke "$onchain_id" "$asset_name" "$cat_id"
        done
    done
    echo "=== All Spoke categories configured ==="
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
# new u32 id — mirrors add_spoke.
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

    # "never": re-executing a hub create would mint a duplicate hub.
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
    # The generic execute prints the controller's returned hub id on its last line.
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
    local hub_asset
    hub_asset=$(build_hub_assets_json "$market_name" | jq -c '.[0]')
    if stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" --send=no -- get_spoke_asset --spoke_id 0 --hub_asset "$hub_asset" &>/dev/null; then
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
    # The controller deploys markets in a pending state (base spoke-0 listing
    # not collateralizable/borrowable); activation happens via spoke listings.

    # Post-audit (T1-7): the controller gates `create_liquidity_pool` behind an
    # admin allow-list. Pre-approve the token first (separate timelocked op,
    # executed before the create op so the allow-list check passes).
    # `approve_token` is idempotent on chain.
    echo "Scheduling token approval for market creation..." >&2
    local approve_args
    approve_args=$(jq -nc --arg t "$asset_address" '[{address:$t}]')
    local approve_salt
    approve_salt=$(gen_salt "approve_token:${market_name}" "$approve_args")
    local approve_op
    approve_op=$(schedule_via_proposer \
        approve_token "$(admin_op ApproveToken "$(jq -nc --arg a "$asset_address" '$a')")" \
        "$approve_args" true "$approve_salt")
    schedule_and_maybe_execute "$approve_op"

    # create_liquidity_pool(hub_id, asset, params) — u32 + Address + one field-map
    # struct. The governance handler resolves CreateLiquidityPool to exactly these
    # three call args; per-asset risk config is applied separately via
    # add_asset_to_spoke, not here. The scheduled args equal these inputs
    # (governance validates but does not transform), so they are CLI-replayable.
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

    # The propose `--op` payload wraps hub_id + asset + the friendly params/config
    # objects (Rust field names) in CreatePoolArgs.
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
    # claim_revenue is operational, not admin: it stays controller-direct.
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
# source. `whitelistBlendPools` reads configs/${NETWORK}/blend.json for the current
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
# Args: <market> <amount_raw> [<account_id:0>] [<spoke_id:0>]
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

    # i128 amounts are decimal strings so large raw values stay exact.
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- supply \
        --caller "$caller" \
        --account_id "$account_id" \
        --spoke_id "$spoke_id" \
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

# `withdraw` — withdraw supplied collateral from an account.
# Args: <market> <amount_raw> <account_id>   (amount 0 = withdraw all / close position)
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
    asset_address=$(require_market_address "$market_name")
    # set_market_oracle_config keys by HubAssetKey (hub_id + asset) in the
    # multi-hub ABI; the oracle op carries hub_asset, not a bare asset.
    local hub_id
    hub_id=$(get_market_value "$market_name" "hub_id")
    if [ -z "$hub_id" ] || [ "$hub_id" = "null" ]; then
        die "market ${market_name} missing hub_id in ${MARKET_CONFIG_FILE}"
    fi
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
    salt_input=$(jq -nc --argjson cfg "$cfg_json" --arg asset "$asset_address" --argjson hub_id "$hub_id" \
        '{hub_asset:{hub_id:$hub_id, asset:$asset}, cfg:$cfg}')
    salt=$(gen_salt "set_market_oracle_config" "$salt_input")
    local resolve_args
    resolve_args=$(jq -nc --arg asset "$asset_address" --argjson cfg "$cfg_json" --argjson hub_id "$hub_id" \
        '{hub_id:$hub_id, asset:$asset, cfg:$cfg}')

    # Idempotency pre-check: derive the scheduled (resolved) args now, compute
    # the deterministic op id, and reuse an op that already exists on-chain
    # instead of re-proposing (which the timelock rejects). A resolve failure
    # falls through to propose, whose validation reports the authoritative error.
    local ctrl resolved_args salt_use known_id state gen
    ctrl=$(get_controller)
    resolved_args=$(resolve_oracle_args_for resolve_market_oracle_config "$ctrl" \
        set_market_oracle_config "$asset_address" "$hub_id" "$cfg_json" 2>/dev/null) || resolved_args=""
    if [ -n "$resolved_args" ] && [ "$resolved_args" != "null" ]; then
        read -r salt_use known_id state gen < <(probe_salt_generations "$ctrl" set_market_oracle_config "$resolved_args" "$salt")
        case "$state" in
            Ready|Waiting)
                echo "Oracle config op ${known_id} for ${market_name} already ${state}; reusing it instead of re-proposing." >&2
                write_oracle_op_record "$known_id" "set_market_oracle_config" \
                    "resolve_market_oracle_config" "$resolve_args" "$salt_use"
                schedule_and_maybe_execute "$known_id"
                return 0
                ;;
            Exhausted)
                die "configureMarketOracle ${market_name}: all ${MAX_SALT_GENERATIONS} salt generations already executed; re-run with a fresh SALT_NONCE=<n>"
                ;;
            Unset)
                if [ "$gen" -gt 0 ]; then
                    if [ "${REAPPLY_ON_DONE:-1}" != "1" ]; then
                        echo "Oracle config for ${market_name} already executed with this exact config; skipping propose (converge mode)." >&2
                        return 0
                    fi
                    echo "Oracle config for ${market_name} already executed with this exact config; RE-APPLYING as generation ${gen}." >&2
                    salt=$salt_use
                fi
                ;;
            *) ;;
        esac
    fi

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
    jq -nc --arg asset "$asset_address" --argjson cfg "$cfg_json" --argjson hub_id "$hub_id" \
        '{ConfigureMarketOracle: {hub_asset:{hub_id:$hub_id, asset:$asset}, cfg:$cfg}}' > "$op_file"

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
    local resolve_args
    resolve_args=$(jq -nc --arg asset "$asset_address" --argjson t "$tolerance" \
        '{asset:$asset, tolerance:$t}')

    # Idempotency pre-check (see configure_market_oracle): reuse an op that is
    # already scheduled or executed instead of re-proposing.
    local ctrl resolved_args salt_use known_id state gen
    ctrl=$(get_controller)
    resolved_args=$(resolve_oracle_args_for resolve_oracle_tolerance "$ctrl" \
        set_oracle_tolerance "$asset_address" "" "$tolerance" 2>/dev/null) || resolved_args=""
    if [ -n "$resolved_args" ] && [ "$resolved_args" != "null" ]; then
        read -r salt_use known_id state gen < <(probe_salt_generations "$ctrl" set_oracle_tolerance "$resolved_args" "$salt")
        case "$state" in
            Ready|Waiting)
                echo "Oracle tolerance op ${known_id} for ${market_name} already ${state}; reusing it instead of re-proposing." >&2
                write_oracle_op_record "$known_id" "set_oracle_tolerance" \
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
                        echo "Oracle tolerance for ${market_name} already executed with this exact value; skipping propose (converge mode)." >&2
                        return 0
                    fi
                    echo "Oracle tolerance for ${market_name} already executed with this exact value; RE-APPLYING as generation ${gen}." >&2
                    salt=$salt_use
                fi
                ;;
            *) ;;
        esac
    fi

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

# Accept either a configured market name or a raw contract strkey. Admin verbs
# that gate tokens (revokeToken, ...) take both so an incident response is not
# blocked on the asset still being in the markets file.
resolve_asset_arg() {
    local v=$1
    if printf '%s' "$v" | grep -qE '^C[A-Z2-7]{55}$'; then
        echo "$v"
        return 0
    fi
    require_market_address "$v"
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

# Schedule deploy_pool (no controller args), await, execute, and print the
# deployed pool Address parsed from the execute result's last line.
schedule_deploy_pool() {
    local args_json="[]"
    local salt
    salt=$(gen_salt "deploy_pool" "$args_json")
    # "never": re-executing deploy_pool would deploy a second central pool.
    local op_id
    op_id=$(schedule_via_proposer \
        deploy_pool "$(admin_op DeployPool)" "$args_json" true "$salt" never)
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
    # Mainnet safety floor: never take the protocol live while the timelock delay
    # is below the configured production floor. A bootstrap deploy may run its
    # market/spoke config at a short DEPLOY_MIN_DELAY while the controller is
    # still paused; unpausing stays blocked until the delay has been raised to
    # timelock_min_delay_ledgers (e.g. `make mainnet updateDelay <floor>`). This
    # closes the window where a live mainnet could be governed by a near-zero
    # timelock if the operator forgot or automation stopped after setup.
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

# ---------------------------------------------------------------------------
# Remaining AdminOperation verbs (incident response + controller admin).
# Each schedules through the generic proposer; args mirror governance op.rs's
# resolve_op mapping so the recorded replay args match byte-for-byte.
# ---------------------------------------------------------------------------

# Schedule a single-Address controller op (shared shape for revoke/approve-style
# verbs): $1 variant, $2 controller_fn, $3 address.
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

revoke_token_cmd() {
    schedule_address_op RevokeToken revoke_token "$(resolve_asset_arg "$1")"
}

approve_token_cmd() {
    schedule_address_op ApproveToken approve_token "$(resolve_asset_arg "$1")"
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
    # remove_asset_from_spoke(hub_asset, spoke_id) per governance op.rs.
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
    # set_spoke_liquidation_curve(spoke_id, target_hf_wad, hf_for_max_bonus_wad,
    # liquidation_bonus_factor_bps) per governance op.rs.
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
    # Multi-field tuple variant: payload is the field array.
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
    echo "Pool:       $(get_pool)"
    # NOT chain-verified: the controller stores its aggregator/accumulator/
    # position-limits/hub-active flags without a view function, so these are
    # inference from the local CLI alias + networks.json, not on-chain reads.
    # They can silently diverge from what governance actually last set.
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
    # Markets that actually reference RedStone (as primary or anchor) in the
    # market config, read through the single shared redstone_adapter_contract
    # + feed_id string (RedStone has no separate per-feed contract; every feed
    # is read via that one adapter's read_price_data_for_feed(feed_id)).
    echo "RedStone markets: $(jq -r '[.markets[] | select((.oracle.primary.tag == "RedStone") or (.oracle.anchor.tag == "Some" and (.oracle.anchor.values[0].tag // "") == "RedStone")) | .name] | if length == 0 then "none" else join(", ") end' "$MARKET_CONFIG_FILE" 2>/dev/null || echo "n/a")"
}

# Compare the LIVE governance min-delay against the configured production value.
# Catches the classic bootstrap footgun: deploying with DEPLOY_MIN_DELAY=1 and
# forgetting to raise the delay afterwards.
check_delay() {
    local live cfg
    live=$(min_delay_ledgers)
    cfg=$(get_network_value "timelock_min_delay_ledgers")
    echo "Timelock min delay: live=${live} ledgers, configured target=${cfg} ledgers" >&2
    if [ -n "$cfg" ] && [ "$cfg" != "null" ] && [ "$live" -lt "$cfg" ] 2>/dev/null; then
        cat >&2 <<EOF
################################################################################
# WARNING: the LIVE timelock min-delay (${live} ledgers) is BELOW the configured
# production value (${cfg} ledgers). If this deploy is past bootstrap, raise it
# now (increase-only):
#     make ${NETWORK} updateDelay ${cfg}
################################################################################
EOF
    fi
    return 0
}

# Hubs referenced by the market config, with their on-chain mapping state.
list_hubs() {
    echo "Hubs (${NETWORK}) referenced by ${MARKET_CONFIG_FILE}:"
    echo "  NOTE: the controller has no get_hub view; this reads the LOCAL id map in"
    echo "  networks.json, not the on-chain HubConfig.is_active flag." >&2
    local h mapped name
    for h in $(jq -r '[.markets[].hub_id] | map(select(. != null)) | unique | .[]' "$MARKET_CONFIG_FILE"); do
        name=""
        if [ -f "$HUBS_FILE" ]; then
            name=$(jq -r --arg h "$h" '.[$h].name // empty' "$HUBS_FILE")
        fi
        mapped=$(get_mapped_hub_id "$h")
        if [ -n "$mapped" ] && [ "$mapped" != "null" ]; then
            echo "  hub ${h}${name:+ (${name})} -> on-chain ${mapped}"
        else
            echo "  hub ${h}${name:+ (${name})} -> not created (created on first createMarket/setupAllMarkets)"
        fi
    done
}

# Per-market oracle wiring as configured in the markets JSON. The stored
# on-chain config is write-only (no view), so the JSON is the source of truth
# for wiring; use getPrice/getOracle for live price components.
list_oracles() {
    echo "=== Configured market oracles (${NETWORK}) ===" >&2
    local m src anchor
    for m in $(jq -r '.markets[].name' "$MARKET_CONFIG_FILE"); do
        jq -r --arg m "$m" 'first(.markets[] | select(.name == $m)) |
            "\(.name) (hub \(.hub_id // "?")): strategy=\(.oracle.strategy // "?") stale=\(.oracle.max_price_stale_seconds // "?")s tolerance=\(.oracle.tolerance_bps // "?")bps sanity=[\(.oracle.min_sanity_price_wad // "?") .. \(.oracle.max_sanity_price_wad // "?")]"' \
            "$MARKET_CONFIG_FILE" >&2
        src=$(jq -c --arg m "$m" 'first(.markets[] | select(.name == $m)) | .oracle.primary // null' "$MARKET_CONFIG_FILE")
        anchor=$(jq -c --arg m "$m" 'first(.markets[] | select(.name == $m)) | .oracle.anchor |
            if type == "object" and .tag == "Some" then .values[0] else null end' "$MARKET_CONFIG_FILE")
        describe_oracle_source "  primary" "$src"
        describe_oracle_source "  anchor " "$anchor"
    done
}

# ---------------------------------------------------------------------------
# XOXNO self-hosted oracle adapter (contracts/xoxno-oracle-adapter)
#
# Not governance-owned: a standalone contract with its own single admin key
# and bot signer set (see the contract's own doc comment). `add_feed` is a
# direct `stellar contract invoke`, not a timelocked governance proposal —
# there is no op record/replay machinery here, unlike the controller-targeted
# actions above.
# ---------------------------------------------------------------------------

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

# Maps a feeds-file `{tag, value}` asset descriptor to the CLI's JSON encoding
# of `ReflectorAsset` (`{"Stellar":"<address>"}` or `{"Other":"<symbol>"}`) —
# same convention `describe_oracle_source` already decodes for market oracles.
_oracle_asset_json() {
    local tag=$1 value=$2
    case "$tag" in
        Stellar) printf '{"Stellar":"%s"}' "$value" ;;
        Other)   printf '{"Other":"%s"}' "$value" ;;
        *) die "Unknown asset.tag '${tag}' (expected Stellar or Other)" ;;
    esac
}

# Lists every feed_id -> asset mapping configured in ${NETWORK}/oracle_feeds.json
# and calls add_feed for each. Tolerates FeedAlreadyMapped (already configured
# from a prior run) instead of aborting, so this is safe to re-run.
configure_oracle_feeds() {
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}. Run: make ${NETWORK} deployOracleAdapter"
    [ -f "$ORACLE_FEEDS_FILE" ] || die "Feeds config file not found: $ORACLE_FEEDS_FILE"

    echo "=== Configuring oracle feeds on ${NETWORK} (adapter ${adapter}) ===" >&2
    local count i feed_id tag value asset_json errfile out rc
    count=$(jq '.feeds | length' "$ORACLE_FEEDS_FILE")
    for ((i = 0; i < count; i++)); do
        feed_id=$(jq -r ".feeds[$i].feed_id" "$ORACLE_FEEDS_FILE")
        tag=$(jq -r ".feeds[$i].asset.tag" "$ORACLE_FEEDS_FILE")
        value=$(jq -r ".feeds[$i].asset.value" "$ORACLE_FEEDS_FILE")
        asset_json=$(_oracle_asset_json "$tag" "$value")

        echo "  add_feed ${feed_id} -> ${asset_json}" >&2
        errfile=$(mktemp)
        out=$(stellar contract invoke --id "$adapter" $SOURCE_FLAG --network "$NETWORK" \
            -- add_feed --feed_id "$feed_id" --asset "$asset_json" 2>"$errfile") && rc=0 || rc=$?
        if [ "$rc" -ne 0 ]; then
            if grep -qi "FeedAlreadyMapped" "$errfile"; then
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

# Read-only: dumps the adapter's live enumerable asset index.
list_oracle_feeds() {
    local adapter
    adapter=$(get_oracle_adapter_address) || die "No oracle adapter deployed for ${NETWORK}."
    echo "=== Oracle adapter feeds (${NETWORK}, ${adapter}) ===" >&2
    invoke_view "$adapter" assets
}

# Registers a bot wallet's Stellar public key as a signer (admin-gated direct
# call, tolerates SignerAlreadyRegistered so this is safe to re-run).
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
        if grep -qi "SignerAlreadyRegistered" "$errfile"; then
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

# ---------------------------------------------------------------------------
# Market-level views
# ---------------------------------------------------------------------------

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
    # get_market_config was removed; the asset's base spoke-0 listing
    # (SpokeAssetConfig) is the per-asset config read-back.
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
    local assets_json
    assets_json=$(all_configured_hub_assets)
    local ctrl
    ctrl=$(get_controller)
    echo "=== All markets (${NETWORK}) ===" >&2
    invoke_view "$ctrl" get_markets_detailed --hub_assets "$assets_json"
}

get_all_indexes_cmd() {
    local assets_json
    assets_json=$(all_configured_hub_assets)
    local ctrl
    ctrl=$(get_controller)
    echo "=== All market indexes (${NETWORK}) ===" >&2
    invoke_view "$ctrl" get_market_indexes_detailed --hub_assets "$assets_json"
}

# Live per-spoke-per-asset config for ANY spoke id (getMarket only reads the
# base spoke-0 listing). This is the "spoke usage/config" read the operator
# actually wants when checking a real spoke's live LTV/threshold/caps/paused.
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

# Largest withdraw/supply/borrow currently executable for the account (0 while
# paused or gated by caps/LTV/HF — useful for sizing before a write).
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

# Estimate seize/repay/refund/bonus for a planned liquidation. debt_payments
# are market/amount pairs (same [[{hub_id,asset},"amount"], ...] tuple-vec
# shape as supply/borrow/withdraw); omit to estimate with no explicit payment.
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

# ---------------------------------------------------------------------------
# Pool-level views (hub utilization / reserves / rates / revenue).
#
# The central pool holds liquidity at HUB scope, not per-spoke — spokes are a
# risk-config layer over shared hub liquidity, so there is no separate
# per-spoke supplied/borrowed figure to read. These are the "usage" numbers.
# ---------------------------------------------------------------------------

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

# ---------------------------------------------------------------------------
# Command dispatch
# ---------------------------------------------------------------------------

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
        # Converge mode: bulk setup treats Done ops as applied. Drift-proven
        # ensure_asset_in_spoke calls re-enable re-apply per call.
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
    "approveToken")
        if [ -z "$2" ]; then
            echo "Usage: $0 approveToken <market_or_contract_id>" >&2
            exit 1
        fi
        approve_token_cmd "$2"
        ;;
    "revokeToken")
        if [ -z "$2" ]; then
            echo "Usage: $0 revokeToken <market_or_contract_id>" >&2
            exit 1
        fi
        revoke_token_cmd "$2"
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
    "approveBlendPools")
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
        echo "  listMarkets                     List configured markets"
        echo "  createMarket <name>             Deploy market from config"
        echo "  configureMarketOracle <name>    Configure full market oracle from config"
        echo "  editOracleTolerance <m> <tol>   Edit a market's oracle tolerance band (bps)"
        echo "  updateIndexes <name> [...]      Sync indexes for one or more markets"
        echo "  setupAllMarkets                 Idempotently configure markets; no deploy/unpause"
        echo ""
        echo "Hubs / Spokes (writes):"
        echo "  listHubs                        Hubs referenced by config + on-chain mapping"
        echo "  createHub <id>                  Ensure hub exists (idempotent; ascending ids)"
        echo "  listSpokes                      List configured spoke categories"
        echo "  addSpoke <id>                   Create spoke category from config"
        echo "  addAssetToSpoke <id> <asset>    Add asset to spoke from config"
        echo "  editAssetInSpoke <id> <asset>   Push updated per-spoke risk params from config"
        echo "  removeAssetFromSpoke <id> <m>   Timelocked remove_asset_from_spoke"
        echo "  removeSpoke <id>                Timelocked remove_spoke (deprecates category)"
        echo "  setupAllSpokes                  Idempotently configure spokes; no deploy/unpause"
        echo ""
        echo "Timelock (admin writes are scheduled then executed after the delay):"
        echo "  Admin verbs (createMarket, configureMarketOracle, spoke,"
        echo "  setAggregator, disableTokenOracle, ...) SCHEDULE a governance op and, by default"
        echo "  (AUTO_EXECUTE=1), await the min-delay then execute it. Set AUTO_EXECUTE=0"
        echo "  to schedule-only and execute later with executeOp."
        echo "  Scheduling is idempotent AND re-apply-aware: an op already Waiting/Ready"
        echo "  is reused; toggling back to a previously-executed setting automatically"
        echo "  re-applies at a fresh salt generation (direct verbs). Bulk setupAll* runs"
        echo "  in converge mode (REAPPLY_ON_DONE=0): Done ops are treated as applied"
        echo "  unless an on-chain probe proves drift. SALT_NONCE=<n> = manual override."
        echo "  listOps                         All recorded ops with live state"
        echo "  executeReady                    Execute every recorded op that is Ready"
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
        echo "  checkDelay                      Compare live timelock delay vs configured target"
        echo "  disableTokenOracle <asset>      Timelock disable_token_oracle on controller"
        echo "  approveToken <m|C...>           Timelocked market-token allow-list add"
        echo "  revokeToken <m|C...>            Timelocked market-token allow-list remove"
        echo "  revokeBlendPool <C...>          Timelocked Blend-pool allow-list remove"
        echo "  setPositionLimits <s> <b>       Timelocked position limits (max supply/borrow positions)"
        echo "  setMinBorrowCollateralUsd <wad> Timelocked min borrow-collateral floor"
        echo "  setPositionManager <addr> <t|f> Timelocked position-manager toggle"
        echo "  transferCtrlOwnership <a> <l>   Timelocked controller ownership handoff"
        echo "  migrateController <version>     Timelocked controller migrate"
        echo "  grantGovRole <account> <role>   Grant governance role (ORACLE|PROPOSER|EXECUTOR|CANCELLER; timelocked)"
        echo "  revokeGovRole <account> <role>  Revoke governance role (timelocked)"
        echo "  upgradeGovernanceHash <hash>    Timelocked governance WASM upgrade"
        echo "  updateDelay <ledgers>           Timelocked min-delay increase (cannot shorten)"
        echo "  transferGovOwnership <addr> <ledger>  Timelocked governance ownership handoff"
        echo "  setAggregator                   Set aggregator (networks.json or AGGREGATOR_CONTRACT)"
        echo "  setAccumulator                  Set revenue treasury (networks.json accumulator or ACCUMULATOR_CONTRACT)"
        echo "  Env: AGGREGATOR_CONTRACT, ACCUMULATOR_CONTRACT, AWAIT_MAX_WAIT_SECONDS"
        echo "  setupAll                        Markets + Spokes only; no deploy/unpause"
        echo "  claimRevenue <name> [...]       Claim revenue one or more markets"
        echo "  claimRevenueAll                 Claim revenue for every configured market"
        echo "  whitelistBlendPools | approveBlendPools   Approve Blend V2 pools from configs/${NETWORK}/blend.json (timelocked)"
        echo ""
        echo "Quick views (reads):"
        echo "  info                            Deployment addresses & signer"
        echo "  listOracles                     Per-market oracle wiring from config"
        echo "  configureOracleFeeds           Call add_feed on xoxno_oracle_adapter for every entry in \${NETWORK}/oracle_feeds.json"
        echo "  listOracleFeeds                Live feed index from the deployed xoxno_oracle_adapter"
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
        echo "  NETWORK=testnet $0 disableTokenOracle C...USDC"
        echo "  SIGNER=ledger NETWORK=mainnet $0 pause"
        ;;
esac
