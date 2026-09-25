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
