_view_int() {
  view "$1" "$CONTROLLER" -- "${@:2}" | tr -d '"' | tr -d '[:space:]'
}

_view_pool_int() {
  view "$1" "$POOL" -- "${@:2}" | tr -d '"' | tr -d '[:space:]'
}

# Reads a scalar view from the contract that `VIEW_AT` names (default: the
# controller). Takes the `_view_int` arguments, so `_retry_until` can call it.
# Callers set `local VIEW_AT`; bash dynamic scoping makes it visible here.
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

# Non-negative integer view against an arbitrary contract.
assert_int_view_at_nonneg() {
  local label="$1" contract="$2"
  shift 2
  local v
  local VIEW_AT="$contract"
  v=$(_view_at_int "$label" "$@")
  [[ "$v" =~ ^[0-9]+$ ]] || _assert_fail "$label" "got '$v' want non-negative int"
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

# Python integers preserve i128 raw amounts (including 18-decimal assets).
raw_add() { python3 -c 'import sys; print(sum(int(x) for x in sys.argv[1:]))' "$@"; }
raw_sub() { python3 -c 'import sys; print(int(sys.argv[1])-int(sys.argv[2]))' "$1" "$2"; }
assert_delta() {
    local label="$1" before="$2" after="$3" expected="$4"
    if ! python3 - "$before" "$after" "$expected" <<'PY'
import re,sys
assert all(re.fullmatch(r'-?[0-9]+', x) for x in sys.argv[1:]), 'invalid raw amount'
before, after, expected = map(int, sys.argv[1:])
assert before >= 0 and after >= 0 and after - before == expected
PY
    then
        _assert_fail "$label" "balance $before -> $after; expected delta $expected"
        return 1
    fi
    record "$label" ok assert "" "" "" "" "" "exact delta $expected"
}

assert_raw_within() {
    local label="$1" actual="$2" expected="$3" tolerance="$4"
    if ! python3 - "$actual" "$expected" "$tolerance" <<'PY'
import re,sys
assert all(re.fullmatch(r'-?[0-9]+', x) for x in sys.argv[1:])
a,e,t=map(int,sys.argv[1:]); assert t>=0 and abs(a-e)<=t
PY
    then
        _assert_fail "$label" "raw $actual expected $expected within $tolerance rounding units"
        return 1
    fi
    record "$label" ok assert "" "" "" "" "" "raw $actual expected $expected tolerance $tolerance"
}

# Independent rational check against stored (committed) indexes. A view's
# projected index can advance after the transaction and is not this evidence.
assert_net_settle() {
    local label="$1" before="$2" after="$3" sync="$4" amount="$5" decimals="$6"
    if ! python3 - "$before" "$after" "$sync" "$amount" "$decimals" <<'PYNET'
import json,sys
from fractions import Fraction
before,after,sync=map(json.loads,sys.argv[1:4]); amount=int(sys.argv[4]); decimals=int(sys.argv[5])
state=sync['state']; ray=10**27
for side,field,ceil in [(0,'supply_index',True),(1,'borrow_index',False)]:
    assert len(before[side])==len(after[side])==1, 'expected one-market partial settlement'
    assert set(before[side])==set(after[side]), 'settlement changed market identity'
    old=int(next(iter(before[side].values()))['scaled_amount'])
    new=int(next(iter(after[side].values()))['scaled_amount'])
    exact=Fraction(amount*10**(27-decimals)*ray,int(state[field]))
    burned=-(-exact.numerator//exact.denominator) if ceil else exact.numerator//exact.denominator
    assert old-new==burned, f'{field}: burned {old-new}, expected {burned}'
PYNET
    then
        _assert_fail "$label" "same-market scaled supply/debt settlement differs from committed-index reference"
        return 1
    fi
    record "$label" ok assert "" "" "" "" "" "exact scaled supply ceil and debt floor at committed indexes"
}

net_close_refund() {
    python3 - "$1" "$2" "$3" <<'PYCLOSE'
import json,sys
from fractions import Fraction
positions,state=json.loads(sys.argv[1]),json.loads(sys.argv[2])['state']
scale=10**(27-int(sys.argv[3])); ray=10**27
s=int(next(iter(positions[0].values()))['scaled_amount'])
d=int(next(iter(positions[1].values()))['scaled_amount'])
si,bi=int(state['supply_index']),int(state['borrow_index'])
debt=Fraction(d*bi,ray*scale); pay=-(-debt.numerator//debt.denominator)
supply_floor=s*si//(ray*scale)
assert 0 < pay < supply_floor, 'expected full debt settlement with surplus collateral'
burn=Fraction(pay*scale*ray,si); remaining=s-(-(-burn.numerator//burn.denominator))
print(remaining*si//(ray*scale))
PYCLOSE
}

# Independent fixture reference: LIQA/LIQG $0.70, LIQB $1, 7 decimals,
# 75% threshold, 8% minimum bonus, 1% protocol fee. No collateral borrowing.
# Optional fifth argument "credit" returns seized/fee RAY shares, not token units.
liquidation_reference() {
    python3 - "$@" <<'PYLIQ'
import json,sys
positions,supply,debt=map(json.loads,sys.argv[1:4]); offered=int(sys.argv[4])
s=int(next(iter(positions[0].values()))['scaled_amount'])
d=int(next(iter(positions[1].values()))['scaled_amount'])
assert len(positions[0])==len(positions[1])==1
si=int(supply['state']['supply_index']); bi=int(debt['state']['borrow_index'])
R,W,U=10**27,10**18,10**20
half=lambda n,d:(n+d//2)//d
ceil=lambda n,d:(n+d-1)//d
assert si==R
# Credit keeps fractional native units: preserve the protocol's staged WAD rounding.
collateral=half(half(s,10**9)*7,10); weighted=(s//10**9*7//10)*3//4
debt_value=ceil(d*bi,R*10**9); hf=weighted*W//debt_value
assert collateral>=debt_value and 0<hf<W
proportion=half(weighted*W,collateral); bonus=hf*10000//proportion-10000
scale=min(W,half((11*W//10-hf)*W,3*W//10))
threshold=ceil(proportion*10000,W); maximum=10000*(10000-threshold)//threshold
assert 800<=bonus<=800+half((maximum-800)*scale,W)
one=W+bonus*10**14; target=11*W//10
ideal=min(half((half(target*debt_value,W)-weighted)*W,target-half(proportion*one,W)),half(collateral*W,one),debt_value)
assert 0<=debt_value-ideal<5*W
paid=min(offered,ceil(d*bi,R*U))
seizure=half(half(paid*10**11*one,W)*W,7*W//10)*10**9
capped=min(seizure,s); gross=capped//U; principal=seizure*W//one
fee_ray=half(max(0,capped-principal)*100,10000)
fee=min(max(1,fee_ray//U) if fee_ray else 0,max(0,(gross*U-principal)//U))
if len(sys.argv)>5:
    assert sys.argv[5]=='credit'
    print(paid,capped,ceil(max(0,capped-principal)*100,10000))
else:
    print(paid,gross,fee)
PYLIQ
}

assert_liquidation_debt_burn() {
    local label="$1" before="$2" after="$3" sync="$4" paid="$5"
    if ! python3 - "$before" "$after" "$sync" "$paid" <<'PYBURN'
import json,sys
before,after,sync=map(json.loads,sys.argv[1:4]); paid=int(sys.argv[4]); ray=10**27; unit=10**20
old=before[1]; new=after[1]; assert len(old)==1 and set(new)<=set(old)
key=next(iter(old)); d=int(old[key]['scaled_amount']); index=int(sync['state']['borrow_index'])
ceiling=(d*index+ray*unit-1)//(ray*unit)
expected=d if paid>=ceiling else paid*unit*ray//index
assert d-int(new.get(key,{'scaled_amount':0})['scaled_amount'])==expected, 'incorrect liquidation debt burn'
PYBURN
    then
        _assert_fail "$label" "debt burn differs from committed-index reference"; return 1
    fi
    record "$label" ok assert "" "" "" "" "" "exact debt share burn"
}

# LIQG has no borrowing: its index and cash stay fixed during share credit.
assert_liquidation_credit() {
    local label="$1" expected; shift
    expected=$(liquidation_reference "$1" "$5" "$9" "${10}" credit) \
        || { _assert_fail "$label" "Credit fixture outside independent liquidation reference"; return 1; }
    if ! python3 - "$@" "$expected" <<'PYCREDIT'
import json,sys
before,after,recipient_before,recipient_after,pool_before,pool_after,attrs=map(json.loads,sys.argv[1:8])
assert int(attrs['spoke_id'])==int(sys.argv[8]) and int(attrs['mode'])==0
old,new=before[0],after[0]
assert len(old)==1 and set(new)==set(old), 'victim collateral market changed'
key=next(iter(old))
assert [int(old[key][field]) for field in ['liquidation_threshold','liquidation_bonus','liquidation_fees']]==[7500,800,100]
assert set(recipient_before[0])<=set(old) and set(recipient_after[0])==set(old), 'receiver collateral market changed'
assert recipient_before[1]==recipient_after[1]=={}, 'receiver acquired debt'
amount=lambda positions: int(positions.get(key,{'scaled_amount':0})['scaled_amount'])
lost=amount(old)-amount(new)
gained=amount(recipient_after[0])-amount(recipient_before[0])
p,q=pool_before['state'],pool_after['state']
fee=int(q['revenue'])-int(p['revenue'])
assert lost>0 and gained>0 and fee>0 and lost==gained+fee, 'share credit or fee lost'
paid,seized,expected_fee=map(int,sys.argv[11].split())
assert paid==int(sys.argv[10]), 'fixture payment was capped or refunded'
assert lost==seized and fee==expected_fee and gained==seized-expected_fee, 'incorrect Credit seizure or fee allocation'
assert int(p['supply_index'])==int(q['supply_index'])==10**27
assert int(p['borrowed'])==int(q['borrowed'])==0
assert int(p['supplied'])==int(q['supplied']), 'credit changed total supply shares'
assert int(p['cash'])==int(q['cash']), 'credit changed collateral cash'
PYCREDIT
    then
        _assert_fail "$label" "Credit exact seizure/fee, share conservation, pool state, or receiver attributes differ"; return 1
    fi
    record "$label" ok assert "" "" "" "" "" "independent exact seizure and ceiling-rounded bonus fee; share conservation; same-spoke Normal receiver; supply/cash unchanged"
}

# Reconstruct supply/revenue accrual at the transaction's committed borrow
# index. ponytail: one native accrual chunk; replay chunks if fixtures span over a year.
pool_accrual_at_committed_index() {
    python3 - "$@" <<'PYACCRUAL'
import json,sys
before,after=map(json.loads,sys.argv[1:3]); p={k:int(v) for k,v in before['state'].items()}; q=after['state']
R=10**27; CAP=10**36; half=lambda n,d:(n+d//2)//d
assert before['params']==after['params'], 'market params changed'
delta=int(q['last_timestamp'])-p['last_timestamp']
assert 0<=delta<=31556926000, 'fixture requires at most one accrual chunk'
bi=int(q['borrow_index']); assert p['borrow_index']<=bi<=CAP
assert delta>0 or bi==p['borrow_index'], 'borrow index moved without elapsed time'
S,B,si=p['supplied'],p['borrowed'],p['supply_index']; assert S>=0 and B>=0 and si>0
interest=half(B*bi,R)-half(B*p['borrow_index'],R)
reserve=int(before['params']['reserve_factor']); assert 0<=reserve<=10000
fee=half(interest*reserve,10000); rewards=interest-fee
value=half(S*si,R); next_si=si
if S and rewards and value:
    next_si=max(min(si,CAP),min((value+rewards)*R//S,CAP))
shortfall=rewards-(half(S*next_si,R)-value); assert shortfall>=0
shares=min((fee+shortfall)*R//next_si,2**127-1-S)
p.update(borrow_index=bi,supply_index=next_si,supplied=S+shares,revenue=p['revenue']+shares,last_timestamp=int(q['last_timestamp']))
print(json.dumps({'state':p,'params':before['params']}))
PYACCRUAL
}
