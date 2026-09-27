prod_upgrade_invocation() {
    python3 - "$@" <<'PYUPGRADE'
import json,sys
verb,wasm,governance,controller,aggregator,payload=sys.argv[1:]
operation,target,function={
 'upgradeControllerHash':('UpgradeController',controller,'upgrade'),
 'upgradePoolHash':('UpgradePool',controller,'upgrade_pool'),
 'upgradePositionNftHash':('UpgradePositionNft',controller,'upgrade_position_nft'),
 'upgradePriceAggregatorHash':('UpgradePriceAggregator',aggregator,'upgrade'),
 'upgradeGovernanceHash':('UpgradeGov',governance,'upgrade'),
}[verb]
v=json.loads(payload); args=v['args']
assert v['contract_address']==governance
expected={'vec':[{'symbol':operation},{'bytes':wasm}]}
if v['function_name'] in ('propose','execute_self'):
    assert len(args)==3 and args[1]==expected
    assert v['function_name']=='propose' or verb=='upgradeGovernanceHash'
elif v['function_name']=='execute':
    assert verb!='upgradeGovernanceHash' and len(args)==6
    assert args[1]=={'address':target} and args[2]=={'symbol':function}
    assert args[3]=={'vec':[{'bytes':wasm}]} and args[4]=={'bytes':'00'*32}
else:
    raise AssertionError('unexpected upgrade invocation')
salt=args[-1]['bytes']; assert len(salt)==64
print(salt)
PYUPGRADE
}

prod_fetch_receipt() {
    local hash="$1"
    rm -f "$LOG_DIR/$hash.res" "$LOG_DIR/$hash.invocation.json"
    [ "$(tx_status "$hash")" = SUCCESS ] && fetch_resources "$hash" || return 1
    jq -r '.result.envelopeXdr' "$LOG_DIR/$hash.receipt.json" | stellar xdr decode --type TransactionEnvelope --output json \
        | jq -ce '.tx.tx.operations[0].body.invoke_host_function.host_function.invoke_contract' > "$LOG_DIR/$hash.invocation.json" || return 1
    printf '%s\n' "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" > "$LOG_DIR/$hash.res"
}

prod_fetch_receipts() {
    local hash pids=() failed=0 pid
    while read -r hash; do
        prod_fetch_receipt "$hash" 2>"$LOG_DIR/$hash.fetch.err" & pids+=("$!")
        if [ "${#pids[@]}" -ge "${PROD_FETCH_JOBS:-16}" ]; then
            for pid in "${pids[@]}"; do wait "$pid" || failed=1; done
            pids=()
        fi
    done
    for pid in ${pids[@]+"${pids[@]}"}; do wait "$pid" || failed=1; done
    return "$failed"
}

PROD_UPGRADES='upgradeControllerHash upgradePoolHash upgradePositionNftHash upgradePriceAggregatorHash upgradeGovernanceHash'

prod_upgrade_hash() {
    case "$1" in
        upgradeControllerHash) echo "$CTRL_HASH";;
        upgradePoolHash) echo "$POOL_HASH";;
        upgradePositionNftHash) echo "$NFT_HASH";;
        upgradePriceAggregatorHash) echo "$PA_HASH";;
        upgradeGovernanceHash) jq -er '.artifacts["governance.wasm"]' "$RUN_DIR/candidate.json";;
        *) return 1;;
    esac
}

prod_propose() {
    local tag="$1"; shift
    PROD_OP_TAG="${tag}_propose" PROD_SPLIT_TAG="$tag" PROD_PROPOSE_ONLY=1 prod_ops "$@" >/dev/null
}

prod_execute_split() {
    local tag="$1" verb="$2" wasm="${3:-}" op_var="PROD_SPLIT_${1}_OP" salt_var="PROD_SPLIT_${1}_SALT"
    local op="${!op_var:-}" salt="${!salt_var:-}"
    [ -n "$op" ] && [ -n "$salt" ] || { _assert_fail "operator_$tag" 'no proposed operation to execute'; return 1; }
    NETWORK=testnet CONFIG_ROOT="$RUN_DIR/config" OPS_ROOT="$RUN_DIR/ops" SIGNER="$ADMIN" \
        AWAIT_MAX_WAIT_SECONDS=180 AWAIT_POLL_SECONDS=1 UNSET_MAX_POLLS=30 bash "$REPO_ROOT/configs/script.sh" awaitOp "$op" \
        >"$LOG_DIR/operator_${tag}_await.out" 2>"$LOG_DIR/operator_${tag}_await.err" \
        || { _assert_fail "operator_$tag" "proposed operation $op never became ready"; return 1; }
    PROD_OP_TAG="$tag" PROD_OP_VERB="$verb" PROD_OP_WASM="$wasm" PROD_SPLIT_OP="$op" PROD_SPLIT_SALT="$salt" prod_ops executeOp "$op"
}

