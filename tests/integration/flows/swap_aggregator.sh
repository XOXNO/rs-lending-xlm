: "${SA_FEE_CAP:=1000}"

# Tests the swap-aggregator's owner calls: fee, whitelist, referrals, fee
# claims and sweep. The run does not own the shared `$AGGREGATOR` from
# configs/networks.json, so it uses `$OWNED_AGGREGATOR`, a throwaway instance
# that deploy_protocol deploys with ADMIN as owner.
flow_swap_aggregator_admin() {
    phase swap_agg_admin
    if [ -z "${OWNED_AGGREGATOR:-}" ]; then
        log "swap_agg_admin: no OWNED_AGGREGATOR, skipping"
        return 0
    fi
    local agg="$OWNED_AGGREGATOR"

    # Every setter below is owner-only. `admin` and `get_owner` must both
    # return ADMIN; `admin` panics with NotAdmin (#20) when no owner is set.
    assert_view_eq_at "$agg" sa_owner_initial "$ADMIN_ADDR" get_owner
    assert_view_eq_at "$agg" sa_admin_initial "$ADMIN_ADDR" admin

    # --- static fee ---
    assert_view_eq_at "$agg" sa_fee_initial 0 static_fee_bps
    inv sa_set_fee "$ADMIN" "$agg" -- set_static_fee --fee_bps 50 >/dev/null
    assert_view_eq_at "$agg" sa_fee_after_set 50 static_fee_bps

    # FEE_CAP (1000 BPS) bounds the owner's fee: above it reverts FeeTooHigh.
    xfail sa_fee_above_cap 'Error\(Contract, #21\)' "$ADMIN" "$agg" -- set_static_fee \
        --fee_bps $((SA_FEE_CAP + 1))
    # At the cap exactly it must still be accepted.
    inv sa_set_fee_at_cap "$ADMIN" "$agg" -- set_static_fee --fee_bps "$SA_FEE_CAP" >/dev/null
    assert_view_eq_at "$agg" sa_fee_at_cap "$SA_FEE_CAP" static_fee_bps
    inv sa_reset_fee "$ADMIN" "$agg" -- set_static_fee --fee_bps 0 >/dev/null

    # A non-owner cannot set the fee.
    xfail sa_fee_not_owner "Missing signing key for account $ADMIN_ADDR" "$BOB" "$agg" -- set_static_fee --fee_bps 10

    # --- whitelist ---
    # XLM_SAC, not a LIQ* asset: deploy_protocol sets XLM_SAC in every lane,
    # but only the `liq` lane runs flow_liq_setup. An unset SAC_LIQA aborts the
    # `agg` lane under `set -u`.
    local tok="$XLM_SAC"
    assert_view_eq_at "$agg" sa_wl_before false is_whitelisted --token "$tok"
    inv sa_wl_add "$ADMIN" "$agg" -- add_to_whitelist --token "$tok" >/dev/null
    assert_view_eq_at "$agg" sa_wl_after_add true is_whitelisted --token "$tok"

    local wl
    wl=$(view sa_wl_list "$agg" -- whitelisted_tokens)
    if jq -e --arg t "$tok" 'index($t)' >/dev/null 2>&1 <<<"$wl"; then
        record sa_wl_list_contains ok whitelisted_tokens "" "" "" "" "" "$tok listed"
    else
        _assert_fail sa_wl_list_contains "whitelisted_tokens missing $tok: $wl"
    fi

    # Idempotent: adding twice must not duplicate the entry.
    inv sa_wl_add_again "$ADMIN" "$agg" -- add_to_whitelist --token "$tok" >/dev/null
    local wl_count
    wl_count=$(view sa_wl_list_again "$agg" -- whitelisted_tokens | jq 'length')
    if [ "$wl_count" = "1" ]; then
        record sa_wl_no_duplicate ok whitelisted_tokens "" "" "" "" "" "len=1"
    else
        _assert_fail sa_wl_no_duplicate "whitelist length $wl_count after duplicate add; want 1"
    fi

    inv sa_wl_remove "$ADMIN" "$agg" -- remove_from_whitelist --token "$tok" >/dev/null
    assert_view_eq_at "$agg" sa_wl_after_remove false is_whitelisted --token "$tok"
    # Removing an absent token is a no-op, not an error.
    inv sa_wl_remove_absent "$ADMIN" "$agg" -- remove_from_whitelist --token "$tok" >/dev/null

    # --- referrals ---
    assert_view_eq_at "$agg" sa_ref_counter_initial 0 referral_counter
    local ref_id
    ref_id=$(inv sa_ref_add "$ADMIN" "$agg" -- add_referral \
        --owner "$BOB_ADDR" --fee_bps 25 | tr -d '"')
    if [ -z "$ref_id" ] || [ "$ref_id" = "0" ]; then
        _assert_fail sa_ref_id "add_referral returned '$ref_id'; want a non-zero id"
        return 0
    fi
    record sa_ref_id ok add_referral "" "" "" "" "" "id=$ref_id"
    assert_view_eq_at "$agg" sa_ref_counter_after 1 referral_counter

    local ref
    ref=$(view sa_ref_view "$agg" -- referral --id "$ref_id")
    if [ "$(jq -r '.fee_bps // empty' <<<"$ref")" = "25" ]; then
        record sa_ref_fee_stored ok referral "" "" "" "" "" "fee_bps=25"
    else
        _assert_fail sa_ref_fee_stored "referral fee_bps not 25: $ref"
    fi

    xfail sa_ref_fee_above_cap 'Error\(Contract, #21\)' "$ADMIN" "$agg" -- set_referral_fee \
        --id "$ref_id" --fee_bps $((SA_FEE_CAP + 1))
    xfail sa_ref_missing 'Error\(Contract, #22\)' "$ADMIN" "$agg" -- set_referral_fee \
        --id 999999 --fee_bps 10

    inv sa_ref_set_fee "$ADMIN" "$agg" -- set_referral_fee --id "$ref_id" --fee_bps 40 >/dev/null
    inv sa_ref_deactivate "$ADMIN" "$agg" -- set_referral_active --id "$ref_id" --active false >/dev/null
    ref=$(view sa_ref_after_updates "$agg" -- referral --id "$ref_id")
    # `.active` is read without `// empty`: jq's `//` treats `false` as absent,
    # so `.active // empty` yields nothing for exactly the value under test.
    if [ "$(jq -r '.fee_bps // empty' <<<"$ref")" = "40" ] \
        && [ "$(jq -r '.active' <<<"$ref")" = "false" ]; then
        record sa_ref_updates_applied ok referral "" "" "" "" "" "fee=40 active=false"
    else
        _assert_fail sa_ref_updates_applied "referral not updated: $ref"
    fi
    inv sa_ref_reactivate "$ADMIN" "$agg" -- set_referral_active --id "$ref_id" --active true >/dev/null

    # Moves the referral to CAROL, who then runs the claim below.
    # `claim_referral_fees` takes no auth and pays the stored owner.
    inv sa_ref_set_owner "$ADMIN" "$agg" -- set_referral_owner \
        --id "$ref_id" --new_owner "$CAROL_ADDR" >/dev/null
    ref=$(view sa_ref_after_owner "$agg" -- referral --id "$ref_id")
    if [ "$(jq -r '.owner // empty' <<<"$ref")" = "$CAROL_ADDR" ]; then
        record sa_ref_owner_moved ok referral "" "" "" "" "" "owner=$CAROL_ADDR"
    else
        _assert_fail sa_ref_owner_moved "referral owner not CAROL: $ref"
    fi

    # --- fee balances and claims ---
    # No swap routes through this instance, so both balances are zero. The
    # claims must succeed as no-ops on an empty balance, not revert.
    assert_view_eq_at "$agg" sa_admin_fee_zero 0 admin_fee_balance --token "$tok"
    assert_view_eq_at "$agg" sa_ref_fee_zero 0 referral_fee_balance --id "$ref_id" --token "$tok"

    inv sa_claim_admin_fees "$ADMIN" "$agg" -- claim_admin_fees \
        --recipient "$ADMIN_ADDR" --tokens "$(jq -nc --arg t "$tok" '[$t]')" >/dev/null
    inv sa_claim_referral_fees "$CAROL" "$agg" -- claim_referral_fees \
        --id "$ref_id" --tokens "$(jq -nc --arg t "$tok" '[$t]')" >/dev/null
    inv sa_sweep_balance "$ADMIN" "$agg" -- sweep_balance \
        --recipient "$ADMIN_ADDR" --tokens "$(jq -nc --arg t "$tok" '[$t]')" >/dev/null
}
