# Live xoxno-oracle coverage: deploy with run wallets as the signer set,
# exercise the whole admin surface with read-backs, drive threshold-gated
# median aggregation through real multi-signer submissions, hit every
# designed revert (#NN = contracts/xoxno-oracle Error), and same-hash
# upgrade with state preserved. Self-contained: no controller wiring.

flow_xoxno_oracle() {
    phase xoxno_oracle
    local wasm="$WASM_DIR/xoxno_oracle.wasm"
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
        record deploy_xoxno_oracle ok deploy "$(extract_signing_hash "$err_f")" "" "" "" "" "$xo"
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
    # Skew must sit strictly above MAX_FUTURE_SKEW_SECONDS (60) and at or
    # below the submission age — both designed rejects covered here.
    xfail xo_skew_too_low 'Error\(Contract, #18\)' "$ADMIN" "$XO" -- set_max_relative_skew_seconds --seconds 60
    xfail xo_skew_over_age 'Error\(Contract, #18\)' "$ADMIN" "$XO" -- set_max_relative_skew_seconds --seconds 1000
    inv xo_set_skew "$ADMIN" "$XO" -- set_max_relative_skew_seconds --seconds 120 >/dev/null
    assert_view_eq_at "$XO" xo_skew_read 120 max_relative_skew_seconds
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
    local agg px
    agg=$(view xo_read_aggregate "$XO" -- read_price_data_for_feed --feed_id USDX)
    px=$(jq -r '.price' <<<"$agg" 2>/dev/null)
    if [[ "$px" =~ ^[0-9]+$ ]] && _uint_ge "$px" 100000000 && _uint_le "$px" 102000000; then
        record xo_aggregate_in_band ok read_price_data_for_feed "" "" "" "" "" "median=$px"
    else
        _assert_fail xo_aggregate_in_band "aggregate price '$px' outside [100000000,102000000]"
    fi
    inv xo_submit_carol "$CAROL" "$XO" -- submit_price \
        --signer "$CAROL_ADDR" --feed_id USDX --price 101000000 --package_timestamp $((ts + 4000)) >/dev/null
    agg=$(view xo_read_median3 "$XO" -- read_price_data_for_feed --feed_id USDX)
    px=$(jq -r '.price' <<<"$agg" 2>/dev/null)
    [ "$px" = "101000000" ] || _assert_fail xo_median3 "3-signer median '$px' want 101000000"

    # A signer may not roll its own package timestamp backwards.
    xfail xo_submit_backwards 'Error\(Contract, #16\)' "$ALICE" "$XO" -- submit_price \
        --signer "$ALICE_ADDR" --feed_id USDX --price 100000000 --package_timestamp $((ts - 5000))

    # Batched submission: same timestamp across feeds, strict length check.
    local ts2=$((ts + 6000))
    xfail xo_batch_mismatch 'Error\(Contract, #10\)' "$ALICE" "$XO" -- submit_prices \
        --signer "$ALICE_ADDR" --feed_ids '["USDX","XLMX"]' --prices '["103000000"]' \
        --package_timestamp "$ts2"
    inv xo_batch_submit "$ALICE" "$XO" -- submit_prices \
        --signer "$ALICE_ADDR" --feed_ids '["USDX","XLMX"]' --prices '["103000000","50000000"]' \
        --package_timestamp "$ts2" >/dev/null

    # Reflector-compat read surface over the mapped asset.
    local lp
    lp=$(view xo_lastprice "$XO" -- lastprice --asset "$usdx")
    px=$(jq -r '.price' <<<"$lp" 2>/dev/null)
    [[ "$px" =~ ^[0-9]+$ ]] && [ "$px" -gt 0 ] \
        || _assert_fail xo_lastprice_pos "lastprice price '$px' want > 0"
    view xo_price_at "$XO" -- price --asset "$usdx" --timestamp "$(date +%s)" >/dev/null
    view xo_prices "$XO" -- prices --asset "$usdx" --records 3 >/dev/null
    local hist
    hist=$(view xo_history "$XO" -- read_price_history --feed_id USDX --limit 5)
    [ "$(jq 'length' <<<"$hist" 2>/dev/null)" -ge 1 ] \
        || _assert_fail xo_history_len "read_price_history empty"
    view xo_read_batch "$XO" -- read_price_data --feed_ids '["USDX"]' >/dev/null

    inv xo_recompute "$ADMIN" "$XO" -- recompute_feeds --feed_ids '["USDX"]' >/dev/null
    xfail xo_recompute_unknown 'Error\(Contract, #14\)' "$ADMIN" "$XO" -- recompute_feeds \
        --feed_ids '["NOPE"]'

    # Same-hash upgrade, then prove config survived the code swap.
    local xo_out="$LOG_DIR/upload_xo.out" xo_err="$LOG_DIR/upload_xo.err" xo_hash
    run_deploy "$xo_out" "$xo_err" -- stellar contract upload --wasm "$wasm" \
        --source "$ADMIN" "${NET_ARGS[@]}"
    xo_hash=$(sanitize_output "$xo_out")
    if is_wasm_hash "$xo_hash"; then
        record xo_upload_wasm ok upload "$(extract_signing_hash "$xo_err")" "" "" "" "" "$xo_hash"
        inv xo_upgrade "$ADMIN" "$XO" -- upgrade --new_wasm_hash "$xo_hash" >/dev/null
        assert_view_eq_at "$XO" xo_resolution_post_upgrade 60 resolution
    else
        _assert_fail xo_upload_wasm "oracle wasm upload failed: $(tail_err_note "$xo_err")"
    fi

    # Feed removal and purge, each with its designed follow-up reject.
    inv xo_remove_feed "$ADMIN" "$XO" -- remove_feed --asset "$usdx" >/dev/null
    xfail xo_remove_feed_gone 'Error\(Contract, #13\)' "$ADMIN" "$XO" -- remove_feed --asset "$usdx"
    inv xo_purge_xlmx "$ADMIN" "$XO" -- purge_feed --feed_id XLMX >/dev/null
    xfail xo_purge_gone 'Error\(Contract, #14\)' "$ADMIN" "$XO" -- purge_feed --feed_id XLMX
}