prod_ops() {
    local verb="$1" tag="${PROD_OP_TAG:-$1}"; shift
    [ -z "${E2E_JOB:-}" ] || { job_refuse prod_ops "operator_$tag"; return 1; }
    [ -z "${GROUP_DIR:-}" ] || group_guard "prod_ops $verb" || return 1
    local logical="${PROD_OP_VERB:-$verb}" wasm="${PROD_OP_WASM:-${1:-}}" auto=1 executed_call='' op record_path
    local proposal_salts='' execution_salts='' wave_log=''
    [ "${PROD_SETUP_JOBS:-1}" -le 1 ] || wave_log="$LOG_DIR/operator_${tag}_wave"
    [ -z "${PROD_PROPOSE_ONLY:-}" ] || auto=0
    if NETWORK=testnet CONFIG_ROOT="$RUN_DIR/config" OPS_ROOT="$RUN_DIR/ops" SIGNER="$ADMIN" AUTO_EXECUTE="$auto" STELLAR_SEND=yes \
        AWAIT_MAX_WAIT_SECONDS=180 AWAIT_POLL_SECONDS=1 UNSET_MAX_POLLS=30 SETUP_JOBS="${PROD_SETUP_JOBS:-1}" SETUP_SOURCES="${PROD_SETUP_SOURCES:-}" \
        WAVE_LOG_DIR="$wave_log" bash "$REPO_ROOT/configs/script.sh" "$verb" "$@" >"$LOG_DIR/operator_$tag.out" 2>"$LOG_DIR/operator_$tag.err"; then
        local hash n=0 invocation method target proposed=0 executed=0 proposal_salt='' execution_salt='' salt
        local hashes="$LOG_DIR/operator_$tag.hashes"
        grep -oE 'Signing transaction: [0-9a-f]{64}' "$LOG_DIR/operator_$tag.err" | awk '{print $3}' | sort -u > "$hashes"
        prod_fetch_receipts < "$hashes" || true
        while read -r hash; do
            n=$((n+1))
            { [ -s "$LOG_DIR/$hash.res" ] && [ -s "$LOG_DIR/$hash.invocation.json" ]; } || { _assert_fail "operator_$tag" "unconfirmed receipt/resources $hash"; return 1; }
            { read -r RES_INSTR; read -r RES_READ; read -r RES_WRITE; read -r RES_FEE; } < "$LOG_DIR/$hash.res"
            invocation=$(cat "$LOG_DIR/$hash.invocation.json")
            method=$(jq -r '.function_name' <<<"$invocation"); target=$(jq -r '.contract_address' <<<"$invocation")
            if [ "$target" = "$GOVERNANCE" ]; then
                case "$method" in
                    propose) proposed=$((proposed+1)); proposal_salt=$(jq -r '.args[-1].bytes // empty' <<<"$invocation"); proposal_salts="$proposal_salts $proposal_salt";;
                    execute|execute_self) executed=$((executed+1)); execution_salt=$(jq -r '.args[-1].bytes // empty' <<<"$invocation"); executed_call="$invocation"; execution_salts="$execution_salts $execution_salt";;
                esac
            fi
            if [[ "$logical" = upgrade*Hash ]]; then
                salt=$(prod_upgrade_invocation "$logical" "$wasm" "$GOVERNANCE" "$CONTROLLER" "$PRICE_AGGREGATOR" "$invocation") \
                    || { _assert_fail "operator_${tag}_binding" 'upgrade receipt differs from requested target, operation or WASM hash'; return 1; }
                if [ "$method" = propose ]; then proposal_salt="$salt"; else execution_salt="$salt"; fi
            fi
            record "operator_${tag}_tx_$n" ok "$method" "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" "operator confirmed receipt" transaction "$target"
        done < "$hashes"
        case "$tag" in
            validateConfigs) ;;
            setupAll_replay) [ "$n" -eq 0 ] || { _assert_fail "operator_$tag" 'serial replay submitted transactions; the setup left work'; return 1; };;
            *) [ "$n" -gt 0 ] || { _assert_fail "operator_$tag" 'mutation returned without a confirmed submission'; return 1; };;
        esac
        if [ "$tag" = setupAll ]; then
            [ "$(printf '%s\n' $proposal_salts | sort)" = "$(printf '%s\n' $execution_salts | sort)" ] \
                || { _assert_fail "operator_$tag" 'setup proposals and executions differ'; return 1; }
        fi
        if [ -n "${PROD_PROPOSE_ONLY:-}" ]; then
            op=$(grep -oE 'Scheduled op [0-9a-f]+ \(AUTO_EXECUTE=0' "$LOG_DIR/operator_$tag.err" | awk '{print $3}' | tail -n1)
            record_path="$RUN_DIR/ops/testnet/$op.json"
            [ "$proposed" -eq 1 ] && [ "$executed" -eq 0 ] && [ -n "$op" ] && [ -f "$record_path" ] \
                && [ "$(jq -r '.salt' "$record_path")" = "$proposal_salt" ] \
                || { _assert_fail "operator_$tag" 'propose-only call needs one confirmed proposal matching its op record'; return 1; }
            save_state "PROD_SPLIT_${PROD_SPLIT_TAG}_OP" "$op"
            save_state "PROD_SPLIT_${PROD_SPLIT_TAG}_SALT" "$proposal_salt"
            record "operator_$tag" ok propose "" "" "" "" "" "scheduled op $op; execute pending"
            return 0
        fi
        if [ -n "${PROD_SPLIT_OP:-}" ]; then
            record_path="$RUN_DIR/ops/testnet/$PROD_SPLIT_OP.json"
            [ "$proposed" -eq 0 ] && [ "$executed" -eq 1 ] && [ "$execution_salt" = "$PROD_SPLIT_SALT" ] && [ -f "$record_path" ] \
                && jq -e --argjson call "$executed_call" '.salt == $call.args[-1].bytes and ((.kind // "controller") == "governance_self"
                    or ($call.function_name == "execute" and $call.args[1].address == .target and $call.args[2].symbol == .function))' "$record_path" >/dev/null \
                || { _assert_fail "operator_$tag" 'execute differs from the proposed operation record'; return 1; }
            proposed=1; proposal_salt="$PROD_SPLIT_SALT"
        fi
        if [[ "$logical" = upgrade*Hash ]]; then
            [ "$proposed" -eq 1 ] && [ "$executed" -eq 1 ] && [ "$proposal_salt" = "$execution_salt" ] || { _assert_fail "operator_${tag}_receipts" 'upgrade needs confirmed governance propose and execute'; return 1; }
            record "operator_${tag}_receipts" ok assert "" "" "" "" "" 'confirmed governance schedule and execution'
        fi
        record "operator_$tag" ok "$logical" "" "" "" "" "" "disposable config and operation records; $n confirmed submissions"
        cat "$LOG_DIR/operator_$tag.out"
    else
        record "operator_$tag" FAIL "$verb" "" "" "" "" "" "$(tail_err_note "$LOG_DIR/operator_$tag.err")"
        return 1
    fi
}

