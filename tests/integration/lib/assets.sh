sac_live() {
    cli_read stellar contract invoke --instruction-leeway "${INSTRUCTION_LEEWAY:-20000000}" --id "$1" --source "$ADMIN" "${NET_ARGS[@]}" --send=no \
        -- decimals >/dev/null 2>&1
}

sac_wait_live() {
    local probe
    for probe in $(seq 1 10); do
        sac_live "$1" && return 0
        sleep 2
    done
    return 1
}

issue_sac() {
    local var="$1" code="$2"
    if [ -n "${!var:-}" ]; then return 0; fi
    local asset="$code:$ADMIN_ADDR"
    local out_f="$LOG_DIR/sac_$code.out" err_f="$LOG_DIR/sac_$code.err"
    local sac hash
    sac=$(stellar contract id asset --asset "$asset" "${NET_ARGS[@]}")
    if sac_live "$sac"; then

        record "issue_sac_$code" ok "asset_id" "" "" "" "" "" "$sac (pre-existing)"
    else

        if ! run_deploy "$out_f" "$err_f" -- stellar contract asset deploy --asset "$asset" --source "${E2E_SRC:-$ADMIN}" "${NET_ARGS[@]}"; then
            record "issue_sac_$code" FAIL asset_deploy "$(extract_signing_hash "$err_f")" "" "" "" "" "SAC deployment unconfirmed; never resubmit"
            return 1
        fi
        hash=$(extract_signing_hash "$err_f")
        if [ "$(sanitize_output "$out_f")" != "$sac" ]; then
            record "issue_sac_$code" FAIL asset_deploy "$hash" "" "" "" "" "deployed SAC id differs from $sac"
            return 1
        fi
        if ! sac_wait_live "$sac"; then
            die "issue_sac_$code" \
                "SAC $code not live after $DEPLOY_ATTEMPTS deploy attempt(s): $(tail_err_note "$err_f" 200)"
        fi
        record "issue_sac_$code" ok asset_deploy "$hash" "$RES_INSTR" "$RES_READ" "$RES_WRITE" "$RES_FEE" "$sac" deployment "$sac"
    fi
    save_state "$var" "$sac"
    log "SAC $code = $sac"
}

classic_batch() {
    local label="$1" fn="$2" signer="$3"; shift 3
    [ -z "${E2E_JOB:-}" ] || { job_refuse classic_batch "$label"; return 1; }
    local base="$LOG_DIR/$label" per_op="${E2E_CLASSIC_OP_FEE:-${STELLAR_INCLUSION_FEE:-1000}}" count=$# i=0 item kind a b c d addr hash rc=0 st sequence
    local -a op
    if [ "$count" -lt 1 ] || [ "$count" -gt 40 ] || [[ ! "$per_op" =~ ^[1-9][0-9]{0,6}$ ]]; then
        record "$label" FAIL "$fn" "" "" "" "" "" "classic batch refused: $count items (allowed 1-40), per-op fee '$per_op'"
        return 1
    fi
    : >"$base.build.err"
    for item in "$@"; do
        IFS=: read -r kind a b c d <<<"$item"
        case "$kind" in
            trust) op=(change-trust --line "$a:$b") ;;
            pay) op=(payment --destination "$a" --asset "$b:$c" --amount "$d") ;;
            *) record "$label" FAIL "$fn" "" "" "" "" "" "unknown classic batch item '$item'"; return 1 ;;
        esac
        if [ "$i" -eq 0 ]; then
            stellar tx new "${op[@]}" --build-only --source-account "$signer" "${NET_ARGS[@]}" \
                >"$base.built.xdr" 2>>"$base.build.err" || rc=$?
        else
            stellar tx op add "${op[@]}" --source-account "$signer" <"$base.built.xdr" >"$base.next.xdr" 2>>"$base.build.err" \
                && mv "$base.next.xdr" "$base.built.xdr" || rc=$?
        fi
        i=$((i + 1))
        [ "$rc" -eq 0 ] || { record "$label" FAIL "$fn" "" "" "" "" "" "classic batch build failed at item $i: $(tail_err_note "$base.build.err")"; return 1; }
    done
    stellar tx decode <"$base.built.xdr" 2>>"$base.build.err" \
        | jq -c --argjson fee "$((per_op * count))" '.tx.tx.fee = $fee' \
        | stellar tx encode >"$base.unsigned.xdr" 2>>"$base.build.err" \
        && addr=$(stellar keys address "$signer" 2>>"$base.build.err") \
        && stellar tx decode <"$base.unsigned.xdr" >"$base.unsigned.json" 2>>"$base.build.err" \
        && python3 - "$base.unsigned.json" "$addr" "$((per_op * count))" "$fn" "$@" 2>>"$base.build.err" <<'PYCLASSIC' \
        && stellar tx sign --sign-with-key "$signer" "${NET_ARGS[@]}" <"$base.unsigned.xdr" >"$base.signed.xdr" 2>>"$base.build.err" \
        && hash=$(stellar tx hash --network-passphrase "$NETWORK_PASSPHRASE" <"$base.signed.xdr" 2>>"$base.build.err") \
        && [[ "$hash" =~ ^[0-9a-f]{64}$ ]] || rc=$?
