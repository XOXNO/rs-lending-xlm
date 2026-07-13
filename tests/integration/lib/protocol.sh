# Protocol deployment and market administration against a fresh controller.
#
# deploy_protocol() is a **fast-path integration harness**: EOA-owned controller,
# immediate admin calls (no governance timelock). Production / operator deploys
# use the governance-owned path instead:
#   make testnet setup   (or make mainnet setup with AGGREGATOR_CONTRACT=...)
# See configs/script.sh + Makefile _deploy / configure-controller.
#
# Market creation follows the production sequence on an explicit created hub:
# create_liquidity_pool (pending primary spoke listing: not
# collateralizable/borrowable) → resolve_market_oracle_config (governance view) →
# set_market_oracle_config → add_asset_to_spoke on the primary spoke. Oracle
# configs for mock markets must use Twap (Spot-only primaries reject with
# SpotOnlyNotProductionSafe #38) and market params must include max_utilization.

# Uploads pool wasm, deploys controller + central pool + flash receiver,
# wires aggregator/accumulator/roles, unpauses. Persists:
# CONTROLLER, POOL, POOL_HASH, FLASH_RECEIVER, XLM_SAC, PRIMARY_HUB_ID,
# SECONDARY_HUB_ID.
deploy_protocol() {
    if [ -z "${XLM_SAC:-}" ]; then
        save_state XLM_SAC "$(stellar contract id asset --asset native --network "$NETWORK")"
    fi
    if [ -z "${POOL_HASH:-}" ]; then
        local out_f="$LOG_DIR/upload_pool.out" err_f="$LOG_DIR/upload_pool.err"
        run_deploy "$out_f" "$err_f" -- stellar contract upload --wasm "$WASM_DIR/pool.wasm" \
            --source "$ADMIN" --network "$NETWORK"
        local hash txh
        hash=$(sanitize_output "$out_f")
        txh=$(extract_signing_hash "$err_f")
        is_wasm_hash "$hash" || die upload_pool_wasm "pool wasm upload produced no hash after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
        save_state POOL_HASH "$hash"
        record upload_pool_wasm ok upload "$txh" "" "" "" "" "$hash"
    fi
    if [ -z "${CONTROLLER:-}" ]; then
        local out_f="$LOG_DIR/deploy_controller.out" err_f="$LOG_DIR/deploy_controller.err"
        run_deploy "$out_f" "$err_f" -- stellar contract deploy --wasm "$WASM_DIR/controller.wasm" \
            --source "$ADMIN" --network "$NETWORK" -- --admin "$ADMIN_ADDR"
        local ctrl txh
        ctrl=$(sanitize_output "$out_f")
        txh=$(extract_signing_hash "$err_f")
        is_contract_id "$ctrl" || die deploy_controller "controller deploy produced no id after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
        save_state CONTROLLER "$ctrl"
        record deploy_controller ok deploy "$txh" "" "" "" "" "$ctrl"
        log "controller = $ctrl"
    fi
    if [ -z "${POOL:-}" ]; then
        inv set_pool_template "$ADMIN" "$CONTROLLER" -- set_liquidity_pool_template --hash "$POOL_HASH" >/dev/null \
            || die set_pool_template "set_liquidity_pool_template failed after $INV_MAX_ATTEMPTS attempts"
        local pool
        # deploy_pool reads the template set_pool_template just wrote; a lagging
        # RPC replica that has not synced that write panics TemplateNotSet (#26).
        # The write committed, so re-simulate the contract error with backoff
        # until the replica catches up (same read-after-write handling as
        # create_market) — a genuinely unset template recurs and dies below.
        pool=$(INV_TRANSIENT_CONTRACT_RE='Error\(Contract, #26\)' \
            inv deploy_pool "$ADMIN" "$CONTROLLER" -- deploy_pool | tr -d '"\n')
        is_contract_id "$pool" || die deploy_pool "central pool deploy produced no id after $INV_MAX_ATTEMPTS attempts"
        save_state POOL "$pool"
        log "central pool = $pool"
    fi
    if [ -z "${WIRED:-}" ]; then
        inv set_aggregator "$ADMIN" "$CONTROLLER" -- set_aggregator --addr "$AGGREGATOR" >/dev/null
        # Revenue treasury (wallet ok). Not the swap aggregator — claim_revenue
        # forwards SAC balances here and fails with NoAccumulator (#211) if unset.
        inv set_accumulator "$ADMIN" "$CONTROLLER" -- set_accumulator --addr "$ADMIN_ADDR" >/dev/null
        save_state WIRED 1
    fi
    if [ -z "${PRIMARY_HUB_ID:-}" ]; then
        create_test_hub PRIMARY
    fi
    if [ -z "${SECONDARY_HUB_ID:-}" ]; then
        create_test_hub SECONDARY
    fi
    if [ -z "${PRIMARY_SPOKE_ID:-}" ]; then
        create_test_spoke PRIMARY
    fi
    if [ -z "${SECONDARY_SPOKE_ID:-}" ]; then
        create_test_spoke SECONDARY
    fi
    if [ -z "${FLASH_RECEIVER:-}" ]; then
        local out_f="$LOG_DIR/deploy_flashrecv.out" err_f="$LOG_DIR/deploy_flashrecv.err"
        run_deploy "$out_f" "$err_f" -- stellar contract deploy --wasm "$WASM_DIR/flash_loan_receiver.wasm" \
            --source "$ADMIN" --network "$NETWORK"
        local recv txh
        recv=$(sanitize_output "$out_f")
        txh=$(extract_signing_hash "$err_f")
        is_contract_id "$recv" || die deploy_flash_receiver "flash receiver deploy produced no id after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
        save_state FLASH_RECEIVER "$recv"
        record deploy_flash_receiver ok deploy "$txh" "" "" "" "" "$recv"
    fi
    if [ -z "${UNPAUSED:-}" ]; then
        inv unpause "$ADMIN" "$CONTROLLER" -- unpause >/dev/null
        save_state UNPAUSED 1
    fi
    # Governance contract: drives the timelock e2e (flows/governance.sh) and
    # resolves oracle configs (input -> resolved MarketOracleConfig) for the
    # EOA controller's markets via its read-only resolver views. Owner is the
    # EOA admin so propose/execute/cancel/pause run without a separate signer.
    if [ -z "${GOVERNANCE:-}" ]; then
        local out_f="$LOG_DIR/deploy_governance.out" err_f="$LOG_DIR/deploy_governance.err"
        run_deploy "$out_f" "$err_f" -- stellar contract deploy --wasm "$WASM_DIR/governance.wasm" \
            --source "$ADMIN" --network "$NETWORK" \
            -- --admin "$ADMIN_ADDR" --min_delay "$INTEG_MIN_DELAY"
        local gov txh
        gov=$(sanitize_output "$out_f")
        txh=$(extract_signing_hash "$err_f")
        is_contract_id "$gov" || die deploy_governance "governance deploy produced no id after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
        save_state GOVERNANCE "$gov"
        record deploy_governance ok deploy "$txh" "" "" "" "" "$gov"
        log "governance = $gov"
    fi
    # Controller WASM hash for the governance-owned controller below. Uploading
    # the same bytes the EOA controller runs keeps the resolver probe faithful.
    if [ -z "${CTRL_HASH:-}" ]; then
        local out_f="$LOG_DIR/upload_controller.out" err_f="$LOG_DIR/upload_controller.err"
        run_deploy "$out_f" "$err_f" -- stellar contract upload --wasm "$WASM_DIR/controller.wasm" \
            --source "$ADMIN" --network "$NETWORK"
        local chash txh
        chash=$(sanitize_output "$out_f")
        txh=$(extract_signing_hash "$err_f")
        is_wasm_hash "$chash" || die upload_controller_wasm "controller wasm upload produced no hash after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
        save_state CTRL_HASH "$chash"
        record upload_controller_wasm ok upload "$txh" "" "" "" "" "$chash"
    fi
    # Governance-owned controller: required so resolve_market_oracle_config has a
    # controller to read (get_controller); also the target of the timelock e2e.
    if [ -z "${GOV_CONTROLLER:-}" ]; then
        local gc
        gc=$(inv deploy_controller "$ADMIN" "$GOVERNANCE" -- deploy_controller \
            --wasm_hash "$CTRL_HASH" | tr -d '"\n')
        is_contract_id "$gc" || die deploy_gov_controller "governance-owned controller deploy produced no id after $INV_MAX_ATTEMPTS attempts"
        save_state GOV_CONTROLLER "$gc"
        log "governance-owned controller = $gc"
    fi
}

