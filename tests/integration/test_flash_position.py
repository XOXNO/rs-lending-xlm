#!/usr/bin/env python3
"""Offline rollback regression: run the real snapshot and rejection helpers."""
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
setup = f'''
set -uo pipefail
source "{HERE}/lib/assert.sh"
source "{HERE}/flows/flash_position.sh"
ALICE=alice; ALICE_ADDR=alice-address; CONTROLLER=controller; POOL=pool
POSITION_NFT=nft; XLM_SAC=xlm; USDC_SAC=usdc; PRIMARY_SPOKE_ID=1
FP_RECV=receiver; FP_ACCOUNT_ID=9; FP_DEBT=debt; FP_COLS=collaterals
shares=200; nfts=1; calls=0; failures=0; passed=0; changed_balance=none
log() {{ :; }}
record() {{ if [ "$2" = FAIL ]; then failures=$((failures+1)); else passed=$((passed+1)); fi; }}
view() {{
    case "$4" in
        get_account_positions) printf '[{{}},{{"asset":{{"scaled_amount":"%s"}}}}]\n' "$shares" ;;
        total_supply) echo "$nfts" ;;
        *) return 1 ;;
    esac
}}
balance() {{
    [ "${{unreadable:-0}}" != 1 ] || return 1
    if [ "${{2}}_${{1}}" = "$changed_balance" ]; then echo 99; else echo 100; fi
}}
xfail() {{
    calls=$((calls+1))
    case "$mutation" in
        none) : ;;
        *_xlm|*_usdc) changed_balance="$mutation" ;;
        shares) shares=201 ;;
        nfts) nfts=2 ;;
        read) unreadable=1 ;;
        wrong_error) return 1 ;;
    esac
}}
'''
for mutation in ('none', 'controller_xlm', 'pool_xlm', 'receiver_xlm',
                 'controller_usdc', 'pool_usdc', 'receiver_usdc', 'shares', 'nfts', 'read', 'wrong_error'):
    expected = '''
fp_reject_unchanged callback expected || exit 1
[ "$calls" = 1 ] && [ "$failures" = 0 ] && [ "$passed" = 1 ]
''' if mutation == 'none' else '''
if fp_reject_unchanged callback expected; then exit 1; fi
[ "$calls" = 1 ] && [ "$passed" = 0 ]
if [ "$mutation" != wrong_error ]; then [ "$failures" = 1 ]; fi
'''
    result = subprocess.run(['bash', '-c', setup + f'mutation={mutation}\n' + expected], capture_output=True, text=True)
    assert result.returncode == 0, f'{mutation}: {result.stdout}{result.stderr}'
print('Flash-position rollback checks reject token/share/NFT changes, failed reads, and wrong errors')

