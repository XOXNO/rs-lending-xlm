#!/bin/bash
# ===========================================================================
# Stellar Lending Protocol — Deployment & Configuration Script
#
# Mirrors the MultiversX configs/script.sh pattern:
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

get_oracle() {
    # Reflector contracts are fixed per network, no need for alias
    if [ "$NETWORK" = "mainnet" ]; then
        echo "CAFJZQWSED6YAWZU3GWRTOCNPPCGBN32L7QV43XX5LFTK6JLN34DLN"
    else
        echo "CCYOZJCOPG34LLQQ7N24YXBM7LL62R7ONMZ3G6WZAAYPB5OYKOMJRN63"
    fi
}

get_signer_address() {
    echo "$SIGNER_ADDRESS"
}

invoke_view() {
    stellar contract invoke --id "$1" $SOURCE_FLAG --network "$NETWORK" --send=no -- "${@:2}"
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
            first_tolerance_bps: (.oracle.first_tolerance_bps | tostring),
            last_tolerance_bps: (.oracle.last_tolerance_bps | tostring),
            cex_oracle: .reflector.cex_oracle,
            cex_asset_kind: .reflector.cex_asset_kind,
            cex_symbol: .reflector.cex_symbol,
            dex_oracle: (.reflector.dex_oracle // null),
            dex_asset_kind: .reflector.dex_asset_kind,
            dex_symbol: (.reflector.dex_symbol // .reflector.cex_symbol),
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
# Command dispatch (matches MultiversX script.sh pattern)
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
    *)
        echo "Stellar Lending Protocol — Configuration Script"
        echo ""
        echo "Usage: NETWORK=$NETWORK $0 <command> [args...]"
        echo ""
        echo "Markets:"
        echo "  listMarkets                     List configured markets"
        echo "  createMarket <name>             Deploy market from config"
        echo "  editAssetConfig <name>          Update asset risk params from config"
        echo "  configureMarketOracle <name>    Configure full market oracle from config"
        echo "  updateIndexes <name> [...]      Sync indexes for one or more markets"
        echo "  setupAllMarkets                 Create, configure oracle, then enable all markets"
        echo ""
        echo "E-Mode:"
        echo "  listEModeCategories             List configured e-mode categories"
        echo "  addEModeCategory <id>           Create e-mode category from config"
        echo "  addAssetToEMode <id> <asset>    Add asset to e-mode from config"
        echo "  setupAllEModes                  Create all e-modes from config"
        echo ""
        echo "Full Setup:"
        echo "  setupAll                        Markets + E-Modes from config"
        echo ""
        echo "System:"
        echo "  setAggregator                   Set aggregator address from networks.json"
        echo ""
        echo "Examples:"
        echo "  NETWORK=testnet $0 addEModeCategory 1"
        echo "  NETWORK=testnet $0 addAssetToEMode 1 USDC"
        echo "  NETWORK=testnet $0 updateIndexes USDC XLM"
        echo "  NETWORK=mainnet $0 setupAll"
        echo "  SIGNER=ledger NETWORK=mainnet $0 createMarket USDC"
        ;;
esac