# create_test_hub <LABEL>
# Creates a controller hub and saves HUB_<LABEL>_ID. PRIMARY and SECONDARY also
# populate PRIMARY_HUB_ID / SECONDARY_HUB_ID for existing flows.
create_test_hub() {
    local label="$1" id var
    var="HUB_${label}_ID"
    [ -n "${!var:-}" ] && return 0
    id=$(inv "create_hub_${label}" "$ADMIN" "$CONTROLLER" -- create_hub | tr -d '"[:space:]') || return 1
    [[ "$id" =~ ^[1-9][0-9]*$ ]] || die "create_hub_${label}" "create_hub returned invalid hub id '$id'"
    save_state "$var" "$id"
    case "$label" in
        PRIMARY) save_state PRIMARY_HUB_ID "$id" ;;
        SECONDARY) save_state SECONDARY_HUB_ID "$id" ;;
    esac
    record "hub_${label}_created" ok create_hub "" "" "" "" "" "hub_id=$id"
}

create_test_spoke() {
    local label="$1" id var
    var="${label}_SPOKE_ID"
    [ -n "${!var:-}" ] && return 0
    id=$(inv "add_spoke_${label}" "$ADMIN" "$CONTROLLER" -- add_spoke | tr -d '"[:space:]') || return 1
    [[ "$id" =~ ^[1-9][0-9]*$ ]] || die "add_spoke_${label}" "add_spoke returned invalid spoke id '$id'"
    save_state "$var" "$id"
    record "spoke_${label}_created" ok add_spoke "" "" "" "" "" "spoke_id=$id"
}

