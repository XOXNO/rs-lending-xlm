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

prod_ops() {
    local verb="$1" tag="${PROD_OP_TAG:-$1}"; shift
    if NETWORK=testnet CONFIG_ROOT="$RUN_DIR/config" OPS_ROOT="$RUN_DIR/ops" SIGNER="$ADMIN" AUTO_EXECUTE=1 STELLAR_SEND=yes \
        AWAIT_MAX_WAIT_SECONDS=180 bash "$REPO_ROOT/configs/script.sh" "$verb" "$@" >"$LOG_DIR/operator_$tag.out" 2>"$LOG_DIR/operator_$tag.err"; then
        local hash n=0 invocation method target proposed=0 executed=0 proposal_salt='' execution_salt='' salt
        while read -r hash; do
            n=$((n+1))
            [ "$(tx_status "$hash")" = SUCCESS ] && fetch_resources "$hash" || { _assert_fail "operator_$tag" "unconfirmed receipt/resources $hash"; return 1; }
            invocation=$(jq -r '.result.envelopeXdr' "$LOG_DIR/$hash.receipt.json" | stellar xdr decode --type TransactionEnvelope --output json | jq -ce '.tx.tx.operations[0].body.invoke_host_function.host_function.invoke_contract') || return 1
            method=$(jq -r '.function_name' <<<"$invocation"); target=$(jq -r '.contract_address' <<<"$invocation")
            if [ "$target" = "$GOVERNANCE" ]; then
                case "$method" in propose) proposed=$((proposed+1));; execute|execute_self) executed=$((executed+1));; esac
            fi
            if [[ "$verb" = upgrade*Hash ]]; then
                salt=$(prod_upgrade_invocation "$verb" "$1" "$GOVERNANCE" "$CONTROLLER" "$PRICE_AGGREGATOR" "$invocation") \
                    || { _assert_fail "operator_${tag}_binding" 'upgrade receipt differs from requested target, operation or WASM hash'; return 1; }
                if [ "$method" = propose ]; then proposal_salt="$salt"; else execution_salt="$salt"; fi
            fi
            record "operator_${tag}_tx_$n" ok "$method" "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" "operator confirmed receipt" transaction "$target"
        done < <(grep -oE 'Signing transaction: [0-9a-f]{64}' "$LOG_DIR/operator_$tag.err" | awk '{print $3}' | sort -u)
        case "$tag" in
            validateConfigs|setupAll_replay) ;;
            *) [ "$n" -gt 0 ] || { _assert_fail "operator_$tag" 'mutation returned without a confirmed submission'; return 1; };;
        esac
        if [[ "$verb" = upgrade*Hash ]]; then
            [ "$proposed" -eq 1 ] && [ "$executed" -eq 1 ] && [ "$proposal_salt" = "$execution_salt" ] || { _assert_fail "operator_${tag}_receipts" 'upgrade needs confirmed governance propose and execute'; return 1; }
            record "operator_${tag}_receipts" ok assert "" "" "" "" "" 'confirmed governance schedule and execution'
        fi
        record "operator_$tag" ok "$verb" "" "" "" "" "" "disposable config and operation records; $n confirmed submissions"
        cat "$LOG_DIR/operator_$tag.out"
    else
        record "operator_$tag" FAIL "$verb" "" "" "" "" "" "$(tail_err_note "$LOG_DIR/operator_$tag.err")"
        return 1
    fi
}

