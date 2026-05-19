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

require_tool() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "ERROR: Missing required tool: $1" >&2
        exit 1
    fi
}

require_tool stellar
require_tool jq

# Source account flag
SIGNER_ADDRESS=$(stellar keys public-key "$SIGNER" 2>/dev/null || stellar keys address "$SIGNER" 2>/dev/null || echo "$SIGNER")
if [ "$SIGNER" = "ledger" ]; then
    SOURCE_FLAG="--source-account $SIGNER_ADDRESS --sign-with-ledger"
else
    SIGNER_SECRET=$(stellar keys secret "$SIGNER" 2>/dev/null || echo "$SIGNER")
    SOURCE_FLAG="--source-account $SIGNER_SECRET"
fi

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

get_redstone_feed() {
    local feed=$1
    jq -r --arg network "$NETWORK" --arg feed "$feed" \
        '.[$network].redstone_feeds[$feed] // empty' "$NETWORKS_FILE"
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
            "  \(.key) -> on-chain \($ids[.key] // "unmapped"): \(.value.name) — LTV=\(.value.ltv) Threshold=\(.value.liquidation_threshold) Bonus=\(.value.liquidation_bonus)"
        ' "$EMODES_FILE"
    else
        echo "  No emodes config found: $EMODES_FILE"
    fi
}

build_asset_addresses_json() {
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

        if [ $first -eq 0 ]; then
            assets_json+=","
        fi
        assets_json+="\"$asset_address\""
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

    local name=$(get_emode_value "$category_id" ".name")
    local ltv=$(get_emode_value "$category_id" ".ltv")
    local threshold=$(get_emode_value "$category_id" ".liquidation_threshold")
    local bonus=$(get_emode_value "$category_id" ".liquidation_bonus")

    echo "Adding E-Mode category ${category_id}: ${name}" >&2
    echo "  LTV: ${ltv}" >&2
    echo "  Liquidation Threshold: ${threshold}" >&2
    echo "  Liquidation Bonus: ${bonus}" >&2

    local ctrl=$(get_controller)
    local admin=$(get_signer_address)

    local result
    result=$(stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- add_e_mode_category \
        --ltv "$ltv" \
        --threshold "$threshold" \
        --bonus "$bonus")

    local onchain_id
    onchain_id=$(echo "$result" | sed -nE 's/.*([0-9]+).*/\1/p' | tail -n1)
    if [ -z "$onchain_id" ]; then
        echo "ERROR: Could not parse on-chain e-mode category id from result: $result" >&2
        exit 1
    fi

    echo "E-Mode category ${category_id} created with on-chain id ${onchain_id}." >&2
    echo "$onchain_id"
}

edit_emode_category() {
    local config_category_id=$1
    local onchain_id=$2

    local name=$(get_emode_value "$config_category_id" ".name")
    local ltv=$(get_emode_value "$config_category_id" ".ltv")
    local threshold=$(get_emode_value "$config_category_id" ".liquidation_threshold")
    local bonus=$(get_emode_value "$config_category_id" ".liquidation_bonus")
    local ctrl=$(get_controller)

    echo "Editing E-Mode category ${config_category_id} (${name}) on-chain id ${onchain_id}..."
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- edit_e_mode_category \
        --id "$onchain_id" \
        --ltv "$ltv" \
        --threshold "$threshold" \
        --bonus "$bonus"
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
    local onchain_id=$1
    local ctrl=$(get_controller)
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        --send=no -- get_e_mode_category --category_id "$onchain_id"
}

emode_params_match_config() {
    local category_json=$1
    local config_category_id=$2
    local ltv=$(get_emode_value "$config_category_id" ".ltv")
    local threshold=$(get_emode_value "$config_category_id" ".liquidation_threshold")
    local bonus=$(get_emode_value "$config_category_id" ".liquidation_bonus")

    printf '%s' "$category_json" | jq -e \
        --argjson ltv "$ltv" \
        --argjson threshold "$threshold" \
        --argjson bonus "$bonus" \
        '.loan_to_value_bps == $ltv and
         .liquidation_threshold_bps == $threshold and
         .liquidation_bonus_bps == $bonus' >/dev/null
}

emode_is_deprecated() {
    local category_json=$1
    printf '%s' "$category_json" | jq -e '.is_deprecated == true' >/dev/null
}

