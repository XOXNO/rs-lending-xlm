agg_route_hex() {
    local from="$1" to="$2" amount_in="$3" slippage="${4:-0.05}"
    local max_hops="${AGGREGATOR_MAX_HOPS:-2}"
    local quote_f="$LOG_DIR/quote_$(date +%s%N).json"

    local hdr=()
    [ -n "${AGGREGATOR_HEADER:-}" ] && hdr=(-H "$AGGREGATOR_HEADER")

    local try hops
    for try in 1 2 3 4; do
        curl --fail-with-body -sS -m 30 "${hdr[@]+"${hdr[@]}"}" "$AGGREGATOR_API/quote?from=$from&to=$to&amount_in=$amount_in&slippage=$slippage&max_splits=1&max_hops=$max_hops" \
            >"$quote_f" || { _assert_fail quote_transport "quote request failed: $quote_f"; return 1; }
        hops=$(jq -r '.hops | length' "$quote_f" 2>/dev/null)
        [ "$hops" = "1" ] && break
        sleep 2
    done
    local xdr
    xdr=$(jq -r '.routeXdr // empty' "$quote_f")
    [ -z "$xdr" ] && { _assert_fail quote_route "missing route: $quote_f"; return 1; }
    python3 - "$xdr" <<'PYROUTE'
import base64,sys
route=base64.b64decode(sys.argv[1],validate=True)
assert route, 'empty route'
print(route.hex())
PYROUTE
    [ "$?" -eq 0 ] || { _assert_fail quote_xdr "invalid route encoding: $quote_f"; return 1; }
}

# Keep the API's venue route, changing only its documented referral header.
# This is route XDR, never a prepared or signed transaction envelope.
agg_referral_route_hex() {
    python3 - "$1" "$2" <<'PYROUTE'
import base64,json,subprocess,sys
route=bytes.fromhex(sys.argv[1]); referral=int(sys.argv[2])
assert 0<referral<2**32
value=json.loads(subprocess.check_output(['stellar','xdr','decode','--type','ScVal','--output','json'],input=base64.b64encode(route),stderr=subprocess.PIPE))
fields={e['key']['symbol']:e['val'] for e in value['map']}
assert set(fields)=={'amounts','assets','ops'}
ops=bytearray.fromhex(fields['ops']['bytes']); assert len(ops)>=10 and ops[0]==1
ops[4:8]=referral.to_bytes(4,'big'); fields['ops']['bytes']=ops.hex()
encoded=subprocess.check_output(['stellar','xdr','encode','--type','ScVal'],input=json.dumps(value).encode(),stderr=subprocess.PIPE)
print(base64.b64decode(encoded.strip(),validate=True).hex())
PYROUTE
}
