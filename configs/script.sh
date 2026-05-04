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
        jq -r ".\"$NETWORK\" | to_entries[] | \"  \(.key): \(.value.name) — LTV=\(.value.ltv) Threshold=\(.value.liquidation_threshold) Bonus=\(.value.liquidation_bonus)\"" "$EMODES_FILE"
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

setup_all_emodes() {
    echo "=== Setting up all E-Mode categories for ${NETWORK} ==="
    local categories=$(jq -r ".\"$NETWORK\" | keys[]" "$EMODES_FILE")

    for cat_id in $categories; do
        local onchain_id
        onchain_id=$(add_emode_category "$cat_id")

        local assets=$(jq -r ".\"$NETWORK\".\"$cat_id\".assets | keys[]" "$EMODES_FILE")
        for asset_name in $assets; do
            add_asset_to_emode "$onchain_id" "$asset_name" "$cat_id"
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
    local pending_config=$(jq -c \
        ".markets[] | select(.name == \"$market_name\") | .asset_config | \
         .is_collateralizable = false | \
         .is_borrowable = false | \
         .is_flashloanable = false | \
         .e_mode_enabled = false | \
         .isolation_borrow_enabled = false" \
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
    local config=$(jq -c ".markets[] | select(.name == \"$market_name\") | .asset_config" "$MARKET_CONFIG_FILE")

    local ctrl=$(get_controller)
    local admin=$(get_signer_address)

    stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        -- edit_asset_config \
        --asset "$asset_address" \
        --cfg "$config"

    echo "Asset config updated for ${market_name}."
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
        -- borrow_batch \
        --caller "$caller" \
        --account_id "$account_id" \
        --borrows "[[\"$asset_addr\", $amount_raw]]"
}

configure_market_oracle() {
    local market_name=$1

    echo "Configuring market oracle for ${market_name}..."

    local asset_address=$(get_market_value "$market_name" "asset_address")
    local cfg_file
    cfg_file=$(mktemp)
    jq -c "
        .markets[] | select(.name == \"$market_name\") | {
            exchange_source: .oracle.exchange_source,
            max_price_stale_seconds: .oracle.max_price_stale_seconds,
            first_tolerance_bps: .oracle.first_tolerance_bps,
            last_tolerance_bps: .oracle.last_tolerance_bps,
            cex_oracle: .reflector.cex_oracle,
            cex_asset_kind: .reflector.cex_asset_kind,
            cex_symbol: .reflector.cex_symbol,
            dex_oracle: (.reflector.dex_oracle // null),
            dex_asset_kind: .reflector.dex_asset_kind,
            # Dead metadata when dex_oracle is null — send empty string so
            # storage doesn't carry a misleading ticker for an unused leg.
            dex_symbol: (if .reflector.dex_oracle == null
                         then \"\"
                         else (.reflector.dex_symbol // .reflector.cex_symbol)
                         end),
            twap_records: .reflector.twap_records
        }
    " "$MARKET_CONFIG_FILE" > "$cfg_file"

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
    echo "Reflector CEX: $(get_cex_oracle)"
    echo "Reflector DEX: $(get_dex_oracle)"
    echo "Reflector FX:  $(get_fx_oracle)"
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

# Compound view: reads a market's stored CEX (and DEX if set) oracle addresses
# and dumps live data from each. This is what you want to verify DualOracle
# wiring end-to-end without trusting your own JSON config.
get_reflector_cmd() {
    local market_name=$1
    local asset_address
    asset_address=$(require_market_address "$market_name")
    local ctrl=$(get_controller)

    # Pull the raw MarketConfig once and cache.
    local mc_json
    mc_json=$(stellar contract invoke --id "$ctrl" $SOURCE_FLAG --network "$NETWORK" \
        --send=no -- get_market_config --asset "$asset_address")

    local cex_oracle cex_kind cex_symbol dex_oracle dex_kind dex_symbol twap_records
    cex_oracle=$(echo "$mc_json" | jq -r '.cex_oracle // empty')
    cex_kind=$(echo "$mc_json"   | jq -r '.cex_asset_kind')
    cex_symbol=$(echo "$mc_json" | jq -r '.cex_symbol')
    dex_oracle=$(echo "$mc_json" | jq -r '.dex_oracle // empty')
    dex_kind=$(echo "$mc_json"   | jq -r '.dex_asset_kind')
    dex_symbol=$(echo "$mc_json" | jq -r '.dex_symbol')
    twap_records=$(echo "$mc_json" | jq -r '.twap_records')

    local cex_value="$cex_symbol"
    [ "$cex_kind" = "0" ] && cex_value="$asset_address"
    local dex_value="$dex_symbol"
    [ "$dex_kind" = "0" ] && dex_value="$asset_address"
    local cex_kind_str="other"
    [ "$cex_kind" = "0" ] && cex_kind_str="stellar"
    local dex_kind_str="other"
    [ "$dex_kind" = "0" ] && dex_kind_str="stellar"

    echo "=== Live Reflector view for ${market_name} (${asset_address}) ===" >&2
    echo "[CEX leg]  ${cex_oracle} kind=${cex_kind_str} value=${cex_value}" >&2
    if [ -n "$cex_oracle" ]; then
        query_reflector_price_cmd "$cex_oracle" "$cex_kind_str" "$cex_value"
        query_reflector_twap_cmd "$cex_oracle" "$cex_kind_str" "$cex_value" "$twap_records"
    fi

    if [ -n "$dex_oracle" ]; then
        echo "[DEX leg]  ${dex_oracle} kind=${dex_kind_str} value=${dex_value}" >&2
        query_reflector_price_cmd "$dex_oracle" "$dex_kind_str" "$dex_value"
        query_reflector_twap_cmd "$dex_oracle" "$dex_kind_str" "$dex_value" "$twap_records"
    else
        echo "[DEX leg]  not configured (dex_oracle=null) — market is SpotVsTwap-only" >&2
    fi
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
        echo "  setupAllMarkets                 Create, configure oracle, then enable all markets"
        echo ""
        echo "E-Mode (writes):"
        echo "  listEModeCategories             List configured e-mode categories"
        echo "  addEModeCategory <id>           Create e-mode category from config"
        echo "  addAssetToEMode <id> <asset>    Add asset to e-mode from config"
        echo "  setupAllEModes                  Create all e-modes from config"
        echo ""
        echo "Protocol control (writes):"
        echo "  pause | unpause                 Pause/unpause protocol"
        echo "  grantRole <account> <role>      Grant KEEPER | REVENUE | ORACLE"
        echo "  revokeRole <account> <role>     Revoke role"
        echo "  setAggregator                   Set aggregator from networks.json"
        echo "  setupAll                        Markets + E-Modes from config"
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
        echo "Reflector probes (debug DualOracle wiring):"
        echo "  getReflector <market>                                Live CEX + DEX data for a market"
        echo "  queryReflector <oracle>                              decimals + resolution"
        echo "  queryReflectorPrice <oracle> stellar|other <sym|sac> lastprice"
        echo "  queryReflectorTwap  <oracle> stellar|other <sym|sac> [records] prices history"
        echo ""
        echo "Examples:"
        echo "  NETWORK=testnet $0 getPrice USDC"
        echo "  NETWORK=testnet $0 getHealth 1"
        echo "  NETWORK=testnet $0 getCollateral 1 XLM"
        echo "  NETWORK=testnet $0 grantRole GAB... KEEPER"
        echo "  SIGNER=ledger NETWORK=mainnet $0 pause"
        ;;
esac