# Execute the gate phase with the stale 1-XLM delegate plan left by the
# preceding phase. Mock only invocation/funding boundaries; retain fp_set_plan
# and fp_run so both receiver configuration and controller arguments are checked.
gates_setup = f'''
set -uo pipefail
source "{HERE}/flows/flash_position.sh"
ADMIN=admin; ALICE=alice; ALICE_ADDR=alice-address; CONTROLLER=controller
XLM_SAC=xlm; USDC_SAC=usdc; PRIMARY_HUB_ID=1; PRIMARY_SPOKE_ID=1; ALICE_FP_ACCT=2
FLASH_POSITION_RECEIVER=old-receiver; FLASH_POSITION_RECEIVER_V2=receiver
plan_amount=10000000; funded=0; flash_enabled=true; negatives=0; created=0; done_flag=0
arg() {{
    local key="$1"; shift
    while [ "$#" -gt 1 ]; do
        if [ "$1" = "$key" ]; then printf '%s\\n' "$2"; return 0; fi
        shift
    done
    return 1
}}
phase() {{ :; }}
log() {{ :; }}
die() {{ return 1; }}
hub_key() {{ jq -nc --arg a "$2" --argjson h "$1" '{{asset:$a,hub_id:$h}}'; }}
pay_vec() {{ jq -nc --arg a "$2" --argjson h "$1" --arg n "$3" '[[{{asset:$a,hub_id:$h}},$n]]'; }}
market_params_json() {{ echo '{{"is_flashloanable":true}}'; }}
fp_ensure_token() {{
    [ "$fail_at" != funding ] || return 1
    [ "$FP_RECV" = receiver ] && [ "$1" = xlm ] || return 1
    funded="$2"
}}
inv() {{
    [ "$fail_at" != "$1" ] || return 1
    case "$1" in
        fp_plan_gates_create)
            [ "$3" = receiver ] && [ "$(arg --collateral "$@")" = xlm ] || return 1
            [ "$(arg --mode "$@")" = 0 ] && [ "$(arg --extra_amount "$@")" = 0 ] || return 1
            plan_amount=$(arg --amount "$@")
            [ "$funded" -ge "$plan_amount" ] || return 1 ;;
        fp_usdc_flash_off|fp_restore_usdc_curve)
            flash_enabled=$(arg --params "$@" | jq -r '.is_flashloanable') ;;
        *) return 1 ;;
    esac
}}
xfail() {{
    [ "$fail_at" != "$1" ] || return 1
    case "$1" in
        flash_position_normal_mode_*)
            [ "$2" = 'Error\\(Contract, #111\\)' ] && [ "$(arg --mode "$@")" = 0 ] || return 1 ;;
        flash_position_flash_disabled_*)
            [ "$2" = 'Error\\(Contract, #401\\)' ] && [ "$flash_enabled" = false ] || return 1 ;;
        *) return 1 ;;
    esac
    negatives=$((negatives+1))
}}
inv_create() {{
    [ "$fail_at" != "$1" ] || return 1
    [ "$flash_enabled" = true ] && [ "$(arg --account_id "$@")" = 0 ] || return 1
    [ "$(arg --receiver "$@")" = receiver ] && [ "$(arg --mode "$@")" = 1 ] || return 1
    local minimum debt
    minimum=$(arg --collaterals "$@" | jq -r '.[0][1]')
    debt=$(arg --amount "$@")
    [ "$minimum" = "$plan_amount" ] && [ "$plan_amount" -le "$funded" ] || return 1
    # Conservative live snapshot prices: XLM $0.195, USDC $1.10, LTV 70%.
    # New collateral must cover debt and the $5 minimum-borrow collateral floor.
    [ "$((plan_amount*195*70))" -ge "$((debt*1100*100))" ] || return 1
    [ "$((plan_amount*195*70))" -ge "$((5*10000000*1000*100))" ] || return 1
    created=$((created+1))
    echo 7
}}
save_state() {{ [ "$1" = FP_GATES_DONE ] && [ "$2" = 1 ] || return 1; done_flag=1; }}
'''
for fail_at in ('none', 'funding', 'fp_plan_gates_create', 'flash_position_normal_mode_new',
                'flash_position_normal_mode_ex', 'fp_usdc_flash_off',
                'flash_position_flash_disabled_new', 'flash_position_flash_disabled_ex',
                'fp_restore_usdc_curve', 'flash_position_after_flag_restored'):
    expected = '''
flow_flash_position_gates || exit 1
[ "$negatives" = 4 ] && [ "$created" = 1 ] && [ "$done_flag" = 1 ]
''' if fail_at == 'none' else '''
if flow_flash_position_gates; then exit 1; fi
[ "$done_flag" = 0 ] && [ "$created" = 0 ]
'''
    result = subprocess.run(['bash', '-c', gates_setup + f'fail_at={fail_at}\n' + expected], capture_output=True, text=True)
    assert result.returncode == 0, f'gate {fail_at}: {result.stdout}{result.stderr}'
print('Flash-position gates reset and fund the create plan; every failure prevents completion')

