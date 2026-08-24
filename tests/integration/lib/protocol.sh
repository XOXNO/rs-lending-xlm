deploy_protocol() {
    if [ -z "${XLM_SAC:-}" ]; then
        save_state XLM_SAC "$(stellar contract id asset --asset native "${NET_ARGS[@]}")"
    fi
    if [ -z "${POOL_HASH:-}" ]; then
        local out_f="$LOG_DIR/upload_pool.out" err_f="$LOG_DIR/upload_pool.err"
        run_deploy "$out_f" "$err_f" -- stellar contract upload --wasm "$WASM_DIR/pool.wasm" \
            --source "$ADMIN" "${NET_ARGS[@]}"
        local hash txh
        hash=$(sanitize_output "$out_f")
        txh=$(extract_signing_hash "$err_f")
        is_wasm_hash "$hash" || die upload_pool_wasm "pool wasm upload produced no hash after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
        save_state POOL_HASH "$hash"
        record upload_pool_wasm ok upload "$txh" "" "" "" "" "$hash"
    fi
    if [ -z "${NFT_HASH:-}" ]; then
        local nft_wasm="$WASM_DIR/position_nft.wasm"
        [ -f "$nft_wasm" ] || die upload_position_nft_wasm "position_nft.wasm missing under $WASM_DIR (run make integration-wasm)"
        local out_f="$LOG_DIR/upload_position_nft.out" err_f="$LOG_DIR/upload_position_nft.err"
        run_deploy "$out_f" "$err_f" -- stellar contract upload --wasm "$nft_wasm" \
            --source "$ADMIN" "${NET_ARGS[@]}"
        local hash txh
        hash=$(sanitize_output "$out_f")
        txh=$(extract_signing_hash "$err_f")
        is_wasm_hash "$hash" || die upload_position_nft_wasm "position nft wasm upload produced no hash after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
        save_state NFT_HASH "$hash"
        record upload_position_nft_wasm ok upload "$txh" "" "" "" "" "$hash"
    fi
    # Governance's `deploy_price_aggregator` takes a wasm hash, not a wasm, so
    # the price-aggregator code has to be on-ledger by hash as well as deployed
    # directly. Non-fatal: only the governance-owned aggregator coverage needs
    # it, and the harness's own $PRICE_AGGREGATOR is deployed from the file.
    if [ -z "${PA_HASH:-}" ] && [ -f "$WASM_DIR/price_aggregator.wasm" ]; then
        local pa_out="$LOG_DIR/upload_price_agg.out" pa_err="$LOG_DIR/upload_price_agg.err"
        run_deploy "$pa_out" "$pa_err" -- stellar contract upload \
            --wasm "$WASM_DIR/price_aggregator.wasm" \
            --source "$ADMIN" "${NET_ARGS[@]}"
        local pa_hash pa_txh
        pa_hash=$(sanitize_output "$pa_out")
        pa_txh=$(extract_signing_hash "$pa_err")
        if is_wasm_hash "$pa_hash"; then
            save_state PA_HASH "$pa_hash"
            record upload_price_agg_wasm ok upload "$pa_txh" "" "" "" "" "$pa_hash"
        else
            log "price-aggregator wasm upload produced no hash; governance aggregator coverage will skip"
        fi
    fi
    if [ -z "${CONTROLLER:-}" ]; then
        local out_f="$LOG_DIR/deploy_controller.out" err_f="$LOG_DIR/deploy_controller.err"
        run_deploy "$out_f" "$err_f" -- stellar contract deploy --wasm "$WASM_DIR/controller.wasm" \
            --source "$ADMIN" "${NET_ARGS[@]}" -- --admin "$ADMIN_ADDR"
        local ctrl txh
        ctrl=$(sanitize_output "$out_f")
        txh=$(extract_signing_hash "$err_f")
        is_contract_id "$ctrl" || die deploy_controller "controller deploy produced no id after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
        save_state CONTROLLER "$ctrl"
        record deploy_controller ok deploy "$txh" "" "" "" "" "$ctrl"
        log "controller = $ctrl"
    fi
    if [ -z "${POOL:-}" ]; then
        local pool
        pool=$(inv deploy_pool "$ADMIN" "$CONTROLLER" -- deploy_pool --wasm_hash "$POOL_HASH" | tr -d '"\n')
        is_contract_id "$pool" || die deploy_pool "central pool deploy produced no id after $INV_MAX_ATTEMPTS attempts"
        save_state POOL "$pool"
        log "central pool = $pool"
    fi
    # Account creation mints a position NFT and fail-closes with #53 if this
    # was skipped. Integration deploys are EOA-owned, so this is immediate
    # rather than the production timelock path in configs/script.sh.
    if [ -z "${POSITION_NFT:-}" ]; then
        local nft
        nft=$(inv deploy_position_nft "$ADMIN" "$CONTROLLER" -- deploy_position_nft \
            --wasm_hash "$NFT_HASH" \
            --uri "https://api.xoxno.com/user/lending/image/" \
            --name "XOXNO Lending Position" \
            --symbol "XLEND" | tr -d '"\n')
        is_contract_id "$nft" || die deploy_position_nft "position nft deploy produced no id after $INV_MAX_ATTEMPTS attempts"
        save_state POSITION_NFT "$nft"
        log "position nft = $nft"
    fi

    if [ -z "${PRICE_AGGREGATOR:-}" ]; then
        local out_f="$LOG_DIR/deploy_price_agg.out" err_f="$LOG_DIR/deploy_price_agg.err"
        local pa_wasm=""
        for cand in "$WASM_DIR/price_aggregator.wasm" "$WASM_DIR/price-aggregator.wasm"; do
            [ -f "$cand" ] && pa_wasm="$cand" && break
        done
        [ -n "$pa_wasm" ] || die deploy_price_aggregator "price_aggregator.wasm missing under $WASM_DIR (run make integration-wasm / deploy-artifacts)"
        run_deploy "$out_f" "$err_f" -- stellar contract deploy --wasm "$pa_wasm" \
            --source "$ADMIN" "${NET_ARGS[@]}" -- --owner "$ADMIN_ADDR"
        local pa txh
        pa=$(sanitize_output "$out_f")
        txh=$(extract_signing_hash "$err_f")
        is_contract_id "$pa" || die deploy_price_aggregator "price-aggregator deploy produced no id after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
        save_state PRICE_AGGREGATOR "$pa"
        record deploy_price_aggregator ok deploy "$txh" "" "" "" "" "$pa"
        log "price-aggregator = $pa"
    fi
    # A second, throwaway swap-aggregator owned by this run's ADMIN. The
    # `$AGGREGATOR` from configs/networks.json is a shared testnet deployment
    # whose owner we are not, so its owner-only surface (fees, whitelist,
    # referrals, sweeps) is untestable there — and renounce_ownership would be
    # destructive on a contract other runs depend on. Swaps keep using the
    # shared one; only the admin surface is exercised here.
    if [ -z "${OWNED_AGGREGATOR:-}" ] && [ -f "$WASM_DIR/swap_aggregator.wasm" ]; then
        local sa_out="$LOG_DIR/deploy_owned_agg.out" sa_err="$LOG_DIR/deploy_owned_agg.err"
        run_deploy "$sa_out" "$sa_err" -- stellar contract deploy \
            --wasm "$WASM_DIR/swap_aggregator.wasm" \
            --source "$ADMIN" "${NET_ARGS[@]}" -- --admin "$ADMIN_ADDR"
        local sa sa_tx
        sa=$(sanitize_output "$sa_out")
        sa_tx=$(extract_signing_hash "$sa_err")
        if is_contract_id "$sa"; then
            save_state OWNED_AGGREGATOR "$sa"
            record deploy_owned_aggregator ok deploy "$sa_tx" "" "" "" "" "$sa"
            log "owned swap-aggregator = $sa"
        else
            log "owned swap-aggregator deploy failed; admin-surface coverage will skip: $(tail_err_note "$sa_err")"
        fi
    fi

    if [ -z "${WIRED:-}" ]; then
        inv set_swap_aggregator "$ADMIN" "$CONTROLLER" -- set_swap_aggregator --addr "$AGGREGATOR" >/dev/null

        inv set_accumulator "$ADMIN" "$CONTROLLER" -- set_accumulator --addr "$ADMIN_ADDR" >/dev/null
        inv set_price_aggregator "$ADMIN" "$CONTROLLER" -- set_price_aggregator --addr "$PRICE_AGGREGATOR" >/dev/null
        # Read back: every price the protocol acts on comes through whichever
        # aggregator this points at, so a setter that silently kept the old one
        # would route the whole run's valuations to the wrong contract.
        # set_swap_aggregator and set_accumulator have no getter on the
        # controller, so they can only be asserted through their effects — the
        # strategies phase exercises the swap path.
        assert_view_eq_at "$CONTROLLER" wired_price_aggregator "$PRICE_AGGREGATOR" price_aggregator
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
            --source "$ADMIN" "${NET_ARGS[@]}"
        local recv txh
        recv=$(sanitize_output "$out_f")
        txh=$(extract_signing_hash "$err_f")
        is_contract_id "$recv" || die deploy_flash_receiver "flash receiver deploy produced no id after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
        save_state FLASH_RECEIVER "$recv"
        record deploy_flash_receiver ok deploy "$txh" "" "" "" "" "$recv"
    fi
    if [ -z "${FLASH_POSITION_RECEIVER:-}" ] && [ -f "$WASM_DIR/flash_position_receiver.wasm" ]; then
        local out_f="$LOG_DIR/deploy_flashposrecv.out" err_f="$LOG_DIR/deploy_flashposrecv.err"
        run_deploy "$out_f" "$err_f" -- stellar contract deploy --wasm "$WASM_DIR/flash_position_receiver.wasm" \
            --source "$ADMIN" "${NET_ARGS[@]}"
        local recv txh
        recv=$(sanitize_output "$out_f")
        txh=$(extract_signing_hash "$err_f")
        if is_contract_id "$recv"; then
            save_state FLASH_POSITION_RECEIVER "$recv"
            record deploy_flash_position_receiver ok deploy "$txh" "" "" "" "" "$recv"
            log "flash_position receiver = $recv"
        else
            log "flash_position receiver deploy failed; live flash_position coverage will skip: $(tail_err_note "$err_f")"
        fi
    fi
    if [ -z "${UNPAUSED:-}" ]; then
        inv unpause "$ADMIN" "$CONTROLLER" -- unpause >/dev/null
        save_state UNPAUSED 1
    fi

    if [ -z "${GOVERNANCE:-}" ]; then
        local out_f="$LOG_DIR/deploy_governance.out" err_f="$LOG_DIR/deploy_governance.err"
        run_deploy "$out_f" "$err_f" -- stellar contract deploy --wasm "$WASM_DIR/governance.wasm" \
            --source "$ADMIN" "${NET_ARGS[@]}" \
            -- --admin "$ADMIN_ADDR" --min_delay "$INTEG_MIN_DELAY"
        local gov txh
        gov=$(sanitize_output "$out_f")
        txh=$(extract_signing_hash "$err_f")
        is_contract_id "$gov" || die deploy_governance "governance deploy produced no id after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
        save_state GOVERNANCE "$gov"
        record deploy_governance ok deploy "$txh" "" "" "" "" "$gov"
        log "governance = $gov"
    fi

    if [ -z "${CTRL_HASH:-}" ]; then
        local out_f="$LOG_DIR/upload_controller.out" err_f="$LOG_DIR/upload_controller.err"
        run_deploy "$out_f" "$err_f" -- stellar contract upload --wasm "$WASM_DIR/controller.wasm" \
            --source "$ADMIN" "${NET_ARGS[@]}"
        local chash txh
        chash=$(sanitize_output "$out_f")
        txh=$(extract_signing_hash "$err_f")
        is_wasm_hash "$chash" || die upload_controller_wasm "controller wasm upload produced no hash after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
        save_state CTRL_HASH "$chash"
        record upload_controller_wasm ok upload "$txh" "" "" "" "" "$chash"
    fi

    if [ -z "${GOV_CONTROLLER:-}" ]; then
        local gc
        gc=$(inv deploy_controller "$ADMIN" "$GOVERNANCE" -- deploy_controller \
            --wasm_hash "$CTRL_HASH" | tr -d '"\n')
        is_contract_id "$gc" || die deploy_gov_controller "governance-owned controller deploy produced no id after $INV_MAX_ATTEMPTS attempts"
        save_state GOV_CONTROLLER "$gc"
        log "governance-owned controller = $gc"
    fi
}

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
        is_flashloanable: true,
        flashloan_fee: 5,
        asset_id: $sac,
        asset_decimals: $dec
    }'
}

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
        supply_cap: "1000000000000000000",
        borrow_cap: "1000000000000000000"
    }' | jq -c "$overrides"
}

