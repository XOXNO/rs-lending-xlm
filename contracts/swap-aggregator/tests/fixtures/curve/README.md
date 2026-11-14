# Curve pool fixtures

These files are Curve StableSwap pool executables (`stableswap-pool` 0.1.0,
soroban-sdk 26.1.1) fetched from Stellar testnet contracts. Their upstream
source repository and commit are not recorded. They test the router's Curve
venue, the price aggregator's `CurveStableLp` provider, and Curve LP
collateral in the harness.

| File | Source contract | SHA-256 |
|---|---|---|
| `pool-2coin.wasm` | CD65V5XD6B3AVNALJAAXNR4ZCIIKISYO4GKOIAT6ZXBQFPEI26WDVV3Z | `af5c110c313f1bb9d7605415ff53ad44c6197eabe8d8c8204972c1fb12cb895d` |
| `pool-3coin.wasm` | CCAFTS7MA6ZXKNSGZWVPK7WNXSZULY2UEHQNHS7TJ3CFHWNH3HMTFI6K | `d04f845f374548e30cb72c921d8e50d90f96165722d4816ee51f11eebb2e21d1` |

To refetch a file, run
`stellar contract fetch --id <source contract> --network testnet -o <file>`.

Tests build fresh local pool storage and liquidity. They do not reproduce
deployed balances. The default-budget resource checks are in
[`curve_collateral.rs`](../../../../../tests/test-harness/tests/curve_collateral.rs).

## Allowlists

Only the testnet allowlist, `configs/testnet/curve_reviewed_pools.json`, names
these executables. The mainnet allowlist is empty.

## Mainnet replay

The mainnet replay (`tests/integration/production_config.py`) deploys one of
these files for each `CurveStableLp` source whose `wasm_hash` matches it. It
uses the admin account as the factory, then seeds one balanced deposit. A
source with any other hash fails `plan` until its reviewed executable is added
here. `plan` also refuses, before any deploy, a hash missing from the source
network's `curve_reviewed_pools.json`.

## Factory

Both executables expose the same pool ABI. [`factory.rs`](factory.rs) is shared
by the router, price-aggregator and harness tests. It imports the two-coin spec
once and deploys the executable that matches the coin count through a small
native factory.

Router setup and swaps mock only exact account authorization trees. Contract
transfer authorization comes from the production router. The tests write
nothing to a chain.