flow_production_fixtures() {
    phase production_fixtures
    mkdir -p "$RUN_DIR/config/testnet"
    local plan="$RUN_DIR/fixture-plan.json" mapping="$RUN_DIR/address-map.json" row original kind wasm id n=0
    python3 "$INTEG_DIR/production_config.py" plan "$REPO_ROOT/configs/mainnet" > "$plan" || return 1
    printf '{}\n' > "$mapping"
    while read -r row; do
        original=$(jq -r '.key' <<<"$row"); kind=$(jq -r '.value.kind' <<<"$row"); n=$((n+1))
        case "$kind" in
            Reflector) wasm=mock_oracle;; RedStone) wasm=mock_redstone;; Xoxno) wasm=xoxno-oracle-adapter;; *) wasm=production_fixture;;
        esac
        local args=()
        if [ "$wasm" = production_fixture ]; then
            args=(-- --admin "$ADMIN_ADDR" --decimals "$(jq -r '.value.decimals' <<<"$row")" --symbol "$(jq -r '.value.name' <<<"$row")")
        fi
        local wasm_path="$FIXTURE_WASM_DIR/$wasm.wasm"
        if [ "$kind" = Xoxno ]; then
            wasm_path="$WASM_DIR/$wasm.wasm"
            args=(-- --admin "$ADMIN_ADDR" --signers "[\"$ADMIN_ADDR\"]" --threshold 1 --resolution 60)
        fi
        run_deploy "$LOG_DIR/fixture_$n.out" "$LOG_DIR/fixture_$n.err" -- stellar contract deploy \
            --source "$ADMIN" "${NET_ARGS[@]}" --wasm "$wasm_path" ${args[@]+"${args[@]}"} || return 1
        id=$(sanitize_output "$LOG_DIR/fixture_$n.out")
        is_contract_id "$id" || return 1
        jq --arg o "$original" --arg i "$id" '.[$o]=$i' "$mapping" > "$mapping.tmp" && mv "$mapping.tmp" "$mapping"
        record "production_fixture_$n" ok deploy "" "" "" "" "" "$kind $original -> $id (fixture)"
    done < <(jq -c 'to_entries[]' "$plan")
    python3 "$INTEG_DIR/production_config.py" materialize "$REPO_ROOT/configs/mainnet" "$RUN_DIR/config/testnet" "$mapping" || return 1
    local fixtures="$RUN_DIR/config/testnet/fixtures.json" seed price
    cp "$fixtures" "$RUN_DIR/production-fixtures.json" || return 1
    while read -r row; do
        inv prod_reflector_base "$ADMIN" "$(jq -r '.key' <<<"$row")" -- set_base --base "$(jq -c '.value' <<<"$row")" >/dev/null || return 1
    done < <(jq -c '.bases|to_entries[]' "$fixtures")
    while read -r seed; do
        kind=$(jq -r '.kind' <<<"$seed"); id=$(jq -r '.contract' <<<"$seed"); price=$(jq -r '.price' <<<"$seed")
        if [ "$kind" = Reflector ]; then
            inv prod_reflector_seed "$ADMIN" "$id" -- set_price --asset "$(jq -c '.asset' <<<"$seed")" --price_wad "$price" >/dev/null || return 1
        elif [ "$kind" = Xoxno ]; then
            local feed_id
            feed_id=$(jq -r '.feed' <<<"$seed")
            inv prod_xoxno_register "$ADMIN" "$id" -- register_feed --feed_id "$feed_id" >/dev/null || return 1
            inv prod_xoxno_seed "$ADMIN" "$id" -- submit_price --signer "$ADMIN_ADDR" --feed_id "$feed_id" \
                --price "$(python3 -c 'import sys; print(int(sys.argv[1])//10**10)' "$price")" \
                --package_timestamp "$(( ($(date +%s) - 10) * 1000 ))" >/dev/null || return 1
        else
            inv prod_redstone_seed "$ADMIN" "$id" -- set_price --feed_id "$(jq -r '.feed' <<<"$seed")" --price_wad "$price" >/dev/null || return 1
        fi
    done < <(jq -c '.seeds[]' "$fixtures")
    while read -r row; do
        inv prod_lp_fixture "$ADMIN" "$(jq -r '.contract' <<<"$row")" -- configure_pool --snapshot "$(jq -c '.snapshot' <<<"$row")" >/dev/null || return 1
    done < <(jq -c '.pools[]' "$fixtures")
}

# Reflector TWAP includes two older 300-second samples. Keep fixture prices
# fresh after the long governance setup without changing production policy.
prod_refresh_reflectors() {
    local label="$1" seeds seed n=0
    seeds=$(jq -ce '[.seeds[] | select(.kind == "Reflector")] | if length > 0 then . else error("missing Reflector fixtures") end' \
        "$RUN_DIR/config/testnet/fixtures.json") || return 1
    while read -r seed; do
        inv "$label" "$ADMIN" "$(jq -r '.contract' <<<"$seed")" -- set_price \
            --asset "$(jq -c '.asset' <<<"$seed")" --price_wad "$(jq -r '.price' <<<"$seed")" >/dev/null || return 1
        n=$((n+1))
    done < <(jq -c '.[]' <<<"$seeds")
    record "${label}_complete" ok assert "" "" "" "" "" "$n Reflector fixture timestamps refreshed; prices and policy unchanged"
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
    prod_ops setPriceAggregator >/dev/null || return 1
    prod_ops setAggregator >/dev/null || return 1
    prod_ops setAccumulator >/dev/null || return 1
    prod_ops validateConfigs >/dev/null || return 1
    prod_ops setupAll >/dev/null || return 1
    cp "$RUN_DIR/config/networks.json" "$RUN_DIR/operator-before-replay.json"
    PROD_OP_TAG=setupAll_replay prod_ops setupAll >/dev/null || return 1
    cmp -s "$RUN_DIR/operator-before-replay.json" "$RUN_DIR/config/networks.json" || { _assert_fail operator_replay "setup replay changed deployment mappings"; return 1; }
    prod_ops unpause >/dev/null || return 1
    prod_refresh_reflectors prod_reflector_after_setup || return 1
    local m asset hub key oracle decimals
    save_state MARKETS ''
    while read -r m; do
        asset=$(jq -r '.asset_address' <<<"$m"); hub=$(jq -r '.hub_id' <<<"$m")
        decimals=$(jq -r '.oracle.asset_decimals' <<<"$m")
        assert_view_eq_at "$asset" "prod_decimals_${asset:0:8}" "$decimals" decimals || return 1
        key=$(price_key_token "$asset")
        view "prod_price_${asset:0:8}" "$PRICE_AGGREGATOR" -- prices --keys "[$key]" >/dev/null || return 1
        python3 "$INTEG_DIR/production_config.py" verify-price "$RUN_DIR/config/testnet" "$(jq -r '.name' <<<"$m")" "$LOG_DIR/prod_price_${asset:0:8}.out" \
            || { _assert_fail "prod_price_${asset:0:8}" "price/decimals differ from seeded production fixture"; return 1; }
        oracle=$(view "prod_oracle_${asset:0:8}" "$PRICE_AGGREGATOR" -- oracle --key "$key") || return 1
        [ -n "$oracle" ] || return 1
        save_state MARKETS "${MARKETS:+$MARKETS }$hub:$asset"
    done < <(jq -c '.markets[]' "$RUN_DIR/config/testnet/markets.json")
    save_state PRIMARY_HUB_ID 1
    save_state PRIMARY_SPOKE_ID "$(jq -r '.testnet.spoke_ids["1"]' "$RUN_DIR/config/networks.json")"
    prod_verify_policy || return 1
}