primary_hub_id() {
    echo "${PRIMARY_HUB_ID:?PRIMARY_HUB_ID missing; deploy_protocol must create hub first}"
}

primary_spoke_id() {
    echo "${PRIMARY_SPOKE_ID:?PRIMARY_SPOKE_ID missing; deploy_protocol must create spoke first}"
}

# Standard interest-rate model + caps for a market. Flash-loan eligibility and
# fee live on MarketParamsRaw (moved off the per-asset spoke config).
#   market_params_json <sac-id> <decimals>
market_params_json() {
    local sac="$1" decimals="$2"
    jq -nc --arg sac "$sac" --argjson dec "$decimals" '{
        max_borrow_rate: "2000000000000000000000000000",
        base_borrow_rate: "10000000000000000000000000",
        slope1: "40000000000000000000000000",
        slope2: "100000000000000000000000000",
        slope3: "1500000000000000000000000000",
        mid_utilization: "500000000000000000000000000",
        optimal_utilization: "800000000000000000000000000",
        max_utilization: "950000000000000000000000000",
        reserve_factor: 1000,
        supply_cap: "0",
        borrow_cap: "0",
        is_flashloanable: true,
        flashloan_fee: 5,
        asset_id: $sac,
        asset_decimals: $dec
    }'
}

# Per-asset spoke risk config (SpokeAssetConfig) for the primary spoke listing
# passed to create_liquidity_pool. Extra jq filter applied last.
#   asset_config_json <ltv-bps> <threshold-bps> <bonus-bps> [jq-overrides]
asset_config_json() {
    local ltv="$1" thr="$2" bonus="$3" overrides="${4:-.}"
    jq -nc --argjson ltv "$ltv" --argjson thr "$thr" --argjson bonus "$bonus" '{
        is_collateralizable: true,
        is_borrowable: true,
        paused: false,
        frozen: false,
        loan_to_value: $ltv,
        liquidation_threshold: $thr,
        liquidation_bonus: $bonus,
        liquidation_fees: 100,
        supply_cap: "0",
        borrow_cap: "0",
        oracle_override: "None"
    }' | jq -c "$overrides"
}

