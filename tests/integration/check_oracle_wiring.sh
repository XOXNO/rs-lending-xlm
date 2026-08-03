#!/usr/bin/env bash


set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
fail=0

echo "== legacy ABI symbols (executable paths) =="
if grep -RIn --exclude-dir=runs \
    -e 'set_oracle_config' \
    -e 'resolve_market_oracle_config' \
    -e 'ConfigureMarketOracle' \
    tests/integration/lib tests/integration/flows tests/integration/scenarios \
    tests/integration/README.md configs/script.sh 2>/dev/null; then
    echo "FAIL: legacy oracle ABI symbols still present"
    fail=1
else
    echo "OK: no legacy symbols in harness/script executable paths"
fi

echo "== markets.json AssetOracle schema =="
for net in testnet mainnet; do
    f="configs/$net/markets.json"
    if ! jq -e '
        .markets | all(
            (.oracle | has("sources"))
            and ((.oracle.sources | length) == 1 or (.oracle.sources | length) == 2)
            and (.oracle | has("tolerance"))
            and (.oracle | has("independence"))
            and (.oracle | has("max_price_stale_seconds"))
            and (.oracle | has("primary") | not)
            and (.oracle | has("anchor") | not)
            and (.oracle | has("strategy") | not)
            and (.oracle | has("tolerance_bps") | not)
            and (
                (.oracle.max_price_stale_seconds) as $ceil
                | all(.oracle.sources[]; (.Feed.max_stale_seconds // 0) <= $ceil)
            )
        )
    ' "$f" >/dev/null; then
        echo "FAIL: $f oracle schema invalid"
        fail=1
    else
        echo "OK: $f"
    fi
done

echo "== protocol.sh builders emit sources =="

builder_snip=$(mktemp)
awk '
  /^price_key_token\(\)/ {p=1}
  /^oracle_tolerance_band\(\)/ {p=1}
  /^oracle_cfg_mock_single\(\)/ {p=1}
  /^oracle_cfg_mock_dual\(\)/ {p=1}
  /^oracle_cfg_reflector\(\)/ {p=1}
  p {print}
  /^}$/ && p {print ""; p=0}
' tests/integration/lib/protocol.sh > "$builder_snip"

source "$builder_snip"
rm -f "$builder_snip"
export MOCK="${MOCK:-CMOCKREFLECTORXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX}"
export MOCKRS="${MOCKRS:-CMOCKREDSTONEXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX}"
export REFLECTOR_CEX="${REFLECTOR_CEX:-CCYOZJCOPG34LLQQ7N24YXBM7LL62R7ONMZ3G6WZAAYPB5OYKOMJRN63}"
if ! type price_key_token >/dev/null 2>&1; then
    echo "FAIL: price_key_token not extracted from protocol.sh"
    fail=1
else
    key=$(price_key_token "CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA")
    echo "$key" | jq -e '.Token' >/dev/null
    single=$(oracle_cfg_mock_single "CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA")
    dual=$(oracle_cfg_mock_dual "CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA" "USDC")
    refl=$(oracle_cfg_reflector "XLM" "45000000000000000" "500000000000000000")
    for name in single dual refl; do
        val="${!name}"
        if ! printf '%s' "$val" | jq -e '
            has("sources") and has("tolerance") and has("independence")
            and (has("primary") | not) and (has("strategy") | not)
        ' >/dev/null; then
            echo "FAIL: oracle_cfg builder $name missing AssetOracle fields"
            fail=1
        fi
    done
    if ! printf '%s' "$dual" | jq -e '(.sources | length) == 2' >/dev/null; then
        echo "FAIL: dual builder must emit 2 sources"
        fail=1
    fi
    if ! printf '%s' "$single" | jq -e '(.sources | length) == 1' >/dev/null; then
        echo "FAIL: single builder must emit 1 source"
        fail=1
    fi
    echo "OK: oracle_cfg builders"
fi

if [ "$fail" -ne 0 ]; then
    echo "check_oracle_wiring FAILED"
    exit 1
fi
echo "check_oracle_wiring PASSED"
exit 0