ensure_emode_category() {
    local config_category_id=$1
    local mapped_id
    local category_json

    mapped_id=$(get_mapped_emode_category_id "$config_category_id")
    if [ -n "$mapped_id" ] && [ "$mapped_id" != "null" ]; then
        if category_json=$(fetch_emode_category_json "$mapped_id" 2>/dev/null); then
            if emode_is_deprecated "$category_json"; then
                echo "Mapped E-Mode id ${mapped_id} for config ${config_category_id} is deprecated; creating a replacement."
            elif emode_params_match_config "$category_json" "$config_category_id"; then
                echo "E-Mode config ${config_category_id} already mapped to on-chain id ${mapped_id}."
                echo "$mapped_id"
                return 0
            else
                edit_emode_category "$config_category_id" "$mapped_id"
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
        elif emode_params_match_config "$category_json" "$config_category_id"; then
            persist_emode_category_id "$config_category_id" "$config_category_id"
            echo "E-Mode config ${config_category_id} matches existing on-chain id ${config_category_id}."
            echo "$config_category_id"
            return 0
        else
            echo "On-chain E-Mode id ${config_category_id} exists but does not match config; editing it."
            edit_emode_category "$config_category_id" "$config_category_id"
            persist_emode_category_id "$config_category_id" "$config_category_id"
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

    local asset_address=$(get_market_value "$asset_name" "asset_address")
    local can_collateral=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".can_be_collateral")
    local can_borrow=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".can_be_borrowed")

    echo "  Asset Address: ${asset_address}"
    echo "  Config Category: ${config_category_id}"
    echo "  Can Be Collateral: ${can_collateral}"
    echo "  Can Be Borrowed: ${can_borrow}"

    if [ -z "$asset_address" ] || [ "$asset_address" = "null" ] || [ "$asset_address" = "" ]; then
        echo "ERROR: No asset address found for ${asset_name} in ${MARKET_CONFIG_FILE}"
        exit 1
    fi

    local ctrl=$(get_controller)
    local admin=$(get_signer_address)

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- add_asset_to_e_mode_category \
        --asset "$asset_address" \
        --category_id "$category_id" \
        --can_collateral "$can_collateral" \
        --can_borrow "$can_borrow"

    echo "Asset ${asset_name} added to E-Mode category ${category_id}."
}

edit_asset_in_emode() {
    local category_id=$1
    local asset_name=$2
    local config_category_id=${3:-$category_id}

    local asset_address=$(get_market_value "$asset_name" "asset_address")
    local can_collateral=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".can_be_collateral")
    local can_borrow=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".can_be_borrowed")
    local ctrl=$(get_controller)

    echo "Editing asset ${asset_name} in E-Mode category ${category_id}..."
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- edit_asset_in_e_mode_category \
        --asset "$asset_address" \
        --category_id "$category_id" \
        --can_collateral "$can_collateral" \
        --can_borrow "$can_borrow"
}

ensure_asset_in_emode() {
    local category_id=$1
    local asset_name=$2
    local config_category_id=${3:-$category_id}

    local asset_address=$(get_market_value "$asset_name" "asset_address")
    local can_collateral=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".can_be_collateral")
    local can_borrow=$(get_emode_value "$config_category_id" ".assets.\"$asset_name\".can_be_borrowed")
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
            '.assets[$asset].is_collateralizable == $can_collateral and
             .assets[$asset].is_borrowable == $can_borrow' >/dev/null; then
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
    local categories=$(jq -r ".\"$NETWORK\" | keys[]" "$EMODES_FILE")

    for cat_id in $categories; do
        local onchain_id
        onchain_id=$(ensure_emode_category "$cat_id" | tail -n1)

        local assets=$(jq -r ".\"$NETWORK\".\"$cat_id\".assets | keys[]" "$EMODES_FILE")
        for asset_name in $assets; do
            ensure_asset_in_emode "$onchain_id" "$asset_name" "$cat_id"
        done
    done
    echo "=== All E-Mode categories configured ==="
}

# ---------------------------------------------------------------------------
# Market functions
# ---------------------------------------------------------------------------

