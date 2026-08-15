_view_int() {
  view "$1" "$CONTROLLER" -- "${@:2}" | tr -d '"' | tr -d '[:space:]'
}

_view_pool_int() {
  view "$1" "$POOL" -- "${@:2}" | tr -d '"' | tr -d '[:space:]'
}

# Reads a scalar view from whichever contract `VIEW_AT` names, for surfaces that
# are neither the controller nor the central pool (an owned swap-aggregator, a
# second price-aggregator). Kept to the `_view_int` argument shape so it can be
# passed to `_retry_until`; the contract travels via the caller's `local
# VIEW_AT`, which dynamic scoping makes visible here.
_view_at_int() {
  view "$1" "${VIEW_AT:-$CONTROLLER}" -- "${@:2}" | tr -d '"' | tr -d '[:space:]'
}

_assert_fail() {
  local label="$1" msg="$2"
  log "ASSERT FAIL [$label]: $msg"
  record "$label" FAIL assert "" "" "" "" "" "$msg"
  return 1
}

_strip0() {
  local s="$1"
  while [ "${s:0:1}" = "0" ] && [ "${#s}" -gt 1 ]; do s="${s:1}"; done
  printf '%s' "$s"
}
_is_uint() { [[ "$1" =~ ^[0-9]+$ ]]; }
_uint_ge() {
  _is_uint "$1" && _is_uint "$2" || return 1
  local a b; a="$(_strip0 "$1")"; b="$(_strip0 "$2")"
  if [ "${#a}" -ne "${#b}" ]; then [ "${#a}" -gt "${#b}" ]; return; fi
  [[ "$a" > "$b" || "$a" == "$b" ]]
}
_uint_lt() {

  _is_uint "$1" && _is_uint "$2" || return 1
  ! _uint_ge "$1" "$2"
}
_uint_le() { _uint_ge "$2" "$1"; }
_str_eq() { [ "$1" = "$2" ]; }

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

assert_int_view_eq() {
  local label="$1" expected="$2"
  shift 2
  local actual
  actual=$(_retry_until _view_int _str_eq "$expected" "$label" "$@") \
    || _assert_fail "$label" "got '$actual', want '$expected'"
}

# `assert_int_view_eq` against an arbitrary contract. Compares as strings, so it
# serves bool views too.
assert_view_eq_at() {
  local contract="$1" label="$2" expected="$3"
  shift 3
  local actual
  local VIEW_AT="$contract"
  actual=$(_retry_until _view_at_int _str_eq "$expected" "$label" "$@") \
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
