deploy_mock_reflector() {
    if [ -n "${MOCK:-}" ]; then return 0; fi
    local base out_f err_f mock hash
    base=$(job_log deploy_mock); out_f="$base.out" err_f="$base.err"
    run_deploy "$out_f" "$err_f" -- stellar contract deploy --wasm "$FIXTURE_WASM_DIR/mock_oracle.wasm" \
        --source "${E2E_SRC:-$ADMIN}" "${NET_ARGS[@]}"
    mock=$(sanitize_output "$out_f")
    hash=$(extract_signing_hash "$err_f")
    is_contract_id "$mock" || die deploy_mock_reflector "mock reflector deploy produced no id after $DEPLOY_ATTEMPTS attempt(s): $(tail_err_note "$err_f")"
    save_state MOCK "$mock"
    record deploy_mock_reflector ok deploy "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" "$mock" deployment "$mock"
    log "mock reflector = $mock"
}

deploy_mock_redstone() {
    if [ -n "${MOCKRS:-}" ]; then return 0; fi
    local base out_f err_f mock hash
    base=$(job_log deploy_mockrs); out_f="$base.out" err_f="$base.err"
    run_deploy "$out_f" "$err_f" -- stellar contract deploy --wasm "$FIXTURE_WASM_DIR/mock_redstone.wasm" \
        --source "${E2E_SRC:-$ADMIN}" "${NET_ARGS[@]}"
    mock=$(sanitize_output "$out_f")
    hash=$(extract_signing_hash "$err_f")
    is_contract_id "$mock" || die deploy_mock_redstone "mock redstone deploy produced no id after $DEPLOY_ATTEMPTS attempt(s): $(tail_err_note "$err_f")"
    save_state MOCKRS "$mock"
    record deploy_mock_redstone ok deploy "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" "$mock" deployment "$mock"
    log "mock redstone = $mock"
}

set_mock_price() {
    local sac="$1" price="$2" label="${3:-set_px_${sac:0:6}}"
    inv "$label" "${E2E_SRC:-$ADMIN}" "$MOCK" -- set_price \
        --asset "{\"Stellar\":\"$sac\"}" --price_wad "$price" >/dev/null
}

set_rs_price() {
    local feed="$1" price="$2" label="${3:-set_rs_${feed}}"
    inv "$label" "${E2E_SRC:-$ADMIN}" "$MOCKRS" -- set_price \
        --feed_id "$feed" --price_wad "$price" >/dev/null
}

dual_px() {
    local sac="$1" feed="$2" price="$3" label="${4:-dual_px_${feed}}"
    set_mock_price "$sac" "$price" "${label}_p"
    set_rs_price "$feed" "$price" "${label}_a"
}
