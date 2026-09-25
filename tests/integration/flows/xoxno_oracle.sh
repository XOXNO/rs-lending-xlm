# Live xoxno-oracle coverage: deploys with run wallets as the signer set,
# tests the admin calls with read-backs, drives threshold-gated median
# aggregation through real multi-signer submissions, checks the designed
# reverts (#NN = contracts/xoxno-oracle Error), and upgrades to the same hash
# with state kept. Self-contained: no controller wiring.

# Prices fit exactly in jq's integer range; accept CLI numeric/string encoding.
_xo_assert_json() {
    local label="$1" payload="$2"; shift 2
    if ! jq -e "$@" <<<"$payload" >/dev/null; then
        _assert_fail "$label" "oracle response differs from exact reference: $payload"
        return 1
    fi
    record "$label" ok assert "" "" "" "" "" "exact oracle response"
}

_xo_assert_aggregate() {
    local label="$1" payload="$2" price="$3" package="$4" mutation="$5" hash written
    hash=$(extract_signing_hash "$LOG_DIR/$mutation.err") || return 1
    written=$(jq -er '.result.createdAt | tonumber | select(. > 0)' "$LOG_DIR/$hash.receipt.json") \
        || { _assert_fail "$label" "confirmed mutation lacks ledger close time"; return 1; }
    _xo_assert_json "$label" "$payload" --argjson price "$price" --argjson package "$package" --argjson written "$((written * 1000))" \
        '(.price | tonumber) == $price and (.package_timestamp | tonumber) == $package and (.write_timestamp | tonumber) == $written'
}

xo_upgrade_snapshot() {
    local tag="$1" contract="$2" method value state='{}' feed
    for method in get_owner assets feeds base decimals resolution max_stale_seconds max_submission_age_seconds max_relative_skew_seconds max_cluster_spread_bps; do
        value=$(view "${tag}_$method" "$contract" -- "$method") || return 1
        state=$(jq -ncS --argjson s "$state" --arg k "$method" --argjson v "$value" '$s+{($k):$v}') || return 1
    done
    for feed in USDX XLMX; do
        value=$(view "${tag}_$feed" "$contract" -- read_price_history --feed_id "$feed" --limit 5) || return 1
        state=$(jq -ncS --argjson s "$state" --arg k "$feed" --argjson v "$value" '$s+{($k):$v}') || return 1
    done
    printf '%s\n' "$state"
}

