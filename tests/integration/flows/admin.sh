flow_admin() {
    phase admin

    inv admin_pause "$ADMIN" "$CONTROLLER" -- pause >/dev/null
    xfail paused_supply 'Error\(Contract, #1000\)' "$ALICE" "$CONTROLLER" -- supply \
        --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 1000000000)"
    inv admin_unpause "$ADMIN" "$CONTROLLER" -- unpause >/dev/null
    xfail unpause_when_live 'Error\(Contract, #1001\)' "$ADMIN" "$CONTROLLER" -- unpause

    inv set_position_limits "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":5,"max_borrow_positions":5}' >/dev/null

    inv update_pool_params "$ADMIN" "$CONTROLLER" -- upgrade_liquidity_pool_params \
        --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$EURC_SAC")" \
        --params "$(market_params_json "$EURC_SAC" 7 | jq -c '{
            max_borrow_rate, base_borrow_rate, slope1, slope2, slope3,
            mid_utilization, optimal_utilization, max_utilization,
            reserve_factor: 1500, is_flashloanable, flashloan_fee
        }')" >/dev/null

    inv edit_asset_config_admin "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$EURC_SAC" "$PRIMARY_SPOKE_ID" true true 6500 7000 900)" >/dev/null
    assert_market_field market_cfg_ltv "$EURC_SAC" loan_to_value 6500
    assert_market_field market_cfg_thr "$EURC_SAC" liquidation_threshold 7000
    assert_market_field market_cfg_bonus "$EURC_SAC" liquidation_bonus 900

    xfail asset_cfg_bad_bounds 'Error\(Contract, #113\)' "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$EURC_SAC" "$PRIMARY_SPOKE_ID" true true 9000 7000 900)"

    local tol_bands eurc_key
    eurc_key=$(price_key_token "$EURC_SAC")
    tol_bands=$(view oracle_tol_resolve "$GOVERNANCE" -- resolve_oracle_tolerance \
        --tolerance 300)
    inv set_tolerance "$ADMIN" "$PRICE_AGGREGATOR" -- set_tolerance \
        --key "$eurc_key" --tolerance "$tol_bands" >/dev/null

    xfail oracle_tol_owner_guard "Missing signing key for account $ADMIN_ADDR" "$ALICE" "$PRICE_AGGREGATOR" -- set_tolerance \
        --key "$eurc_key" --tolerance "$tol_bands"

    flow_price_aggregator_extra "$eurc_key"

    inv update_indexes "$ADMIN" "$CONTROLLER" -- update_indexes \
        --caller "$ADMIN_ADDR" --assets "$(hub_vec "$PRIMARY_HUB_ID" "$XLM_SAC" "$USDC_SAC" "$EURC_SAC")" >/dev/null
    inv update_indexes_alice "$ALICE" "$CONTROLLER" -- update_indexes \
        --caller "$ALICE_ADDR" --assets "$(hub_vec "$PRIMARY_HUB_ID" "$XLM_SAC")" >/dev/null

    inv update_account_threshold "$ADMIN" "$CONTROLLER" -- update_account_threshold \
        --caller "$ADMIN_ADDR" --has_risks false \
        --account_ids "[${ADMIN_ACCT:-1}]" >/dev/null
    inv update_account_threshold "$ALICE" "$CONTROLLER" -- update_account_threshold \
        --caller "$ALICE_ADDR" --has_risks false --account_ids "[${ADMIN_ACCT:-1}]" >/dev/null

    local pool_rev_before
    pool_rev_before=$(_view_pool_int pool_revenue_pre get_revenue --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    inv claim_revenue "$ADMIN" "$CONTROLLER" -- claim_revenue \
        --caller "$ADMIN_ADDR" --assets "$(hub_vec "$PRIMARY_HUB_ID" "$USDC_SAC")" >/dev/null
    assert_pool_revenue_decreased pool_revenue_post "$USDC_SAC" "${pool_rev_before:-0}"
    # claim_revenue is permissionless — anyone may trigger the sweep — but it
    # must pay out to the configured accumulator, not the caller. ALICE calling
    # it right after ADMIN's sweep must therefore move nothing.
    local rev_before_alice rev_after_alice
    rev_before_alice=$(_view_pool_int pool_revenue_pre_alice get_revenue --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    inv claim_revenue "$ALICE" "$CONTROLLER" -- claim_revenue \
        --caller "$ALICE_ADDR" --assets "$(hub_vec "$PRIMARY_HUB_ID" "$USDC_SAC")" >/dev/null
    rev_after_alice=$(_view_pool_int pool_revenue_post_alice get_revenue --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")")
    if _uint_le "$rev_after_alice" "${rev_before_alice:-0}"; then
        record claim_revenue_permissionless_safe ok claim_revenue "" "" "" "" "" "$rev_before_alice -> $rev_after_alice"
    else
        _assert_fail claim_revenue_permissionless_safe "revenue rose $rev_before_alice -> $rev_after_alice on a non-admin claim"
    fi
    view pool_rates_view "$POOL" -- get_borrow_rate --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" >/dev/null
    view pool_util_view "$POOL" -- get_utilisation --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")" >/dev/null

    # The remaining pool read surface. These back the accounting the controller
    # reports, so a non-negative, well-formed answer for a live market is the
    # property worth pinning — a panicking or absent getter would break every
    # consumer reading pool state.
    local usdc_hub
    usdc_hub=$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")
    assert_int_view_at_nonneg pool_reserves_view "$POOL" get_reserves --hub_asset "$usdc_hub"
    assert_int_view_at_nonneg pool_supplied_view "$POOL" get_supplied_amount --hub_asset "$usdc_hub"
    assert_int_view_at_nonneg pool_borrowed_view "$POOL" get_borrowed_amount --hub_asset "$usdc_hub"
    assert_int_view_at_nonneg pool_deposit_rate_view "$POOL" get_deposit_rate --hub_asset "$usdc_hub"
    assert_int_view_at_nonneg pool_delta_time_view "$POOL" get_delta_time --hub_asset "$usdc_hub"

    local tmp_cat
    tmp_cat=$(inv spoke_tmp_add "$ADMIN" "$CONTROLLER" -- add_spoke | tr -d '"')
    inv spoke_tmp_add_asset "$ADMIN" "$CONTROLLER" -- add_asset_to_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$EURC_SAC" "$tmp_cat" true true 8000 8500 300)" >/dev/null

    xfail spoke_bad_bounds 'Error\(Contract, #113\)' "$ADMIN" "$CONTROLLER" -- add_asset_to_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$EURC_SAC" "$tmp_cat" true true 8600 8500 300)"
    inv spoke_tmp_edit_asset "$ADMIN" "$CONTROLLER" -- edit_asset_in_spoke \
        --input "$(spoke_args "$PRIMARY_HUB_ID" "$EURC_SAC" "$tmp_cat" true false 8100 8600 250)" >/dev/null
inv spoke_tmp_remove_asset "$ADMIN" "$CONTROLLER" -- remove_asset_from_spoke \
--hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$EURC_SAC")" --spoke_id "$tmp_cat" >/dev/null
    inv spoke_tmp_deprecate "$ADMIN" "$CONTROLLER" -- remove_spoke --id "$tmp_cat" >/dev/null
    xfail spoke_deprecated_supply 'Error\(Contract, #301\)' "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$tmp_cat" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 1000000000)"

    local bob_minb_acct
    bob_minb_acct=$(inv_create minb_supply "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 5000000000)" | tr -d '"')
    inv minb_set_high "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd \
        --floor_wad 1000000000000000000000000000000000 >/dev/null
    assert_int_view_eq minb_read_high 1000000000000000000000000000000000 get_min_borrow_collateral_usd
    xfail minb_borrow_blocked 'Error\(Contract, #126\)' "$BOB" "$CONTROLLER" -- borrow \
        --caller "$BOB_ADDR" --account_id "$bob_minb_acct" \
        --borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1000000)" --to null
    inv minb_reset "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd --floor_wad 0 >/dev/null
    assert_int_view_eq minb_read_zero 0 get_min_borrow_collateral_usd
    xfail minb_negative 'Error\(Contract, #116\)' "$ADMIN" "$CONTROLLER" -- set_min_borrow_collateral_usd \
