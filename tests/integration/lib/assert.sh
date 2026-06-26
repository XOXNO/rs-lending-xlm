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
_uint_lt() { ! _uint_ge "$1" "$2"; }   # A < B
_uint_le() { _uint_ge "$2" "$1"; }     # A <= B

assert_bool_view() {
  local label="$1" expected="$2"
  shift 2
  local actual
  actual=$(_view_int "$label" "$@")
  [ "$actual" = "$expected" ] || _assert_fail "$label" "got '$actual', want '$expected'"
}

# Exact-equality assertion on a controller int view: assert_int_view_eq <label> <expected> <fn> [args...]
assert_int_view_eq() {
  local label="$1" expected="$2"
  shift 2
  local actual
  actual=$(_view_int "$label" "$@")
  [ "$actual" = "$expected" ] || _assert_fail "$label" "got '$actual', want '$expected'"
}

assert_hf_at_least() {
  local label="$1" acct="$2" min_wad="$3"
  local hf
  hf=$(_view_int "$label" health_factor --account_id "$acct")
  _uint_ge "$hf" "$min_wad" || _assert_fail "$label" "hf=$hf want >= $min_wad"
}

assert_hf_below_wad() {
  local label="$1" acct="$2"
  local hf
  hf=$(_view_int "$label" health_factor --account_id "$acct")
  _uint_lt "$hf" "$WAD" || _assert_fail "$label" "hf=$hf want < $WAD (liquidatable)"
}

assert_borrow_at_most() {
  local label="$1" acct="$2" asset="$3" max_raw="$4"
  local debt
  debt=$(_view_int "$label" borrow_amount_for_token --account_id "$acct" --asset "$asset")
  _uint_le "$debt" "$max_raw" || _assert_fail "$label" "borrow=$debt want <= $max_raw"
}

assert_borrow_at_least() {
  local label="$1" acct="$2" asset="$3" min_raw="$4"
  local debt
  debt=$(_view_int "$label" borrow_amount_for_token --account_id "$acct" --asset "$asset")
  _uint_ge "$debt" "$min_raw" || _assert_fail "$label" "borrow=$debt want >= $min_raw"
}

assert_borrow_decreased() {
  local label="$1" acct="$2" asset="$3" before_raw="$4"
  local debt
  debt=$(_view_int "$label" borrow_amount_for_token --account_id "$acct" --asset "$asset")
  _uint_lt "$debt" "$before_raw" || _assert_fail "$label" "borrow=$debt want < $before_raw"
}

assert_can_liquidated() {
  local label="$1" acct="$2" expected="$3"
  assert_bool_view "$label" "$expected" can_be_liquidated --account_id "$acct"
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

# Parses get_market_config and asserts a single asset_config BPS field.
#   assert_market_field <label> <asset> <jq-field> <expected>
assert_market_field() {
  local label="$1" asset="$2" field="$3" expected="$4"
  local got
  got=$(view "$label" "$CONTROLLER" -- get_market_config --asset "$asset" \
    | jq -r ".asset_config.${field}")
  [ "$got" = "$expected" ] || _assert_fail "$label" "asset_config.$field=$got want $expected"
}

# Asserts the market status. MarketStatus is a discriminant enum, so the CLI
# renders `.status` as the integer code (PendingOracle=0, Active=1, Disabled=2);
# the name is mapped here so callers stay readable.
assert_market_status() {
  local label="$1" asset="$2" want="$3" want_code
  case "$want" in
    PendingOracle) want_code=0 ;;
    Active)        want_code=1 ;;
    Disabled)      want_code=2 ;;
    *)             want_code="$want" ;;
  esac
  local got
  got=$(view "$label" "$CONTROLLER" -- get_market_config --asset "$asset" | jq -r '.status')
  [ "$got" = "$want_code" ] || _assert_fail "$label" "status=$got want $want ($want_code)"
}

assert_pool_revenue_decreased() {
  local label="$1" asset="$2" before_raw="$3"
  local after
  after=$(_view_pool_int "$label" get_revenue --asset "$asset")
  { [ -n "$after" ] && [ "$after" -lt "$before_raw" ]; } || _assert_fail "$label" "pool_revenue=$after want < $before_raw after claim"
}