# Run the actual cap/liquidity blocks from both flows, including the real
# _view_pool_int, raw_add and _uint_ge helpers. Avoid unrelated live setup.
flow_source = (HERE/'flows/flash_position.sh').read_text()
for flow, start, end, suffix in (
        ('matrix', '    local supplied cap\n', '    assert_hf_at_least hf_flash_position_matrix_end', ''),
        ('gaps', '    local supplied cap borrowed\n', '    # Raise the USD floor', '_gaps')):
    block = flow_source.split(start, 1)[1].split(end, 1)[0]
    reads = ['fp_xlm_supplied'+suffix, 'fp_usdc_borrowed'+suffix,
             'fp_usdc_cash'+suffix, 'fp_usdc_borrowed_liq'+suffix]
    amount_setup = f'''
set -uo pipefail
source "{HERE}/lib/assert.sh"
PRIMARY_HUB_ID=1; PRIMARY_SPOKE_ID=1; XLM_SAC=xlm; USDC_SAC=usdc
ADMIN=admin; CONTROLLER=controller; POOL=pool
FP_MODE_SUCCESS=0; FP_EXTEND_COLLATERAL=10000000; FP_SMALL_DEBT=10000000
FP_AMOUNT="$FP_SMALL_DEBT"; probe_amount=''; finished=0
hub_key() {{ echo '{{}}'; }}
spoke_args() {{ printf '%s\\n' "${{@}}"; }}
view() {{
    if [ "$1" = "$failed_read" ]; then
        # Even useful-looking output cannot turn a failed RPC into a cap.
        [ "$partial" = 0 ] || echo 123
        return 1
    fi
    case "$4" in
        get_supplied_amount) echo '"100000000000000000000"' ;;
        get_reserves) printf '"%s"\\n' "$cash_value" ;;
        get_borrowed_amount) printf '"%s"\\n' "$borrowed_value" ;;
        *) return 1 ;;
    esac
}}
inv() {{ :; }}
fp_set_plan() {{ :; }}
fp_restore_xlm_listing() {{ :; }}
fp_restore_usdc_listing() {{ :; }}
fp_restore_usdc_curve() {{ :; }}
fp_run() {{
    case "$2" in *insufficient_liquidity*) probe_amount="$FP_AMOUNT";; esac
}}
fp_xfail_pair() {{
    case "$1" in *insufficient_liquidity*) probe_amount="$FP_AMOUNT";; esac
}}
checked_block() {{
{start}{block}
    finished=1
}}
'''
    for failed_read in ['none', *reads]:
        for partial in (0, 1):
            expected = '''
checked_block || exit 1
[ "$finished" = 1 ] && [ "$probe_amount" = 100000000000000000008 ]
[ "$FP_AMOUNT" = "$FP_SMALL_DEBT" ]
''' if failed_read == 'none' else '''
if checked_block; then exit 1; fi
[ "$finished" = 0 ] && [ -z "$probe_amount" ]
'''
            result = subprocess.run(['bash', '-c', amount_setup +
                f'failed_read={failed_read}; partial={partial}; cash_value=100000000000000000000; borrowed_value=7\n' + expected],
                capture_output=True, text=True)
            assert result.returncode == 0, f'{flow}/{failed_read}/{partial}: {result.stdout}{result.stderr}'
    # Zero cash/debt yields one; comparison must retain the existing skip.
    result = subprocess.run(['bash', '-c', amount_setup + '''
failed_read=none; partial=0; cash_value=0; borrowed_value=0
checked_block || exit 1
[ "$finished" = 1 ] && [ -z "$probe_amount" ]
'''], capture_output=True, text=True)
    assert result.returncode == 0, f'{flow}/zero: {result.stdout}{result.stderr}'
print('Flash-position cap/liquidity blocks propagate failed reads and preserve wide integer probes')

# F2: Long and Short reject the debt token among the declared
# collaterals with #7 AssetsAreTheSame. Run the real Long-open block of
# fp_long_multi_collateral against a balance model of the controller: the
# debt-token declaration must be refused with no state change, the open must
# declare only XLM and refund the returned USDC to the caller, and every
# manifest predicate of the block must be recorded. A controller that still
# admits the declaration (the pre-fix behaviour) must fail the block.
import json