flow_xoxno_oracle() {
    phase xoxno_oracle
    local wasm="$WASM_DIR/xoxno-oracle-adapter.wasm"
    [ -f "$wasm" ] || die xo_wasm "xoxno_oracle.wasm missing under $WASM_DIR (run make integration-wasm)"

    if [ -z "${XO_ORACLE:-}" ]; then
        local out_f="$LOG_DIR/deploy_xo.out" err_f="$LOG_DIR/deploy_xo.err"
        run_deploy "$out_f" "$err_f" -- stellar contract deploy --wasm "$wasm" \
            --source "$ADMIN" "${NET_ARGS[@]}" \
            -- --admin "$ADMIN_ADDR" \
            --signers "[\"$ALICE_ADDR\",\"$BOB_ADDR\",\"$CAROL_ADDR\"]" \
            --threshold 2 --resolution 60
        local xo
        xo=$(sanitize_output "$out_f")
        is_contract_id "$xo" || die deploy_xoxno_oracle "oracle deploy produced no id: $(tail_err_note "$err_f")"
        record deploy_xoxno_oracle ok deploy "$(extract_signing_hash "$err_f")" "" "" "" "" "$xo" deployment "$xo"
        save_state XO_ORACLE "$xo"
    fi
    local XO="$XO_ORACLE"
    local usdx='{"Other":"USDX"}'

    # Static views and owner identity first.
    assert_view_eq_at "$XO" xo_owner "$ADMIN_ADDR" get_owner
    assert_view_eq_at "$XO" xo_resolution 60 resolution
    assert_int_view_at_nonneg xo_decimals "$XO" decimals
    view xo_base "$XO" -- base >/dev/null

    # Admin surface with read-backs and the designed rejects.
    xfail xo_threshold_zero 'Error\(Contract, #3\)' "$ADMIN" "$XO" -- set_threshold --threshold 0
    xfail xo_threshold_over 'Error\(Contract, #3\)' "$ADMIN" "$XO" -- set_threshold --threshold 9
    inv xo_set_threshold "$ADMIN" "$XO" -- set_threshold --threshold 2 >/dev/null
    xfail xo_owner_guard "Missing signing key for account $ADMIN_ADDR" "$ALICE" "$XO" -- set_threshold --threshold 2

    inv xo_add_signer_dave "$ADMIN" "$XO" -- add_signer --signer "$DAVE_ADDR" >/dev/null
    xfail xo_add_signer_dup 'Error\(Contract, #4\)' "$ADMIN" "$XO" -- add_signer --signer "$DAVE_ADDR"
    inv xo_remove_signer_dave "$ADMIN" "$XO" -- remove_signer --signer "$DAVE_ADDR" >/dev/null
    xfail xo_remove_signer_gone 'Error\(Contract, #5\)' "$ADMIN" "$XO" -- remove_signer --signer "$DAVE_ADDR"
    # 3 signers at threshold 3: removing any of them must refuse (#6).
    inv xo_threshold_three "$ADMIN" "$XO" -- set_threshold --threshold 3 >/dev/null
    xfail xo_remove_below_threshold 'Error\(Contract, #6\)' "$ADMIN" "$XO" -- remove_signer --signer "$CAROL_ADDR"
    inv xo_threshold_two "$ADMIN" "$XO" -- set_threshold --threshold 2 >/dev/null

    inv xo_set_stale "$ADMIN" "$XO" -- set_max_stale_seconds --seconds 3600 >/dev/null
    assert_view_eq_at "$XO" xo_stale_read 3600 max_stale_seconds
    inv xo_set_sub_age "$ADMIN" "$XO" -- set_max_submission_age_seconds --seconds 900 >/dev/null
    assert_view_eq_at "$XO" xo_sub_age_read 900 max_submission_age_seconds
    # Skew must be above MAX_FUTURE_SKEW_SECONDS (60) and at most the
    # submission age; both bounds revert #18.
    xfail xo_skew_too_low 'Error\(Contract, #18\)' "$ADMIN" "$XO" -- set_max_relative_skew_seconds --seconds 60
    xfail xo_skew_over_age 'Error\(Contract, #18\)' "$ADMIN" "$XO" -- set_max_relative_skew_seconds --seconds 1000
    inv xo_set_skew "$ADMIN" "$XO" -- set_max_relative_skew_seconds --seconds 120 >/dev/null
    assert_view_eq_at "$XO" xo_skew_read 120 max_relative_skew_seconds
    xfail xo_spread_zero 'Error\(Contract, #19\)' "$ADMIN" "$XO" -- set_max_cluster_spread_bps --bps 0
    xfail xo_spread_over 'Error\(Contract, #19\)' "$ADMIN" "$XO" -- set_max_cluster_spread_bps --bps 10001
    inv xo_set_spread "$ADMIN" "$XO" -- set_max_cluster_spread_bps --bps 200 >/dev/null
    assert_view_eq_at "$XO" xo_spread_read 200 max_cluster_spread_bps
    inv xo_set_resolution "$ADMIN" "$XO" -- set_resolution --resolution 60 >/dev/null
    assert_view_eq_at "$XO" xo_resolution_reread 60 resolution

    # Feed registry: bare feed (XLMX) and asset-mapped feed (USDX).
    inv xo_register_xlmx "$ADMIN" "$XO" -- register_feed --feed_id XLMX >/dev/null
    xfail xo_register_dup 'Error\(Contract, #17\)' "$ADMIN" "$XO" -- register_feed --feed_id XLMX
    inv xo_add_feed_usdx "$ADMIN" "$XO" -- add_feed --feed_id USDX --asset "$usdx" >/dev/null
    xfail xo_add_feed_dup 'Error\(Contract, #12\)' "$ADMIN" "$XO" -- add_feed --feed_id USDX --asset "$usdx"
    local nfeeds
    nfeeds=$(view xo_feeds "$XO" -- feeds | jq 'length' 2>/dev/null)
    [ "${nfeeds:-0}" -ge 2 ] || _assert_fail xo_feeds_len "feeds()=$nfeeds want >= 2"
    local nassets
    nassets=$(view xo_assets "$XO" -- assets | jq 'length' 2>/dev/null)
    [ "${nassets:-0}" -ge 1 ] || _assert_fail xo_assets_len "assets()=$nassets want >= 1"

    # Zero resolution appends every aggregate, even when transactions share a
    # close timestamp. Restore the production-style value before the upgrade.
    inv xo_history_resolution "$ADMIN" "$XO" -- set_resolution --resolution 0 >/dev/null || return 1

    # Submission validation chain. Timestamps are milliseconds; anchor a few
    # seconds behind wall clock so ledger-time skew cannot make them future.
    local ts
    ts=$(( ($(date +%s) - 10) * 1000 ))
    xfail xo_submit_unregistered 'Error\(Contract, #1\)' "$DAVE" "$XO" -- submit_price \
        --signer "$DAVE_ADDR" --feed_id USDX --price 100000000 --package_timestamp "$ts"
    xfail xo_submit_unknown_feed 'Error\(Contract, #14\)' "$ALICE" "$XO" -- submit_price \
        --signer "$ALICE_ADDR" --feed_id NOPE --price 100000000 --package_timestamp "$ts"
    xfail xo_submit_zero_price 'Error\(Contract, #2\)' "$ALICE" "$XO" -- submit_price \
        --signer "$ALICE_ADDR" --feed_id USDX --price 0 --package_timestamp "$ts"
    xfail xo_submit_price_cap 'Error\(Contract, #9\)' "$ALICE" "$XO" -- submit_price \
        --signer "$ALICE_ADDR" --feed_id USDX --price 2000000000000000000000000 --package_timestamp "$ts"
    xfail xo_submit_future 'Error\(Contract, #11\)' "$ALICE" "$XO" -- submit_price \
        --signer "$ALICE_ADDR" --feed_id USDX --price 100000000 \
        --package_timestamp $(( ($(date +%s) + 3600) * 1000 ))

    # One live submission is below the threshold of two: no aggregate yet.
    inv xo_submit_alice "$ALICE" "$XO" -- submit_price \
        --signer "$ALICE_ADDR" --feed_id USDX --price 100000000 --package_timestamp "$ts" >/dev/null
    xfail_sim xo_read_below_threshold 'Error\(Contract, #7\)' "$ALICE" "$XO" -- read_price_data_for_feed \
        --feed_id USDX

    # Second signer meets quorum; the aggregate is the median of the cluster.
    inv xo_submit_bob "$BOB" "$XO" -- submit_price \
        --signer "$BOB_ADDR" --feed_id USDX --price 102000000 --package_timestamp $((ts + 2000)) >/dev/null
    local agg agg2 agg3 hist_before
    agg2=$(view xo_read_aggregate "$XO" -- read_price_data_for_feed --feed_id USDX) || return 1
    _xo_assert_aggregate xo_aggregate_in_band "$agg2" 100000000 "$ts" xo_submit_bob || return 1
    inv xo_submit_carol "$CAROL" "$XO" -- submit_price \
        --signer "$CAROL_ADDR" --feed_id USDX --price 101000000 --package_timestamp $((ts + 4000)) >/dev/null || return 1
    agg3=$(view xo_read_median3 "$XO" -- read_price_data_for_feed --feed_id USDX) || return 1
    _xo_assert_aggregate xo_median3 "$agg3" 101000000 "$ts" xo_submit_carol || return 1
    hist_before=$(view xo_history_before_batch "$XO" -- read_price_history --feed_id USDX --limit 5) || return 1
    _xo_assert_json xo_history_before_exact "$hist_before" --argjson two "$agg2" --argjson three "$agg3" \
        '. == [$three, $two]' || return 1

    # A signer may not roll its own package timestamp backwards.
    xfail xo_submit_backwards 'Error\(Contract, #16\)' "$ALICE" "$XO" -- submit_price \
        --signer "$ALICE_ADDR" --feed_id USDX --price 100000000 --package_timestamp $((ts - 5000))

    # A later invalid price must leave both feeds, history and per-signer
    # monotonicity unchanged. The valid retry deliberately has an older time.
    local ts2=$((ts + 6000))
    xfail xo_batch_mismatch 'Error\(Contract, #10\)' "$ALICE" "$XO" -- submit_prices \
        --signer "$ALICE_ADDR" --feed_ids '["USDX","XLMX"]' --prices '["103000000"]' \
        --package_timestamp "$ts2" || return 1
    xfail xo_batch_invalid_price 'Error\(Contract, #2\)' "$ALICE" "$XO" -- submit_prices \
        --signer "$ALICE_ADDR" --feed_ids '["USDX","XLMX"]' --prices '["103000000","0"]' \
        --package_timestamp "$((ts2 + 1000))" || return 1
    agg=$(view xo_batch_rollback_read "$XO" -- read_price_data_for_feed --feed_id USDX) || return 1
    _xo_assert_json xo_batch_rollback_aggregate "$agg" --argjson before "$agg3" '. == $before' || return 1
    local hist
    hist=$(view xo_batch_rollback_history_read "$XO" -- read_price_history --feed_id USDX --limit 5) || return 1
    _xo_assert_json xo_batch_rollback_history "$hist" --argjson before "$hist_before" '. == $before' || return 1
    xfail_sim xo_batch_rollback_no_xlmx 'Error\(Contract, #7\)' "$ALICE" "$XO" -- read_price_data_for_feed --feed_id XLMX || return 1
    inv xo_batch_submit "$ALICE" "$XO" -- submit_prices \
        --signer "$ALICE_ADDR" --feed_ids '["USDX","XLMX"]' --prices '["103000000","50000000"]' \
        --package_timestamp "$ts2" >/dev/null || return 1
    agg=$(view xo_batch_aggregate "$XO" -- read_price_data_for_feed --feed_id USDX) || return 1
    _xo_assert_aggregate xo_batch_median "$agg" 102000000 "$((ts + 2000))" xo_batch_submit || return 1

    # Reflector conversions use package milliseconds / 1000, not write time.
    local lp prices at before_first batch xlmx
    lp=$(view xo_lastprice "$XO" -- lastprice --asset "$usdx") || return 1
    _xo_assert_json xo_lastprice_pos "$lp" --argjson ts "$(((ts + 2000) / 1000))" \
        '(.price | tonumber) == 102000000 and (.timestamp | tonumber) == $ts' || return 1
    at=$(view xo_price_at "$XO" -- price --asset "$usdx" --timestamp "$((ts / 1000))") || return 1
    _xo_assert_json xo_price_at_exact "$at" --argjson ts "$((ts / 1000))" \
        '(.price | tonumber) == 101000000 and (.timestamp | tonumber) == $ts' || return 1
    before_first=$(view xo_price_before_first "$XO" -- price --asset "$usdx" --timestamp "$((ts / 1000 - 1))") || return 1
    _xo_assert_json xo_price_before_first_null "$before_first" '. == null' || return 1
    prices=$(view xo_prices "$XO" -- prices --asset "$usdx" --records 3) || return 1
    _xo_assert_json xo_prices_exact "$prices" --argjson ts "$((ts / 1000))" \
        'map({price:(.price | tonumber), timestamp:(.timestamp | tonumber)}) == [{price:102000000,timestamp:($ts+2)},{price:101000000,timestamp:$ts},{price:100000000,timestamp:$ts}]' || return 1
    hist=$(view xo_history "$XO" -- read_price_history --feed_id USDX --limit 5) || return 1
    _xo_assert_json xo_history_len "$hist" --argjson two "$agg2" --argjson three "$agg3" --argjson latest "$agg" \
        '. == [$latest, $three, $two]' || return 1

    inv xo_xlmx_bob "$BOB" "$XO" -- submit_price --signer "$BOB_ADDR" --feed_id XLMX \
        --price 51000000 --package_timestamp "$((ts2 + 2000))" >/dev/null || return 1
    xlmx=$(view xo_xlmx_aggregate "$XO" -- read_price_data_for_feed --feed_id XLMX) || return 1
    _xo_assert_aggregate xo_xlmx_median "$xlmx" 50000000 "$ts2" xo_xlmx_bob || return 1
    batch=$(view xo_read_batch "$XO" -- read_price_data --feed_ids '["XLMX","USDX","XLMX"]') || return 1
    _xo_assert_json xo_batch_order "$batch" --argjson usd "$agg" --argjson xlm "$xlmx" '. == [$xlm, $usd, $xlm]' || return 1

    inv xo_recompute "$ADMIN" "$XO" -- recompute_feeds --feed_ids '["USDX"]' >/dev/null || return 1
    agg=$(view xo_recompute_read "$XO" -- read_price_data_for_feed --feed_id USDX) || return 1
    _xo_assert_aggregate xo_recompute_exact "$agg" 102000000 "$((ts + 2000))" xo_recompute || return 1
    xfail xo_recompute_unknown 'Error\(Contract, #14\)' "$ADMIN" "$XO" -- recompute_feeds \
        --feed_ids '["NOPE"]' || return 1

    # Removal re-aggregates USDX from Bob/Carol but clears XLMX's lost quorum.
    inv xo_remove_active_alice "$ADMIN" "$XO" -- remove_signer --signer "$ALICE_ADDR" >/dev/null || return 1
    agg=$(view xo_after_removal "$XO" -- read_price_data_for_feed --feed_id USDX) || return 1
    _xo_assert_aggregate xo_removal_median "$agg" 101000000 "$((ts + 2000))" xo_remove_active_alice || return 1
    xfail_sim xo_removal_quorum_lost 'Error\(Contract, #7\)' "$BOB" "$XO" -- read_price_data_for_feed --feed_id XLMX || return 1
    xfail_sim xo_removal_history_hidden 'Error\(Contract, #7\)' "$BOB" "$XO" -- read_price_history --feed_id XLMX --limit 5 || return 1
    xfail xo_removed_signer_denied 'Error\(Contract, #1\)' "$ALICE" "$XO" -- submit_price \
        --signer "$ALICE_ADDR" --feed_id XLMX --price 52000000 --package_timestamp "$((ts2 + 3000))" || return 1
    inv xo_readd_alice "$ADMIN" "$XO" -- add_signer --signer "$ALICE_ADDR" >/dev/null || return 1
    inv xo_recompute_missing_quorum "$ADMIN" "$XO" -- recompute_feeds --feed_ids '["XLMX"]' >/dev/null || return 1
    xfail_sim xo_readd_no_resurrection 'Error\(Contract, #7\)' "$BOB" "$XO" -- read_price_data_for_feed --feed_id XLMX || return 1
    inv xo_recover_submission "$ALICE" "$XO" -- submit_price --signer "$ALICE_ADDR" --feed_id XLMX \
        --price 52000000 --package_timestamp "$((ts2 + 3000))" >/dev/null || return 1
    agg=$(view xo_recovered_read "$XO" -- read_price_data_for_feed --feed_id XLMX) || return 1
    _xo_assert_aggregate xo_recovered_exact "$agg" 51000000 "$((ts2 + 2000))" xo_recover_submission || return 1
    hist=$(view xo_recovered_history "$XO" -- read_price_history --feed_id XLMX --limit 5) || return 1
    _xo_assert_json xo_recovered_history_exact "$hist" --argjson before "$xlmx" --argjson latest "$agg" \
        '. == [$latest, $before]' || return 1
    inv xo_restore_resolution "$ADMIN" "$XO" -- set_resolution --resolution 60 >/dev/null || return 1

    # Same-hash upgrade, then prove config survived the code swap.
    local xo_out="$LOG_DIR/upload_xo.out" xo_err="$LOG_DIR/upload_xo.err" xo_hash
    run_deploy "$xo_out" "$xo_err" -- stellar contract upload --wasm "$wasm" \
        --source "$ADMIN" "${NET_ARGS[@]}"
    xo_hash=$(sanitize_output "$xo_out")
    if is_wasm_hash "$xo_hash"; then
        record xo_upload_wasm ok upload "$(extract_signing_hash "$xo_err")" "" "" "" "" "$xo_hash"
        local upgrade_before upgrade_after
        upgrade_before=$(xo_upgrade_snapshot xo_upgrade_before "$XO") || return 1
        inv xo_upgrade "$ADMIN" "$XO" -- upgrade --new_wasm_hash "$xo_hash" >/dev/null || return 1
        upgrade_after=$(xo_upgrade_snapshot xo_upgrade_after "$XO") || return 1
        _xo_assert_json xo_upgrade_state "$upgrade_after" --argjson before "$upgrade_before" '. == $before' || return 1
        printf '%s\n' "$upgrade_before" > "$RUN_DIR/oracle-upgrade-before.json"
        printf '%s\n' "$upgrade_after" > "$RUN_DIR/oracle-upgrade-after.json"
        assert_view_eq_at "$XO" xo_resolution_post_upgrade 60 resolution
        local upgrade_ts upgrade_quote
        upgrade_ts="$(( ($(date +%s) - 10) * 1000 ))"
        inv xo_upgrade_feed "$ADMIN" "$XO" -- register_feed --feed_id UPGRADE >/dev/null || return 1
        xfail_sim xo_upgrade_unsigned 'Error\(Contract, #1\)' "$DAVE" "$XO" -- submit_price --signer "$DAVE_ADDR" --feed_id UPGRADE --price 100000000 --package_timestamp "$upgrade_ts" || return 1
        inv xo_upgrade_first "$ALICE" "$XO" -- submit_price --signer "$ALICE_ADDR" --feed_id UPGRADE --price 100000000 --package_timestamp "$upgrade_ts" >/dev/null || return 1
        xfail_sim xo_upgrade_needs_quorum 'Error\(Contract, #7\)' "$ADMIN" "$XO" -- read_price_data_for_feed --feed_id UPGRADE || return 1
        inv xo_upgrade_second "$BOB" "$XO" -- submit_price --signer "$BOB_ADDR" --feed_id UPGRADE --price 100000000 --package_timestamp "$upgrade_ts" >/dev/null || return 1
        upgrade_quote=$(view xo_upgrade_second_quote "$XO" -- read_price_data_for_feed --feed_id UPGRADE) || return 1
        _xo_assert_aggregate xo_upgrade_two_signers "$upgrade_quote" 100000000 "$upgrade_ts" xo_upgrade_second || return 1
        inv xo_upgrade_third "$CAROL" "$XO" -- submit_price --signer "$CAROL_ADDR" --feed_id UPGRADE --price 100000000 --package_timestamp "$upgrade_ts" >/dev/null || return 1
        upgrade_quote=$(view xo_upgrade_quote "$XO" -- read_price_data_for_feed --feed_id UPGRADE) || return 1
        _xo_assert_aggregate xo_upgrade_quorum "$upgrade_quote" 100000000 "$upgrade_ts" xo_upgrade_third || return 1
    else
        _assert_fail xo_upload_wasm "oracle wasm upload failed: $(tail_err_note "$xo_err")"
    fi

    # Feed removal and purge, each with its designed follow-up reject.
    inv xo_remove_feed "$ADMIN" "$XO" -- remove_feed --asset "$usdx" >/dev/null
    xfail xo_remove_feed_gone 'Error\(Contract, #13\)' "$ADMIN" "$XO" -- remove_feed --asset "$usdx"
    inv xo_purge_xlmx "$ADMIN" "$XO" -- purge_feed --feed_id XLMX >/dev/null
    xfail xo_purge_gone 'Error\(Contract, #14\)' "$ADMIN" "$XO" -- purge_feed --feed_id XLMX
    local ledger
    ledger=$(curl --fail-with-body -sS -m 30 "$RPC_URL" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' | jq -er '.result.sequence') || return 1
    inv xo_transfer_owner "$ADMIN" "$XO" -- transfer_ownership --new_owner "$BOB_ADDR" --live_until_ledger "$((ledger+1000))" >/dev/null || return 1
    inv xo_accept_owner "$BOB" "$XO" -- accept_ownership >/dev/null || return 1
    assert_view_eq_at "$XO" xo_new_owner "$BOB_ADDR" get_owner || return 1
    xfail xo_old_owner_denied "Missing signing key for account $BOB_ADDR" "$ADMIN" "$XO" -- set_threshold --threshold 2 || return 1

}