--floor_wad=-1

view pool_address_view "$CONTROLLER" -- get_pool_address >/dev/null
view market_index_xlm "$CONTROLLER" -- get_market_index \
--hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$XLM_SAC")" >/dev/null
view total_borrow_bob_minb "$CONTROLLER" -- get_total_borrow_usd \
--account_id "$bob_minb_acct" >/dev/null
inv manager_activate_alice "$ADMIN" "$CONTROLLER" -- set_position_manager \
--manager "$ALICE_ADDR" --is_active true >/dev/null
inv delegate_add_alice "$BOB" "$CONTROLLER" -- add_delegate \
--caller "$BOB_ADDR" --account_id "$bob_minb_acct" --delegate "$ALICE_ADDR" >/dev/null
inv delegated_borrow_usdc "$ALICE" "$CONTROLLER" -- borrow \
--caller "$ALICE_ADDR" --account_id "$bob_minb_acct" \
--borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1000000)" --to null >/dev/null
inv delegate_remove_alice "$BOB" "$CONTROLLER" -- remove_delegate \
--caller "$BOB_ADDR" --account_id "$bob_minb_acct" --delegate "$ALICE_ADDR" >/dev/null
xfail delegated_borrow_removed 'Error\(Contract, #44\)' "$ALICE" "$CONTROLLER" -- borrow \
--caller "$ALICE_ADDR" --account_id "$bob_minb_acct" \
--borrows "$(pay_vec "$PRIMARY_HUB_ID" "$USDC_SAC" 1000000)" --to null
inv manager_deactivate_alice "$ADMIN" "$CONTROLLER" -- set_position_manager \
--manager "$ALICE_ADDR" --is_active false >/dev/null