# Builds the `SpokeAssetArgs` map. Every key the Rust struct declares must be
# present: the CLI rejects the call outright ("Missing key <k> in map") rather
# than defaulting, so a field added to common/src/types/controller.rs has to be
# mirrored here or every add_asset_to_spoke in the suite fails.
spoke_args() {
    jq -nc --argjson hub "$1" --arg asset "$2" --argjson spoke "$3" --argjson cc "$4" --argjson cb "$5" \
        --argjson ltv "$6" --argjson thr "$7" --argjson bonus "$8" \
        --arg sc "${9:-1000000000000000000}" --arg bc "${10:-1000000000000000000}" \
        --argjson ns "${11:-false}" '{
        hub_id: $hub,
        asset: $asset,
        spoke_id: $spoke,
        can_collateral: $cc,
        can_borrow: $cb,
        paused: false,
        frozen: false,
        no_seize: $ns,
        ltv: $ltv,
        threshold: $thr,
        bonus: $bonus,
        liquidation_fees: 100,
        supply_cap: $sc,
        borrow_cap: $bc
    }'
}

# `SeizeMode` on the wire, following the same convention the oracle configs use
# for `read_mode`: a unit variant is a bare JSON string, a data variant is
# {Variant: value}. Emitted as JSON rather than a bare word so the CLI parses it
# as a value instead of falling back to string coercion.
seize_transfer() {
    jq -nc '"Transfer"'
}

