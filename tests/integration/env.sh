# Network constants and per-run environment for the live-testnet harness.
# Sourced by every scenario; safe to source multiple times.

INTEG_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$INTEG_DIR/../.." && pwd)"

NETWORK="${NETWORK:-testnet}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
EXPLORER_TX="${EXPLORER_TX:-https://stellar.expert/explorer/testnet/tx}"
AGGREGATOR_API="${AGGREGATOR_API:-https://testnet-stellar-swap.xoxno.com/api/v1}"

# Pre-existing testnet infrastructure (from configs/networks.json).
# Load aggregator (swap router) from networks.json so it stays in sync with
# the deployed contract used by the controller for strategies + test funding.
NETWORKS_FILE="${NETWORKS_FILE:-$REPO_ROOT/configs/networks.json}"
if [ -z "${AGGREGATOR:-}" ]; then
  AGGREGATOR=$(jq -r ".\"$NETWORK\".aggregator // empty" "$NETWORKS_FILE" 2>/dev/null || echo "")
fi
# Fallback to the known current testnet value if json read fails.
AGGREGATOR="${AGGREGATOR:-CDVGOXSKBQUCYD2MLCJNMJGLQSAPSRMESYRHXQETL7O72NK2H577TH3O}"
REFLECTOR_CEX="${REFLECTOR_CEX:-CCYOZJCOPG34LLQQ7N24YXBM7LL62R7ONMZ3G6WZAAYPB5OYKOMJRN63}"
USDC_SAC="${USDC_SAC:-CBIELTK6YBZJU5UP2WWQEUCYKLPU6AUNZ2BQ4WWFEIE3USCIHMXQDAMA}"
EURC_SAC="${EURC_SAC:-CCUUDM434BMZMYWYDITHFXHDMIVTGGD6T2I5UKNX5BSLXLW7HVR4MCGZ}"

# Deploy-sized WASM for controller/pool/flash receiver + optimized mocks.
# Default layout from `make integration-wasm` (see target/optimized/).
WASM_DIR="${WASM_DIR:-$REPO_ROOT/target/optimized}"

# Single run directory keyed by RUN_TS. Re-using a RUN_TS resumes that run
# (state.env is sourced back, so contract addresses survive across phases).
RUN_TS="${RUN_TS:?set RUN_TS=<unique-run-name> (e.g. \$(date +%Y%m%d-%H%M%S))}"
RUN_DIR="$INTEG_DIR/runs/$RUN_TS"
STATE_ENV="$RUN_DIR/state.env"
ACTIONS_TSV="$RUN_DIR/actions.tsv"
LOG_DIR="$RUN_DIR/logs"

# Governance timelock min delay, in ledgers (~5s/ledger on testnet). 1 keeps the
# propose -> await -> execute lifecycle a real but fast delay; min_delay==0 is
# rejected at the governance constructor (#39). Override for slower/faster runs.
INTEG_MIN_DELAY="${INTEG_MIN_DELAY:-1}"

# Amounts (7-decimal token units unless noted).
XLM_FUND_STROOPS=100000000000        # 10,000 XLM friendbot grant
WAD=1000000000000000000              # 1e18
RAY=1000000000000000000000000000     # 1e27

# --- Quality / guard constants (centralized for maintainability) ---

# Required external tools for the harness. Checked by preflight targets.
REQUIRED_TOOLS="jq xxd stellar curl base64 awk grep tr"

# Minimum stellar CLI version (major.minor). Update when new features or output
# formats are required. Checked in integration-preflight.
STELLAR_CLI_MIN_VERSION="22.0"

# Centralized magic constants previously scattered in flows/stress.sh, liq flows,
# and liq20_width.sh. Override via env if needed for experiments.
STRESS_N=20
STRESS_UNIT=10000000                 # 1.0 token at 7 decimals

LIQ_CODES=(LIQA LIQB LIQC LIQD LIQE LIQF LIQG)
LIQ_UNIT=10000000

LIQ20_TX_CAP="${LIQ20_TX_CAP:-400000000}"
LIQ20_DEFAULT_REPAY_EACH="${LIQ20_DEFAULT_REPAY_EACH:-$((3000 * STRESS_UNIT))}"
LIQ20_DEFAULT_LEEWAY="${LIQ20_DEFAULT_LEEWAY:-8000000}"

# DFX (DeFindex) dedicated market unit for the defindex lane.
DFX_UNIT=10000000

# --- End quality constants ---