local blend_pool blend_seeded
blend_pool=$(jq -r '.pools[0].address // empty' "$REPO_ROOT/configs/$NETWORK/blend.json")
if [ -n "$blend_pool" ] && [ "$blend_pool" != "null" ]; then
# The allowlist decides which external pools this protocol will migrate
# positions from, so each transition is asserted rather than read and
# discarded: an approve or revoke that returned successfully without moving the
# flag would have been invisible here.
view blend_pool_initial "$CONTROLLER" -- is_blend_pool_approved --pool "$blend_pool" >/dev/null
inv blend_pool_approve "$ADMIN" "$CONTROLLER" -- approve_blend_pool --pool "$blend_pool" >/dev/null
assert_bool_view blend_pool_true true is_blend_pool_approved --pool "$blend_pool"
inv blend_pool_revoke "$ADMIN" "$CONTROLLER" -- revoke_blend_pool --pool "$blend_pool" >/dev/null
assert_bool_view blend_pool_false false is_blend_pool_approved --pool "$blend_pool"
inv blend_pool_reapprove "$ADMIN" "$CONTROLLER" -- approve_blend_pool --pool "$blend_pool" >/dev/null
assert_bool_view blend_pool_reapproved true is_blend_pool_approved --pool "$blend_pool"
if [ "${BLEND_MIGRATION_LIVE:-0}" = "1" ]; then
# Full live edge coverage lives in tests/integration/scenarios/blend.sh
# (make integration-blend). This flag keeps a single happy-path migrate in
# the admin lane for CI that opts in.

local coll_amt supply_amt debt_amt debt_cap seed_requests coll_json supply_json debt_json migrate_acct
coll_amt="${BLEND_XLM_COLLATERAL_AMOUNT:-${BLEND_XLM_AMOUNT:-2000000000}}"
supply_amt="${BLEND_XLM_SUPPLY_AMOUNT:-500000000}"
debt_amt="${BLEND_XLM_DEBT_AMOUNT:-300000000}"
if [ "${debt_amt:-0}" -gt 0 ]; then
    debt_cap="${BLEND_XLM_DEBT_CAP:-$((debt_amt + debt_amt / 5))}"
else
    debt_cap=0
fi

if [ -n "${BLEND_SEED_REQUESTS_JSON:-}" ]; then
    seed_requests="$BLEND_SEED_REQUESTS_JSON"
else

    seed_requests="[{\"request_type\":2,\"address\":\"$XLM_SAC\",\"amount\":\"$coll_amt\"}"
    [ "${supply_amt:-0}" -gt 0 ] && \
        seed_requests+=",{\"request_type\":0,\"address\":\"$XLM_SAC\",\"amount\":\"$supply_amt\"}"
    [ "${debt_amt:-0}" -gt 0 ] && \
        seed_requests+=",{\"request_type\":4,\"address\":\"$XLM_SAC\",\"amount\":\"$debt_amt\"}"
    seed_requests+="]"
fi
inv blend_seed_xlm_positions "$ALICE" "$blend_pool" -- submit \
    --from "$ALICE_ADDR" --spender "$ALICE_ADDR" --to "$ALICE_ADDR" \
    --requests "$seed_requests" >/dev/null