long_block = flow_source.split('fp_long_multi_collateral() {\n', 1)[1].split('    inv fp_delegate_manager_on', 1)[0]
long_setup = r'''
set -uo pipefail
source "@HERE@/lib/assert.sh"
source "@HERE@/flows/flash_position.sh"
ALICE=alice; ALICE_ADDR=alice-address; ADMIN=admin; CONTROLLER=controller; POOL=pool
POSITION_NFT=nft; XLM_SAC=xlm; USDC_SAC=usdc; PRIMARY_HUB_ID=1; PRIMARY_SPOKE_ID=1
FLASH_POSITION_RECEIVER=receiver; WAD=1000000000000000000
# Chain state lives in files: the flow reads and writes it from subshells.
model=$(mktemp -d); trap 'rm -rf "$model"' EXIT
declare -A bal=([pool:xlm]=900000000000 [pool:usdc]=900000000000 [receiver:xlm]=2010000000
    [receiver:usdc]=0 [controller:xlm]=41 [controller:usdc]=37 [alice-address:xlm]=0 [alice-address:usdc]=5)
declare -A credit=() pushed=()
plan_amount=0; plan_extra=xlm; plan_extra_amount=0; debt_amount=0; rejected=0; created=0; finished=0
save() {
    declare -p bal credit plan_amount plan_extra plan_extra_amount debt_amount rejected created \
        | sed -e 's/^declare -- /declare -g /' -e 's/^declare -A /declare -gA /' > "$model/state"
}
load() { source "$model/state"; }
save; : > "$model/records"
log() { :; }
record() { printf '%s %s\n' "$1" "$2" >> "$model/records"; }
failures() { awk '$2 == "FAIL" || $2 == "UNEXPECTED-OK"' "$model/records" | wc -l; }
recorded() { awk '{print $1}' "$model/records"; }
save_state() { :; }
hub_key() { jq -nc --argjson h "$1" --arg a "$2" '{hub_id:$h, asset:$a}'; }
pay_vec() {
    local hub_id="$1" out="[" first=1
    shift
    while [ $# -gt 0 ]; do
        [ $first -eq 0 ] && out+=","
        out+="[{\"hub_id\":$hub_id,\"asset\":\"$1\"},\"$2\"]"
        first=0
        shift 2
    done
    echo "$out]"
}
arg() {
    local key="$1"; shift
    while [ "$#" -gt 1 ]; do
        if [ "$1" = "$key" ]; then printf '%s\n' "$2"; return 0; fi
        shift
    done
    return 1
}
balance() { load; printf '%s\n' "${bal[$2:$1]}"; }
fp_ensure_token() { load; [ "$1" = xlm ] && [ "${bal[receiver:xlm]}" -ge "$2" ] && record "$3" ok; }
inv() {
    case "$1" in
        fp_plan_long_multi)
            [ "$3" = receiver ] && [ "$(arg --collateral "$@")" = xlm ] || return 1
            load
            plan_amount=$(arg --amount "$@")
            plan_extra=$(arg --extra "$@")
            plan_extra_amount=$(arg --extra_amount "$@")
            save ;;
        *) return 1 ;;
    esac
    record "$1" ok
}
# Controller model: the F2 rule, then mint and forward, the receiver's plan,
# deposit of declared receipts and refund of refund-listed receipts.
admits() {
    local mode debt
    mode=$(arg --mode "$@"); debt=$(arg --debt "$@" | jq -r .asset)
    if [ "$mode" != 1 ] && arg --collaterals "$@" | jq -e --arg d "$debt" 'any(.[]; .[0].asset == $d)' >/dev/null; then
        [ "$prefix_controller" = 1 ] || return 1
    fi
}
push() {
    [ "$2" -gt 0 ] || return 0
    bal[receiver:$1]=$((bal[receiver:$1]-$2)); bal[controller:$1]=$((bal[controller:$1]+$2))
    pushed[$1]=$((${pushed[$1]:-0}+$2))
}
apply_flash() {
    local amount debt a v
    load; pushed=()
    amount=$(arg --amount "$@"); debt=$(arg --debt "$@" | jq -r .asset)
    bal[pool:$debt]=$((bal[pool:$debt]-amount)); bal[receiver:$debt]=$((bal[receiver:$debt]+amount))
    debt_amount=$amount
    push xlm "$plan_amount"; push "$plan_extra" "$plan_extra_amount"
    for a in $(arg --collaterals "$@" | jq -r '.[][0].asset'); do
        v=${pushed[$a]:-0}; pushed[$a]=0
        bal[controller:$a]=$((bal[controller:$a]-v)); bal[pool:$a]=$((bal[pool:$a]+v))
        credit[$a]=$((${credit[$a]:-0}+v))
    done
    for a in $(arg --refund_assets "$@" | jq -r '.[]'); do
        v=${pushed[$a]:-0}; pushed[$a]=0
        bal[controller:$a]=$((bal[controller:$a]-v)); bal[alice-address:$a]=$((bal[alice-address:$a]+v))
    done
    save
}
xfail() {
    local label="$1" pattern="$2"; shift 4
    [ "$1" = -- ] && shift
    [ "$1" = flash_position ] || return 1
    if admits "$@"; then apply_flash "$@"; record "$label" UNEXPECTED-OK; return 1; fi
    [ "$pattern" = 'Error\(Contract, #7\)' ] || { record "$label" FAIL; return 1; }
    load; rejected=$((rejected+1)); save
    record "$label" xfail
}
inv_create() {
    local label="$1"; shift 3
    [ "$1" = -- ] && shift
    [ "$1" = flash_position ] && [ "$(arg --mode "$@")" = 2 ] && [ "$(arg --account_id "$@")" = 0 ] || return 1
    admits "$@" || { record "$label" FAIL; return 1; }
    apply_flash "$@"
    load; created=$((created+1)); save
    record "$label" ok
    echo 11
}
view() {
    local label="$1" a; shift 2
    [ "$1" = -- ] && shift
    load
    record "$label" read
    case "$1" in
        get_account_positions) echo '[{},{}]' ;;
        total_supply) echo '"0"' ;;
        get_collateral_amount) a=$(arg --hub_asset "$@" | jq -r .asset); printf '"%s"\n' "${credit[$a]:-0}" ;;
        get_borrow_amount) printf '"%s"\n' "$debt_amount" ;;
        get_health_factor) echo '"2000000000000000000"' ;;
        get_account_attributes) echo '{"mode":2,"spoke_id":1}' ;;
        *) return 1 ;;
    esac
}
checked_long_block() {
@BLOCK@
    finished=1
}
'''.replace('@HERE@', str(HERE)).replace('@BLOCK@', long_block)
gaps = next(c for c in json.loads((HERE/'cases.json').read_text()) if c['id'] == 'flow_flash_position_gaps')
long_labels = sorted(r['label'] for r in gaps['required_actions']
                     if r['label'].startswith(('fp_long_', 'flash_position_long_')) or r['label'] == 'fp_plan_long_multi')
