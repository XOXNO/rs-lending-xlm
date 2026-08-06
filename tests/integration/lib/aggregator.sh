agg_route_hex() {
    local from="$1" to="$2" amount_in="$3" slippage="${4:-0.05}"
    local max_hops="${AGGREGATOR_MAX_HOPS:-2}"
    local quote_f="$LOG_DIR/quote_$(date +%s%N).json"

    local hdr=()
    [ -n "${AGGREGATOR_HEADER:-}" ] && hdr=(-H "$AGGREGATOR_HEADER")

    local try hops
    for try in 1 2 3 4; do
        curl -s -m 30 "${hdr[@]+"${hdr[@]}"}" "$AGGREGATOR_API/quote?from=$from&to=$to&amount_in=$amount_in&slippage=$slippage&max_splits=1&max_hops=$max_hops" \
            >"$quote_f" || return 1
        hops=$(jq -r '.hops | length' "$quote_f" 2>/dev/null)
        [ "$hops" = "1" ] && break
        sleep 2
    done
    local xdr
    xdr=$(jq -r '.routeXdr // empty' "$quote_f")
    [ -z "$xdr" ] && { log "no route: $(head -c 200 "$quote_f")"; return 1; }
    echo "$xdr" | base64 -d | xxd -p | tr -d '\n'
}