# The migration below reads from this position, so an empty seed would make the
# whole migration test vacuously pass against nothing.
blend_seeded=$(view blend_position_seeded "$blend_pool" -- get_positions --address "$ALICE_ADDR")
if [ -n "$blend_seeded" ] && [ "$blend_seeded" != "null" ] && [ "$blend_seeded" != "{}" ]; then
record blend_position_nonempty ok get_positions "" "" "" "" "" "seeded"
else
_assert_fail blend_position_nonempty "blend position empty after seeding; migration would test nothing: $(head -c 120 <<<"$blend_seeded")"
fi

coll_json="${BLEND_MIGRATE_COLLATERAL_ASSETS_JSON:-[\"$XLM_SAC\"]}"
if [ -n "${BLEND_MIGRATE_SUPPLY_ASSETS_JSON:-}" ]; then
    supply_json="$BLEND_MIGRATE_SUPPLY_ASSETS_JSON"
elif [ "${supply_amt:-0}" -gt 0 ]; then
    supply_json="[\"$XLM_SAC\"]"
else
    supply_json="[]"
fi
if [ -n "${BLEND_MIGRATE_DEBT_CAPS_JSON:-}" ]; then
    debt_json="$BLEND_MIGRATE_DEBT_CAPS_JSON"
elif [ "${debt_amt:-0}" -gt 0 ]; then
    debt_json="[[\"$XLM_SAC\",\"$debt_cap\"]]"
else
    debt_json="[]"
fi

migrate_acct=$(inv_create migrate_blend_live "$ALICE" "$CONTROLLER" -- migrate_from_blend \
    --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" --hub_id "$PRIMARY_HUB_ID" \
    --blend_pool "$blend_pool" \
    --collateral_assets "$coll_json" \
    --supply_assets "$supply_json" \
    --debt_caps "$debt_json" | tr -d '"')

view blend_position_swept "$blend_pool" -- get_positions --address "$ALICE_ADDR" >/dev/null

assert_bool_view migrate_blend_account_exists true account_exists --account_id "$migrate_acct"
assert_hf_at_least migrate_blend_hf "$migrate_acct" "$WAD"
if [ "${debt_amt:-0}" -gt 0 ]; then

    assert_borrow_at_least migrate_blend_debt_min "$migrate_acct" "$XLM_SAC" $((debt_amt * 95 / 100))
    assert_borrow_at_most migrate_blend_debt_max "$migrate_acct" "$XLM_SAC" $((debt_amt * 105 / 100))
    assert_borrow_at_most migrate_blend_debt_below_cap "$migrate_acct" "$XLM_SAC" $((debt_cap - 1))
fi
else
record migrate_blend_live environment-blocked migrate_from_blend "" "" "" "" "" \
    "set BLEND_MIGRATION_LIVE=1 (seeds XLM coll+supply+debt on Blend, migrates with refund buffer)"
fi
fi

    local xlm_sec_band
    xlm_sec_band=$(reflector_band XLM) || { log "XLM live price unavailable; cannot calibrate secondary sanity band"; return 1; }
    create_market XLM_SECONDARY "$SECONDARY_HUB_ID" "$XLM_SAC" 7 \
        "$(oracle_cfg_reflector XLM $xlm_sec_band)" \
        "$(asset_config_json 7000 7500 1000)"
    view market_index_secondary_xlm "$CONTROLLER" -- get_market_index \
        --hub_asset "$(hub_key "$SECONDARY_HUB_ID" "$XLM_SAC")" >/dev/null
    local secondary_acct
    secondary_acct=$(inv_create secondary_hub_supply "$CAROL" "$CONTROLLER" -- supply \
        --caller "$CAROL_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$SECONDARY_HUB_ID" "$XLM_SAC" 1000000000)" | tr -d '"')
    assert_bool_view secondary_account_exists true account_exists --account_id "$secondary_acct"

}