# `Credit(0)` opens a fresh account owned by the liquidator; any other id must
# already exist and satisfy the binding rules in ADR-0019.
seize_credit() {
    jq -nc --argjson id "${1:-0}" '{Credit: $id}'
}

price_key_token() {
    jq -nc --arg a "$1" '{Token:$a}'
}

oracle_tolerance_band() {
    local bps="$1"
    jq -nc --argjson t "$bps" '
        def half_up(n; d): ((n + (d/2|floor)) / d | floor);
        {upper_ratio_bps: (10000 + $t),
         lower_ratio_bps: half_up(10000 * 10000; 10000 + $t)}'
}

oracle_cfg_mock_single() {
    local sac="$1"
    jq -nc --arg mock "$MOCK" --arg sac "$sac" --argjson tol "$(oracle_tolerance_band 500)" '{
        asset_decimals: 7,
        max_price_stale_seconds: 3600,
        sources: [{
            Feed: {
                provider: {Reflector: {
                    contract: $mock,
                    asset: {Stellar: $sac},
                    read_mode: {Twap: 3}
                }},
                decimals: 14,
                max_stale_seconds: 3600
            }
        }],
        tolerance: $tol,
        independence: "RequireDisjoint",
        min_sanity_price_wad: "900000000000000000",
        max_sanity_price_wad: "1100000000000000000"
    }'
}