create_market() {
    local market_name=$1

    echo "Creating market for ${market_name}..."

    local asset_address=$(get_market_value "$market_name" "asset_address")
    local decimals=$(get_contract_decimals "$asset_address")

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

    local ctrl=$(get_controller)
    local admin=$(get_signer_address)

    # Check if market already exists to avoid crash on re-runs
    if stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" --send=no -- get_market_config --asset "$asset_address" &>/dev/null; then
        echo "Market for ${market_name} already exists, skipping creation."
        return 0
    fi

    # Build params JSON from config
    local params=$(jq -c --arg decimals "$decimals" \
        ".markets[] | select(.name == \"$market_name\") | .market_params + {asset_id: .asset_address, asset_decimals: ($decimals | tonumber)}" \
        "$MARKET_CONFIG_FILE")
    # Markets are deployed in a pending state so they cannot be used before
    # oracle wiring and explicit activation.
    # `e_mode_categories` is a `Vec<u32>` populated by
    # `add_asset_to_e_mode_category` after the market exists; pin it
    # to an empty array at create time so the contract spec accepts
    # the JSON (jq emits `[]` which decodes to Vec::new).
    local pending_config=$(jq -c \
        ".markets[] | select(.name == \"$market_name\") | .asset_config | \
         .is_collateralizable = false | \
         .is_borrowable = false | \
         .is_flashloanable = false | \
         .isolation_borrow_enabled = false | \
         .e_mode_categories = []" \
        "$MARKET_CONFIG_FILE")

    # Post-audit (T1-7): the controller gates `create_liquidity_pool` behind an
    # admin allow-list. Pre-approve the token contract before creating the
    # market. `approve_token_wasm` is idempotent — calling on an already-approved
    # token is a no-op.
    echo "Approving token for market creation..."
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- approve_token_wasm \
        --token "$asset_address"

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- create_liquidity_pool \
        --asset "$asset_address" \
        --params "$params" \
        --config "$pending_config"

    echo "Market ${market_name} created."
}

edit_asset_config() {
    local market_name=$1

    echo "Editing asset config for ${market_name}..."

    local asset_address=$(get_market_value "$market_name" "asset_address")
    # `e_mode_categories` is contract-managed and ignored by
    # `edit_asset_config` on chain — pin to `[]` so the JSON shape
    # matches the AssetConfig spec.
    local config=$(jq -c \
        ".markets[] | select(.name == \"$market_name\") | .asset_config | .e_mode_categories = []" \
        "$MARKET_CONFIG_FILE")

    local ctrl=$(get_controller)
    local admin=$(get_signer_address)

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- edit_asset_config \
        --asset "$asset_address" \
        --cfg "$config"

    echo "Asset config updated for ${market_name}."
}

# Push the JSON's `market_params` (rate model + max_utilization_ray +
# reserve_factor) onto the pool via the controller's
# `upgrade_liquidity_pool_params` route. Use after changing any
# rate / utilization-ceiling field in the markets JSON.
update_market_params() {
    local market_name=$1

    echo "Updating market params for ${market_name}..."

    local asset_address=$(get_market_value "$market_name" "asset_address")
    # Strip `asset_id` / `asset_decimals` — those are controller-resolved
    # and the InterestRateModel struct does not carry them.
    local params=$(jq -c \
        ".markets[] | select(.name == \"$market_name\") | .market_params" \
        "$MARKET_CONFIG_FILE")

    local ctrl=$(get_controller)

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- upgrade_liquidity_pool_params \
        --asset "$asset_address" \
        --params "$params"

    echo "Market params updated for ${market_name}."
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
    assets_json=$(build_asset_addresses_json "$@")

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
    assets_json=$(build_asset_addresses_json "$@")

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- claim_revenue \
        --caller "$caller" \
        --assets "$assets_json"

    echo "Revenue claimed."
}

claim_revenue_all() {
    local assets_json
    assets_json=$(all_configured_asset_addresses)

    if [ -z "$assets_json" ] || [ "$assets_json" = "[]" ]; then
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
        --assets "$assets_json"

    echo "Revenue claimed for all markets."
}

set_aggregator() {
    echo "Configuring Aggregator for ${NETWORK}..."
    local router=$(jq -r ".\"$NETWORK\".aggregator" "$NETWORKS_FILE")

    if [ -z "$router" ] || [ "$router" = "null" ] || [ "$router" = "" ]; then
        echo "ERROR: No aggregator address found for ${NETWORK} in ${NETWORKS_FILE}"
        exit 1
    fi

    local ctrl=$(get_controller)
    echo "  Aggregator Address: ${router}"

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- set_aggregator \
        --addr "$router"

    echo "Aggregator configured on Controller."
}