import json, sys
path, source, fee, method, *items = sys.argv[1:]
tx = json.load(open(path))['tx']['tx']
def asset(code, issuer):
    return {'credit_alphanum4' if len(code) <= 4 else 'credit_alphanum12': {'asset_code': code, 'issuer': issuer}}
expected = []
for item in items:
    kind, *fields = item.split(':')
    if (kind, len(fields), method) == ('trust', 2, 'change_trust'):
        expected.append({'change_trust': {'line': asset(*fields), 'limit': '9223372036854775807'}})
    elif (kind, len(fields), method) == ('pay', 4, 'payment'):
        expected.append({'payment': {'destination': fields[0], 'asset': asset(*fields[1:3]), 'amount': fields[3]}})
    else:
        sys.exit(f'item {item} does not match {method}')
problems = [name for name, good in [
    ('source', tx['source_account'] == source),
    ('fee', tx['fee'] == int(fee)),
    ('operation count', len(tx['operations']) == len(items)),
    ('operation source', all(op['source_account'] is None for op in tx['operations'])),
    ('operation bodies', [op['body'] for op in tx['operations']] == expected)] if not good]
if problems:
    sys.exit('pre-sign mismatch: ' + ', '.join(problems))
PYCLASSIC
    if [ "$rc" -ne 0 ]; then
        record "$label" FAIL "$fn" "" "" "" "" "" "classic batch not sent; build, verification or signing failed: $(tail_err_note "$base.build.err")"
        return 1
    fi
    sequence=$(wc -l < "$ACTIONS_TSV")
    begin_attempt "$label" "$fn" "$sequence" 1 "$base.out" "$base.err" "" || return 1
    printf 'Signing transaction: %s\n' "$hash" >>"$base.err" || return 1
    stellar tx send "${NET_ARGS[@]}" <"$base.signed.xdr" >"$base.out" 2>>"$base.err" || rc=$?
    record_attempt "$label" "$fn" "$sequence" 1 "$rc" "$hash" "$base.out" "$base.err" "" || return 1
    st=$(tx_status "$hash")
    if [ "$st" = SUCCESS ]; then
        record "$label" ok "$fn" "$hash" "" "" "" "" "$count ops" classic_transaction
        return 0
    fi
    record "$label" FAIL "$fn" "$hash" "" "" "" "" "unconfirmed classic transaction; never resubmit: status=$st cli=$rc; $(tail_err_note "$base.err")"
    return 1
}

trustline() {
    local wallet="$1" code="$2" issuer="$3"
    classic_batch "trust_${code}_${wallet%%_e2e*}" change_trust "$wallet" "trust:$code:$issuer"
}

mint_to() {
    local sac="$1" code="$2" to="$3" amount="$4"

    local bal
    bal=$(balance "$sac" "$to" 2>/dev/null)
    if [[ "$bal" =~ ^[0-9]+$ ]] && _uint_ge "$bal" "$amount"; then
        record "mint_${code}_to_${to:0:6}" ok mint "" "" "" "" "" "holder already funded (resume); skipping mint"
        return 0
    fi
    INV_TRANSIENT_CONTRACT_RE='trustline entry is missing' \
        inv "mint_${code}_to_${to:0:6}" "$ADMIN" "$sac" -- mint --to "$to" --amount "$amount" >/dev/null
}