flow_admin_upgrade() {
    phase admin_upgrade
    local ctrl_hash out_f="$LOG_DIR/upload_ctrl.out" err_f="$LOG_DIR/upload_ctrl.err"
    stellar contract upload --wasm "$WASM_DIR/controller.wasm" \
        --source "$ADMIN" "${NET_ARGS[@]}" >"$out_f" 2>"$err_f" || true
    ctrl_hash=$(sanitize_output "$out_f")
    if [ -n "$ctrl_hash" ]; then
        record upload_controller_wasm ok upload \
            "$(extract_signing_hash "$err_f")" \
            "" "" "" "" "$ctrl_hash"
 inv pool_upgrade "$ADMIN" "$CONTROLLER" -- upgrade_pool --new_wasm_hash "$POOL_HASH" >/dev/null
 view pool_address_after_pool_upgrade "$CONTROLLER" -- get_pool_address >/dev/null
 inv controller_upgrade "$ADMIN" "$CONTROLLER" -- upgrade --new_wasm_hash "$ctrl_hash" >/dev/null

        xfail upgraded_paused_gate 'Error\(Contract, #1000\)' "$ALICE" "$CONTROLLER" -- supply \
            --caller "$ALICE_ADDR" --account_id 0 --spoke_id "$PRIMARY_SPOKE_ID" \
            --assets "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 1000000000)"
        local ver
        ver=$(view app_version_view "$CONTROLLER" -- get_app_version | tr -d '"')
        inv controller_migrate "$ADMIN" "$CONTROLLER" -- migrate --new_version $((ver + 1)) >/dev/null
        inv unpause_after_upgrade "$ADMIN" "$CONTROLLER" -- unpause >/dev/null
    fi

    # Satellite upgrades, same-hash like the pool/controller legs above: the
    # position NFT is controller-owned from deploy, so this proves the
    # owner-gated controller entrypoint plus the NFT's controller-only upgrade
    # auth without changing behavior. Token ids are account ids, so the owner
    # read-back doubles as the post-upgrade liveness check.
    inv nft_upgrade_via_controller "$ADMIN" "$CONTROLLER" -- upgrade_position_nft \
        --new_wasm_hash "$NFT_HASH" >/dev/null
    assert_view_eq_at "$POSITION_NFT" nft_owner_after_upgrade "$ADMIN_ADDR" \
        owner_of --token_id "${ADMIN_ACCT:-1}"

    # Permissionless TTL renew on a live token, then the designed failure on a
    # token that was never minted.
    inv nft_renew "$ALICE" "$POSITION_NFT" -- renew --token_id "${ADMIN_ACCT:-1}" >/dev/null
    xfail nft_renew_missing 'Error\(Contract, #200\)' "$ALICE" "$POSITION_NFT" -- renew --token_id 4000000000

    local ledger
    ledger=$(curl -s -m 30 -X POST "$RPC_URL" -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' | jq -r '.result.sequence')
    # Ownership is the root of every #[only_owner] gate on the controller, so
    # each leg asserts who actually holds it. A transfer that succeeded without
    # moving ownership — or an accept that left the old owner in place — would
    # otherwise pass unnoticed.
    # The controller exposes no get_owner, so ownership is asserted through its
    # effect: who can still drive an #[only_owner] entry point. set_position_limits
    # is the probe, always re-set to the same valid value so the probe itself
    # changes nothing.
    local limits='{"max_supply_positions":5,"max_borrow_positions":5}'

    inv ownership_transfer "$ADMIN" "$CONTROLLER" -- transfer_ownership \
        --new_owner "$CAROL_ADDR" --live_until_ledger $((ledger + 1000)) >/dev/null
    # A pending transfer must not hand over control on its own — ADMIN still owns
    # it until CAROL accepts.
    inv ownership_admin_still_owner "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits "$limits" >/dev/null

    inv ownership_accept "$CAROL" "$CONTROLLER" -- accept_ownership >/dev/null
    # Ownership really moved: ADMIN is now locked out, CAROL is in.
    xfail ownership_admin_locked_out "Missing signing key for account $CAROL_ADDR" "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits "$limits"
    inv ownership_carol_now_owner "$CAROL" "$CONTROLLER" -- set_position_limits \
        --limits "$limits" >/dev/null

    inv ownership_transfer_back "$CAROL" "$CONTROLLER" -- transfer_ownership \
        --new_owner "$ADMIN_ADDR" --live_until_ledger $((ledger + 1000)) >/dev/null
    inv ownership_accept_back "$ADMIN" "$CONTROLLER" -- accept_ownership >/dev/null
    # Restored, or every later owner-gated step in the suite would be testing the
    # wrong signer.
    inv ownership_admin_restored "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits "$limits" >/dev/null
}

# The two price-aggregator entry points the tolerance work above never reached.
#
# `seed_oracle` and `remove_oracle` are deliberately NOT covered: they sit
# behind #[cfg(any(test, feature = "testing"))] and are absent from the wasm the
# harness deploys, since integration-wasm builds without the `testing` feature.
# Calling them on testnet would fail because they do not exist there.
flow_price_aggregator_extra() {
    local key="${1:-}"
    [ -n "$key" ] || return 0

    # Reads first, band last. Narrowing the sanity band changes what these reads
    # are allowed to return, so setting it up front made every later read of the
    # same key fail with #223 SanityBoundViolated — the band test breaking the
    # data it was set on.

    # price_spread returns the (low, high) pair behind a resolved price: what a
    # caller inspects to see how far the two legs disagree.
    local spread
    spread=$(view pa_price_spread "$PRICE_AGGREGATOR" -- price_spread --key "$key")
    if [ "$(jq -r 'if type == "array" then length else 0 end' <<<"$spread" 2>/dev/null)" = "2" ]; then
        record pa_price_spread_shape ok price_spread "" "" "" "" "" "$(jq -c . <<<"$spread")"
    else
        _assert_fail pa_price_spread_shape "price_spread did not return a 2-tuple: $spread"
    fi

    # Ownership gates every setter below, so assert the identity.
    assert_view_eq_at "$PRICE_AGGREGATOR" pa_get_owner "$ADMIN_ADDR" get_owner

    # `oracle` must return the config set_oracle registered for this key. An
    # empty answer would mean the setters wrote somewhere else.
    local orc
    orc=$(view pa_oracle "$PRICE_AGGREGATOR" -- oracle --key "$key")
    if [ -n "$orc" ] && [ "$(jq -r 'if type=="object" then "obj" else . end' <<<"$orc" 2>/dev/null)" = "obj" ]; then
        record pa_oracle_registered ok oracle "" "" "" "" "" "config present for key"
    else
        _assert_fail pa_oracle_registered "oracle returned no config for a registered key: $(head -c 160 <<<"$orc")"
    fi

    # prices/quotes are keyed maps: one key in must yield an entry for that same
    # key, or a consumer silently reads another asset's price.
    local keys_json px qt
    keys_json=$(jq -nc --argjson k "$key" '[$k]')
    px=$(view pa_prices "$PRICE_AGGREGATOR" -- prices --keys "$keys_json")
    qt=$(view pa_quotes "$PRICE_AGGREGATOR" -- quotes --keys "$keys_json")
    if [ "$(jq -r 'if type=="object" then (keys|length) elif type=="array" then length else 0 end' <<<"$px" 2>/dev/null)" -ge 1 ]; then
        record pa_prices_keyed ok prices "" "" "" "" "" "1 key -> 1 entry"
    else
        _assert_fail pa_prices_keyed "prices returned no entry for the requested key: $(head -c 160 <<<"$px")"
    fi
    if [ "$(jq -r 'if type=="object" then (keys|length) elif type=="array" then length else 0 end' <<<"$qt" 2>/dev/null)" -ge 1 ]; then
        record pa_quotes_keyed ok quotes "" "" "" "" "" "1 key -> 1 entry"
    else
        _assert_fail pa_quotes_keyed "quotes returned no entry for the requested key: $(head -c 160 <<<"$qt")"
    fi

    # The sanity band is the outer bound on any price the protocol will accept,
    # so it is owner-only. Width is capped for a single-source feed:
    # ceil((max-min)*10000/(max+min)) <= MAX_SINGLE_SOURCE_SANITY_BAND_BPS
    # (1000); +/-8% is 800 bps. Centred on the asset's *current* price rather
    # than on parity, so the band contains the price it is guarding.
    local band_min band_max band_px cur_min cur_max
    band_px=$(jq -r '[.. | objects | select(has("price")) | .price] | first // empty' <<<"$px" 2>/dev/null)
    [[ "$band_px" =~ ^[0-9]+$ ]] || band_px="$WAD"
    band_min=$((band_px / 100 * 92))
    band_max=$((band_px / 100 * 108))
    # set_sanity_band is a one-way ratchet: the new band must sit inside the
    # registered one or it reverts with SanityBandMustTighten (#227). A live
    # feed drifts between registration and here, so clamp into the registered
    # band -- the call must narrow, never widen. Clamping keeps the live price
    # inside the result (any healthy oracle already has reg_min <= px <= reg_max)
    # and can only lower the width in bps, so both band-width bounds still hold.
    cur_min=$(jq -r '.min_sanity_price_wad // empty' <<<"$orc" 2>/dev/null)
    cur_max=$(jq -r '.max_sanity_price_wad // empty' <<<"$orc" 2>/dev/null)
    if [[ "$cur_min" =~ ^[0-9]+$ ]] && [ "$cur_min" -gt "$band_min" ]; then band_min="$cur_min"; fi
    if [[ "$cur_max" =~ ^[0-9]+$ ]] && [ "$cur_max" -lt "$band_max" ]; then band_max="$cur_max"; fi
    inv pa_set_sanity_band "$ADMIN" "$PRICE_AGGREGATOR" -- set_sanity_band \
        --key "$key" --min_wad "$band_min" --max_wad "$band_max" >/dev/null

    xfail pa_set_sanity_band_owner_guard "Missing signing key for account $ADMIN_ADDR" "$ALICE" "$PRICE_AGGREGATOR" -- set_sanity_band \
        --key "$key" --min_wad "$band_min" --max_wad "$band_max"
}

# The pool's own surface, which the harness only ever reached through the
# controller. Two properties are worth pinning here.
#
# First, the pool's privileged entry points are #[only_owner] and the pool's
# owner is the CONTROLLER, not ADMIN. So a direct call from ADMIN must be
# rejected: that is what stops anyone minting markets, rewriting rate models or
# seizing positions behind the controller's back. Asserting the rejection is the
# direct assertion these endpoints can carry — their happy path is only
# reachable through the controller, and is already covered there.
#
# Second, the two read endpoints back hub-side valuation, so they are asserted
# on shape, not merely on not-reverting.
flow_pool_surface() {
    phase pool_surface
    local hub_asset
    hub_asset=$(hub_key "$PRIMARY_HUB_ID" "$USDC_SAC")

    # --- privileged: must reject a non-owner ---
    # require_auth never reverts in recording mode, so the only proof of the
    # owner gate is the CLI failing to sign for the pool's owner (the
    # controller). Every argument must therefore pass body validation: an
    # unused hub id keeps create_market clear of AssetAlreadySupported.
    xfail pool_create_market_not_owner "Missing signing key for account $CONTROLLER" "$ADMIN" "$POOL" -- create_market \
        --hub_id 4242 --params "$(market_params_json "$USDC_SAC" 7)"
    # InterestRateModel carries is_flashloanable and flashloan_fee too; omitting
    # them makes the CLI reject the argument before auth is ever checked, which
    # would pass the xfail for the wrong reason.
    xfail pool_update_params_not_owner "Missing signing key for account $CONTROLLER" "$ADMIN" "$POOL" -- update_params \
        --hub_asset "$hub_asset" --model "$(market_params_json "$USDC_SAC" 7 | jq -c '{
            max_borrow_rate, base_borrow_rate, slope1, slope2, slope3,
            mid_utilization, optimal_utilization, max_utilization,
            reserve_factor, is_flashloanable, flashloan_fee
        }')"
    xfail pool_seize_positions_not_owner "Missing signing key for account $CONTROLLER" "$ADMIN" "$POOL" -- seize_positions \
        --entries '[]'
    xfail pool_net_settle_not_owner "Missing signing key for account $CONTROLLER" "$ADMIN" "$POOL" -- net_settle \
        --entry "$(jq -nc --argjson h "$PRIMARY_HUB_ID" --arg a "$USDC_SAC" \
            '{hub_asset:{hub_id:$h,asset:$a},amount:"0",
              supply_position:{scaled_amount:"0"},debt_position:{scaled_amount:"0"}}')"
    # amount 1 (not 0) so the body clears AmountMustBePositive and auth is the
    # only remaining failure.
    xfail pool_create_strategy_not_owner "Missing signing key for account $CONTROLLER" "$ADMIN" "$POOL" -- create_strategy \
        --receiver "$ADMIN_ADDR" --charge_fee false \
        --action "$(jq -nc --argjson h "$PRIMARY_HUB_ID" --arg a "$USDC_SAC" \
            '{position:{scaled_amount:"0"},amount:"1",hub_asset:{hub_id:$h,asset:$a}}')"

    # --- reads: assert shape, since these back hub-side valuation ---
    local sync idx
    sync=$(view pool_get_sync_data "$POOL" -- get_sync_data --hub_asset "$hub_asset")
    if [ "$(jq -r 'has("params") and has("state")' <<<"$sync" 2>/dev/null)" = "true" ]; then
        record pool_sync_data_shape ok get_sync_data "" "" "" "" "" "params+state present"
    else
        _assert_fail pool_sync_data_shape "get_sync_data missing params/state: $(head -c 160 <<<"$sync")"
    fi

    # One key in, one index out — a mismatch would silently misalign the hub's
    # per-asset valuations.
    idx=$(view pool_get_bulk_indexes "$POOL" -- get_bulk_indexes \
        --hub_assets "$(jq -nc --argjson h "$PRIMARY_HUB_ID" --arg a "$USDC_SAC" '[{hub_id:$h,asset:$a}]')")
    if [ "$(jq -r 'if type=="array" then length else 0 end' <<<"$idx" 2>/dev/null)" = "1" ]; then
        record pool_bulk_indexes_shape ok get_bulk_indexes "" "" "" "" "" "1 key -> 1 index"
    else
        _assert_fail pool_bulk_indexes_shape "get_bulk_indexes returned $(jq -c 'length' <<<"$idx" 2>/dev/null) entries for 1 key"
    fi
}