set_accumulator() {
    echo "Configuring Accumulator for ${NETWORK}..."
    local accumulator=$(jq -r ".\"$NETWORK\".accumulator // .\"$NETWORK\".aggregator" "$NETWORKS_FILE")

    if [ -z "$accumulator" ] || [ "$accumulator" = "null" ] || [ "$accumulator" = "" ]; then
        echo "ERROR: No accumulator or aggregator address found for ${NETWORK} in ${NETWORKS_FILE}"
        exit 1
    fi

    local ctrl=$(get_controller)
    echo "  Accumulator Address: ${accumulator}"

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- set_accumulator \
        --addr "$accumulator"

    echo "Accumulator configured on Controller."
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
# Args: <market> <amount_raw> [<account_id:0>]
supply_position() {
    local market=$1
    local amount_raw=$2
    local account_id=${3:-0}

    local ctrl=$(get_controller)
    local caller=$SIGNER_ADDRESS
    local asset_addr=$(get_market_value "$market" "asset_address")

    echo "=== supply ==="
    echo "  Account: $account_id  (0 = create new)"
    echo "  Asset:   $market ($asset_addr)"
    echo "  Amount:  $amount_raw"
    echo

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- supply \
        --caller "$caller" \
        --account_id "$account_id" \
        --assets "[[\"$asset_addr\", $amount_raw]]"
}

# `borrow` — open a borrow position against existing collateral.
# Args: <market> <amount_raw> <account_id>
borrow_position() {
    local market=$1
    local amount_raw=$2
    local account_id=$3

    local ctrl=$(get_controller)
    local caller=$SIGNER_ADDRESS
    local asset_addr=$(get_market_value "$market" "asset_address")

    echo "=== borrow ==="
    echo "  Account: $account_id"
    echo "  Asset:   $market ($asset_addr)"
    echo "  Amount:  $amount_raw"
    echo

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- borrow \
        --caller "$caller" \
        --account_id "$account_id" \
        --borrows "[[\"$asset_addr\", $amount_raw]]"
}

configure_market_oracle() {
    local market_name=$1

    echo "Configuring market oracle for ${market_name}..."

    # Preflight: every oracle config must carry sanity bounds. On mainnet
    # the `(0, 0)` disabled-sentinel is rejected — that combination is for
    # test setups only. (Codex adversarial-review #4.)
    local missing=$(jq -r --arg m "$market_name" '
        .markets[] | select(.name == $m) | .oracle |
        (if has("min_sanity_price_wad") and has("max_sanity_price_wad")
            then "" else "missing min_sanity_price_wad / max_sanity_price_wad" end)
    ' "$MARKET_CONFIG_FILE")
    if [ -n "$missing" ]; then
        echo "ERROR: $market_name oracle config $missing" >&2
        exit 1
    fi
    if [ "$NETWORK" = "mainnet" ]; then
        local zero=$(jq -r --arg m "$market_name" '
            .markets[] | select(.name == $m) | .oracle |
            (if (.min_sanity_price_wad == "0" and .max_sanity_price_wad == "0")
                then "yes" else "no" end)
        ' "$MARKET_CONFIG_FILE")
        if [ "$zero" = "yes" ]; then
            echo "ERROR: $market_name uses (0, 0) sanity-bound sentinel on mainnet" >&2
            exit 1
        fi
    fi

    local asset_address=$(get_market_value "$market_name" "asset_address")
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

    local ctrl=$(get_controller)
    local admin=$(get_signer_address)

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- configure_market_oracle \
        --caller "$admin" \
        --asset "$asset_address" \
        --cfg-file-path "$cfg_file"

    rm -f "$cfg_file"

    echo "Market oracle configured for ${market_name}."
}

setup_all_markets() {
    echo "=== Setting up all markets for ${NETWORK} ==="
    local markets=$(jq -r '.markets[].name' "$MARKET_CONFIG_FILE")

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

# ---------------------------------------------------------------------------
# Pause / unpause
# ---------------------------------------------------------------------------

pause_protocol() {
    local ctrl=$(get_controller)
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" -- pause
    echo "Protocol paused on ${NETWORK}."
}

unpause_protocol() {
    local ctrl=$(get_controller)
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" -- unpause
    echo "Protocol unpaused on ${NETWORK}."
}

# ---------------------------------------------------------------------------
# Role management
# ---------------------------------------------------------------------------

grant_role_cmd() {
    local account=$1
    local role=$2
    local ctrl=$(get_controller)
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- grant_role --account "$account" --role "$role"
    echo "Role ${role} granted to ${account}."
}

revoke_role_cmd() {
    local account=$1
    local role=$2
    local ctrl=$(get_controller)
    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- revoke_role --account "$account" --role "$role"
    echo "Role ${role} revoked from ${account}."
}

has_role_cmd() {
    local account=$1
    local role=$2
    local ctrl=$(get_controller)
    invoke_view "$ctrl" has_role --account "$account" --role "$role"
}

# ---------------------------------------------------------------------------
# Info
# ---------------------------------------------------------------------------

show_info() {
    echo "=== Deployment info (${NETWORK}) ==="
    local ctrl_alias
    ctrl_alias=$(stellar contract alias show controller --network "$NETWORK" 2>/dev/null || echo "not deployed")
    local agg_alias
    agg_alias=$(stellar contract alias show aggregator --network "$NETWORK" 2>/dev/null || echo "not set")
    echo "Signer:     $(get_signer_address)"
    echo "Controller: ${ctrl_alias}"
    echo "Aggregator: ${agg_alias}"
    echo "Configured Aggregator: $(get_network_value "aggregator")"
    echo "Pool WASM Hash: $(get_network_value "pool_wasm_hash")"
    echo "E-Mode ID Map: $(jq -c --arg network "$NETWORK" '.[$network].emode_category_ids // {}' "$NETWORKS_FILE")"
    echo "Reflector CEX: $(get_cex_oracle)"
    echo "Reflector DEX: $(get_dex_oracle)"
    echo "Reflector FX:  $(get_fx_oracle)"
    echo "RedStone adapter: $(get_redstone_adapter)"
    echo "RedStone feeds: $(jq -r --arg network "$NETWORK" '(.[$network].redstone_feeds // {}) | keys | length' "$NETWORKS_FILE")"
}

# ---------------------------------------------------------------------------
# Market-level views
# ---------------------------------------------------------------------------

get_price() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local ctrl=$(get_controller)
    echo "=== Price for ${market_name} (${asset_address}) ===" >&2
    invoke_view "$ctrl" get_all_market_indexes_detailed --assets "[\"$asset_address\"]"
}

get_market_config_view_cmd() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local ctrl=$(get_controller)
    echo "=== Market config for ${market_name} (${asset_address}) ===" >&2
    invoke_view "$ctrl" get_market_config --asset "$asset_address"
}

get_index_cmd() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local ctrl=$(get_controller)
    echo "=== Index for ${market_name} (${asset_address}) ===" >&2
    invoke_view "$ctrl" get_all_market_indexes_detailed --assets "[\"$asset_address\"]"
}

get_isolated_debt_cmd() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local ctrl=$(get_controller)
    invoke_view "$ctrl" get_isolated_debt --asset "$asset_address"
}