flow_production_fixtures() {
    phase production_fixtures
    mkdir -p "$RUN_DIR/config/testnet"
    local plan="$RUN_DIR/fixture-plan.json" mapping="$RUN_DIR/address-map.json" original got id n=0
    python3 "$INTEG_DIR/production_config.py" plan "$REPO_ROOT/configs/mainnet" > "$plan" || return 1
    group_each production_fixtures 40 "$(jq -c 'to_entries | to_entries[] | .value + {n: (.key + 1)}' "$plan")" prod_fixture_job || return 1
    printf '{}\n' > "$mapping"
    while read -r original; do
        n=$((n+1))
        read -r got id <<<"$(group_out "$n" | tail -n1)"
        [ "$got" = "$original" ] && is_contract_id "$id" \
            || { _assert_fail "production_fixture_$n" "fixture job output '$got $id' does not map $original"; return 1; }
        jq --arg o "$original" --arg i "$id" '.[$o]=$i' "$mapping" > "$mapping.tmp" && mv "$mapping.tmp" "$mapping" || return 1
    done < <(jq -r 'keys_unsorted[]' "$plan")
    python3 "$INTEG_DIR/production_config.py" materialize "$REPO_ROOT/configs/mainnet" "$RUN_DIR/config/testnet" "$mapping" || return 1
    local fixtures="$RUN_DIR/config/testnet/fixtures.json" seeds seed row feed_id
    cp "$fixtures" "$RUN_DIR/production-fixtures.json" || return 1
    seeds=$(prod_price_seeds "$fixtures" 2>"$LOG_DIR/prod_seed_dedupe.err") \
        || { _assert_fail prod_seed_dedupe "$(tail_err_note "$LOG_DIR/prod_seed_dedupe.err")"; return 1; }
    group_each prod_bases 40 "$(jq -c '.bases|to_entries[]' "$fixtures")" prod_base_job || return 1
    group_each prod_seeds 40 "$seeds" prod_seed_job || return 1
    while read -r seed; do
        id=$(jq -r '.contract' <<<"$seed"); feed_id=$(jq -r '.feed' <<<"$seed")
        inv prod_xoxno_register "$ADMIN" "$id" -- register_feed --feed_id "$feed_id" >/dev/null || return 1
        inv prod_xoxno_seed "$ADMIN" "$id" -- submit_price --signer "$ADMIN_ADDR" --feed_id "$feed_id" \
            --price "$(python3 -c 'import sys; print(int(sys.argv[1])//10**10)' "$(jq -r '.price' <<<"$seed")")" \
            --package_timestamp "$(( ($(date +%s) - 10) * 1000 ))" >/dev/null || return 1
    done < <(jq -c '.seeds[] | select(.kind == "Xoxno")' "$fixtures")
    while read -r row; do
        inv prod_lp_fixture "$ADMIN" "$(jq -r '.contract' <<<"$row")" -- configure_pool --snapshot "$(jq -c '.snapshot' <<<"$row")" >/dev/null || return 1
    done < <(jq -c '.pools[]' "$fixtures")
}

prod_fixture_job() {
    local row="$1" n original kind wasm wasm_path id base args=()
    n=$(jq -r '.n' <<<"$row"); original=$(jq -r '.key' <<<"$row"); kind=$(jq -r '.value.kind' <<<"$row")
    case "$kind" in
        Reflector) wasm=mock_oracle;; RedStone) wasm=mock_redstone;; Xoxno) wasm=xoxno-oracle-adapter;; *) wasm=production_fixture;;
    esac
    if [ "$wasm" = production_fixture ]; then
        args=(-- --admin "$ADMIN_ADDR" --decimals "$(jq -r '.value.decimals' <<<"$row")" --symbol "$(jq -r '.value.name' <<<"$row")")
    fi
    wasm_path="$FIXTURE_WASM_DIR/$wasm.wasm"
    if [ "$kind" = Xoxno ]; then
        wasm_path="$WASM_DIR/$wasm.wasm"
        args=(-- --admin "$ADMIN_ADDR" --signers "[\"$ADMIN_ADDR\"]" --threshold 1 --resolution 60)
    fi
    base=$(job_log "fixture_$n")
    run_deploy "$base.out" "$base.err" -- stellar contract deploy \
        --source "${E2E_SRC:-$ADMIN}" "${NET_ARGS[@]}" --wasm "$wasm_path" ${args[@]+"${args[@]}"} || return 1
    id=$(sanitize_output "$base.out")
    is_contract_id "$id" || return 1
    record "production_fixture_$n" ok deploy "$(extract_signing_hash "$base.err")" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" "$kind $original -> $id (fixture)" deployment "$id"
    printf '%s %s\n' "$original" "$id"
}