assert 'flash_position_long_debt_collateral' in long_labels and 'fp_long_usdc_refund' in long_labels, long_labels
assert 'fp_long_usdc_credit' not in long_labels, 'the debt token is no longer Long collateral'
result = subprocess.run(['bash', '-c', long_setup + 'prefix_controller=0\n' + '''
checked_long_block || exit 1
load
[ "$finished" = 1 ] && [ "$(failures)" = 0 ] && [ "$rejected" = 1 ] && [ "$created" = 1 ] || exit 2
[ "${credit[usdc]:-0}" = 0 ] && [ "${credit[xlm]}" = 2000000000 ] || exit 3
[ "${bal[alice-address:usdc]}" = 10000005 ] && [ "${bal[pool:usdc]}" = 899990000000 ] || exit 4
[ "${bal[controller:usdc]}" = 37 ] && [ "${bal[controller:xlm]}" = 41 ] || exit 5
recorded
'''], capture_output=True, text=True)
assert result.returncode == 0, f'long block: {result.returncode} {result.stdout}{result.stderr}'
missing = sorted(set(long_labels) - set(result.stdout.split()))
assert not missing, f'long block does not record manifest predicates {missing}'
result = subprocess.run(['bash', '-c', long_setup + 'prefix_controller=1\n' + '''
if checked_long_block; then exit 1; fi
load
[ "$finished" = 0 ] && [ "$created" = 0 ] && [ "$(failures)" -ge 1 ]
'''], capture_output=True, text=True)
assert result.returncode == 0, f'long block on a controller that admits the debt token: {result.stdout}{result.stderr}'
print('Flash-position Long open refuses the debt token as collateral (#7), refunds it, and records every manifest predicate')