get_emode_cmd() {
    local cat_id=$1
    local ctrl=$(get_controller)
    invoke_view "$ctrl" get_e_mode_category --category_id "$cat_id"
}

get_all_markets_cmd() {
    local assets_json
    assets_json=$(all_configured_asset_addresses)
    local ctrl=$(get_controller)
    echo "=== All markets (${NETWORK}) ===" >&2
    invoke_view "$ctrl" get_all_markets_detailed --assets "$assets_json"
}

get_all_indexes_cmd() {
    local assets_json
    assets_json=$(all_configured_asset_addresses)
    local ctrl=$(get_controller)
    echo "=== All market indexes (${NETWORK}) ===" >&2
    invoke_view "$ctrl" get_all_market_indexes_detailed --assets "$assets_json"
}

# ---------------------------------------------------------------------------
# Account-level views
# ---------------------------------------------------------------------------

get_health_cmd() {
    local account_id=$1
    local ctrl=$(get_controller)
    invoke_view "$ctrl" health_factor --account_id "$account_id"
}

get_account_cmd() {
    local account_id=$1
    local ctrl=$(get_controller)
    echo "=== Positions for account ${account_id} ===" >&2
    invoke_view "$ctrl" get_account_positions --account_id "$account_id"
    echo "=== Attributes for account ${account_id} ===" >&2
    invoke_view "$ctrl" get_account_attributes --account_id "$account_id"
}

get_collateral_usd_cmd() {
    local account_id=$1
    local ctrl=$(get_controller)
    invoke_view "$ctrl" total_collateral_in_usd --account_id "$account_id"
}

get_borrow_usd_cmd() {
    local account_id=$1
    local ctrl=$(get_controller)
    invoke_view "$ctrl" total_borrow_in_usd --account_id "$account_id"
}

get_ltv_usd_cmd() {
    local account_id=$1
    local ctrl=$(get_controller)
    invoke_view "$ctrl" ltv_collateral_in_usd --account_id "$account_id"
}

