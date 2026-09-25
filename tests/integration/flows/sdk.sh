sdk_inv() {
    local label="$1" builder="$2" args="$3" result="$LOG_DIR/$1.sdk.json"
    local method
    case "$builder" in
        buildStellarSupplyTx) method=supply;; buildStellarBorrowTx) method=borrow;;
        buildStellarRepayTx) method=repay;; buildStellarWithdrawTx) method=withdraw;;
        buildStellarMultiplyTx) method=multiply;; buildStellarMigrateFromBlendTx) method=migrate_from_blend;;
        *) _assert_fail "$label" "unmapped published builder $builder"; return 1;;
    esac
    export RPC_URL CONTROLLER
    if stellar keys secret "$ALICE" | "${NODE_BIN:-node}" "$INTEG_DIR/sdk/invoke.mjs" "$builder" "$args" "$LOG_DIR/$label" > "$result" 2> "$LOG_DIR/$label.err"; then
        if [ -n "${EXPECT_ERROR:-}" ]; then
            record "$label" xfail "$method" "" "" "" "" "" "published SDK error mapping: $EXPECT_ERROR" simulation "$CONTROLLER"
        else
            record "$label" ok "$method" "$(jq -r '.hash // empty' "$result")" "" "" "" "" "published SDK 1.0.220; Stellar SDK 16.0.1" transaction "$CONTROLLER"
        fi
        jq -r '.value' "$result"
    else
        local hash='' execution=simulation
        if [ -f "$LOG_DIR/$label.hash" ]; then
            hash=$(cat "$LOG_DIR/$label.hash")
            [[ "$hash" =~ ^[0-9a-f]{64}$ ]] || hash=''
            [ -z "$hash" ] || execution=transaction
        fi
        record "$label" FAIL "$method" "$hash" "" "" "" "" "$(tail_err_note "$LOG_DIR/$label.err")" "$execution" "$CONTROLLER"
        return 1
    fi
}

# Same financial checker as CLI lifecycle; only public-builder transport differs.
sdk_lifecycle_submit() {
    local label="$1" signer="$2" contract="$3"; shift 3
    local method="$2" caller='' acct=0 spoke="$PRIMARY_SPOKE_ID" payments='' builder args
    shift 2
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --caller) caller="$2";; --account_id) acct="$2";; --spoke_id) spoke="$2";;
            --assets|--borrows|--payments|--withdrawals) payments="$2";;
            --to) [ "$2" = null ] || return 1;;
            *) return 1;;
        esac
        shift 2
    done
    [ "$signer" = "$ALICE" ] && [ "$caller" = "$ALICE_ADDR" ] && [ "$contract" = "$CONTROLLER" ] || return 1
    args=$(jq -ce --arg id "$acct" --argjson spoke "$spoke" \
        'select(length == 1) | {asset:.[0][0].asset,hubId:.[0][0].hub_id,amount:.[0][1],accountNonce:$id,spokeId:$spoke}' <<<"$payments") || return 1
    case "$method" in
        supply) builder=buildStellarSupplyTx;; borrow) builder=buildStellarBorrowTx;;
        repay) builder=buildStellarRepayTx;; withdraw) builder=buildStellarWithdrawTx;; *) return 1;;
    esac
    sdk_inv "$label" "$builder" "$args"
}

flow_sdk_lifecycle() {
    phase sdk_lifecycle
    local acct
    acct=$(lifecycle_checked sdk_lifecycle_submit sdk_supply "$ALICE" "$CONTROLLER" -- supply --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" --assets "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1000000000)") || return 1
    save_state SDK_ACCT "$acct"
    DELAY_SIGN_SECONDS=15 lifecycle_checked sdk_lifecycle_submit sdk_borrow "$ALICE" "$CONTROLLER" -- borrow --caller "$ALICE_ADDR" --account_id "$acct" --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 100000000)" --to null >/dev/null || return 1
    lifecycle_checked sdk_lifecycle_submit sdk_repay "$ALICE" "$CONTROLLER" -- repay --caller "$ALICE_ADDR" --account_id "$acct" --payments "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 110000000)" >/dev/null || return 1
    assert_borrow_at_most sdk_debt_cleared "$acct" "$USDC_SAC" 0 || return 1
    lifecycle_checked sdk_lifecycle_submit sdk_withdraw "$ALICE" "$CONTROLLER" -- withdraw --caller "$ALICE_ADDR" --account_id "$acct" --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 0)" --to null >/dev/null || return 1
    assert_bool_view sdk_closed false account_exists --account_id "$acct"
}

flow_sdk_errors() {
    phase sdk_errors
    EXPECT_ERROR=AmountMustBePositive sdk_inv sdk_error buildStellarSupplyTx "$(jq -nc --arg a "$USDC_SAC" --argjson h "$PRIMARY_HUB_ID" --argjson s "$PRIMARY_SPOKE_ID" '{asset:$a,hubId:$h,spokeId:$s,amount:"0",accountNonce:0}')" >/dev/null || return 1
}