# Spoke asset input struct (SpokeAssetArgs) add_asset_to_spoke /
# edit_asset_in_spoke (single argument). spoke_id 0 base listing.
# `paused`/`frozen` default to false: a freshly listed asset is fully active.
# spoke_args <hub_id> <asset> <spoke_id> <can_collateral> <can_borrow> \
# <ltv> <threshold> <bonus> [supply_cap] [borrow_cap]
spoke_args() {
    jq -nc --argjson hub "$1" --arg asset "$2" --argjson spoke "$3" --argjson cc "$4" --argjson cb "$5" \
        --argjson ltv "$6" --argjson thr "$7" --argjson bonus "$8" \
        --arg sc "${9:-0}" --arg bc "${10:-0}" '{
        hub_id: $hub,
        asset: $asset,
        spoke_id: $spoke,
        can_collateral: $cc,
        can_borrow: $cb,
        paused: false,
        frozen: false,
        ltv: $ltv,
        threshold: $thr,
        bonus: $bonus,
        liquidation_fees: 100,
        supply_cap: $sc,
        borrow_cap: $bc,
        oracle_override: "None"
    }'
}

# Single-source oracle config: Reflector-shaped mock, Twap(3). The sanity band
# is a tight +/-10% around $1 (the mock's fixed price), the widest a `Single`
# strategy may use. Flows that crash the mock price below this band must use the
# anchored (`oracle_cfg_mock_dual`) shape, which is exempt from the cap.
#   oracle_cfg_mock_single <sac-id>
oracle_cfg_mock_single() {
    local sac="$1"
    jq -nc --arg mock "$MOCK" --arg sac "$sac" '{
        max_price_stale_seconds: 3600,
        tolerance_bps: 500,
        strategy: 0,
        primary: {Reflector: {contract: $mock, asset: {Stellar: $sac}, read_mode: {Twap: 3}}},
        anchor: "None",
        min_sanity_price_wad: "900000000000000000",
        max_sanity_price_wad: "1100000000000000000"
    }'
}

# Dual-source (mainnet-faithful) oracle config: mock Reflector primary +
# mock RedStone anchor. Anchor MUST be a different provider kind.
#   oracle_cfg_mock_dual <sac-id> <feed-id>
oracle_cfg_mock_dual() {
    local sac="$1" feed="$2"
    jq -nc --arg mock "$MOCK" --arg mockrs "$MOCKRS" --arg sac "$sac" --arg feed "$feed" '{
        max_price_stale_seconds: 3600,
        tolerance_bps: 500,
        strategy: 1,
        primary: {Reflector: {contract: $mock, asset: {Stellar: $sac}, read_mode: {Twap: 3}}},
        anchor: {Some: {RedStone: {contract: $mockrs, feed_id: $feed, max_stale_seconds: 3600}}},
        min_sanity_price_wad: "1000000000000000",
        max_sanity_price_wad: "1000000000000000000000"
    }'
}

# Real Reflector CEX feed by symbol, Twap(3), with sanity bounds.
#   oracle_cfg_reflector <SYMBOL> <min-sanity-wad> <max-sanity-wad>
oracle_cfg_reflector() {
    local sym="$1" min_wad="$2" max_wad="$3"
    jq -nc --arg orc "$REFLECTOR_CEX" --arg sym "$sym" --arg min "$min_wad" --arg max "$max_wad" '{
        max_price_stale_seconds: 3600,
        tolerance_bps: 500,
        strategy: 0,
        primary: {Reflector: {contract: $orc, asset: {Symbol: $sym}, read_mode: {Twap: 3}}},
        anchor: "None",
        min_sanity_price_wad: $min,
        max_sanity_price_wad: $max
    }'
}

# True when asset already has a primary spoke listing on the created hub - i.e.
# create_liquidity_pool already ran (it writes last step and duplicate calls
# panic AssetAlreadySupported). Lets create_market skip the non-idempotent
# create step when resuming a run interrupted before activation.
# market_listing_exists <hub-id> <sac-id>
market_listing_exists() {
    local hub_id="$1" sac="$2"
    stellar contract invoke --id "$CONTROLLER" --source "$ADMIN" --network "$NETWORK" \
        --send=no -- get_spoke_asset --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$hub_id" "$sac")" >/dev/null 2>&1
}

