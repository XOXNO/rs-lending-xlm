nft_assert_enumeration() {
    local label="$1" owner="$2" token="$3" present="$4" count id ids='' i
    count=$(view "${label}_count" "$POSITION_NFT" -- balance --account "$owner" | tr -d '\"[:space:]') || return 1
    [[ "$count" =~ ^[0-9]+$ ]] || { _assert_fail "$label" "invalid NFT balance"; return 1; }
    for ((i=0; i<count; i++)); do
        id=$(view "${label}_item_$i" "$POSITION_NFT" -- get_owner_token_id --owner "$owner" --index "$i" | tr -d '\"[:space:]') || return 1
        ids="$ids $id"
    done
    if ! python3 - "$ids" "$count" "$token" "$present" <<'PYNFT'
import sys
ids=list(map(int,sys.argv[1].split()))
assert len(ids)==int(sys.argv[2]) and len(set(ids))==len(ids)
assert (int(sys.argv[3]) in ids)==(sys.argv[4]=='true')
PYNFT
    then _assert_fail "$label" "owner enumeration incorrect or duplicated"; return 1; fi
    record "$label" ok assert "" "" "" "" "" "unique enumeration; token membership=$present"
}

flow_nft() {
    phase nft
    local acct until ledger total_before
    total_before=$(view nft_total_before "$POSITION_NFT" -- total_supply | tr -d '"[:space:]') || return 1
    acct=$(inv_create nft_supply "$ALICE" "$CONTROLLER" -- supply --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" --assets "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 10000000)") || return 1
    ledger=$(curl --fail-with-body -sS -m 30 "$RPC_URL" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' | jq -er '.result.sequence') || return 1
    until=$((ledger + 1000))
    local name
    name=$(view nft_name "$POSITION_NFT" -- name | jq -er '.') || return 1
    [ "$name" = 'XOXNO Lending Position' ] || { _assert_fail nft_name 'incorrect collection name'; return 1; }
    assert_view_eq_at "$POSITION_NFT" nft_symbol XLEND symbol || return 1
    assert_view_eq_at "$POSITION_NFT" nft_uri "https://api.xoxno.com/user/lending/image/${acct}?isStatic=true&chain=STELLAR" token_uri --token_id "$acct" || return 1
    assert_view_eq_at "$POSITION_NFT" nft_mint_total "$((total_before + 1))" total_supply || return 1
    nft_assert_enumeration nft_mint_enumeration "$ALICE_ADDR" "$acct" true || return 1
    inv nft_transfer "$ALICE" "$POSITION_NFT" -- transfer --from "$ALICE_ADDR" --to "$BOB_ADDR" --token_id "$acct" >/dev/null || return 1
    assert_view_eq_at "$POSITION_NFT" nft_transferred "$BOB_ADDR" owner_of --token_id "$acct" || return 1
    nft_assert_enumeration nft_sender_enumeration "$ALICE_ADDR" "$acct" false || return 1
    nft_assert_enumeration nft_receiver_enumeration "$BOB_ADDR" "$acct" true || return 1
    xfail nft_old_owner_denied 'Error\(Contract, #44\)' "$ALICE" "$CONTROLLER" -- withdraw --caller "$ALICE_ADDR" --account_id "$acct" --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1)" --to null || return 1
    view nft_global_enumeration "$POSITION_NFT" -- get_token_id --index 0 >/dev/null || return 1
    local bob_pre
    bob_pre=$(balance "$USDC_SAC" "$BOB_ADDR") || return 1
    inv nft_bob_withdraw "$BOB" "$CONTROLLER" -- withdraw --caller "$BOB_ADDR" --account_id "$acct" --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1000000)" --to null >/dev/null || return 1
    assert_delta nft_new_owner_receives "$bob_pre" "$(balance "$USDC_SAC" "$BOB_ADDR")" 1000000 || return 1
    inv nft_approve "$BOB" "$POSITION_NFT" -- approve --approver "$BOB_ADDR" --approved "$ALICE_ADDR" --token_id "$acct" --live_until_ledger "$until" >/dev/null || return 1
    assert_view_eq_at "$POSITION_NFT" nft_approved "$ALICE_ADDR" get_approved --token_id "$acct" || return 1
    inv nft_transfer_from "$ALICE" "$POSITION_NFT" -- transfer_from --spender "$ALICE_ADDR" --from "$BOB_ADDR" --to "$ALICE_ADDR" --token_id "$acct" >/dev/null || return 1
    assert_view_eq_at "$POSITION_NFT" nft_approval_cleared null get_approved --token_id "$acct" || return 1
    nft_assert_enumeration nft_return_enumeration "$ALICE_ADDR" "$acct" true || return 1
    inv nft_approve_all "$ALICE" "$POSITION_NFT" -- approve_for_all --owner "$ALICE_ADDR" --operator "$BOB_ADDR" --live_until_ledger "$until" >/dev/null || return 1
    assert_view_eq_at "$POSITION_NFT" nft_all_approved true is_approved_for_all --owner "$ALICE_ADDR" --operator "$BOB_ADDR" || return 1
    inv nft_revoke_all "$ALICE" "$POSITION_NFT" -- approve_for_all --owner "$ALICE_ADDR" --operator "$BOB_ADDR" --live_until_ledger 0 >/dev/null || return 1
    assert_view_eq_at "$POSITION_NFT" nft_all_revoked false is_approved_for_all --owner "$ALICE_ADDR" --operator "$BOB_ADDR" || return 1
    xfail nft_revoked_transfer 'Error\(Contract, #202\)' "$BOB" "$POSITION_NFT" -- transfer_from --spender "$BOB_ADDR" --from "$ALICE_ADDR" --to "$BOB_ADDR" --token_id "$acct" || return 1
    view nft_owner_enumeration "$POSITION_NFT" -- get_owner_token_id --owner "$ALICE_ADDR" --index 0 >/dev/null || return 1
    view nft_owner_balance "$POSITION_NFT" -- balance --account "$ALICE_ADDR" >/dev/null || return 1
    inv nft_new_owner_withdraw "$ALICE" "$CONTROLLER" -- withdraw --caller "$ALICE_ADDR" --account_id "$acct" --withdrawals "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 0)" --to null >/dev/null || return 1
    assert_bool_view nft_burned_account false account_exists --account_id "$acct" || return 1
    assert_view_eq_at "$POSITION_NFT" nft_burn_total "$total_before" total_supply || return 1
    nft_assert_enumeration nft_burn_enumeration "$ALICE_ADDR" "$acct" false || return 1
    xfail nft_burned_metadata 'Error\(Contract, #200\)' "$ALICE" "$POSITION_NFT" -- token_uri --token_id "$acct" || return 1
    xfail nft_burned_owner 'Error\(Contract, #200\)' "$ALICE" "$POSITION_NFT" -- owner_of --token_id "$acct"
}
