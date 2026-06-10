# Swap-aggregator route API client.
#
# Routes MUST be constrained to max_splits=1 — multi-hop split payloads blow
# the Soroban per-tx budget inside strategy calls (Error(Budget,ExceededLimit)).
# The returned routeXdr is a base64 ScVal; strategy endpoints and
# execute_strategy take it as raw Bytes, which the CLI accepts as hex.

# Forward quote: spend exactly amount_in of <from>.
# Prints the route as hex bytes; quote JSON saved for inspection.
#   agg_route_hex <from-sac> <to-sac> <amount_in> [slippage-fraction, e.g. 0.05 = 5%]
agg_route_hex() {
    local from="$1" to="$2" amount_in="$3" slippage="${4:-0.05}"
    local quote_f="$LOG_DIR/quote_$(date +%s%N).json"
    # Small amounts sometimes quote through stale multi-hop paths whose
    # middle pools cannot meet min-out on-chain — prefer a direct route.
    local try hops
    for try in 1 2 3 4; do
        curl -s -m 30 "$AGGREGATOR_API/quote?from=$from&to=$to&amount_in=$amount_in&slippage=$slippage&max_splits=1" \
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

# Reverse quote: receive at least amount_out of <to>. Sets AGG_AMOUNT_IN.
#   agg_route_hex_out <from-sac> <to-sac> <amount_out> [slippage-fraction, e.g. 0.05 = 5%]
agg_route_hex_out() {
    local from="$1" to="$2" amount_out="$3" slippage="${4:-0.05}"
    local quote_f="$LOG_DIR/quote_$(date +%s%N).json"
    curl -s -m 30 "$AGGREGATOR_API/quote?from=$from&to=$to&amount_out=$amount_out&slippage=$slippage&max_splits=1" \
        >"$quote_f" || return 1
    local xdr
    xdr=$(jq -r '.routeXdr // empty' "$quote_f")
    [ -z "$xdr" ] && { log "no route: $(head -c 200 "$quote_f")"; return 1; }
    AGG_AMOUNT_IN=$(jq -r '.amountIn' "$quote_f")
    echo "$xdr" | base64 -d | xxd -p | tr -d '\n'
}

# Expected output of a forward quote (for sizing follow-up calls).
#   agg_quote_out <from-sac> <to-sac> <amount_in>  → prints amountOut
agg_quote_out() {
    local from="$1" to="$2" amount_in="$3"
    curl -s -m 30 "$AGGREGATOR_API/quote?from=$from&to=$to&amount_in=$amount_in&slippage=0.05&max_splits=1" \
        | jq -r '.amountOut // empty'
}