balance() {
    local sac="$1" who="$2" value label="balance_${1:0:8}_${2:0:8}"
    value=$(view "$label" "$sac" -- balance --id "$who") || return 1
    value=$(tr -d '\"[:space:]' <<<"$value")
    [[ "$value" =~ ^[0-9]+$ ]] || { _assert_fail "$label" "invalid balance '$value'"; return 1; }
    printf '%s\n' "$value"
}

sac_transfer() {
    local signer="$1" sac="$2" from="$3" to="$4" amount="$5" label="$6"
    inv "$label" "$signer" "$sac" -- transfer --from "$from" --to "$to" --amount "$amount" >/dev/null
}

swap_xlm_to() (
    local wallet="$1" addr="$2" to_sac="$3" amount_in="$4" label="$5"
    [ -z "${E2E_JOB:-}" ] || { job_refuse swap_xlm_to "$label"; return 1; }
    local swap_hex AGGREGATOR_MIN_LEDGER rc=0 hash pending="$INTEG_DIR/runs/.external-funding.${RUN_TS}.pending.json"
    # ponytail: one checkout-wide funding lock; use per-pool locks if throughput matters.
    # The subshell retains fd 9 through confirmation; exit/cancellation releases it.
    exec 9>"$INTEG_DIR/runs/.external-funding.lock" || { _assert_fail "$label" 'cannot open funding lock'; return 1; }
    python3 -c 'import fcntl; fcntl.flock(9, fcntl.LOCK_EX)' || { _assert_fail "$label" 'funding lock failed'; return 1; }
    [ ! -e "$pending" ] || { _assert_fail "$label" "earlier funding submission unresolved; reconcile evidence in $pending before removing it"; return 1; }
    # The quote indexer must include trades confirmed by the preceding holder.
    if ! rpc_post 30 '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' >"$LOG_DIR/$label.funding-ledger.json"; then
        _assert_fail "$label" 'funding ledger transport failed'; return 1
    fi
    AGGREGATOR_MIN_LEDGER=$(jq -er 'select(.jsonrpc=="2.0" and .id==1 and (has("error")|not)) | .result.sequence | select(type=="number" and .>0 and floor==.)' \
        "$LOG_DIR/$label.funding-ledger.json") || { _assert_fail "$label" 'invalid funding ledger'; return 1; }
    swap_hex=$(agg_route_hex "$XLM_SAC" "$to_sac" "$amount_in") || {
        record "$label" FAIL execute_strategy "" "" "" "" "" "no aggregator route"
        return 1
    }
    # A cancelled/unknown submission can still commit after releasing the lock.
    # Quarantine subsequent funding until that operation has a terminal receipt.
    jq -nc --arg label "$label" --arg logs "$LOG_DIR" '{label:$label,logs:$logs}' >"$pending" \
        || { _assert_fail "$label" 'cannot record pending funding'; return 1; }
    inv "$label" "$wallet" "$AGGREGATOR" -- execute_strategy \
        --sender "$addr" --total_in "$amount_in" --swap_xdr "$swap_hex" >/dev/null || rc=$?
    if [ "$rc" -eq 0 ]; then
        rm "$pending" || return 1
    else
        hash=$(extract_signing_hash "$LOG_DIR/$label.err")
        if [[ "$hash" =~ ^[0-9a-f]{64}$ ]] && jq -e --arg h "$hash" \
            '.jsonrpc=="2.0" and .id==1 and (has("error")|not) and .result.txHash==$h and (.result.status=="SUCCESS" or .result.status=="FAILED")' \
            "$LOG_DIR/$hash.receipt.json" >/dev/null 2>&1; then
            rm "$pending" || return 1
        fi
    fi
    return "$rc"
)

# Compare economic token movement separately from Stellar transaction fees.
# Raw balance() remains unchanged; fee evidence is retained beside receipts.
financial_balance() {
    local token="$1" address="$2" raw
    raw=$(balance "$token" "$address") || return 1
    if [ "$token" = "$XLM_SAC" ] && [[ "$address" = G* ]]; then
        python3 "$INTEG_DIR/network_fees.py" "$LOG_DIR" "$address" "$raw"
    else
        printf '%s\n' "$raw"
    fi
}
