# On-chain assertion helpers (parse view output; require WAD, view(), CONTROLLER, log()).
#
# A failed assertion records a FAIL action row so assert_green.sh gates on it —
# a value mismatch is a suite failure, not just a log line. The view() the
# helper runs also records its own `read` row; the FAIL row carries the verdict.

_view_int() {
  view "$1" "$CONTROLLER" -- "${@:2}" | tr -d '"' | tr -d '[:space:]'
}

_view_pool_int() {
  view "$1" "$POOL" -- "${@:2}" | tr -d '"' | tr -d '[:space:]'
}

# Records a gate-visible FAIL row for a broken assertion and returns non-zero.
_assert_fail() {
  local label="$1" msg="$2"
  log "ASSERT FAIL [$label]: $msg"
  record "$label" FAIL assert "" "" "" "" "" "$msg"
  return 1
}

# Overflow-safe unsigned-integer comparison. View values (health factor, scaled
# debt) are WAD/i128-scale and routinely exceed bash's signed 64-bit range — a
# health factor above ~9.2 is already > 2^63, where `[ "$a" -ge "$b" ]` errors
# and silently fails the assertion. These compare decimal-integer STRINGS
# exactly: first by digit count, then lexicographically at equal length.
_strip0() {
  local s="$1"
  while [ "${s:0:1}" = "0" ] && [ "${#s}" -gt 1 ]; do s="${s:1}"; done
  printf '%s' "$s"
}
_is_uint() { [[ "$1" =~ ^[0-9]+$ ]]; }
_uint_ge() {  # A >= B
  _is_uint "$1" && _is_uint "$2" || return 1
  local a b; a="$(_strip0 "$1")"; b="$(_strip0 "$2")"
  if [ "${#a}" -ne "${#b}" ]; then [ "${#a}" -gt "${#b}" ]; return; fi
  [[ "$a" > "$b" || "$a" == "$b" ]]
}
_uint_lt() {  # A < B
  # Validate independently of _uint_ge: bare `! _uint_ge` would treat a
  # non-numeric/empty A (which makes _uint_ge fail) as "less than", so a view
  # returning "" or an error string would spuriously satisfy the assertion.
  _is_uint "$1" && _is_uint "$2" || return 1
  ! _uint_ge "$1" "$2"
}
_uint_le() { _uint_ge "$2" "$1"; }     # A <= B
_str_eq() { [ "$1" = "$2" ]; }

# Re-reads a view until cmp(value, bound) holds, absorbing read-after-write
# replica lag: a view can return a SUCCESSFUL but STALE value right after the
# state-changing tx it is asserting on (the post-change value hasn't synced to
# the replica yet), so the condition transiently looks false. Echoes the
# settling value on success, or the last-read value on exhaustion (the caller
# then records the FAIL). A genuinely-wrong value never settles and falls
# through to FAIL — so this defers a spurious failure, it never hides a real one.
#   _retry_until <reader-fn> <cmp-fn> <bound> <label> <view-fn> [args...]
_retry_until() {
  local reader="$1" cmp="$2" bound="$3" label="$4"; shift 4
  local v attempt
  for attempt in 1 2 3 4 5; do
    [ "$attempt" -gt 1 ] && sleep $(( (attempt - 1) * 3 ))
    v=$("$reader" "$label" "$@")
    "$cmp" "$v" "$bound" && { printf '%s' "$v"; return 0; }
  done
  printf '%s' "$v"
  return 1
}

assert_bool_view() {
  local label="$1" expected="$2"
  shift 2
  local actual
  actual=$(_retry_until _view_int _str_eq "$expected" "$label" "$@") \
    || _assert_fail "$label" "got '$actual', want '$expected'"
}

# Exact-equality assertion on a controller int view: assert_int_view_eq <label> <expected> <fn> [args...]
assert_int_view_eq() {
  local label="$1" expected="$2"
  shift 2
  local actual
  actual=$(_retry_until _view_int _str_eq "$expected" "$label" "$@") \
    || _assert_fail "$label" "got '$actual', want '$expected'"
}

assert_hf_at_least() {
  local label="$1" acct="$2" min_wad="$3"
  local hf
  hf=$(_retry_until _view_int _uint_ge "$min_wad" "$label" get_health_factor --account_id "$acct") \
    || _assert_fail "$label" "hf=$hf want >= $min_wad"
}

assert_hf_below_wad() {
  local label="$1" acct="$2"
  local hf
  hf=$(_retry_until _view_int _uint_lt "$WAD" "$label" get_health_factor --account_id "$acct") \
    || _assert_fail "$label" "hf=$hf want < $WAD (liquidatable)"
}

assert_borrow_at_most() {
  local label="$1" acct="$2" asset="$3" max_raw="$4"
  local debt
debt=$(_retry_until _view_int _uint_le "$max_raw" "$label" get_borrow_amount --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$asset")") \
    || _assert_fail "$label" "borrow=$debt want <= $max_raw"
}

assert_borrow_at_least() {
  local label="$1" acct="$2" asset="$3" min_raw="$4"
  local debt
debt=$(_retry_until _view_int _uint_ge "$min_raw" "$label" get_borrow_amount --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$asset")") \
    || _assert_fail "$label" "borrow=$debt want >= $min_raw"
}

assert_borrow_decreased() {
  local label="$1" acct="$2" asset="$3" before_raw="$4"
  local debt
debt=$(_retry_until _view_int _uint_lt "$before_raw" "$label" get_borrow_amount --account_id "$acct" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$asset")") \
    || _assert_fail "$label" "borrow=$debt want < $before_raw"
}

assert_can_liquidated() {
  local label="$1" acct="$2" expected="$3"
  assert_bool_view "$label" "$expected" is_liquidatable --account_id "$acct"
}

# Regex checks (not arithmetic) so i128::MAX-scale view values don't overflow
# bash's 64-bit integer comparison. assert_int_view_positive <label> <fn> [args...]
assert_int_view_positive() {
  local label="$1"; shift
  local v
  v=$(_view_int "$label" "$@")
  [[ "$v" =~ ^[1-9][0-9]*$ ]] || _assert_fail "$label" "got '$v' want positive int"
}

assert_int_view_nonneg() {
  local label="$1"; shift
  local v
  v=$(_view_int "$label" "$@")
  [[ "$v" =~ ^[0-9]+$ ]] || _assert_fail "$label" "got '$v' want non-negative int"
}

# Reads the base spoke-0 listing (SpokeAssetConfig) for an asset and asserts a
# single top-level BPS field (loan_to_value / liquidation_threshold /
# liquidation_bonus).
#   assert_market_field <label> <asset> <jq-field> <expected>
assert_market_field() {
  local label="$1" asset="$2" field="$3" expected="$4"
  local got
    got=$(view "$label" "$CONTROLLER" -- get_spoke_asset --spoke_id "$PRIMARY_SPOKE_ID" --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$asset")" \
        | jq -r ".${field}")
  [ "$got" = "$expected" ] || _assert_fail "$label" "spoke_asset.$field=$got want $expected"
}

assert_pool_revenue_decreased() {
  local label="$1" asset="$2" before_raw="$3"
  local after
  after=$(_retry_until _view_pool_int _uint_lt "$before_raw" "$label" get_revenue --hub_asset "$(hub_key "$PRIMARY_HUB_ID" "$asset")") \
    || _assert_fail "$label" "pool_revenue=$after want < $before_raw after claim"
}