oracle_cfg_mock_dual() {
    local sac="$1" feed="$2"
    jq -nc --arg mock "$MOCK" --arg mockrs "$MOCKRS" --arg sac "$sac" --arg feed "$feed" \
        --argjson tol "$(oracle_tolerance_band 500)" '{
        asset_decimals: 7,
        max_price_stale_seconds: 3600,
        sources: [
            {Feed: {
                provider: {Reflector: {
                    contract: $mock,
                    asset: {Stellar: $sac},
                    read_mode: {Twap: 3}
                }},
                decimals: 14,
                max_stale_seconds: 3600
            }},
            {Feed: {
                provider: {RedStone: {
                    contract: $mockrs,
                    feed_id: $feed,
                    nature: "Fundamental"
                }},
                decimals: 8,
                max_stale_seconds: 3600
            }}
        ],
        tolerance: $tol,
        independence: "RequireDisjoint",
        min_sanity_price_wad: "1000000000000000",
        max_sanity_price_wad: "1000000000000000000000"
    }'
}

oracle_cfg_reflector() {
    local sym="$1" min_wad="$2" max_wad="$3"
    jq -nc --arg orc "$REFLECTOR_CEX" --arg sym "$sym" --arg min "$min_wad" --arg max "$max_wad" \
        --argjson tol "$(oracle_tolerance_band 500)" '{
        asset_decimals: 7,
        max_price_stale_seconds: 3600,
        sources: [{
            Feed: {
                provider: {Reflector: {
                    contract: $orc,
                    asset: {Symbol: $sym},
                    read_mode: {Twap: 3}
                }},
                decimals: 14,
                max_stale_seconds: 3600
            }
        }],
        tolerance: $tol,
        independence: "RequireDisjoint",
        min_sanity_price_wad: $min,
        max_sanity_price_wad: $max
    }'
}

