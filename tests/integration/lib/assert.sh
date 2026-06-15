# On-chain assertion helpers (parse view output; require WAD, view(), CONTROLLER, log()).

_view_int() {
  view "$1" "$CONTROLLER" -- "${@:3}" | tr -d '"' | tr -d '[:space:]'
}

assert_bool_view() {
  local label="$1" expected="$2"
  shift 2
  local actual
  actual=$(_view_int "$label" "$@")
  if [ "$actual" != "$expected" ]; then
    log "ASSERT FAIL [$label]: got '$actual', want '$expected'"
    return 1
  fi
}

assert_hf_at_least() {
  local label="$1" acct="$2" min_wad="$3"
  local hf
  hf=$(_view_int "$label" health_factor --account_id "$acct")
  if [ -z "$hf" ] || [ "$hf" -lt "$min_wad" ]; then
    log "ASSERT FAIL [$label]: hf=$hf want >= $min_wad"
    return 1
  fi
}

assert_hf_below_wad() {
  local label="$1" acct="$2"
  local hf
  hf=$(_view_int "$label" health_factor --account_id "$acct")
  if [ -z "$hf" ] || [ "$hf" -ge "$WAD" ]; then
    log "ASSERT FAIL [$label]: hf=$hf want < $WAD (liquidatable)"
    return 1
  fi
}

assert_borrow_at_most() {
  local label="$1" acct="$2" asset="$3" max_raw="$4"
  local debt
  debt=$(_view_int "$label" borrow_amount_for_token --account_id "$acct" --asset "$asset")
  if [ -z "$debt" ] || [ "$debt" -gt "$max_raw" ]; then
    log "ASSERT FAIL [$label]: borrow=$debt want <= $max_raw"
    return 1
  fi
}

assert_borrow_at_least() {
  local label="$1" acct="$2" asset="$3" min_raw="$4"
  local debt
  debt=$(_view_int "$label" borrow_amount_for_token --account_id "$acct" --asset "$asset")
  if [ -z "$debt" ] || [ "$debt" -lt "$min_raw" ]; then
    log "ASSERT FAIL [$label]: borrow=$debt want >= $min_raw"
    return 1
  fi
}

assert_borrow_decreased() {
  local label="$1" acct="$2" asset="$3" before_raw="$4"
  local debt
  debt=$(_view_int "$label" borrow_amount_for_token --account_id "$acct" --asset "$asset")
  if [ -z "$debt" ] || [ "$debt" -ge "$before_raw" ]; then
    log "ASSERT FAIL [$label]: borrow=$debt want < $before_raw"
    return 1
  fi
}

assert_can_liquidated() {
  local label="$1" acct="$2" expected="$3"
  assert_bool_view "$label" "$expected" can_be_liquidated --account_id "$acct"
}

_view_pool_int() {
  view "$1" "$POOL" -- "${@:3}" | tr -d '"' | tr -d '[:space:]'
}

assert_pool_revenue_decreased() {
  local label="$1" asset="$2" before_raw="$3"
  local after
  after=$(_view_pool_int "$label" protocol_revenue --asset "$asset")
  if [ -z "$after" ] || [ "$after" -ge "$before_raw" ]; then
    log "ASSERT FAIL [$label]: pool_revenue=$after want < $before_raw after claim"
    return 1
  fi
}