prod_price_seeds() {
    jq -c '[.seeds[] | select(.kind == "Reflector" or .kind == "RedStone")]
        | group_by([.contract, (.asset // .feed)])
        | map(if (map(.price) | unique | length) == 1 then .[0]
              else error("seed \(.[0].contract) \(.[0].asset // .[0].feed | tojson) has conflicting prices \(map(.price) | unique)") end)
        | .[]' "$1"
}

prod_base_job() {
    inv prod_reflector_base "${E2E_SRC:-$ADMIN}" "$(jq -r '.key' <<<"$1")" -- set_base --base "$(jq -c '.value' <<<"$1")" >/dev/null
}

prod_seed_job() {
    local id price
    id=$(jq -r '.contract' <<<"$1"); price=$(jq -r '.price' <<<"$1")
    if [ "$(jq -r '.kind' <<<"$1")" = Reflector ]; then
        inv prod_reflector_seed "${E2E_SRC:-$ADMIN}" "$id" -- set_price --asset "$(jq -c '.asset' <<<"$1")" --price_wad "$price" >/dev/null
    else
        inv prod_redstone_seed "${E2E_SRC:-$ADMIN}" "$id" -- set_price --feed_id "$(jq -r '.feed' <<<"$1")" --price_wad "$price" >/dev/null
    fi
}

# Reflector TWAP includes two older 300-second samples. Keep fixture prices
# fresh after the long governance setup without changing production policy.
prod_refresh_reflectors() {
    local label="$1" seeds
    seeds=$(jq -ce '[.seeds[] | select(.kind == "Reflector")] | if length > 0 then . else error("missing Reflector fixtures") end' \
        "$RUN_DIR/config/testnet/fixtures.json") || return 1
    group_each prod_refresh 40 "$(jq -c '.[]' <<<"$seeds")" prod_refresh_job "$label" || return 1
    record "${label}_complete" ok assert "" "" "" "" "" "$(jq length <<<"$seeds") Reflector fixture timestamps refreshed; prices and policy unchanged"
}

prod_refresh_job() {
    inv "$1" "${E2E_SRC:-$ADMIN}" "$(jq -r '.contract' <<<"$2")" -- set_price \
        --asset "$(jq -c '.asset' <<<"$2")" --price_wad "$(jq -r '.price' <<<"$2")" >/dev/null
}

flow_production_operator() {
    phase production_operator
    save_state CONTROLLER "$GOV_CONTROLLER"
    local pa pool nft
    pa=$(inv prod_deploy_pa "$ADMIN" "$GOVERNANCE" -- deploy_price_aggregator --wasm_hash "$PA_HASH" | tr -d '\"[:space:]') || return 1
    save_state PRICE_AGGREGATOR "$pa"
    jq --arg c "$CONTROLLER" --arg g "$GOVERNANCE" --arg p "$pa" --arg a "$OWNED_AGGREGATOR" --arg owner "$ADMIN_ADDR" --arg rpc "$RPC_URL" --arg pass "$NETWORK_PASSPHRASE" \
        '.testnet.rpc_url=$rpc | .testnet.network_passphrase=$pass | .testnet.controller=$c | .testnet.governance=$g | .testnet.price_aggregator=$p | .testnet.aggregator=$a | .testnet.accumulator=$owner | .testnet.hub_ids={} | .testnet.spoke_ids={}' \
        "$NETWORKS_FILE" > "$RUN_DIR/config/networks.json" || return 1
    prod_propose setPriceAggregator setPriceAggregator || return 1
    prod_propose setAggregator setAggregator || return 1
    pool=$(prod_ops deployPool "$POOL_HASH" | tail -n1 | tr -d '\"[:space:]') || return 1
    is_contract_id "$pool" || return 1
    nft=$(prod_ops deployPositionNft "$NFT_HASH" | tail -n1 | tr -d '\"[:space:]') || return 1
    is_contract_id "$nft" || return 1
    save_state POOL "$pool"; save_state POSITION_NFT "$nft"
    verify_candidate_contract production_pool "$POOL" pool || return 1
    verify_candidate_contract production_nft "$POSITION_NFT" position_nft || return 1
    verify_candidate_contract production_pa "$PRICE_AGGREGATOR" price_aggregator || return 1
    jq --arg p "$pool" --arg n "$nft" '.testnet.pool=$p | .testnet.position_nft=$n' "$RUN_DIR/config/networks.json" > "$RUN_DIR/config/networks.tmp" || return 1
    mv "$RUN_DIR/config/networks.tmp" "$RUN_DIR/config/networks.json"
    prod_execute_split setPriceAggregator setPriceAggregator >/dev/null || return 1
    prod_execute_split setAggregator setAggregator >/dev/null || return 1
    prod_ops setAccumulator >/dev/null || return 1
    prod_ops validateConfigs >/dev/null || return 1
    PROD_SETUP_JOBS=12 PROD_SETUP_SOURCES="$CHANNELS" prod_ops setupAll >/dev/null || return 1
    cp "$RUN_DIR/config/networks.json" "$RUN_DIR/operator-before-replay.json"
    PROD_OP_TAG=setupAll_replay prod_ops setupAll >/dev/null || return 1
    cmp -s "$RUN_DIR/operator-before-replay.json" "$RUN_DIR/config/networks.json" || { _assert_fail operator_replay "setup replay changed deployment mappings"; return 1; }
    prod_ops unpause >/dev/null || return 1
    prod_refresh_reflectors prod_reflector_after_setup || return 1
    local m asset oracle markets="$RUN_DIR/config/testnet/markets.json"
    save_state MARKETS ''
    group_begin prod_markets 8 reads || return 1
    while read -r m; do group_spawn prod_market_job "$m"; done < <(jq -c '.markets[]' "$markets")
    group_end || return 1
    while read -r m; do
        asset=$(jq -r '.asset_address' <<<"$m")
        python3 "$INTEG_DIR/production_config.py" verify-price "$RUN_DIR/config/testnet" "$(jq -r '.name' <<<"$m")" "$LOG_DIR/prod_price_${asset:0:8}.out" \
            || { _assert_fail "prod_price_${asset:0:8}" "price/decimals differ from seeded production fixture"; return 1; }
        oracle=$(cat "$LOG_DIR/prod_oracle_${asset:0:8}.out") && [ -n "$oracle" ] || return 1
        save_state MARKETS "${MARKETS:+$MARKETS }$(jq -r '.hub_id' <<<"$m"):$asset"
    done < <(jq -c '.markets[]' "$markets")
    save_state PRIMARY_HUB_ID 1
    save_state PRIMARY_SPOKE_ID "$(jq -r '.testnet.spoke_ids["1"]' "$RUN_DIR/config/networks.json")"
    prod_verify_policy || return 1
}

prod_market_job() {
    local asset key
    asset=$(jq -r '.asset_address' <<<"$1"); key=$(price_key_token "$asset")
    assert_view_eq_at "$asset" "prod_decimals_${asset:0:8}" "$(jq -r '.oracle.asset_decimals' <<<"$1")" decimals || return 1
    view "prod_price_${asset:0:8}" "$PRICE_AGGREGATOR" -- prices --keys "[$key]" >/dev/null || return 1
    view "prod_oracle_${asset:0:8}" "$PRICE_AGGREGATOR" -- oracle --key "$key" >/dev/null
}

prod_verify_policy() {
    local checks="$RUN_DIR/production-policy-checks.json" row i=0 n
    python3 "$INTEG_DIR/production_config.py" checks "$RUN_DIR/config/testnet" "$RUN_DIR/config/networks.json" > "$checks" || return 1
    group_begin prod_policy 8 reads || return 1
    while read -r row; do
        group_spawn prod_policy_job "$i" "$row"
        i=$((i+1))
    done < <(jq -c '.[]' "$checks")
    group_end || return 1
    for ((n = 0; n < i; n++)); do
        python3 "$INTEG_DIR/production_config.py" verify "$checks" "$n" "$LOG_DIR/prod_policy_$n.out" \
            || { _assert_fail "prod_policy_$n" "$(jq -r --argjson n "$n" '.[$n].method' "$checks") differs from production policy"; return 1; }
    done
    record prod_policy_equal ok assert "" "" "" "" "" "$i oracle/market/spoke policy readbacks matched"
}

prod_policy_job() {
    local target method arg args=()
    target=$(jq -r '.contract' <<<"$2"); method=$(jq -r '.method' <<<"$2")
    while IFS= read -r arg; do args+=("$arg"); done < <(jq -r '.args[]' <<<"$2")
    view "prod_policy_$1" "${!target}" -- "$method" ${args[@]+"${args[@]}"} >/dev/null
}

prod_decimal_roundtrips() {
    local cases="$RUN_DIR/production-decimals.json" row decimals asset hub spoke amount partial acct wallet_pre pool_pre wallet_post pool_post
    python3 "$INTEG_DIR/production_config.py" decimals "$RUN_DIR/config/testnet" "$RUN_DIR/config/networks.json" > "$cases" || return 1
    while read -r row; do
        decimals=$(jq -r '.decimals' <<<"$row"); asset=$(jq -r '.asset' <<<"$row")
        hub=$(jq -r '.hub_id' <<<"$row"); spoke=$(jq -r '.spoke_id' <<<"$row")
        amount=$(jq -r '.amount' <<<"$row"); partial=$(jq -r '.partial' <<<"$row")
        inv "prod_decimal_${decimals}_mint" "$ADMIN" "$asset" -- mint --to "$ALICE_ADDR" --amount "$amount" >/dev/null || return 1
        wallet_pre=$(balance "$asset" "$ALICE_ADDR") || return 1
        pool_pre=$(balance "$asset" "$POOL") || return 1
        acct=$(inv_create "prod_decimal_${decimals}_supply" "$ALICE" "$CONTROLLER" -- supply \
            --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$spoke" --assets "$(pay_vec "$hub" "$asset" "$amount")" | tr -d '\"[:space:]') || return 1
        assert_delta "prod_decimal_${decimals}_paid" "$wallet_pre" "$(balance "$asset" "$ALICE_ADDR")" "-$amount" || return 1
        assert_delta "prod_decimal_${decimals}_received" "$pool_pre" "$(balance "$asset" "$POOL")" "$amount" || return 1
        assert_int_view_eq "prod_decimal_${decimals}_credited" "$amount" get_collateral_amount --account_id "$acct" --hub_asset "$(hub_key "$hub" "$asset")" || return 1
        inv "prod_decimal_${decimals}_partial" "$ALICE" "$CONTROLLER" -- withdraw --caller "$ALICE_ADDR" --account_id "$acct" \
            --withdrawals "$(pay_vec "$hub" "$asset" "$partial")" --to null >/dev/null || return 1
        assert_int_view_eq "prod_decimal_${decimals}_remaining" "$(raw_sub "$amount" "$partial")" get_collateral_amount --account_id "$acct" --hub_asset "$(hub_key "$hub" "$asset")" || return 1
        inv "prod_decimal_${decimals}_close" "$ALICE" "$CONTROLLER" -- withdraw --caller "$ALICE_ADDR" --account_id "$acct" \
            --withdrawals "$(pay_vec "$hub" "$asset" 0)" --to null >/dev/null || return 1
        wallet_post=$(balance "$asset" "$ALICE_ADDR") || return 1
        pool_post=$(balance "$asset" "$POOL") || return 1
        assert_delta "prod_decimal_${decimals}_wallet_restored" "$wallet_pre" "$wallet_post" 0 || return 1
        assert_delta "prod_decimal_${decimals}_pool_restored" "$pool_pre" "$pool_post" 0 || return 1
        assert_bool_view "prod_decimal_${decimals}_burned" false account_exists --account_id "$acct" || return 1
    done < <(jq -c '.[]' "$cases")
}

# Stable state only: ledger timestamps are deliberately excluded.
prod_position_snapshot() {
    local label="$1" acct="$2" asset="$3" runner="$4" positions attributes usage owner wallet pool
    local controller_cash book nft_state roles='[]' role held pa_owner n=9
    group_begin "$label" 8 reads || return 1
    group_spawn view "${label}_positions" "$CONTROLLER" -- get_account_positions --account_id "$acct"
    group_spawn view "${label}_attributes" "$CONTROLLER" -- get_account_attributes --account_id "$acct"
    group_spawn view "${label}_usage" "$CONTROLLER" -- get_spoke_usage --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key 1 "$asset")"
    group_spawn view "${label}_owner" "$POSITION_NFT" -- owner_of --token_id "$acct"
    group_spawn view "${label}_pool_book" "$POOL" -- get_sync_data --hub_asset "$(hub_key 1 "$asset")"
    group_spawn view "${label}_name" "$POSITION_NFT" -- name
    group_spawn view "${label}_symbol" "$POSITION_NFT" -- symbol
    group_spawn view "${label}_uri" "$POSITION_NFT" -- token_uri --token_id "$acct"
    group_spawn view "${label}_total" "$POSITION_NFT" -- total_supply
    for role in PROPOSER EXECUTOR CANCELLER GUARDIAN ORACLE; do
        group_spawn view "${label}_role_$role" "$GOVERNANCE" -- has_role --account "$ADMIN_ADDR" --role "$role"
    done
    group_spawn view "${label}_pa_owner" "$PRICE_AGGREGATOR" -- get_owner
    group_spawn balance "$asset" "$CONTROLLER"
    group_spawn balance "$asset" "$runner"
    group_spawn balance "$asset" "$POOL"
    group_end || return 1
    positions=$(group_out 1); attributes=$(group_out 2); usage=$(group_out 3); owner=$(group_out 4); book=$(group_out 5)
    nft_state=$(jq -nc --argjson name "$(group_out 6)" --argjson symbol "$(group_out 7)" \
        --argjson uri "$(group_out 8)" --argjson total "$(group_out 9)" \
        '{name:$name,symbol:$symbol,uri:$uri,total:$total}') || return 1
    for role in PROPOSER EXECUTOR CANCELLER GUARDIAN ORACLE; do
        n=$((n+1)); held=$(group_out "$n")
        roles=$(jq -nc --argjson roles "$roles" --arg role "$role" --argjson held "$held" '$roles+[{role:$role,held:$held}]') || return 1
    done
    pa_owner=$(group_out 15); controller_cash=$(group_out 16); wallet=$(group_out 17); pool=$(group_out 18)
    jq -ncS --argjson positions "$positions" --argjson attributes "$attributes" --argjson usage "$usage" \
        --argjson owner "$owner" --arg wallet "$wallet" --arg pool "$pool" --arg controller_cash "$controller_cash" \
        --argjson book "$book" --argjson nft "$nft_state" --argjson roles "$roles" --argjson pa_owner "$pa_owner" \
        '{positions:$positions,attributes:$attributes,usage:$usage,owner:$owner,wallet:$wallet,pool:$pool,controller_cash:$controller_cash,book:$book,nft:$nft,roles:$roles,pa_owner:$pa_owner}'
}

# Authority-only calls may change the NFT owner, never principal, usage or cash.
prod_caller_state() {
    local label="$1"
    if ! python3 - "$2" "$3" "$4" <<'PYAUTHSTATE'
import json,sys
before,after=map(json.loads,sys.argv[1:3]); before['owner']=sys.argv[3]
assert before==after, 'authority call changed financial/config state or wrong owner'
PYAUTHSTATE
    then
        _assert_fail "$label" 'authority transition changed unrelated state or left the wrong owner'
        return 1
    fi
    record "$label" ok assert "" "" "" "" "" 'expected NFT owner; unchanged positions, attributes, usage, pool books and cash'
}

prod_caller_authority() {
    local runner="$1" acct="$2" asset="$3" before after ops deny_ops
    before=$(prod_position_snapshot prod_auth_before "$acct" "$asset" "$runner") || return 1
    ops=$(jq -nc --argjson id "$acct" '[{RenewAccount:{account_id:$id}}]')
    xfail prod_contract_external_denied "Missing signing key for account $ADMIN_ADDR" "$BOB" "$runner" -- run \
        --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$ops" || return 1
    after=$(prod_position_snapshot prod_auth_after "$acct" "$asset" "$runner") || return 1
    prod_caller_state prod_contract_external_unchanged "$before" "$after" "$runner" || return 1

    prod_execute_split prod_manager_on setPositionManager >/dev/null || return 1
    ops=$(jq -nc --argjson id "$acct" --arg delegate "$BOB_ADDR" \
        '[{AddDelegate:{account_id:$id,delegate:$delegate}},{RenewAccount:{account_id:$id}}]')
    inv prod_contract_delegate_add "$ADMIN" "$runner" -- run --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$ops" >/dev/null || return 1
    after=$(prod_position_snapshot prod_grant_after "$acct" "$asset" "$runner") || return 1
    prod_caller_state prod_contract_grant_unchanged "$before" "$after" "$runner" || return 1
    lifecycle_inv prod_delegate_withdraw "$BOB" "$CONTROLLER" -- withdraw --caller "$BOB_ADDR" --account_id "$acct" \
        --withdrawals "$(pay_vec 1 "$asset" 1)" --to null >/dev/null || return 1
    lifecycle_inv prod_delegate_restore "$BOB" "$CONTROLLER" -- supply --caller "$BOB_ADDR" --account_id "$acct" \
        --spoke_id "$PRIMARY_SPOKE_ID" --assets "$(pay_vec 1 "$asset" 1)" >/dev/null || return 1

    before=$(prod_position_snapshot prod_revoke_before "$acct" "$asset" "$runner") || return 1
    ops=$(jq -nc --argjson id "$acct" --arg delegate "$BOB_ADDR" '[{RemoveDelegate:{account_id:$id,delegate:$delegate}}]')
    inv prod_contract_delegate_remove "$ADMIN" "$runner" -- run --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$ops" >/dev/null || return 1
    xfail_sim prod_delegate_revoked 'Error\(Contract, #44\)' "$BOB" "$CONTROLLER" -- withdraw \
        --caller "$BOB_ADDR" --account_id "$acct" --withdrawals "$(pay_vec 1 "$asset" 1)" --to null || return 1
    after=$(prod_position_snapshot prod_revoke_after "$acct" "$asset" "$runner") || return 1
    prod_caller_state prod_delegate_revoked_unchanged "$before" "$after" "$runner" || return 1

    ops=$(jq -nc --argjson id "$acct" --arg to "$CAROL_ADDR" '[{NftTransfer:{token_id:$id,to:$to}}]')
    inv prod_contract_nft_transfer "$ADMIN" "$runner" -- run --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$ops" >/dev/null || return 1
    after=$(prod_position_snapshot prod_transfer_after "$acct" "$asset" "$runner") || return 1
    prod_caller_state prod_contract_transfer_state "$before" "$after" "$CAROL_ADDR" || return 1
    before="$after"
    deny_ops=$(jq -nc --argjson id "$acct" --arg a "$asset" '[{Withdraw:{account_id:$id,withdrawals:[[{hub_id:1,asset:$a},"1"]],to:null}}]')
    xfail_sim prod_contract_previous_owner_denied 'Error\(Contract, #44\)' "$ADMIN" "$runner" -- run \
        --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$deny_ops" || return 1
    after=$(prod_position_snapshot prod_previous_owner_after "$acct" "$asset" "$runner") || return 1
    prod_caller_state prod_contract_previous_owner_unchanged "$before" "$after" "$CAROL_ADDR" || return 1
    lifecycle_inv prod_new_owner_withdraw "$CAROL" "$CONTROLLER" -- withdraw --caller "$CAROL_ADDR" --account_id "$acct" \
        --withdrawals "$(pay_vec 1 "$asset" 1)" --to null >/dev/null || return 1
    lifecycle_inv prod_new_owner_restore "$CAROL" "$CONTROLLER" -- supply --caller "$CAROL_ADDR" --account_id "$acct" \
        --spoke_id "$PRIMARY_SPOKE_ID" --assets "$(pay_vec 1 "$asset" 1)" >/dev/null || return 1
    before=$(prod_position_snapshot prod_restore_before "$acct" "$asset" "$runner") || return 1
    inv prod_contract_nft_restore "$CAROL" "$POSITION_NFT" -- transfer --from "$CAROL_ADDR" --to "$runner" --token_id "$acct" >/dev/null || return 1
    after=$(prod_position_snapshot prod_restore_after "$acct" "$asset" "$runner") || return 1
    prod_caller_state prod_contract_restore_state "$before" "$after" "$runner" || return 1
    before="$after"
    ops=$(jq -nc --argjson id "$acct" '[{RenewAccount:{account_id:$id}}]')
    inv prod_contract_renew_restored "$ADMIN" "$runner" -- run --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$ops" >/dev/null || return 1
    after=$(prod_position_snapshot prod_renew_after "$acct" "$asset" "$runner") || return 1
    prod_caller_state prod_contract_renew_unchanged "$before" "$after" "$runner" || return 1

    prod_execute_split prod_manager_off setPositionManager >/dev/null || return 1
    ops=$(jq -nc --argjson id "$acct" --arg delegate "$BOB_ADDR" '[{AddDelegate:{account_id:$id,delegate:$delegate}}]')
    xfail_sim prod_contract_inactive_manager_denied 'Error\(Contract, #44\)' "$ADMIN" "$runner" -- run \
        --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$ops" || return 1
    after=$(prod_position_snapshot prod_manager_off_after "$acct" "$asset" "$runner") || return 1
    prod_caller_state prod_contract_inactive_manager_unchanged "$before" "$after" "$runner"
}

flow_production_caller() {
    phase production_caller
    prod_decimal_roundtrips || return 1
    local asset runner acct ops amount pool_before controller_before positions synced decimals paid
    asset=$(jq -r '.markets[]|select(.name=="USDC")|.asset_address' "$RUN_DIR/config/testnet/markets.json")
    amount=10000000
    run_deploy "$LOG_DIR/script_runner.out" "$LOG_DIR/script_runner.err" -- stellar contract deploy \
        --source "$ADMIN" "${NET_ARGS[@]}" --wasm "$FIXTURE_WASM_DIR/script_runner.wasm" || return 1
    runner=$(sanitize_output "$LOG_DIR/script_runner.out")
    is_contract_id "$runner" || { _assert_fail prod_contract_runner 'invalid runner address'; return 1; }
    record prod_contract_runner ok deploy "$(extract_signing_hash "$LOG_DIR/script_runner.err")" "" "" "" "" "$runner" deployment "$runner"
    inv prod_contract_configure "$ADMIN" "$runner" -- configure_vault --owner "$ADMIN_ADDR" >/dev/null || return 1
    inv prod_mint_caller "$ADMIN" "$asset" -- mint --to "$runner" --amount "$amount" >/dev/null || return 1
    pool_before=$(balance "$asset" "$POOL") || return 1
    controller_before=$(balance "$asset" "$CONTROLLER") || return 1
    ops=$(jq -nc --arg a "$asset" --argjson s "$PRIMARY_SPOKE_ID" '[{Supply:{account_id:0,spoke_id:$s,assets:[[{hub_id:1,asset:$a},"10000000"]]}}]')
    acct=$(inv prod_contract_supply "$ADMIN" "$runner" -- run --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$ops" | tr -d '\"[:space:]') || return 1
    assert_view_eq_at "$POSITION_NFT" prod_contract_owner "$runner" owner_of --token_id "$acct" || return 1
    assert_delta prod_contract_payment "$amount" "$(balance "$asset" "$runner")" "-$amount" || return 1
    positions=$(view prod_contract_supply_positions "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    synced=$(view prod_contract_supply_sync "$POOL" -- get_sync_data --hub_asset "$(hub_key 1 "$asset")") || return 1
    decimals=$(view prod_contract_decimals "$asset" -- decimals | tr -d '\"[:space:]') || return 1
    paid=$(lifecycle_amount '"supply"' "$(hub_key 1 "$asset")" '[{},{}]' "$positions" "$synced" "$amount" "$decimals") \
        || { _assert_fail prod_contract_supply_shares 'contract supply credited wrong principal'; return 1; }
    record prod_contract_supply_shares ok assert "" "" "" "" "" 'exact committed-index contract supply shares'
    assert_delta prod_contract_supply_pool "$pool_before" "$(balance "$asset" "$POOL")" "$paid" || return 1
    assert_delta prod_contract_supply_controller "$controller_before" "$(balance "$asset" "$CONTROLLER")" 0 || return 1
    prod_caller_authority "$runner" "$acct" "$asset" || return 1
    # A successful first leg followed by an invalid second leg must leave no effects.
    local before after
    inv prod_mint_rollback "$ADMIN" "$asset" -- mint --to "$runner" --amount 1 >/dev/null || return 1
    before=$(prod_position_snapshot prod_rollback_before "$acct" "$asset" "$runner") || return 1
    ops=$(jq -nc --arg a "$asset" --argjson id "$acct" --argjson s "$PRIMARY_SPOKE_ID" \
        '[{Supply:{account_id:$id,spoke_id:$s,assets:[[{hub_id:1,asset:$a},"1"]]}},{Borrow:{account_id:$id,borrows:[[{hub_id:1,asset:$a},"0"]],to:null}}]')
    xfail_sim prod_contract_rollback 'Error\(Contract, #14\)' "$ADMIN" "$runner" -- run --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$ops" || return 1
    after=$(prod_position_snapshot prod_rollback_after "$acct" "$asset" "$runner") || return 1
    [ "$before" = "$after" ] || { _assert_fail prod_contract_rollback_state "failed script changed position, owner, usage or balances"; return 1; }
    record prod_contract_rollback_state ok assert "" "" "" "" "" "simulation rejected; stored position, usage and balances unchanged"
    # Snapshot active positions, attributes, usage, ownership and cash across upgrade.
    view prod_position_before "$CONTROLLER" -- get_collateral_amount --account_id "$acct" --hub_asset "$(hub_key 1 "$asset")" >/dev/null || return 1
    before=$(prod_position_snapshot prod_upgrade_before "$acct" "$asset" "$runner") || return 1
    local verb hash
    for verb in $PROD_UPGRADES; do
        hash=$(prod_upgrade_hash "$verb") || return 1
        prod_execute_split "$verb" "$verb" "$hash" >/dev/null || return 1
    done
    after=$(prod_position_snapshot prod_upgrade_after "$acct" "$asset" "$runner") || return 1
    [ "$before" = "$after" ] || { _assert_fail prod_upgrade_state "position, owner, usage or balances changed"; return 1; }
    record prod_upgrade_state ok assert "" "" "" "" "" "current-schema state retained across same-hash upgrade"
    view prod_position_after "$CONTROLLER" -- get_collateral_amount --account_id "$acct" --hub_asset "$(hub_key 1 "$asset")" >/dev/null || return 1
    printf '%s\n' "$before" > "$RUN_DIR/upgrade-before.json"
    printf '%s\n' "$after" > "$RUN_DIR/upgrade-after.json"
    prod_verify_policy || return 1
    PROD_OP_TAG=unpause_after_upgrade prod_ops unpause >/dev/null || return 1
    assert_view_eq_at "$POSITION_NFT" prod_upgrade_owner "$runner" owner_of --token_id "$acct" || return 1
    printf '{"baseline":"%s","candidate":"%s","executable_differs":false,"schema":"current"}\n' "$CTRL_HASH" "$CTRL_HASH" > "$RUN_DIR/upgrade.json"
    positions=$(view prod_contract_close_before "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    pool_before=$(balance "$asset" "$POOL") || return 1
    controller_before=$(balance "$asset" "$CONTROLLER") || return 1
    local wallet_before closed
    wallet_before=$(balance "$asset" "$runner") || return 1
    ops=$(jq -nc --arg a "$asset" --argjson id "$acct" '[{Withdraw:{account_id:$id,withdrawals:[[{hub_id:1,asset:$a},"0"]],to:null}}]')
    inv prod_contract_withdraw "$ADMIN" "$runner" -- run --controller "$CONTROLLER" --nft "$POSITION_NFT" --ops "$ops" >/dev/null || return 1
    closed=$(view prod_contract_close_after "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    synced=$(view prod_contract_close_sync "$POOL" -- get_sync_data --hub_asset "$(hub_key 1 "$asset")") || return 1
    paid=$(lifecycle_amount '"withdraw"' "$(hub_key 1 "$asset")" "$positions" "$closed" "$synced" 0 "$decimals") \
        || { _assert_fail prod_contract_close_shares 'contract close burned wrong principal'; return 1; }
    record prod_contract_close_shares ok assert "" "" "" "" "" 'exact committed-index contract close payout and shares'
    assert_delta prod_contract_return "$wallet_before" "$(balance "$asset" "$runner")" "$paid" || return 1
    assert_delta prod_contract_close_pool "$pool_before" "$(balance "$asset" "$POOL")" "-$paid" || return 1
    assert_delta prod_contract_close_controller "$controller_before" "$(balance "$asset" "$CONTROLLER")" 0 || return 1
    assert_bool_view prod_contract_closed false account_exists --account_id "$acct" || return 1
    local market
    market=$(jq -c '.markets[]|select(.name=="USDC")' "$RUN_DIR/config/testnet/markets.json") || return 1
    inv prod_governance_band "$ADMIN" "$GOVERNANCE" -- set_sanity_band --caller "$ADMIN_ADDR" --key "$(price_key_token "$asset")" --min_wad "$(jq -r '.oracle.min_sanity_price_wad' <<<"$market")" --max_wad "$(jq -r '.oracle.max_sanity_price_wad' <<<"$market")" >/dev/null || return 1
    inv prod_governance_flags "$ADMIN" "$GOVERNANCE" -- set_spoke_asset_flags --caller "$ADMIN_ADDR" --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key 1 "$asset")" --paused true --frozen true --no_seize false >/dev/null || return 1
    prod_ops pause >/dev/null || return 1
    PROD_OP_TAG=unpause_after_pause prod_ops unpause >/dev/null
}

# Real candidate XOXNO adapter feeds the production-shaped risk path. The
# underlying token/provider data are explicitly disposable fixtures.
flow_production_lending() {
    phase production_lending
    local plan asset hub amount spoke debt debt_hub supplier acct verb hash
    for verb in $PROD_UPGRADES; do
        hash=$(prod_upgrade_hash "$verb") || return 1
        prod_propose "$verb" "$verb" "$hash" || return 1
    done
    prod_propose prod_manager_on setPositionManager "$BOB_ADDR" true || return 1
    prod_propose prod_manager_off setPositionManager "$BOB_ADDR" false || return 1
    plan=$(python3 "$INTEG_DIR/production_config.py" lending "$RUN_DIR/config/testnet" "$RUN_DIR/config/networks.json") || return 1
    asset=$(jq -r '.asset' <<<"$plan"); hub=$(jq -r '.hub_id' <<<"$plan"); amount=$(jq -r '.amount' <<<"$plan")
    spoke=$(jq -r '.spoke_id' <<<"$plan"); debt=$(jq -r '.debt' <<<"$plan"); debt_hub=$(jq -r '.debt_hub' <<<"$plan")
    inv prod_lending_mint_liquidity "$ADMIN" "$debt" -- mint --to "$ADMIN_ADDR" --amount 10000000000 >/dev/null || return 1
    inv prod_lending_mint_collateral "$ADMIN" "$asset" -- mint --to "$BOB_ADDR" --amount "$amount" >/dev/null || return 1
    inv prod_lending_interest_buffer "$ADMIN" "$debt" -- mint --to "$BOB_ADDR" --amount 10000000 >/dev/null || return 1
    supplier=$(lifecycle_inv prod_lending_seed "$ADMIN" "$CONTROLLER" -- supply --caller "$ADMIN_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" --assets "$(pay_vec "$debt_hub" "$debt" 10000000000)" | tr -d '\"[:space:]') || return 1
    acct=$(lifecycle_inv prod_lending_supply "$BOB" "$CONTROLLER" -- supply --caller "$BOB_ADDR" --account_id 0 --spoke_id "$spoke" --assets "$(pay_vec "$hub" "$asset" "$amount")" | tr -d '\"[:space:]') || return 1
    lifecycle_inv prod_lending_borrow "$BOB" "$CONTROLLER" -- borrow --caller "$BOB_ADDR" --account_id "$acct" --borrows "$(pay_vec "$debt_hub" "$debt" 100000000)" --to null >/dev/null || return 1
    assert_hf_at_least prod_lending_hf "$acct" "$WAD" || return 1
    lifecycle_inv prod_lending_repay "$BOB" "$CONTROLLER" -- repay --caller "$BOB_ADDR" --account_id "$acct" --payments "$(pay_vec "$debt_hub" "$debt" 110000000)" >/dev/null || return 1
    assert_int_view_eq prod_lending_debt_zero 0 get_borrow_amount --account_id "$acct" --hub_asset "$(hub_key "$debt_hub" "$debt")" || return 1
    lifecycle_inv prod_lending_withdraw "$BOB" "$CONTROLLER" -- withdraw --caller "$BOB_ADDR" --account_id "$acct" --withdrawals "$(pay_vec "$hub" "$asset" 0)" --to null >/dev/null || return 1
    lifecycle_inv prod_lending_unseed "$ADMIN" "$CONTROLLER" -- withdraw --caller "$ADMIN_ADDR" --account_id "$supplier" --withdrawals "$(pay_vec "$debt_hub" "$debt" 0)" --to null >/dev/null || return 1
    assert_bool_view prod_lending_closed false account_exists --account_id "$acct" || return 1
}