get_liq_available_cmd() {
    local account_id=$1
    local ctrl=$(get_controller)
    invoke_view "$ctrl" liquidation_collateral_available --account_id "$account_id"
}

can_liquidate_cmd() {
    local account_id=$1
    local ctrl=$(get_controller)
    invoke_view "$ctrl" can_be_liquidated --account_id "$account_id"
}

get_collateral_cmd() {
    local account_id=$1
    local market_name=$2
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local ctrl=$(get_controller)
    invoke_view "$ctrl" collateral_amount_for_token --account_id "$account_id" --asset "$asset_address"
}

get_borrow_cmd() {
    local account_id=$1
    local market_name=$2
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local ctrl=$(get_controller)
    invoke_view "$ctrl" borrow_amount_for_token --account_id "$account_id" --asset "$asset_address"
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

# Compound view: reads a market's stored Oracle V2 config and prints the
# provider-agnostic primary/anchor wiring.
get_oracle_cmd() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local ctrl=$(get_controller)

    local mc_json
    mc_json=$(stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        --send=no -- get_market_config --asset "$asset_address")

    local oracle_json primary_json anchor_json anchor_tag anchor_value
    oracle_json=$(printf '%s' "$mc_json" | jq -c '.oracle_config // .')
    primary_json=$(printf '%s' "$oracle_json" | jq -c '.primary')
    anchor_json=$(printf '%s' "$oracle_json" | jq -c '.anchor // null')
    anchor_tag=$(printf '%s' "$anchor_json" | oracle_union_tag 2>/dev/null || echo "None")

    echo "=== Oracle V2 config for ${market_name} (${asset_address}) ===" >&2
    printf '%s\n' "$oracle_json" | jq .
    describe_oracle_source "primary" "$primary_json"
    if [ "$anchor_tag" = "Some" ]; then
        anchor_value=$(printf '%s' "$anchor_json" | oracle_union_value)
        describe_oracle_source "anchor" "$anchor_value"
    else
        echo "[anchor] not configured" >&2
    fi
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
    "configureMarketOracle")
        if [ -z "$2" ]; then
            echo "Usage: $0 configureMarketOracle <market_name>"
            list_markets
            exit 1
        fi
        configure_market_oracle "$2"
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
    "setAggregator")
        set_aggregator
        ;;
    "setAccumulator")
        set_accumulator
        ;;
    "supply")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 supply <market> <amount_raw> [<account_id:0>]" >&2
            list_markets >&2
            exit 1
        fi
        supply_position "$2" "$3" "$4"
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
    "grantRole")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 grantRole <account> <role>" >&2
            echo "Roles: KEEPER | REVENUE | ORACLE" >&2
            exit 1
        fi
        grant_role_cmd "$2" "$3"
        ;;
    "revokeRole")
        if [ -z "$2" ] || [ -z "$3" ]; then
            echo "Usage: $0 revokeRole <account> <role>" >&2
            exit 1
        fi
        revoke_role_cmd "$2" "$3"
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
    "getIsolatedDebt")
        if [ -z "$2" ]; then echo "Usage: $0 getIsolatedDebt <market>" >&2; list_markets >&2; exit 1; fi
        get_isolated_debt_cmd "$2"
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
        echo "  updateIndexes <name> [...]      Sync indexes for one or more markets"
        echo "  setupAllMarkets                 Idempotently configure markets; no deploy/unpause"
        echo ""
        echo "E-Mode (writes):"
        echo "  listEModeCategories             List configured e-mode categories"
        echo "  addEModeCategory <id>           Create e-mode category from config"
        echo "  addAssetToEMode <id> <asset>    Add asset to e-mode from config"
        echo "  setupAllEModes                  Idempotently configure e-modes; no deploy/unpause"
        echo ""
        echo "Protocol control (writes):"
        echo "  pause | unpause                 Pause/unpause protocol"
        echo "  grantRole <account> <role>      Grant KEEPER | REVENUE | ORACLE"
        echo "  revokeRole <account> <role>     Revoke role"
        echo "  setAggregator                   Set aggregator from networks.json"
        echo "  setAccumulator                  Set accumulator from networks.json or aggregator fallback"
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
        echo "  getIsolatedDebt <market>        Isolated-mode borrow usage"
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
        echo "  NETWORK=testnet $0 grantRole GAB... KEEPER"
        echo "  SIGNER=ledger NETWORK=mainnet $0 pause"
        ;;
esac