flow_sdk_strategy() {
    phase sdk_strategy
    local route route_hex args acct hash
    local before="$LOG_DIR/sdk_multiply.before.json" after="$LOG_DIR/sdk_multiply.after.json" request="$LOG_DIR/sdk_multiply.request.json"
    route_hex=$(agg_route_hex "$USDC_SAC" "$XLM_SAC" 100000000) || { _assert_fail sdk_quote "route missing"; return 1; }
    route=$(printf '%s' "$route_hex" | xxd -r -p | base64 | tr -d '\n') || return 1
    args=$(jq -nc --arg x "$XLM_SAC" --arg u "$USDC_SAC" --arg r "$route" --argjson h "$PRIMARY_HUB_ID" --argjson s "$PRIMARY_SPOKE_ID" \
        '{spokeId:$s,accountNonce:0,collateral:{hubId:$h,asset:$x},debt:{hubId:$h,asset:$u},debtToFlashLoan:"100000000",mode:1,steps:{routeXdr:$r},initialPayment:{hubId:$h,asset:$x,amount:"10000000000"}}') || return 1
    # Match the native financial reference to the exact published-builder inputs.
    jq -c --arg caller "$ALICE_ADDR" --arg swap "$route_hex" '
        def key: {hub_id:.hubId,asset:.asset};
        {method:"multiply","--caller":$caller,"--account_id":(.accountNonce|tostring),
         "--spoke_id":(.spokeId|tostring),"--mode":(.mode|tostring),
         "--collateral":(.collateral|key|tojson),"--debt":(.debt|key|tojson),
         "--debt_to_flash_loan":.debtToFlashLoan,"--swap":$swap,
         "--initial_payment":([(.initialPayment|key),.initialPayment.amount]|tojson),
         "--convert_swap":"null"}' <<<"$args" >"$request" || return 1
    strategy_snapshot sdk_multiply_before "$ALICE_ADDR" 0 >"$before" || return 1
    acct=$(sdk_inv sdk_multiply buildStellarMultiplyTx "$args") || return 1
    [[ "$acct" =~ ^[1-9][0-9]*$ ]] || { _assert_fail sdk_multiply "invalid strategy account"; return 1; }
    strategy_snapshot sdk_multiply_after "$ALICE_ADDR" "$acct" >"$after" || return 1
    hash=$(jq -er '.hash | select(type == "string" and test("^[0-9a-f]{64}$"))' "$LOG_DIR/sdk_multiply.sdk.json") || return 1
    strategy_assert_financial sdk_multiply_financial "$before" "$after" "$LOG_DIR/$hash.receipt.json" "$request" "$acct" "$CONTROLLER" "$AGGREGATOR" || return 1
    assert_hf_at_least sdk_multiply_hf "$acct" "$WAD"
}

# Transport adapter only: snapshot, committed rates and financial predicates
# stay in the same wrapper exercised by native CLI migrations.
sdk_blend_submit() {
    local label="$1" signer="$2" contract="$3"; shift 3
    [ "$1" = -- ] && [ "$2" = migrate_from_blend ] || return 1
    shift 2
    local caller='' acct='' spoke='' hub='' pool='' coll='' supply='' debt='' args
    while [ "$#" -gt 0 ]; do
        [ "$#" -ge 2 ] || return 1
        case "$1" in
            --caller) caller="$2";; --account_id) acct="$2";; --spoke_id) spoke="$2";;
            --hub_id) hub="$2";; --blend_pool) pool="$2";;
            --collateral_assets) coll="$2";; --supply_assets) supply="$2";; --debt_caps) debt="$2";;
            *) return 1;;
        esac
        shift 2
    done
    [ "$signer" = "$ALICE" ] && [ "$caller" = "$ALICE_ADDR" ] && [ "$contract" = "$CONTROLLER" ] || return 1
    [[ "$acct" =~ ^[0-9]+$ ]] && [ -n "$pool" ] || return 1
    args=$(jq -nc --arg id "$acct" --arg p "$pool" --argjson h "$hub" --argjson s "$spoke" \
        --argjson c "$coll" --argjson u "$supply" --argjson d "$debt" \
        '{accountId:$id,spokeId:$s,hubId:$h,blendPool:$p,collateralTokens:$c,supplyTokens:$u,debtCaps:($d|map({token:.[0],cap:.[1]}))}') || return 1
    sdk_inv "$label" buildStellarMigrateFromBlendTx "$args"
}

flow_sdk_blend() {
    phase sdk_blend
    blend_seed sdk_blend_seed "$ALICE" "$ALICE_ADDR" 2000000000 500000000 100000000 || return 1
    local acct
    acct=$(blend_migrate_checked sdk_blend_submit sdk_blend "$ALICE" "$ALICE_ADDR" 0 \
        "$(blend_addr_json "$XLM_SAC")" "$(blend_addr_json "$XLM_SAC")" \
        "$(blend_debt_json "$XLM_SAC" 200000000)") || return 1
    save_state SDK_BLEND_ACCT "$acct"
}