# 2026-09 gap hunt, GH-16 and GH-17. The seed account holds XLM and USDC and
# lives until teardown, so it is the fixture for both: a limit lowered below
# its position count must keep top-ups (and only top-ups) open, and the pool
# and the controller must be refused as borrow and withdraw recipients before
# any transfer. Every check runs against ADMIN_ACCT and restores the limits.
flow_gap_hunt_admin() {
    phase gap_hunt_admin
    [ -n "${ADMIN_ACCT:-}" ] || die gap_hunt_admin "ADMIN_ACCT missing; flow_seed_liquidity must run first"
    assert_bool_view gh_seed_account_live true account_exists --account_id "$ADMIN_ACCT" \
        || die gap_hunt_admin "seed account $ADMIN_ACCT is gone (post-teardown resume?); rerun from lifecycle"
    local xlm_leg
    xlm_leg=$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 10000000)
    # `to` is Option<Address>: the CLI wants JSON, so an address goes as a
    # JSON string and None as null.
    local to_pool to_ctrl to_bob
    to_pool="\"$POOL\""
    to_ctrl="\"$CONTROLLER\""
    to_bob="\"$BOB_ADDR\""

    # --- GH-17: recipients inside the protocol are refused with #412 ---
    xfail gh17_borrow_to_pool 'Error\(Contract, #412\)' "$ADMIN" "$CONTROLLER" -- borrow \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --borrows "$xlm_leg" --to "$to_pool"
    xfail gh17_borrow_to_controller 'Error\(Contract, #412\)' "$ADMIN" "$CONTROLLER" -- borrow \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --borrows "$xlm_leg" --to "$to_ctrl"
    xfail gh17_withdraw_to_pool 'Error\(Contract, #412\)' "$ADMIN" "$CONTROLLER" -- withdraw \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --withdrawals "$xlm_leg" --to "$to_pool"
    xfail gh17_withdraw_to_controller 'Error\(Contract, #412\)' "$ADMIN" "$CONTROLLER" -- withdraw \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --withdrawals "$xlm_leg" --to "$to_ctrl"
    # An outside recipient still works: 1 XLM to BOB, then repaid by ADMIN.
    inv gh17_borrow_to_bob "$ADMIN" "$CONTROLLER" -- borrow \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --borrows "$xlm_leg" --to "$to_bob" >/dev/null
    inv gh17_repay_after_borrow "$ADMIN" "$CONTROLLER" -- repay \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" \
        --payments "$(pay_vec "$PRIMARY_HUB_ID" "$XLM_SAC" 20000000)" >/dev/null

    # --- GH-16: a limit below the account's count keeps top-ups open ---
    # The seed account holds two supply positions (XLM, USDC); a limit of one
    # is below that. Topping up XLM opens no slot and passes; EURC would open
    # a slot and is refused with #109 before any token moves.
    inv gh16_lower_limits "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":1,"max_borrow_positions":1}' >/dev/null
    inv gh16_topup_held_asset_over_limit "$ADMIN" "$CONTROLLER" -- supply \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$xlm_leg" >/dev/null
    inv gh16_third_party_topup_over_limit "$BOB" "$CONTROLLER" -- supply \
        --caller "$BOB_ADDR" --account_id "$ADMIN_ACCT" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$xlm_leg" >/dev/null
    xfail gh16_new_slot_over_limit 'Error\(Contract, #109\)' "$ADMIN" "$CONTROLLER" -- supply \
        --caller "$ADMIN_ADDR" --account_id "$ADMIN_ACCT" --spoke_id "$PRIMARY_SPOKE_ID" \
        --assets "$(pay_vec "$PRIMARY_HUB_ID" "$EURC_SAC" 10000000)"
    inv gh16_restore_limits "$ADMIN" "$CONTROLLER" -- set_position_limits \
        --limits '{"max_supply_positions":5,"max_borrow_positions":5}' >/dev/null
}
