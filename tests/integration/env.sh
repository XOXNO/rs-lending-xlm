# Network constants and per-run environment for the live-testnet harness.
# Sourced by every scenario; safe to source multiple times.

INTEG_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$INTEG_DIR/../.." && pwd)"

NETWORK="${NETWORK:-testnet}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
EXPLORER_TX="${EXPLORER_TX:-https://stellar.expert/explorer/testnet/tx}"
AGGREGATOR_API="${AGGREGATOR_API:-https://testnet-stellar-swap.xoxno.com/api/v1}"

# Pre-existing testnet infrastructure (from configs/networks.json).
AGGREGATOR="${AGGREGATOR:-CBAS56L66AUKYZB6KIXX6LYXM76OQXFBJ4A7ORITBIRUO5ZV7SOYRAK7}"
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
