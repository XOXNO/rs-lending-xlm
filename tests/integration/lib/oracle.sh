# Mock oracle deployment and price control.
#
# Liquidation cannot be force-triggered against real Reflector feeds (HF reads
# live prices; the cached per-account threshold refuses to re-sync into
# liquidation range). Mock-priced markets are the only way to crash a price.
# Deploy a FRESH mock per run — reused mocks go stale across sessions (#206).

# Deploys the Reflector-shaped mock; sets/persists MOCK.
deploy_mock_reflector() {
    if [ -n "${MOCK:-}" ]; then return 0; fi
    local out_f="$LOG_DIR/deploy_mock.out" err_f="$LOG_DIR/deploy_mock.err"
    stellar contract deploy --wasm "$WASM_DIR/mock_oracle.wasm" \
        --source "$ADMIN" --network "$NETWORK" >"$out_f" 2>"$err_f"
    local mock hash
    mock=$(tr -d '"\n' < "$out_f")
    hash=$(grep -oE 'Signing transaction: [0-9a-f]{64}' "$err_f" | tail -1 | awk '{print $3}')
    [ -z "$mock" ] && { log "mock reflector deploy failed: $(tail -3 "$err_f")"; return 1; }
    save_state MOCK "$mock"
    record deploy_mock_reflector ok deploy "$hash" "" "" "" "" "$mock"
    log "mock reflector = $mock"
}

# Deploys the RedStone-shaped mock; sets/persists MOCKRS.
deploy_mock_redstone() {
    if [ -n "${MOCKRS:-}" ]; then return 0; fi
    local out_f="$LOG_DIR/deploy_mockrs.out" err_f="$LOG_DIR/deploy_mockrs.err"
    stellar contract deploy --wasm "$WASM_DIR/mock_redstone.wasm" \
        --source "$ADMIN" --network "$NETWORK" >"$out_f" 2>"$err_f"
    local mock hash
    mock=$(tr -d '"\n' < "$out_f")
    hash=$(grep -oE 'Signing transaction: [0-9a-f]{64}' "$err_f" | tail -1 | awk '{print $3}')
    [ -z "$mock" ] && { log "mock redstone deploy failed: $(tail -3 "$err_f")"; return 1; }
    save_state MOCKRS "$mock"
    record deploy_mock_redstone ok deploy "$hash" "" "" "" "" "$mock"
    log "mock redstone = $mock"
}

# Sets the Reflector-mock price for a SAC, in USD WAD.
#   set_mock_price <sac-id> <price-wad> [label]
set_mock_price() {
    local sac="$1" price="$2" label="${3:-set_px_${sac:0:6}}"
    inv "$label" "$ADMIN" "$MOCK" -- set_price \
        --asset "{\"Stellar\":\"$sac\"}" --price_wad "$price" >/dev/null
}

# Backdates the Reflector-mock timestamp for a SAC (staleness tests).
#   set_mock_ts <sac-id> <unix-ts> [label]
set_mock_ts() {
    local sac="$1" ts="$2" label="${3:-set_ts_${sac:0:6}}"
    inv "$label" "$ADMIN" "$MOCK" -- set_ts \
        --asset "{\"Stellar\":\"$sac\"}" --timestamp "$ts" >/dev/null
}

# Sets the RedStone-mock price for a feed id, in USD WAD.
#   set_rs_price <feed-id> <price-wad> [label]
set_rs_price() {
    local feed="$1" price="$2" label="${3:-set_rs_${feed}}"
    inv "$label" "$ADMIN" "$MOCKRS" -- set_price \
        --feed_id "$feed" --price_wad "$price" >/dev/null
}

# Moves primary (Reflector mock) and anchor (RedStone mock) in lock-step so
# tolerance checks keep passing while the price moves.
#   dual_px <sac-id> <feed-id> <price-wad> [label]
dual_px() {
    local sac="$1" feed="$2" price="$3" label="${4:-dual_px_${feed}}"
    set_mock_price "$sac" "$price" "${label}_p"
    set_rs_price "$feed" "$price" "${label}_a"
}
