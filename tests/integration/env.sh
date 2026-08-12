INTEG_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$INTEG_DIR/../.." && pwd)"

NETWORK="${NETWORK:-testnet}"
EXPLORER_TX="${EXPLORER_TX:-https://stellar.expert/explorer/testnet/tx}"
AGGREGATOR_API="${AGGREGATOR_API:-https://testnet-stellar-swap.xoxno.com/api/v1}"

NETWORKS_FILE="${NETWORKS_FILE:-$REPO_ROOT/configs/networks.json}"

# An unknown NETWORK must not fall through to another network's addresses: the
# run would talk to the wrong contracts and report the mismatch as protocol
# failures.
if ! jq -e --arg n "$NETWORK" 'has($n)' "$NETWORKS_FILE" >/dev/null 2>&1; then
    echo "env.sh: NETWORK '$NETWORK' has no entry in $NETWORKS_FILE" >&2
    echo "        known: $(jq -r 'keys | join(", ")' "$NETWORKS_FILE" 2>/dev/null)" >&2
    exit 1
fi

net_field() { jq -r --arg n "$NETWORK" --arg f "$1" '.[$n][$f] // empty' "$NETWORKS_FILE" 2>/dev/null; }

# The public RPC drops heavy multi-hop swap submissions, which strands the run
# with no USDC. Take the endpoint the network config already declares.
RPC_URL="${RPC_URL:-$(net_field rpc_url)}"
: "${RPC_URL:?no rpc_url for network '$NETWORK' in $NETWORKS_FILE}"

AGGREGATOR="${AGGREGATOR:-$(net_field aggregator)}"
: "${AGGREGATOR:?no aggregator for network '$NETWORK' in $NETWORKS_FILE}"

NETWORK_PASSPHRASE="${NETWORK_PASSPHRASE:-$(net_field network_passphrase)}"
: "${NETWORK_PASSPHRASE:?no network_passphrase for network '$NETWORK' in $NETWORKS_FILE}"

# Pass the endpoint explicitly. `--network <name>` would resolve through the
# stellar CLI's own config instead, so RPC_URL would not reach the CLI at all.
NET_ARGS=(--rpc-url "$RPC_URL" --network-passphrase "$NETWORK_PASSPHRASE")
REFLECTOR_CEX="${REFLECTOR_CEX:-CCYOZJCOPG34LLQQ7N24YXBM7LL62R7ONMZ3G6WZAAYPB5OYKOMJRN63}"
USDC_SAC="${USDC_SAC:-CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA}"
EURC_SAC="${EURC_SAC:-CCUUDM434BMZMYWYDITHFXHDMIVTGGD6T2I5UKNX5BSLXLW7HVR4MCGZ}"

WASM_DIR="${WASM_DIR:-$REPO_ROOT/target/optimized}"

RUN_TS="${RUN_TS:?set RUN_TS=<unique-run-name> (e.g. \$(date +%Y%m%d-%H%M%S))}"
RUN_DIR="$INTEG_DIR/runs/$RUN_TS"
STATE_ENV="$RUN_DIR/state.env"
ACTIONS_TSV="$RUN_DIR/actions.tsv"
LOG_DIR="$RUN_DIR/logs"

INTEG_MIN_DELAY="${INTEG_MIN_DELAY:-1}"

XLM_FUND_STROOPS=100000000000
WAD=1000000000000000000
RAY=1000000000000000000000000000

REQUIRED_TOOLS="jq xxd stellar curl base64 awk grep tr"

STELLAR_CLI_MIN_VERSION="22.0"

STRESS_N=20
STRESS_UNIT=10000000

LIQ_CODES=(LIQA LIQB LIQC LIQD LIQE LIQF LIQG)
LIQ_UNIT=10000000

LIQ20_TX_CAP="${LIQ20_TX_CAP:-400000000}"
LIQ20_DEFAULT_REPAY_EACH="${LIQ20_DEFAULT_REPAY_EACH:-$((3000 * STRESS_UNIT))}"
LIQ20_DEFAULT_LEEWAY="${LIQ20_DEFAULT_LEEWAY:-8000000}"

DFX_UNIT=10000000
