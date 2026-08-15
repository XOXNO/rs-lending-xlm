: "${SA_FEE_CAP:=1000}"

# The swap-aggregator's owner-only surface: fees, whitelist, referrals, fee
# balances and sweeps. None of it was reachable before, because the harness only
# ever talked to the shared `$AGGREGATOR` from configs/networks.json, which this
# run does not own. `$OWNED_AGGREGATOR` is a throwaway instance deployed by
# deploy_protocol with ADMIN as owner.
#
# Ordering: renounce_ownership is irreversible, so it runs last and everything
# owner-only must precede it.
flow_swap_aggregator_admin() {
    phase swap_agg_admin
    if [ -z "${OWNED_AGGREGATOR:-}" ]; then
        log "swap_agg_admin: no OWNED_AGGREGATOR, skipping"
        return 0
    fi
    local agg="$OWNED_AGGREGATOR"

    # --- static fee ---
    assert_view_eq_at "$agg" sa_fee_initial 0 static_fee_bps
    inv sa_set_fee "$ADMIN" "$agg" -- set_static_fee --fee_bps 50 >/dev/null
    assert_view_eq_at "$agg" sa_fee_after_set 50 static_fee_bps

    # The cap is the only thing standing between an owner and a 100% fee.
    xfail sa_fee_above_cap 'Error\(Contract, #21\)' "$ADMIN" "$agg" -- set_static_fee \
        --fee_bps $((SA_FEE_CAP + 1))
    # At the cap exactly it must still be accepted.
    inv sa_set_fee_at_cap "$ADMIN" "$agg" -- set_static_fee --fee_bps "$SA_FEE_CAP" >/dev/null
    assert_view_eq_at "$agg" sa_fee_at_cap "$SA_FEE_CAP" static_fee_bps
    inv sa_reset_fee "$ADMIN" "$agg" -- set_static_fee --fee_bps 0 >/dev/null

    # A non-owner must not be able to move the fee at all.
    xfail sa_fee_not_owner 'Missing signing key|Error\(Contract' "$BOB" "$agg" -- set_static_fee --fee_bps 10

    # --- whitelist ---
    # XLM_SAC, not one of the LIQ* assets: those are created by flow_liq_setup,
    # which only the `liq` lane runs. Referencing SAC_LIQA here aborted the whole
    # `agg` lane under `set -u` before governance ever started. XLM_SAC is set by
    # deploy_protocol, so it exists in every lane.
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
    if [ "$(jq -r '.fee_bps // empty' <<<"$ref")" = "40" ] \
        && [ "$(jq -r '.active // empty' <<<"$ref")" = "false" ]; then
        record sa_ref_updates_applied ok referral "" "" "" "" "" "fee=40 active=false"
    else
        _assert_fail sa_ref_updates_applied "referral not updated: $ref"
    fi
    inv sa_ref_reactivate "$ADMIN" "$agg" -- set_referral_active --id "$ref_id" --active true >/dev/null

    # Hand the referral to CAROL so the referral-owner-gated claim below is
    # exercised as CAROL, not as the contract owner.
    inv sa_ref_set_owner "$ADMIN" "$agg" -- set_referral_owner \
        --id "$ref_id" --new_owner "$CAROL_ADDR" >/dev/null
    ref=$(view sa_ref_after_owner "$agg" -- referral --id "$ref_id")
    if [ "$(jq -r '.owner // empty' <<<"$ref")" = "$CAROL_ADDR" ]; then
        record sa_ref_owner_moved ok referral "" "" "" "" "" "owner=$CAROL_ADDR"
    else
        _assert_fail sa_ref_owner_moved "referral owner not CAROL: $ref"
    fi

    # --- fee balances and claims ---
    # No swap has routed through this instance, so both balances are zero and
    # the claims are no-ops. That is the point: they must succeed rather than
    # revert on an empty balance, or a referral with no volume could never call.
    assert_view_eq_at "$agg" sa_admin_fee_zero 0 admin_fee_balance --token "$tok"
    assert_view_eq_at "$agg" sa_ref_fee_zero 0 referral_fee_balance --id "$ref_id" --token "$tok"

    inv sa_claim_admin_fees "$ADMIN" "$agg" -- claim_admin_fees \
        --recipient "$ADMIN_ADDR" --tokens "$(jq -nc --arg t "$tok" '[$t]')" >/dev/null
    inv sa_claim_referral_fees "$CAROL" "$agg" -- claim_referral_fees \
        --id "$ref_id" --tokens "$(jq -nc --arg t "$tok" '[$t]')" >/dev/null
    inv sa_sweep_balance "$ADMIN" "$agg" -- sweep_balance \
        --recipient "$ADMIN_ADDR" --tokens "$(jq -nc --arg t "$tok" '[$t]')" >/dev/null

    # --- ownership, last: irreversible ---
    inv sa_renounce "$ADMIN" "$agg" -- renounce_ownership >/dev/null
    # With no owner left, the owner-only surface must be permanently closed.
    xfail sa_owner_only_after_renounce 'Missing signing key|Error\(Contract' "$ADMIN" "$agg" -- set_static_fee --fee_bps 10
}