# Confirms just-created market's primary spoke listing is visible AND active.
# market_wait_listed <hub-id> <sac-id>
market_wait_listed() {
    local hub_id="$1" sac="$2" probe got
    for probe in $(seq 1 8); do
        got=$(stellar contract invoke --id "$CONTROLLER" --source "$ADMIN" --network "$NETWORK" \
            --send=no -- get_spoke_asset --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$hub_id" "$sac")" 2>/dev/null \
            | jq -r '.is_borrowable // empty' 2>/dev/null)
        [ "$got" = "true" ] && return 0
        sleep $(( probe * 2 ))
    done
    return 1
}

# Full market bring-up on explicit created hub. Active flags come from asset_config_json.
# The pool market is created first, then oracle config is resolved and the
# primary spoke is activated.
# create_market <name> <hub-id> <sac-id> <decimals> <oracle-json> <active-config-json>
create_market() {
    local name="$1" hub_id="$2" sac="$3" decimals="$4" oracle_json="$5" active_cfg="$6"
    local done_var="MKT_${name}_DONE"
    if [ -n "${!done_var:-}" ]; then return 0; fi

    local INV_TRANSIENT_CONTRACT_RE='Error\(Contract, #'
    local params resolved_oracle ltv thr bonus
    params=$(market_params_json "$sac" "$decimals")

    inv "approve_token_$name" "$ADMIN" "$CONTROLLER" -- approve_token --token "$sac" >/dev/null || return 1
    if market_listing_exists "$hub_id" "$sac"; then
        record "create_market_$name" ok create_liquidity_pool "" "" "" "" "" "listing already exists (resume); skipping create"
    else
        inv "create_market_$name" "$ADMIN" "$CONTROLLER" -- create_liquidity_pool \
            --hub_id "$hub_id" --asset "$sac" --params "$params" >/dev/null || return 1
    fi

    resolved_oracle=$(view "resolve_oracle_$name" "$GOVERNANCE" -- resolve_market_oracle_config \
        --asset "$sac" --cfg "$oracle_json" | jq -c '.') || return 1
    inv "set_oracle_$name" "$ADMIN" "$CONTROLLER" -- set_market_oracle_config \
        --hub_asset "$(hub_key "$hub_id" "$sac")" --config "$resolved_oracle" >/dev/null || return 1

    ltv=$(jq -r '.loan_to_value' <<<"$active_cfg")
    thr=$(jq -r '.liquidation_threshold' <<<"$active_cfg")
    bonus=$(jq -r '.liquidation_bonus' <<<"$active_cfg")
    inv "activate_$name" "$ADMIN" "$CONTROLLER" -- add_asset_to_spoke \
        --input "$(spoke_args "$hub_id" "$sac" "$PRIMARY_SPOKE_ID" true true "$ltv" "$thr" "$bonus")" >/dev/null || return 1
    market_wait_listed "$hub_id" "$sac" \
        || die "confirm_market_$name" "market $name primary spoke listing not active after create -> oracle -> activate (read replica lag exhausted)"
    save_state "$done_var" 1
}

# Explicit hub asset coordinate (HubAssetKey) scalar asset args.
# hub_key <hub-id> <sac-id>
hub_key() {
    jq -nc --argjson h "$1" --arg a "$2" '{hub_id:$h, asset:$a}'
}

# Vec<HubAssetKey> for update_indexes / claim_revenue.
# hub_vec <hub-id> <sac1> [<sac2> ...]
hub_vec() {
    local hub_id="$1"
    shift
    local out="[" first=1
    while [ $# -gt 0 ]; do
        [ $first -eq 0 ] && out+=","
        out+="{\"hub_id\":$hub_id,\"asset\":\"$1\"}"
        first=0
        shift
    done
    echo "$out]"
}

# Payments vector JSON Vec<(HubAssetKey, i128)>; amounts are quoted strings.
# pay_vec <hub-id> <sac1> <amt1> [<sac2> <amt2> ...]
pay_vec() {
    local hub_id="$1"
    shift
    local out="[" first=1
    while [ $# -gt 0 ]; do
        [ $first -eq 0 ] && out+=","
        out+="[{\"hub_id\":$hub_id,\"asset\":\"$1\"},\"$2\"]"
        first=0
        shift 2
    done
    echo "$out]"
}