# Self-calibrating single-source sanity band for a live reflector-CEX symbol.
# Reads the current price and brackets it by ±PCT (default 9 → ~900 bps half-width
# ratio, safely under MAX_SINGLE_SOURCE_SANITY_BAND_BPS=1000 and above
# MIN_SANITY_BAND_BPS=50). Echoes "min_wad max_wad" for oracle_cfg_reflector to
# splat as its $2 $3. A single-source band is capped at ~10% half-width, so a
# hardcoded band silently goes stale as the live price drifts and eventually the
# reflector price falls outside it — the aggregator then fails closed with
# OracleError::SanityBoundViolated (#223) and every borrow/multiply reverts.
# Returns non-zero if the live price is unavailable so the caller aborts rather
# than listing a market whose oracle is guaranteed to reject. Reflector CEX prices
# are 14-decimal; WAD is 18-decimal, hence the ×10000. Bracketing is done in
# 14-decimal space first so 64-bit intermediates cannot overflow before scaling.
reflector_band() {
    local sym="$1" pct="${2:-9}" raw px14 min14 max14
    raw=$(stellar contract invoke --id "$REFLECTOR_CEX" --source "$ADMIN" "${NET_ARGS[@]}" \
        --send=no -- lastprice --asset "{\"Other\":\"$sym\"}" 2>/dev/null) || return 1
    px14=$(printf '%s' "$raw" | jq -r '.price // empty' 2>/dev/null)
    case "$px14" in
        ''|*[!0-9]*) log "reflector_band[$sym]: no live price (got '${raw:-<none>}')"; return 1 ;;
    esac
    min14=$(( px14 * (100 - pct) / 100 ))
    max14=$(( px14 * (100 + pct) / 100 ))
    printf '%s %s\n' "$(( min14 * 10000 ))" "$(( max14 * 10000 ))"
}