prod_verify_policy() {
    local checks="$RUN_DIR/production-policy-checks.json" row i=0 target method arg
    python3 "$INTEG_DIR/production_config.py" checks "$RUN_DIR/config/testnet" "$RUN_DIR/config/networks.json" > "$checks" || return 1
    while read -r row; do
        target=$(jq -r '.contract' <<<"$row"); method=$(jq -r '.method' <<<"$row")
        local args=()
        while IFS= read -r arg; do args+=("$arg"); done < <(jq -r '.args[]' <<<"$row")
        view "prod_policy_$i" "${!target}" -- "$method" ${args[@]+"${args[@]}"} >/dev/null || return 1
        python3 "$INTEG_DIR/production_config.py" verify "$checks" "$i" "$LOG_DIR/prod_policy_$i.out" \
            || { _assert_fail "prod_policy_$i" "$method differs from production policy"; return 1; }
        i=$((i+1))
    done < <(jq -c '.[]' "$checks")
    record prod_policy_equal ok assert "" "" "" "" "" "$i oracle/market/spoke policy readbacks matched"
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
    positions=$(view "${label}_positions" "$CONTROLLER" -- get_account_positions --account_id "$acct") || return 1
    attributes=$(view "${label}_attributes" "$CONTROLLER" -- get_account_attributes --account_id "$acct") || return 1
    usage=$(view "${label}_usage" "$CONTROLLER" -- get_spoke_usage --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key 1 "$asset")") || return 1
    owner=$(view "${label}_owner" "$POSITION_NFT" -- owner_of --token_id "$acct") || return 1
    local controller_cash book nft_state roles='[]' role held pa_owner
    book=$(view "${label}_pool_book" "$POOL" -- get_sync_data --hub_asset "$(hub_key 1 "$asset")") || return 1
    nft_state=$(jq -nc --argjson name "$(view "${label}_name" "$POSITION_NFT" -- name)" \
        --argjson symbol "$(view "${label}_symbol" "$POSITION_NFT" -- symbol)" \
        --argjson uri "$(view "${label}_uri" "$POSITION_NFT" -- token_uri --token_id "$acct")" \
        --argjson total "$(view "${label}_total" "$POSITION_NFT" -- total_supply)" \
        '{name:$name,symbol:$symbol,uri:$uri,total:$total}') || return 1
    for role in PROPOSER EXECUTOR CANCELLER GUARDIAN ORACLE; do
        held=$(view "${label}_role_$role" "$GOVERNANCE" -- has_role --account "$ADMIN_ADDR" --role "$role") || return 1
        roles=$(jq -nc --argjson roles "$roles" --arg role "$role" --argjson held "$held" '$roles+[{role:$role,held:$held}]') || return 1
    done
    pa_owner=$(view "${label}_pa_owner" "$PRICE_AGGREGATOR" -- get_owner) || return 1
    controller_cash=$(balance "$asset" "$CONTROLLER") || return 1
    wallet=$(balance "$asset" "$runner") || return 1
    pool=$(balance "$asset" "$POOL") || return 1
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

    PROD_OP_TAG=prod_manager_on prod_ops setPositionManager "$BOB_ADDR" true >/dev/null || return 1
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

    PROD_OP_TAG=prod_manager_off prod_ops setPositionManager "$BOB_ADDR" false >/dev/null || return 1
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
    for verb in upgradeControllerHash upgradePoolHash upgradePositionNftHash upgradePriceAggregatorHash upgradeGovernanceHash; do
        case "$verb" in
            upgradeControllerHash) hash="$CTRL_HASH";;
            upgradePoolHash) hash="$POOL_HASH";;
            upgradePositionNftHash) hash="$NFT_HASH";;
            upgradePriceAggregatorHash) hash="$PA_HASH";;
            upgradeGovernanceHash) hash=$(jq -er '.artifacts["governance.wasm"]' "$RUN_DIR/candidate.json") || return 1;;
        esac
        prod_ops "$verb" "$hash" >/dev/null || return 1
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
    local plan asset hub amount spoke debt debt_hub supplier acct
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
