deploy_mock_reflector() {
    if [ -n "${MOCK:-}" ]; then return 0; fi
    local out_f="$LOG_DIR/deploy_mock.out" err_f="$LOG_DIR/deploy_mock.err"
    run_deploy "$out_f" "$err_f" -- stellar contract deploy --wasm "$WASM_DIR/mock_oracle.wasm" \
        --source "$ADMIN" --network "$NETWORK"
    local mock hash
    mock=$(sanitize_output "$out_f")
    hash=$(extract_signing_hash "$err_f")
    is_contract_id "$mock" || die deploy_mock_reflector "mock reflector deploy produced no id after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
    save_state MOCK "$mock"
    record deploy_mock_reflector ok deploy "$hash" "" "" "" "" "$mock"
    log "mock reflector = $mock"
}

deploy_mock_redstone() {
    if [ -n "${MOCKRS:-}" ]; then return 0; fi
    local out_f="$LOG_DIR/deploy_mockrs.out" err_f="$LOG_DIR/deploy_mockrs.err"
    run_deploy "$out_f" "$err_f" -- stellar contract deploy --wasm "$WASM_DIR/mock_redstone.wasm" \
        --source "$ADMIN" --network "$NETWORK"
    local mock hash
    mock=$(sanitize_output "$out_f")
    hash=$(extract_signing_hash "$err_f")
    is_contract_id "$mock" || die deploy_mock_redstone "mock redstone deploy produced no id after $DEPLOY_MAX_ATTEMPTS attempts: $(tail_err_note "$err_f")"
    save_state MOCKRS "$mock"
    record deploy_mock_redstone ok deploy "$hash" "" "" "" "" "$mock"
    log "mock redstone = $mock"
}

set_mock_price() {
    local sac="$1" price="$2" label="${3:-set_px_${sac:0:6}}"
    inv "$label" "$ADMIN" "$MOCK" -- set_price \
        --asset "{\"Stellar\":\"$sac\"}" --price_wad "$price" >/dev/null
}

set_rs_price() {
    local feed="$1" price="$2" label="${3:-set_rs_${feed}}"
    inv "$label" "$ADMIN" "$MOCKRS" -- set_price \
        --feed_id "$feed" --price_wad "$price" >/dev/null
}

dual_px() {
    local sac="$1" feed="$2" price="$3" label="${4:-dual_px_${feed}}"
    set_mock_price "$sac" "$price" "${label}_p"
    set_rs_price "$feed" "$price" "${label}_a"
}