market_listing_exists() {
    local hub_id="$1" sac="$2"
    stellar contract invoke --id "$CONTROLLER" --source "$ADMIN" "${NET_ARGS[@]}" \
        --send=no -- get_spoke_asset --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$hub_id" "$sac")" >/dev/null 2>&1
}

market_wait_listed() {
    local hub_id="$1" sac="$2" probe got
    for probe in $(seq 1 8); do
        got=$(stellar contract invoke --id "$CONTROLLER" --source "$ADMIN" "${NET_ARGS[@]}" \
            --send=no -- get_spoke_asset --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$hub_id" "$sac")" 2>/dev/null \
            | jq -r '.is_borrowable // empty' 2>/dev/null)
        [ "$got" = "true" ] && return 0
        sleep $(( probe * 2 ))
    done
    return 1
}

create_market() {
    local name="$1" hub_id="$2" sac="$3" decimals="$4" oracle_json="$5" active_cfg="$6"
    local done_var="MKT_${name}_DONE"
    if [ -n "${!done_var:-}" ]; then return 0; fi

    local INV_TRANSIENT_CONTRACT_RE='Error\(Contract, #'
    local params resolved_oracle ltv thr bonus
    params=$(market_params_json "$sac" "$decimals")

    if market_listing_exists "$hub_id" "$sac"; then
        record "create_market_$name" ok create_liquidity_pool "" "" "" "" "" "listing already exists (resume); skipping create"
    else
        inv "create_market_$name" "$ADMIN" "$CONTROLLER" -- create_liquidity_pool \
            --hub_id "$hub_id" --asset "$sac" --params "$params" >/dev/null || return 1
    fi

    local key_json oracle_file resolved_file
    key_json=$(price_key_token "$sac")
    oracle_file=$(mktemp)
    resolved_file=$(mktemp)
    printf '%s' "$oracle_json" > "$oracle_file"

    resolved_oracle=$(view "resolve_oracle_$name" "$GOVERNANCE" -- resolve_asset_oracle \
        --key "$key_json" --oracle-file-path "$oracle_file" | jq -c '.') || {
        rm -f "$oracle_file" "$resolved_file"
        return 1
    }
    printf '%s' "$resolved_oracle" > "$resolved_file"

    inv "set_oracle_$name" "$ADMIN" "$PRICE_AGGREGATOR" -- set_oracle \
        --key "$key_json" --oracle-file-path "$resolved_file" >/dev/null || {
        rm -f "$oracle_file" "$resolved_file"
        return 1
    }
    rm -f "$oracle_file" "$resolved_file"

    ltv=$(jq -r '.loan_to_value' <<<"$active_cfg")
    thr=$(jq -r '.liquidation_threshold' <<<"$active_cfg")
    bonus=$(jq -r '.liquidation_bonus' <<<"$active_cfg")
    inv "activate_$name" "$ADMIN" "$CONTROLLER" -- add_asset_to_spoke \
        --input "$(spoke_args "$hub_id" "$sac" "$PRIMARY_SPOKE_ID" true true "$ltv" "$thr" "$bonus")" >/dev/null || return 1
    market_wait_listed "$hub_id" "$sac" \
        || die "confirm_market_$name" "market $name primary spoke listing not active after create -> oracle -> activate (read replica lag exhausted)"
    # Registry of every market this run listed, consumed by flow_teardown to
    # sweep and zero-check the whole world. Idempotent across resumes.
    case " ${MARKETS:-} " in
        *" ${hub_id}:${sac} "*) ;;
        *) save_state MARKETS "${MARKETS:+$MARKETS }${hub_id}:${sac}" ;;
    esac
    save_state "$done_var" 1
}

hub_key() {
    jq -nc --argjson h "$1" --arg a "$2" '{hub_id:$h, asset:$a}'
}

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
